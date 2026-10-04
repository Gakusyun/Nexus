//! The download model: aria2's wire format normalised into something the UI can render
//! and that can be persisted to disk between sessions.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::i18n::Strings;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Active,
    Waiting,
    Paused,
    Complete,
    Error,
}

impl Status {
    pub fn parse(raw: &str) -> Option<Status> {
        match raw {
            "active" => Some(Status::Active),
            "waiting" => Some(Status::Waiting),
            "paused" => Some(Status::Paused),
            "complete" => Some(Status::Complete),
            "error" => Some(Status::Error),
            // "removed" entries are terminal noise we never surface.
            _ => None,
        }
    }

    pub fn label(self, strings: &Strings) -> &'static str {
        match self {
            Status::Active => strings.status_active,
            Status::Waiting => strings.status_waiting,
            Status::Paused => strings.status_paused,
            Status::Complete => strings.status_complete,
            Status::Error => strings.status_error,
        }
    }

    /// Finished for good — the row offers "open" and "remove" only.
    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Complete | Status::Error)
    }

    /// Still owned by the engine, so it can be paused or resumed.
    pub fn is_live(self) -> bool {
        !self.is_terminal()
    }

    /// Sort weight inside the "still running" group.
    pub fn rank(self) -> u8 {
        match self {
            Status::Active => 0,
            Status::Waiting => 1,
            Status::Paused => 2,
            Status::Complete => 3,
            Status::Error => 4,
        }
    }
}

/// One download. `seq` is our own stable identity (aria2's gid can change when a
/// download is re-added after a restart, and is absent before the RPC returns).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    /// `None` while the engine has not acknowledged the task yet, or after a restart.
    #[serde(default, skip_serializing)]
    pub gid: Option<String>,
    pub seq: u64,
    pub uris: Vec<String>,
    pub name: String,
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub completed: u64,
    #[serde(default)]
    pub speed: u64,
    pub status: Status,
    #[serde(default)]
    pub connections: u64,
    #[serde(default)]
    pub error: Option<String>,
    /// When the row was first added, as unix seconds. Kept so the history has a time axis.
    #[serde(default)]
    pub created_at: i64,
}

impl Task {
    /// A provisional row, shown the instant the user hits Enter.
    pub fn placeholder(seq: u64, uri: String, dir: String) -> Self {
        Self {
            gid: None,
            seq,
            name: display_name_from_uri(&uri),
            uris: vec![uri],
            dir,
            total: 0,
            completed: 0,
            speed: 0,
            status: Status::Waiting,
            connections: 0,
            error: None,
            created_at: crate::store::now_unix(),
        }
    }

    /// Copy engine truth into this row. Only fields the engine actually knows about    /// are touched, so a `None` gid or a stale name survives.
    pub fn absorb(&mut self, value: &serde_json::Value) {
        if let Some(status) = value["status"].as_str().and_then(Status::parse) {
            self.status = status;
        }
        if let Some(gid) = value["gid"].as_str() {
            self.gid = Some(gid.to_string());
        }
        if let Some(dir) = value["dir"].as_str()
            && !dir.is_empty()
        {
            self.dir = dir.to_string();
        }
        if let Some(name) = derive_name(value) {
            self.name = name;
        }
        self.total = num(value, "totalLength");
        self.completed = num(value, "completedLength");
        self.connections = num(value, "connections").min(u64::from(u32::MAX));

        // aria2 reports a live speed for active transfers only; stale values would
        // otherwise linger on a paused row.
        self.speed = if self.status == Status::Active {
            num(value, "downloadSpeed")
        } else {
            0
        };

        self.error = match self.status {
            Status::Error => value["errorMessage"]
                .as_str()
                .filter(|m| !m.is_empty())
                .map(str::to_string)
                .or_else(|| Some("Download failed".into())),
            _ => None,
        };
    }

    /// 0.0..=1.0, or `None` while the size is still unknown (metadata, magnet).
    pub fn progress(&self) -> Option<f32> {
        if self.total == 0 {
            return None;
        }
        Some((self.completed as f64 / self.total as f64).clamp(0.0, 1.0) as f32)
    }

    /// Seconds remaining, or `None` when it cannot be estimated.
    pub fn eta(&self) -> Option<u64> {
        if self.status != Status::Active || self.speed == 0 || self.total == 0 {
            return None;
        }
        let remaining = self.total.saturating_sub(self.completed);
        Some(remaining / self.speed)
    }

