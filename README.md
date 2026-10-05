# Nexus

A small, focused download manager for Windows. The UI is [GPUI-CE](https://github.com/gpui-ce/gpui-ce)
(Zed's GPU-accelerated Rust UI framework); every byte on the wire is moved by aria2 over
JSON-RPC — by default a private `aria2c.exe` that Nexus starts itself and shuts down with the
app, or an instance already running elsewhere that Nexus only talks to.

No Electron, no bundled browser, no daemon to install — one native executable.

## Status

Downloading works end to end, and everything is recorded.

* Paste a link (or a `magnet:` URI) and press <kbd>Enter</kbd>
* Or open the advanced dialog (the sliders button beside Add) to give one download its own name,
  folder, user agent, connection count, proxy, referer and speed limit
* Live progress, speed, ETA and connection count, refreshed twice a second
* Pause / resume / retry, open the file in Explorer, remove a task
* Filter by All / Active / Finished, clear finished in one click
* Removing a download asks what should happen to the file — cancel, keep it, or delete it
* Settings: download folder, theme (light / dark / follow the system), UI font, language
  (English / 简体中文) — in a card over the download list, not a separate page
* Engine settings: user agent, connections per download, simultaneous downloads, overall
  speed limit, proxy, retries and timeout — applied without restarting aria2
* Engine choice: Nexus's own aria2c (random port and secret each launch, or a fixed pair so
  outside clients can follow along), or an aria2 that is already running — Nexus then only
  talks to it, and never starts or stops it
* Download history and an event log in SQLite (`nexus.db`)
* History persists across restarts; unfinished downloads re-queue with `--continue`, so a
  partial file resumes instead of restarting
* Failures surface aria2's own error message instead of a generic "failed"

Deliberately not here yet: a browser for the event log, torrent/BT options, per-download overrides
of the engine settings, multi-select, drag-and-drop of `.torrent` files, auto-update.

Nexus is a GUI binary — it does not open a console window. Failures go to `nexus.log` in the
directory it was started from instead, and unhandled errors are shown in the app.

## Running

First put an `aria2c.exe` in `resources/` — see [The engine](#the-engine-get-aria2-yourself).
Then:

```sh
cargo run
```

`cargo test` runs the suite; it spins up a real aria2 instance against a loopback HTTP
server, so the engine contract is covered without touching the network.

## Layout

```
src/
  main.rs        window setup, bootstrap, panic log
  aria2.rs       owns the aria2c child process + JSON-RPC client
  state.rs       NexusApp: task list, filter, engine polls, write-through
  model.rs       Task/Status plus the formatting and name-derivation rules
  store.rs       SQLite: schema, history, event log, preferences
  settings.rs    the persisted preferences (theme, language, font, folder)
  i18n.rs        every user-visible string, one struct per language
  theme.rs       the palettes, installed as a GPUI global
  assets.rs      icons compiled into the binary
  text_edit.rs   the single-line editing buffer (caret, selection, IME, UTF-16 bridges)
  ui/
    mod.rs         widgets: IconButton, TextButton, segmented, Chip, ProgressBar, StatusBadge, icon()
    text_field.rs  the one text-field builder every box in the app is assembled from
    root.rs        title bar, command bar, filters, list, add-download dialog, confirm dialog
    settings.rs    the settings card (a modal overlay, not a page)
    task_row.rs    one download row
assets/icons/  monochrome SVGs (GPUI tints them from the colour passed to `icon()`)
resources/     put your own aria2c.exe here — see below
STYLE.md       the shared visual language: colours, type scale, spacing, widgets
```

Design notes worth knowing before editing:

* **One source of truth for state.** Every mutation goes through a method on `NexusApp`
  so persistence and `cx.notify()` stay in one place. The UI never mutates the engine
  directly — it calls a method that dispatches the RPC off-thread and folds the result
  back in.
* **aria2 is the model, polls are the sync.** Each 500 ms tick makes a single
  `system.multicall` request that returns global stats plus active/waiting/stopped
  entries, and reconciles them into the list by gid. No websockets, no push, one round
  trip per tick.
* **The database is written through, never read on a frame.** The in-memory `Vec<Task>` is
  what the UI renders; SQLite is updated from it and read only at startup.

## Where it keeps things

Everything lives in the directory Nexus was started from — portable, nothing hidden in the
profile. (Under `cargo run` that is the project root, which is why the runtime files are
gitignored.)

| File | What it is |
| --- | --- |
| `nexus.db` | the SQLite database: history, event log and preferences |
| `nexus.log` | panics, since a GUI binary has no stderr to show them on |
| `state.json.imported` | a one-time backup of the pre-SQLite settings file, if there was one |

`nexus.db` is ordinary SQLite (WAL mode), so any client can read it:

```sql
-- what happened, most recent first
SELECT datetime(at, 'unixepoch', 'localtime') AS when, kind, name, detail
FROM events ORDER BY id DESC LIMIT 20;

-- biggest things ever downloaded
SELECT name, total, datetime(finished_at, 'unixepoch', 'localtime')
FROM downloads WHERE status = 'complete' ORDER BY total DESC LIMIT 10;
```

Three tables:

* **`downloads`** — one row per download: the URI list (JSON), name, folder, status, byte
  counters, connection count, error text, and `created_at`/`updated_at`/`finished_at`. Rows are
  updated in place; removed downloads are deleted, so the table is the current queue plus
  history, not an audit trail.
* **`events`** — append-only: `added`, `queued`, `started`, `resumed`, `paused`, `completed`,
  `failed`, `removed`, `files_deleted`, `engine_ready`, `engine_failed`. This is where the audit
  trail lives, including downloads that were later removed and files that were deleted from disk.
* **`settings`** — key/value; one row per preference (`theme`, `language`, `font`,
  `download_dir`, `user_agent`, `connections`, `max_concurrent`, `speed_limit`, `proxy`,
  `max_tries`, `timeout`). Rows written by older builds as a single `ui` JSON blob are folded into
  these keys once, on startup.

Writing is throttled on purpose: a status change is recorded the moment it happens, but byte
counters only land every three seconds, because they move twice a second and there is no reason
to make the disk watch that. If the database cannot be opened the app still runs — it just
forgets between sessions, and the settings page says so instead of the window refusing to open.

## Engine notes (learned the hard way)

These are aria2 quirks, not GPUI ones — they cost real debugging time, so they are written
down:

* **`system.multicall` does not take a token at the top level.** With `--rpc-secret`, the
  secret goes in the first position of `params` for normal methods, but `multicall` takes a
  single parameter (the call list) and so **each sub-call must carry its own token**. Putting
  it at the top level makes aria2 answer `HTTP 400 "The parameter at 0 has wrong type."`
* **JSON-RPC failures come back as HTTP 400, not 200.** So an HTTP client configured to treat
  4xx as an error swallows the useful message (`GID ... is not found`, and so on). Nexus sets
  `http_status_as_error(false)` and reads the error out of the JSON body.
* **`--no-conf=true` matters.** Without it aria2 reads the user's own `aria2.conf`, which
  silently changes behaviour. (Motrix, for instance, launches its aria2 with `--conf-path`.)
* **Nexus only stops what it started.** The built-in engine gets `aria2.shutdown` and, if it
  dawdles, a kill; an engine it merely attached to gets neither — `Aria2::owned` is the switch
  `shutdown()` (and therefore `Drop`) reads. Otherwise closing Nexus would take Motrix's aria2,
  and every download on it, with it.
* **A borrowed engine still receives `changeGlobalOption`.** Parallel downloads and the overall
  limit are engine-wide in aria2, so Nexus's settings reach whatever it is talking to; everything
  else rides on `addUri` and stays per-download.
* **Rows are reattached by gid first, then by URI.** An engine Nexus did not start may have been
  downloading since the app closed, so at connect time `NexusApp::adopt` claims entries whose gid
  the row remembers, then entries quoting one of the row's own URIs — a row with no match is
  simply not running any more, and Resume queues it again.
* **The bundled Windows build does not need a CA file** — HTTPS works out of the box, which
  is worth re-checking if the aria2 version is ever bumped.
* **`--stop-with-process=<pid>`** keeps a stray `aria2c.exe` from surviving a crash of the GUI.
  `CREATE_NO_WINDOW` is used at spawn time so no console window flashes.
* **`aria2.getGlobalStat`'s `downloadSpeed` is not safe to display.** It sums the *last measured*
  speed of every request group aria2 still holds, and a paused group is never updated again —
  so a queue with nothing running keeps reporting its final speed. Measured on aria2 1.37 with
  `--max-download-limit=2M`: right after `aria2.pauseAll`, `numActive` was `0` and `tellActive`
  was empty, yet `downloadSpeed` still read ~1 MB/s and decayed over *minutes*
  (`tellWaiting` carried that same stale number on the paused entry). Nexus therefore does not
  call `getGlobalStat` at all: the views derive what they show from the rows, where
  `Task::absorb` has already zeroed the speed of everything that is not `active`, and the total is
  summed on demand (`model::total_speed`) rather than cached. It used to be cached, and the cache
  was only refreshed by a *successful poll* — so pausing a download left the old figure on screen
  until the next tick landed.
* Nexus uses a **free random port and a per-launch secret**, and binds to loopback only, so it
  cannot collide with an existing aria2 (Motrix runs one on port 16800).

## The engine: get aria2 yourself

**aria2 is a separate project and is not part of Nexus.** Nothing of it is committed here
(`resources/` is gitignored), so a fresh clone has no download engine until you supply one.
Nexus will tell you so — it starts with a banner reading "The download engine is unavailable"
rather than just failing silently.

1. Download a Windows build from the [aria2 releases page](https://github.com/aria2/aria2/releases)
   (this project is developed against `release-1.37.0`, file `aria2-1.37.0-win-64bit-build1.zip`).
2. Extract `aria2c.exe` from the archive. **It sits inside a versioned folder**, so it has to be
   pulled out rather than unzipped in place.
3. Copy it into this project's `resources/` folder, keeping the name exactly `aria2c.exe`:

   ```
   Nexus/
     resources/
       aria2c.exe      <- must be this exact name
   ```

That is all — `cargo run` will find it. The name matters because it is what the loader looks
for, and the loader searches in this order:

1. `aria2c.exe` next to `nexus.exe` — the layout to use when shipping a build
2. `<project>/resources/aria2c.exe` — the development layout above
3. `aria2c.exe` anywhere on `PATH`, so an existing install works too
   (`winget install aria2.aria2`)

Bumping the version is a drop-in replacement of that one file. Re-check the HTTPS note above
when you do, since certificate handling is the one thing that varies between aria2 builds.

## License

Nexus itself is MIT. `aria2c.exe` is a separate project licensed GPLv2-or-later, and that stays
true of any copy you place in `resources/` or ship next to `nexus.exe`.
