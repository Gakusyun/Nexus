//! Owns the bundled `aria2c.exe` process and speaks JSON-RPC to it.
//!
//! Nexus never talks to the network itself: every transfer is executed by aria2. We
//! start a *private* instance bound to loopback with a per-launch secret, so it cannot
//! collide with a user's own aria2 setup, and control it over HTTP JSON-RPC.
//!
//! All three RPC reads are batched into a single `system.multicall` request, which keeps
//! the polling loop to exactly one round trip per tick.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

/// Fields we ask aria2 to return. Narrowing this keeps a 500 ms poll cheap even when a
/// torrent has thousands of files.
const KEYS: &[&str] = &[
    "gid",
    "status",
    "totalLength",
    "completedLength",
    "downloadSpeed",
    "uploadSpeed",
    "connections",
    "dir",
    "files",
    "errorMessage",
    "bittorrent",
];

/// How many paused and queued downloads we keep pulling into the list.
const WAITING_WINDOW: u32 = 100;

/// How many finished downloads we keep pulling into the list.
const STOPPED_WINDOW: u32 = 100;

/// One poll's worth of engine truth.
pub struct Snapshot {
    pub tasks: Vec<Value>,
}

/// The built-in user agent, used when the user has not set one.
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Nexus/0.1";

/// Engine preferences distilled from the user's settings.
///
/// Handed to [`Aria2::start`] (the process arguments) and to [`Aria2::apply_global`] (the handful
/// of options aria2 lets us change while it runs). The per-download half of these lives on
/// [`DownloadRequest`], so a settings change reaches the next download without a restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineOptions {
    /// `None` keeps aria2's `/ Nexus's` built-in agent.
    pub user_agent: Option<String>,
    /// Segments per download, and connections per server. aria2 caps the latter at 16.
    pub connections: u32,
    pub max_concurrent: u32,
    /// Bytes per second for the whole engine; `0` is unlimited.
    pub speed_limit: u64,
    pub proxy: Option<String>,
    /// `0` means unlimited, as aria2 spells it.
    pub max_tries: u32,
    /// Seconds, applied to both `timeout` and `connect-timeout`.
    pub timeout: u32,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            user_agent: None,
            connections: 16,
            max_concurrent: 5,
            speed_limit: 0,
            proxy: None,
            max_tries: 5,
            timeout: 60,
        }
    }
}

/// Everything needed to queue one download.
///
/// The quick bar builds it from the global engine settings; the add-download dialog lets the user
/// override the per-download fields for a single task.
#[derive(Clone, Debug)]
pub struct DownloadRequest {
    pub uri: String,
    pub dir: String,
    /// Written as aria2's `out`; `None` lets aria2 name the file from the URI.
    pub file_name: Option<String>,
    pub user_agent: Option<String>,
    pub connections: u32,
    pub proxy: Option<String>,
    pub max_tries: u32,
    pub timeout: u32,
    /// This download's `max-download-limit`; `0` is unlimited.
    pub speed_limit: u64,
    pub referer: Option<String>,
}

pub struct Aria2 {
    endpoint: String,
    secret: String,
    agent: ureq::Agent,
    child: Mutex<Option<Child>>,
}