    pub fn path(&self) -> std::path::PathBuf {
        Path::new(&self.dir).join(&self.name)
    }

    /// The single URI we hand back to aria2 when resuming or retrying.
    pub fn primary_uri(&self) -> Option<&str> {
        self.uris.first().map(String::as_str)
    }
}

fn num(value: &serde_json::Value, key: &str) -> u64 {
    value[key]
        .as_str()
        .and_then(|raw| raw.parse().ok())
        .or_else(|| value[key].as_u64())
        .unwrap_or(0)
}

/// Best-effort human name: torrent name, then the resolved file path, then the URI.
fn derive_name(value: &serde_json::Value) -> Option<String> {
    if let Some(name) = value
        .pointer("/bittorrent/info/name")
        .and_then(|v| v.as_str())
        .filter(|n| !n.is_empty())
    {
        return Some(name.to_string());
    }

    let path = value.pointer("/files/0/path").and_then(|v| v.as_str())?;
    if path.is_empty() {
        return None;
    }
    let base = Path::new(path).file_name()?.to_string_lossy().into_owned();
    (!base.is_empty()).then_some(base)
}

/// Turn a URL into something readable before the engine has told us the real name.
pub fn display_name_from_uri(uri: &str) -> String {
    if let Some(rest) = uri.strip_prefix("magnet:?") {
        for pair in rest.split('&') {
            if let Some(name) = pair.strip_prefix("dn=") {
                return percent_decode(name);
            }
        }
        return "Magnet link".into();
    }

    let without_query = uri.split(['?', '#']).next().unwrap_or(uri);
    let trimmed = without_query.trim_end_matches('/');
    if let Some(base) = trimmed.rsplit('/').next()
        && !base.is_empty()
    {
        return percent_decode(base);
    }

    // Fall back to the host so a bare "https://example.com" still reads sensibly.
    trimmed
        .split("//")
        .nth(1)
        .unwrap_or(trimmed)
        .split('/')
        .next()
        .unwrap_or(uri)
        .to_string()
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Total bytes per second across the rows that are actually moving.
///
/// A free function over the list, not a field, on purpose: the aggregate used to be cached on the
/// app and refreshed only by a successful poll, so pausing a download left its last speed on
/// screen until the next tick landed.
///
/// The filter on `Active` is the other half of that. Everything else in the app already zeroes the
/// speed of a row that is not running (`Task::absorb`, `NexusApp::pause`), but the total is what
/// the user reads, so it does not take that on trust — a paused row carrying a leftover number
/// contributes nothing here regardless of who wrote it. And nothing consults the engine's own
/// total, which keeps paying out the last speed of paused request groups.
pub fn total_speed(tasks: &[Task]) -> u64 {
    tasks
        .iter()
        .filter(|task| task.status == Status::Active)
        .map(|task| task.speed)
        .sum()
}

/// "1.4 GB" — one decimal below 100, none above, so columns stay aligned.
pub fn fmt_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn fmt_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec == 0 {
        return "—".into();
    }
    format!("{}/s", fmt_size(bytes_per_sec))
}

pub fn fmt_eta(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else if seconds < 86_400 {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    } else {
        format!("{}d {}h", seconds / 86_400, (seconds % 86_400) / 3600)
    }
}