impl Aria2 {
    /// Spawn aria2 and block until its RPC port answers. Call this from a background
    /// executor — it deliberately waits for readiness so callers never see a
    /// half-started engine.
    pub fn start(download_dir: &Path, options: &EngineOptions) -> Result<Aria2> {
        let binary = locate_binary().context(
            "aria2c.exe not found — expected it next to nexus.exe or in the `resources` folder",
        )?;
        std::fs::create_dir_all(download_dir).ok();

        let port = free_port()?;
        let secret = random_secret();

        let child = Command::new(&binary)
            .args(spawn_args(port, &secret, download_dir, options))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags_quiet()
            .spawn()
            .with_context(|| format!("could not launch {}", binary.display()))?;

        let engine = Aria2 {
            endpoint: format!("http://127.0.0.1:{port}/jsonrpc"),
            secret,
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(10)))
                    // aria2 reports JSON-RPC failures (bad gid, unknown method...) with
                    // HTTP 400 and a JSON error body. Without this, ureq turns those
                    // into an opaque "http status: 400" and the real message is lost.
                    .http_status_as_error(false)
                    .build(),
            ),
            child: Mutex::new(Some(child)),
        };

        engine.wait_until_ready()?;
        Ok(engine)
    }

    fn wait_until_ready(&self) -> Result<()> {
        let mut last = None;
        // aria2 usually binds the port within ~150 ms; 8 s of patience covers a cold
        // start on a loaded machine.
        for _ in 0..80 {
            match self.call("aria2.getVersion", vec![]) {
                Ok(_) => return Ok(()),
                Err(err) => {
                    last = Some(err.to_string());
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        self.kill();
        Err(anyhow!(
            "aria2 did not start listening ({})",
            last.unwrap_or_else(|| "unknown error".into())
        ))
    }

    /// Single JSON-RPC call, with our token injected as the first parameter.
    pub fn call(&self, method: &str, params: Vec<Value>) -> Result<Value> {
        let mut all = Vec::with_capacity(params.len() + 1);
        all.push(self.token());
        all.extend(params);
        self.request(method, all)
    }

    fn token(&self) -> Value {
        Value::String(format!("token:{}", self.secret))
    }

    /// Raw JSON-RPC round trip. Used directly by `system.multicall`, which is the one
    /// method where the token does *not* go at the front (see `snapshot`).
    fn request(&self, method: &str, params: Vec<Value>) -> Result<Value> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": "nexus",
            "method": method,
            "params": params,
        })
        .to_string();

        let mut response = self
            .agent
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .send(body)
            .with_context(|| format!("{method}: request failed"))?;

        let text = response
            .body_mut()
            .read_to_string()
            .with_context(|| format!("{method}: could not read response"))?;
        let parsed: Value = serde_json::from_str(&text)
            .with_context(|| format!("{method}: invalid JSON response"))?;

        if let Some(error) = parsed.get("error")
            && !error.is_null()
        {
            let message = error["message"].as_str().unwrap_or("unknown error");
            let detail = error["data"].as_str().unwrap_or("");
            if detail.is_empty() {
                bail!("{message}");
            }
            bail!("{message} ({detail})");
        }

        Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Everything the UI needs about the queue in one round trip.
    pub fn snapshot(&self) -> Result<Snapshot> {
        // `system.multicall` takes a single parameter — the list of calls — so there is
        // no room for a top-level token. Each sub-call carries its own instead; putting
        // the token at the top level makes aria2 answer HTTP 400 "parameter at 0 has
        // wrong type".
        let token = self.token();
        let sub_call = |method: &str, extra: Vec<Value>| {
            let mut params = Vec::with_capacity(extra.len() + 1);
            params.push(token.clone());
            params.extend(extra);
            json!({ "methodName": method, "params": params })
        };

        // No `getGlobalStat`: it was fetched for its `numActive`/`numStopped` counters and its
        // `downloadSpeed`, and all three turned out to be either unused or untrustworthy — the
        // screenshot-worthy one is that `downloadSpeed` keeps paying out the last speed of paused
        // request groups. The views derive what they show from the rows instead, so this is one
        // fewer RPC per poll.
        let calls = json!([
            sub_call("aria2.tellActive", vec![json!(KEYS)]),
            sub_call(
                "aria2.tellWaiting",
                vec![json!(0), json!(WAITING_WINDOW), json!(KEYS)]
            ),
            sub_call(
                "aria2.tellStopped",
                vec![json!(0), json!(STOPPED_WINDOW), json!(KEYS)]
            ),
        ]);

        let result = self.request("system.multicall", vec![calls])?;
        let slots = result
            .as_array()
            .context("system.multicall returned an unexpected payload")?;

        let mut tasks = Vec::new();
        for index in 0..=2 {
            let entries = slot(slots, index);
            if let Some(entries) = entries.as_array() {
                tasks.extend(entries.iter().cloned());
            }
        }

        Ok(Snapshot { tasks })
    }

    /// Queue a URI and return the gid aria2 assigned to it. The request carries every per-download
    /// option, so the dialog can override the engine settings for one task without a restart.
    pub fn add_uri(&self, request: &DownloadRequest) -> Result<String> {
        let result = self.call(
            "aria2.addUri",
            vec![json!([request.uri]), download_options(request)],
        )?;
        result
            .as_str()
            .map(str::to_string)
            .context("aria2.addUri did not return a gid")
    }

    /// Apply the settings that live on the engine rather than on a single download, so changing
    /// them does not need a restart.
    pub fn apply_global(&self, options: &EngineOptions) -> Result<()> {
        self.call("aria2.changeGlobalOption", vec![global_options(options)])
            .map(drop)
    }

    pub fn pause(&self, gid: &str) -> Result<()> {
        self.call("aria2.pause", vec![json!(gid)]).map(drop)
    }

    pub fn unpause(&self, gid: &str) -> Result<()> {
        self.call("aria2.unpause", vec![json!(gid)]).map(drop)
    }

    pub fn remove(&self, gid: &str) -> Result<()> {
        self.call("aria2.remove", vec![json!(gid)]).map(drop)
    }

    /// Drop a finished/failed entry out of aria2's result list too.
    pub fn forget(&self, gid: &str) -> Result<()> {
        self.call("aria2.removeDownloadResult", vec![json!(gid)])
            .map(drop)
    }

    /// Best-effort orderly shutdown, escalating to a kill if aria2 dawdles.
    pub fn shutdown(&self) {
        let _ = self.call("aria2.shutdown", vec![]);
        let Some(mut child) = self.child.lock().ok().and_then(|mut c| c.take()) else {
            return;
        };
        for _ in 0..12 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    fn kill(&self) {
        if let Some(mut child) = self.child.lock().ok().and_then(|mut c| c.take()) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Aria2 {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// `system.multicall` wraps each result in a one-element array, or swaps in a fault
/// object under `"fault"` when one sub-call fails.
fn slot(slots: &[Value], index: usize) -> Value {
    slots
        .get(index)
        .and_then(|slot| slot.get(0))
        .cloned()
        .unwrap_or(Value::Null)
}

/// A loopback port that was free a moment ago. The tiny race against another process is
/// harmless here: aria2 fails loudly and the UI reports it.
fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("no free loopback port")?;
    Ok(listener.local_addr()?.port())
}

/// Not a security boundary — aria2 is bound to loopback and lives only as long as the
/// app — just enough to keep other local software from poking our queue.
fn random_secret() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}{:x}", std::process::id())
}

fn spawn_args(port: u16, secret: &str, dir: &Path, options: &EngineOptions) -> Vec<String> {
    let user_agent = options
        .user_agent
        .clone()
        .unwrap_or_else(|| DEFAULT_USER_AGENT.to_string());

    let mut args: Vec<String> = vec![
        // Ignore any aria2.conf the user may have, so behaviour is reproducible.
        "--no-conf=true".to_string(),
        "--enable-rpc=true".to_string(),
        "--rpc-listen-all=false".to_string(),
        format!("--rpc-listen-port={port}"),
        format!("--rpc-secret={secret}"),
        format!("--dir={}", dir.display()),
        // Die with the app instead of being orphaned into the background.
        format!("--stop-with-process={}", std::process::id()),
        "--file-allocation=none".to_string(),
        "--continue=true".to_string(),
        format!("--max-concurrent-downloads={}", options.max_concurrent),
        format!("--max-overall-download-limit={}", options.speed_limit),
        format!("--split={}", options.connections),
        format!("--max-connection-per-server={}", options.connections),
        "--min-split-size=1M".to_string(),
        format!("--max-tries={}", options.max_tries),
        "--retry-wait=3".to_string(),
        format!("--timeout={}", options.timeout),
        format!("--connect-timeout={}", options.timeout),
        "--disk-cache=64M".to_string(),
        "--seed-time=0".to_string(),
        "--bt-save-metadata=true".to_string(),
        "--follow-torrent=mem".to_string(),
        // Quiet: the UI is the interface, and we do not want a stray console window.
        "--quiet=true".to_string(),
        "--summary-interval=0".to_string(),
        "--console-log-level=error".to_string(),
        format!("--user-agent={user_agent}"),
    ];
    if let Some(proxy) = &options.proxy {
        args.push(format!("--all-proxy={proxy}"));
    }
    args
}

/// Options sent with every `aria2.addUri`. Unset optionals are omitted rather than sent empty,
/// so an unset user agent still falls back to the engine's own.
fn download_options(request: &DownloadRequest) -> Value {
    let mut download = json!({
        "continue": "true",
        "dir": request.dir,
        "split": request.connections.to_string(),
        "max-connection-per-server": request.connections.to_string(),
        "max-tries": request.max_tries.to_string(),
        "retry-wait": "3",
        "timeout": request.timeout.to_string(),
        "connect-timeout": request.timeout.to_string(),
    });
    if let Some(file_name) = &request.file_name {
        download["out"] = json!(file_name);
    }
    if let Some(user_agent) = &request.user_agent {
        download["user-agent"] = json!(user_agent);
    }
    if let Some(proxy) = &request.proxy {
        download["all-proxy"] = json!(proxy);
    }
    if let Some(referer) = &request.referer {
        download["referer"] = json!(referer);
    }
    if request.speed_limit > 0 {
        download["max-download-limit"] = json!(request.speed_limit.to_string());
    }
    download
}

/// The subset of options aria2 accepts from `aria2.changeGlobalOption`.
fn global_options(options: &EngineOptions) -> Value {
    json!({
        "max-concurrent-downloads": options.max_concurrent.to_string(),
        "max-overall-download-limit": options.speed_limit.to_string(),
    })
}

/// Look for aria2c next to our own executable first (the shipped layout), then in the
/// developer `resources/` folder, then anywhere on PATH.
fn locate_binary() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("aria2c.exe"));
        // target/debug/nexus.exe -> <project>/resources/aria2c.exe
        candidates.push(dir.join("../../resources/aria2c.exe"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/aria2c.exe"));

    if let Some(found) = candidates.into_iter().find(|path| path.is_file()) {
        return Some(found);
    }

    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join("aria2c.exe"))
        .find(|path| path.is_file())
}