pub fn fmt_percent(progress: Option<f32>) -> String {
    match progress {
        Some(p) => format!("{:.0}%", p * 100.0),
        None => "—".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_a_readable_name_from_every_uri_shape() {
        assert_eq!(
            display_name_from_uri("https://example.com/dir/file.zip"),
            "file.zip"
        );
        assert_eq!(
            display_name_from_uri("https://example.com/dir/file.zip?token=abc#frag"),
            "file.zip"
        );
        assert_eq!(
            display_name_from_uri("https://example.com/a%20b%20c.iso"),
            "a b c.iso"
        );
        // A bare origin is still more useful than the raw URL.
        assert_eq!(display_name_from_uri("https://example.com/"), "example.com");
        // Magnet links carry the name in `dn`.
        assert_eq!(
            display_name_from_uri("magnet:?xt=urn:btih:abc&dn=My%20File.iso"),
            "My File.iso"
        );
        assert_eq!(
            display_name_from_uri("magnet:?xt=urn:btih:abc"),
            "Magnet link"
        );
    }

    #[test]
    fn formats_sizes_speeds_and_etas() {
        assert_eq!(fmt_size(0), "0 B");
        assert_eq!(fmt_size(999), "999 B");
        assert_eq!(fmt_size(1536), "1.5 KB");
        assert_eq!(fmt_size(150 * 1024 * 1024), "150 MB");
        assert_eq!(fmt_speed(0), "—");
        assert_eq!(fmt_speed(2 * 1024 * 1024), "2.0 MB/s");
        assert_eq!(fmt_eta(45), "45s");
        assert_eq!(fmt_eta(200), "3m 20s");
        assert_eq!(fmt_eta(7300), "2h 1m");
    }

    #[test]
    fn progress_and_eta_are_unknown_until_the_size_is() {
        let mut task = Task::placeholder(1, "https://example.com/a.bin".into(), r"C:\dl".into());
        assert_eq!(
            task.progress(),
            None,
            "size unknown while fetching metadata"
        );
        assert_eq!(task.eta(), None);

        task.status = Status::Active;
        task.total = 1_000;
        task.completed = 250;
        task.speed = 50;
        assert_eq!(task.progress(), Some(0.25));
        assert_eq!(task.eta(), Some(15));

        // A finished transfer must not report a speed or an ETA, even if aria2 left
        // stale numbers on the entry.
        let mut entry = serde_json::json!({
            "status": "complete",
            "downloadSpeed": "4096",
            "totalLength": "1000",
            "completedLength": "1000",
            "files": [{ "path": "/tmp/a.bin" }],
        });
        task.absorb(&entry);
        assert_eq!(task.speed, 0);
        assert_eq!(task.eta(), None);
        assert_eq!(task.name, "a.bin");

        entry["status"] = serde_json::json!("error");
        entry["errorMessage"] = serde_json::json!("something went wrong");
        task.absorb(&entry);
        assert_eq!(task.status, Status::Error);
        assert_eq!(task.error.as_deref(), Some("something went wrong"));
    }

    /// aria2 keeps an entry's `downloadSpeed` after the transfer is paused (the paused request
    /// group is never updated again, and `getGlobalStat` sums those stale values -- see
    /// `GlobalStat::speed`). A paused row must not carry that number, or the queue's total would
    /// be phantom bytes.
    #[test]
    fn a_paused_entry_does_not_keep_its_speed() {
        let mut task = Task::placeholder(1, "https://example.com/a.bin".into(), r"C:\dl".into());

        let mut entry = serde_json::json!({
            "status": "active",
            "downloadSpeed": "1048576",
            "totalLength": "10000000",
            "completedLength": "1000000",
        });
        task.absorb(&entry);
        assert_eq!(task.speed, 1_048_576);
        assert_eq!(task.eta(), Some(8));

        entry["status"] = serde_json::json!("paused");
        entry["downloadSpeed"] = serde_json::json!("1061738");
        task.absorb(&entry);
        assert_eq!(task.status, Status::Paused);
        assert_eq!(task.speed, 0);
        assert_eq!(task.eta(), None);
    }

    /// The header's total is summed from the rows, and this is the property that makes it honest:
    /// a row that is not moving contributes nothing, whatever number is sitting on it. That
    /// matters because the figure used to be *cached* on the app and refreshed only by a
    /// successful poll, so pausing left the previous speed on screen until the next tick landed.
    #[test]
    fn total_speed_counts_only_rows_that_are_moving() {
        let mut running = Task::placeholder(1, "https://example.com/a.bin".into(), r"C:\dl".into());
        running.absorb(&serde_json::json!({
            "status": "active",
            "downloadSpeed": "1048576",
        }));

        let mut paused = Task::placeholder(2, "https://example.com/b.bin".into(), r"C:\dl".into());
        paused.status = Status::Paused;
        // Deliberately left as if the engine had last reported it: `absorb` would have zeroed it,
        // but a stale cache is exactly the state this total has to be immune to.
        paused.speed = 5_242_880;

        assert_eq!(total_speed(&[running, paused]), 1_048_576);
    }

    #[test]
    fn display_order_puts_running_work_first() {
        assert!(Status::Active.rank() < Status::Waiting.rank());
        assert!(Status::Waiting.rank() < Status::Paused.rank());
        assert!(!Status::Active.is_terminal());
        assert!(Status::Complete.is_terminal());
        assert!(Status::Error.is_terminal());
        assert_eq!(Status::parse("removed"), None);
    }
}