/// Keep the child process windowless on Windows.
trait QuietSpawn {
    fn creation_flags_quiet(&mut self) -> &mut Self;
}

impl QuietSpawn for Command {
    fn creation_flags_quiet(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Serve one fixed payload over loopback HTTP, so the test never touches the network.
    fn serve_once(payload: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 2048];
                let _ = stream.read(&mut request);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(payload);
                let _ = stream.flush();
            }
        });
        format!("http://{address}/hello.bin")
    }

    /// The whole engine contract in one go: spawn, queue, observe, forget, shut down.
    #[test]
    fn downloads_a_file_end_to_end() {
        const PAYLOAD: &[u8] = b"nexus end-to-end payload";
        let uri = serve_once(PAYLOAD);

        let dir = std::env::temp_dir().join(format!("nexus-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let engine = Aria2::start(&dir, &EngineOptions::default())
            .expect("aria2 should start and answer getVersion");
        let gid = engine
            .add_uri(&DownloadRequest {
                uri: uri.clone(),
                dir: dir.to_string_lossy().into_owned(),
                file_name: None,
                user_agent: None,
                connections: 16,
                proxy: None,
                max_tries: 5,
                timeout: 60,
                speed_limit: 0,
                referer: None,
            })
            .expect("addUri should return a gid");

        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut finished = None;
        while std::time::Instant::now() < deadline {
            let snapshot = engine.snapshot().expect("system.multicall should succeed");
            if let Some(entry) = snapshot
                .tasks
                .iter()
                .find(|task| task["gid"] == json!(gid) && task["status"] == json!("complete"))
            {
                finished = Some(entry.clone());
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }

        let entry = finished.expect("the download should complete within 30s");
        assert_eq!(entry["totalLength"], json!(PAYLOAD.len().to_string()));
        assert_eq!(
            std::fs::read(dir.join("hello.bin")).expect("downloaded file"),
            PAYLOAD
        );

        // Removing a finished task must also clear it from `tellStopped`.
        engine.forget(&gid).expect("removeDownloadResult");
        let after = engine.snapshot().expect("snapshot after forget");
        assert!(after.tasks.iter().all(|task| task["gid"] != json!(gid)));

        engine.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn options_are_translated_into_aria2_arguments() {
        let engine = EngineOptions {
            user_agent: Some("Nexus-Test/9".into()),
            connections: 4,
            max_concurrent: 3,
            speed_limit: 2_097_152,
            proxy: Some("http://127.0.0.1:7890".into()),
            max_tries: 7,
            timeout: 45,
        };

        let args = spawn_args(6800, "sekret", Path::new("C:\\dl"), &engine);
        assert!(args.contains(&"--split=4".to_string()));
        assert!(args.contains(&"--max-connection-per-server=4".to_string()));
        assert!(args.contains(&"--max-concurrent-downloads=3".to_string()));
        assert!(args.contains(&"--max-overall-download-limit=2097152".to_string()));
        assert!(args.contains(&"--max-tries=7".to_string()));
        assert!(args.contains(&"--timeout=45".to_string()));
        assert!(args.contains(&"--user-agent=Nexus-Test/9".to_string()));
        assert!(args.contains(&"--all-proxy=http://127.0.0.1:7890".to_string()));

        let request = DownloadRequest {
            uri: "https://example.com/a.bin".into(),
            dir: "C:\\dl".into(),
            file_name: Some("renamed.bin".into()),
            user_agent: engine.user_agent.clone(),
            connections: engine.connections,
            proxy: engine.proxy.clone(),
            max_tries: engine.max_tries,
            timeout: engine.timeout,
            speed_limit: 1_048_576,
            referer: Some("https://example.com/".into()),
        };
        let download = download_options(&request);
        assert_eq!(download["dir"], json!("C:\\dl"));
        assert_eq!(download["split"], json!("4"));
        assert_eq!(download["max-connection-per-server"], json!("4"));
        assert_eq!(download["out"], json!("renamed.bin"));
        assert_eq!(download["user-agent"], json!("Nexus-Test/9"));
        assert_eq!(download["all-proxy"], json!("http://127.0.0.1:7890"));
        assert_eq!(download["referer"], json!("https://example.com/"));
        assert_eq!(download["max-download-limit"], json!("1048576"));

        // Unset optionals are omitted, not sent as empty strings, so the engine keeps its own
        // defaults instead of being handed a blank user agent or a zero speed limit.
        let bare = download_options(&DownloadRequest {
            file_name: None,
            user_agent: None,
            proxy: None,
            referer: None,
            speed_limit: 0,
            ..request.clone()
        });
        assert!(bare.get("user-agent").is_none());
        assert!(bare.get("all-proxy").is_none());
        assert!(bare.get("out").is_none());
        assert!(bare.get("referer").is_none());
        assert!(bare.get("max-download-limit").is_none());
        assert_eq!(
            global_options(&engine)["max-concurrent-downloads"],
            json!("3")
        );
    }

    #[test]
    fn reports_aria2_errors_with_their_message() {
        let dir = std::env::temp_dir().join(format!("nexus-err-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let engine = Aria2::start(&dir, &EngineOptions::default()).expect("aria2 should start");

        // aria2 answers HTTP 400 here; the message must survive the round trip.
        let error = engine
            .pause("deadbeef")
            .expect_err("an unknown gid must fail")
            .to_string();
        assert!(error.contains("not found"), "unexpected error: {error}");

        engine.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn private_engine_rejects_a_missing_token() {
        let dir = std::env::temp_dir().join(format!("nexus-auth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let engine = Aria2::start(&dir, &EngineOptions::default()).expect("aria2 should start");

        // Same request without our token: the loopback guard plus the secret are what
        // keep other local software out of our queue.
        assert!(engine.request("aria2.getGlobalStat", vec![]).is_err());
        assert!(engine.call("aria2.getGlobalStat", vec![]).is_ok());

        engine.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
