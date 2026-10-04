//! SQLite persistence: the download history and the append-only event log.
//!
//! Everything lives in one file, `nexus.db`, next to the app's other data. The in-memory
//! `Vec<Task>` stays the source of truth for the UI — this module is written *through* on
//! changes, and read only at startup, so no frame ever waits on a query.
//!
//! The connection is deliberately optional. If the database cannot be opened the app still
//! runs; it just forgets things between sessions, and says so on the settings page instead of
//! failing to start.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::model::{Status, Task};

/// Bumped when the schema changes in a way that needs migrating.
const SCHEMA_VERSION: i64 = 5;

/// The exact shape a timestamp column has to hold: `2026-10-03T11:52:19Z`.
///
/// Used as a `CHECK`, because a declared type in SQLite is only an affinity hint — the engine will
/// happily store an integer in a `TIMESTAMP` column, and that is precisely what happened: one
/// write path still passed epoch seconds, so the value showed up as a number under a column whose
/// whole point is reading as a time. Same spirit as the `Strings` struct in `i18n` — make the
/// mistake impossible instead of remembering not to make it.
const STAMP_PATTERN: &str =
    "[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z";

/// A `CHECK` for a timestamp column.
///
/// Spelled with an explicit `IS NULL OR` rather than leaning on SQLite's rule that a `CHECK` passes
/// when its expression is NULL: the nullable columns (`finished_at`, and `events` for a download
/// that is gone) are empty on purpose, and that should be readable in the schema instead of
/// inferred from a subtlety of the engine.
fn stamp_check(column: &str) -> String {
    format!("CHECK ({column} IS NULL OR {column} GLOB '{STAMP_PATTERN}')")
}

/// The `downloads` definition, shared by the fresh-file path and the rebuild.
///
/// One definition on purpose: a rebuilt file and a new one must not be able to end up with
/// different schemas, which is what happens the moment this text lives in two places.
fn create_downloads(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {table} (
             seq         INTEGER PRIMARY KEY,
             gid         TEXT,
             uris        TEXT      NOT NULL,
             name        TEXT      NOT NULL,
             dir         TEXT      NOT NULL,
             status      TEXT      NOT NULL,
             total       INTEGER   NOT NULL DEFAULT 0,
             completed   INTEGER   NOT NULL DEFAULT 0,
             connections INTEGER   NOT NULL DEFAULT 0,
             error       TEXT,
             -- ISO-8601 UTC text, not an epoch integer: SQLite has no date type, and of the three
             -- things it can hold, text is the one that reads as a time in any client and still
             -- works with `strftime`. The CHECK is what turns that from a habit into a rule.
             created_at  TIMESTAMP NOT NULL {created},
             updated_at  TIMESTAMP NOT NULL {updated},
             finished_at TIMESTAMP {finished},
             -- Removing a download from the list only hides the row: `seq` is the app's stable
             -- identity and the event log refers to it by number, so the history has to survive
             -- the user tidying up.
             is_deleted  INTEGER   NOT NULL DEFAULT 0
         );",
        created = stamp_check("created_at"),
        updated = stamp_check("updated_at"),
        finished = stamp_check("finished_at"),
    )
}

/// The `events` definition, shared for the same reason as [`create_downloads`].
fn create_events(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {table} (
             id     INTEGER PRIMARY KEY AUTOINCREMENT,
             at     TIMESTAMP NOT NULL {at},
             -- Nullable, because engine-wide events (`engine_ready`) belong to no download, and
             -- because a download row can be gone. Otherwise a real foreign key: `seq` is the
             -- same number `downloads` is keyed by, and a constraint is the only way that stays
             -- true.
             seq    INTEGER REFERENCES downloads(seq),
             kind   TEXT      NOT NULL,
             name   TEXT,
             detail TEXT
         );",
        at = stamp_check("at"),
    )
}

/// The index the log is read in reverse through.
const CREATE_EVENTS_INDEX: &str = "CREATE INDEX IF NOT EXISTS events_at ON events(at DESC);";

/// What happened, for the event log. Stored as text so the file stays readable with any
/// SQLite client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Added,
    Started,
    Resumed,
    Paused,
    Queued,
    Completed,
    Failed,
    Removed,
    FilesDeleted,
    EngineReady,
    EngineFailed,
}

impl Event {
    fn as_str(self) -> &'static str {
        match self {
            Event::Added => "added",
            Event::Started => "started",
            Event::Resumed => "resumed",
            Event::Paused => "paused",
            Event::Queued => "queued",
            Event::Completed => "completed",
            Event::Failed => "failed",
            Event::Removed => "removed",
            Event::FilesDeleted => "files_deleted",
            Event::EngineReady => "engine_ready",
            Event::EngineFailed => "engine_failed",
        }
    }

    /// The event that describes arriving at `status`, given where we came from.
    pub fn for_status(from: Option<Status>, to: Status) -> Option<Event> {
        match to {
            Status::Active if from == Some(Status::Paused) => Some(Event::Resumed),
            Status::Active => Some(Event::Started),
            Status::Waiting => Some(Event::Queued),
            Status::Paused => Some(Event::Paused),
            Status::Complete => Some(Event::Completed),
            Status::Error => Some(Event::Failed),
        }
    }
}

pub struct Store {
    conn: Option<Connection>,
    path: PathBuf,
    /// Why persistence is off, when it is.
    error: Option<String>,
}

impl Store {
    /// Open (creating if needed) the database at `path`. Never fails: a broken database
    /// degrades to a session with no history rather than a window that will not open.
    pub fn open(path: PathBuf) -> Self {
        match Self::connect(&path) {
            Ok(conn) => Self {
                conn: Some(conn),
                path,
                error: None,
            },
            Err(err) => Self {
                conn: None,
                path,
                error: Some(format!("{err}")),
            },
        }
    }

    fn connect(path: &Path) -> rusqlite::Result<Connection> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        // WAL keeps readers from blocking the writer, and NORMAL is the usual pairing: the
        // database survives a crash, only the last transaction can be lost.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(&format!(
            "{downloads}
             {events}
             {index}
             CREATE TABLE IF NOT EXISTS settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             -- Rows first written by the migration carry no `created_at` (the old JSON store
             -- had no such field), which would date them to 1970. `updated_at` was set at
             -- write time, so it is the best available estimate. Idempotent.
             UPDATE downloads SET created_at = updated_at WHERE created_at = 0;",
            downloads = create_downloads("downloads"),
            events = create_events("events"),
            index = CREATE_EVENTS_INDEX,
        ))?;
        add_column(
            &conn,
            "downloads",
            "is_deleted",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        // Two tells, because either can be missing on its own: a file from before the timestamps
        // became text declares `INTEGER` here, and one from before the checks declares `TIMESTAMP`
        // with nothing enforcing it. Neither can be fixed in place — SQLite changes a column's
        // type or adds a constraint only by rebuilding the table — and `CREATE TABLE IF NOT
        // EXISTS` above cannot touch a table that already exists.
        let stale = declared_type(&conn, "downloads", "created_at").as_deref() != Some("TIMESTAMP")
            || !has_stamp_checks(&conn, "downloads");
        if stale {
            rebuild(&conn)?;
        }
        // Enforced from here on. Said out loud rather than left to the default, which for this
        // build already is "on" (`SQLITE_DEFAULT_FOREIGN_KEYS=1`) — the declaration is what a
        // reader should be able to rely on, not a compile flag of a dependency. It comes *after*
        // the rebuild because that step has to be free to rebuild a table other tables point at.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(conn)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn enabled(&self) -> bool {
        self.conn.is_some()
    }

    /// Rows in `downloads` and `events`, for the settings page.
    pub fn counts(&self) -> (i64, i64) {
        let Some(conn) = self.conn.as_ref() else {
            return (0, 0);
        };
        let count = |table: &str| -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap_or(0)
        };
        (count("downloads"), count("events"))
    }

    // --------------------------------------------------------------------------- history

    /// Download rows, oldest first. Hidden rows stay in the file but are not history the list
    /// should show, so they are filtered out here.
    pub fn load_downloads(&self) -> Vec<Task> {
        let Some(conn) = self.conn.as_ref() else {
            return Vec::new();
        };
        let Ok(mut statement) = conn.prepare(&format!(
            "SELECT seq, uris, name, dir, status, total, completed, connections, error, {}
             FROM downloads WHERE is_deleted = 0 ORDER BY seq ASC",
            unix_seconds("created_at")
        )) else {
            return Vec::new();
        };

        let rows = statement.query_map([], |row| {
            let uris: String = row.get(1)?;
            let status: String = row.get(4)?;
            Ok(Task {
                gid: None,
                seq: row.get::<_, i64>(0)? as u64,
                uris: serde_json::from_str(&uris).unwrap_or_default(),
                name: row.get(2)?,
                dir: row.get(3)?,
                total: row.get::<_, i64>(5)? as u64,
                completed: row.get::<_, i64>(6)? as u64,
                speed: 0,
                // An unknown status means the row was written by a newer version; treat it
                // as paused so the user can act on it instead of losing the row.
                status: Status::parse(&status).unwrap_or(Status::Paused),
                connections: row.get::<_, i64>(7)? as u64,
                error: row.get(8)?,
                created_at: row.get(9)?,
            })
        });

        match rows {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn upsert_download(&self, task: &Task, created_at: i64) {
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        let now = now_unix();
        // Rows imported from the old JSON store have no timestamp (`serde(default)` left it
        // at 0), which would date them to 1970. Stamp them on the way in instead.
        let created_at = if created_at == 0 { now } else { created_at };
        let uris = serde_json::to_string(&task.uris).unwrap_or_else(|_| "[]".to_string());
        let finished_at = task.status.is_terminal().then_some(now);
        let _ = conn.execute(
            &format!(
                "INSERT INTO downloads
                     (seq, gid, uris, name, dir, status, total, completed, connections, error,
                      created_at, updated_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, {created}, {updated}, {finished})
                 ON CONFLICT(seq) DO UPDATE SET
                     gid         = excluded.gid,
                     uris        = excluded.uris,
                     name        = excluded.name,
                     dir         = excluded.dir,
                     status      = excluded.status,
                     total       = excluded.total,
                     completed   = excluded.completed,
                     connections = excluded.connections,
                     error       = excluded.error,
                     updated_at  = excluded.updated_at,
                 finished_at = COALESCE(downloads.finished_at, excluded.finished_at),
                     -- A row that was hidden and is being written again is live again.
                     is_deleted  = 0",
                created = stamp_param(11),
                updated = stamp_param(12),
                finished = stamp_optional(13),
            ),
            params![
                task.seq as i64,
                task.gid,
                uris,
                task.name,
                task.dir,
                status_str(task.status),
                task.total as i64,
                task.completed as i64,
                task.connections as i64,
                task.error,
                created_at,
                now,
                finished_at,
            ],
        );
    }

    /// Take a download off the list without losing the record of it.
    ///
    /// The row is what the settings page counts as stored data and what the events refer to, so
    /// it is flagged rather than dropped — the same reason [`Store::highest_seq`] still sees it.
    pub fn mark_deleted(&self, seq: u64) {
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        let _ = conn.execute(
            &format!(
                "UPDATE downloads SET is_deleted = 1, updated_at = {} WHERE seq = ?1",
                stamp_param(2)
            ),
            params![seq as i64, now_unix()],
        );
    }

    /// The highest `seq` ever written, hidden rows included.
    ///
    /// `seq` must never be handed out twice: the event log refers to downloads by it, and a
    /// reused number would silently attach new events to an old story. Deleted rows are not
    /// loaded into the list, so the list alone is not enough to find this — and clearing them
    /// would be a hard delete by the back door.
    pub fn highest_seq(&self) -> u64 {
        let Some(conn) = self.conn.as_ref() else {
            return 0;
        };
        conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM downloads", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or(0)
        .max(0) as u64
    }

    // ----------------------------------------------------------------------------- log

    pub fn log(&self, event: Event, seq: Option<u64>, name: Option<&str>, detail: Option<&str>) {
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        let _ = conn.execute(
            &format!(
                "INSERT INTO events (at, seq, kind, name, detail) VALUES ({}, ?2, ?3, ?4, ?5)",
                stamp_param(1)
            ),
            params![
                now_unix(),
                seq.map(|seq| seq as i64),
                event.as_str(),
                name,
                detail
            ],
        );
    }

    // ------------------------------------------------------------------------ settings

    pub fn setting(&self, key: &str) -> Option<String> {
        self.conn
            .as_ref()?
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Write one preference row. Tests use it to plant values; the app itself goes through
    /// [`Store::update_settings`] so a save is one transaction.
    #[cfg(test)]
    pub fn set_setting(&self, key: &str, value: &str) {
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        let _ = conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        );
    }

    /// Forget a preference, so "not set" and "set to nothing" cannot drift apart.
    pub fn remove_setting(&self, key: &str) {
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        let _ = conn.execute("DELETE FROM settings WHERE key = ?1", params![key]);
    }

    /// Apply a whole batch of preference changes in one transaction.
    ///
    /// Saving the settings is a dozen separate rows; run one statement at a time, a process that
    /// dies halfway leaves a mix of old and new preferences — the theme from this save and the
    /// font from the last. One transaction makes the batch all-or-nothing, so the only thing a
    /// crash can cost is the edit the user had not saved yet.
    pub fn update_settings(&self, writes: &[(&str, String)], removes: &[&str]) {
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        let Ok(tx) = conn.unchecked_transaction() else {
            return;
        };
        for (key, value) in writes {
            let _ = tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            );
        }
        for key in removes {
            let _ = tx.execute("DELETE FROM settings WHERE key = ?1", params![key]);
        }
        let _ = tx.commit();
    }

    /// Whether anything has ever been written to `settings`. This is what tells a fresh
    /// install (import the old `state.json`) from one that already has preferences.
    pub fn has_settings(&self) -> bool {
        let Some(conn) = self.conn.as_ref() else {
            return false;
        };
        conn.query_row("SELECT EXISTS (SELECT 1 FROM settings)", [], |row| {
            row.get::<_, bool>(0)
        })
        .unwrap_or(false)
    }

    /// True when nothing has been recorded yet, so a legacy import cannot resurrect rows
    /// the user has since removed.
    pub fn is_empty(&self) -> bool {
        self.counts() == (0, 0)
    }
}

pub fn status_str(status: Status) -> &'static str {
    match status {
        Status::Active => "active",
        Status::Waiting => "waiting",
        Status::Paused => "paused",
        Status::Complete => "complete",
        Status::Error => "error",
    }
}

/// The type a column was *declared* with, or `None` when there is no such column.
///
/// SQLite is dynamically typed — a declared type is only an affinity hint — which is exactly why
/// this has to be asked rather than assumed: it is how [`Store::connect`] recognises a file
/// written before the timestamps became text.
fn declared_type(conn: &Connection, table: &str, column: &str) -> Option<String> {
    conn.query_row(
        &format!("SELECT type FROM pragma_table_info('{table}') WHERE name = ?1"),
        params![column],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// ISO-8601 UTC text for an epoch-seconds *expression*, as a SQL fragment.
///
/// The conversion happens inside the statement because SQLite is the one thing here that already
/// knows how to format a time — `strftime` both writes these values and reads them back, and it
/// is what a person browsing the table would call on them too.
fn stamp(expr: &str) -> String {
    format!(
        "COALESCE(strftime('%Y-%m-%dT%H:%M:%SZ', {expr}, 'unixepoch'), \
         strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))"
    )
}

/// The same, for a bound epoch-seconds parameter.
fn stamp_param(index: usize) -> String {
    format!("strftime('%Y-%m-%dT%H:%M:%SZ', ?{index}, 'unixepoch')")
}

/// The same, for a parameter that may be NULL (a download that has not finished).
fn stamp_optional(index: usize) -> String {
    format!(
        "CASE WHEN ?{index} IS NULL THEN NULL \
         ELSE strftime('%Y-%m-%dT%H:%M:%SZ', ?{index}, 'unixepoch') END"
    )
}

/// The reverse: epoch seconds for an ISO-8601 column, so the rest of the app can keep working in
/// integers.
fn unix_seconds(column: &str) -> String {
    format!("COALESCE(CAST(strftime('%s', {column}) AS INTEGER), 0)")
}

/// Move both tables to the current schema: epoch-integer timestamps become ISO-8601 text, and
/// `events.seq` becomes a real foreign key.
///
/// SQLite cannot change a column's type or add a constraint in place, so this is a rebuild. It
/// runs before [`Store::connect`] turns foreign keys on, which is not an accident: one of its
/// jobs is repairing rows that could never satisfy the new key. A file written by a build that
/// hard-deleted downloads still has events pointing at rows that are gone — those keep their
/// place in the log with `seq` cleared, which is the honest reading of "this happened, and the
/// download it was about is no longer on file".
fn rebuild(conn: &Connection) -> rusqlite::Result<()> {
    // Off for the duration, and explicitly so. This build of SQLite is compiled with
    // `SQLITE_DEFAULT_FOREIGN_KEYS=1` (see `libsqlite3-sys`), so a fresh connection enforces
    // foreign keys without anyone asking — and `DROP TABLE downloads` does an implicit
    // `DELETE FROM downloads` first, which the `events` still pointing at those rows would then
    // violate. The caller turns them back on, deliberately after this returns.
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let tx = conn.unchecked_transaction()?;
    // Temp names, dropped first so an interrupted rebuild can simply be retried.
    tx.execute_batch(
        "DROP TABLE IF EXISTS downloads_next;
         DROP TABLE IF EXISTS events_next;",
    )?;
    tx.execute_batch(&format!(
        "{downloads}
         INSERT INTO downloads_next
             (seq, gid, uris, name, dir, status, total, completed, connections, error,
              created_at, updated_at, finished_at, is_deleted)
         SELECT seq, gid, uris, name, dir, status, total, completed, connections, error,
                {created}, {updated}, {finished}, is_deleted
         FROM downloads;
         DROP TABLE downloads;
         ALTER TABLE downloads_next RENAME TO downloads;

         {events}
         INSERT INTO events_next (id, at, seq, kind, name, detail)
         SELECT id, {at},
                CASE WHEN seq IN (SELECT seq FROM downloads) THEN seq END,
                kind, name, detail
         FROM events;
         DROP TABLE events;
         ALTER TABLE events_next RENAME TO events;
         {index}",
        downloads = create_downloads("downloads_next"),
        events = create_events("events_next"),
        index = CREATE_EVENTS_INDEX,
        created = stamp_column("created_at"),
        updated = stamp_column("updated_at"),
        finished = stamp_column_or_null("finished_at"),
        at = stamp_column("at"),
    ))?;
    tx.commit()
}

/// Text for a timestamp column during the rebuild: epoch seconds are converted, text passes
/// through untouched.
///
/// The pass-through is what makes the rebuild safe to run on a file that only lacks the checks.
/// Running `strftime` over a value that is already ISO text yields NULL, and [`stamp`]'s fallback
/// would then replace a perfectly good time with "now" -- a migration meant to add a constraint
/// quietly rewriting every timestamp in the file.
fn stamp_column(expr: &str) -> String {
    format!(
        "CASE WHEN typeof({expr}) = 'text' THEN {expr} ELSE {} END",
        stamp(expr)
    )
}

/// The same, preserving NULL: a download that never finished has no `finished_at`, and handing it
/// "now" would date a transfer that was never completed.
fn stamp_column_or_null(expr: &str) -> String {
    format!(
        "CASE WHEN {expr} IS NULL THEN NULL WHEN typeof({expr}) = 'text' THEN {expr}          ELSE strftime('%Y-%m-%dT%H:%M:%SZ', {expr}, 'unixepoch') END"
    )
}

/// Whether `table` carries the timestamp `CHECK`s.
///
/// Column constraints are not in `pragma_table_info` -- the only place SQLite records them is the
/// `CREATE TABLE` text -- so the definition is fetched and looked at. The needle is a fragment
/// nothing else in the schema contains.
fn has_stamp_checks(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .is_some_and(|sql| sql.contains("GLOB '"))
}

/// Add a column only if it is missing.
///
/// SQLite has no `ADD COLUMN IF NOT EXISTS`, so this asks `pragma_table_info` first. Like the
/// other fix-ups in [`Store::connect`] it is safe to run on every start, which is what keeps the
/// migration idempotent.
fn add_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let exists = conn
        .prepare(&format!(
            "SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"
        ))?
        .exists(params![column])?;
    if !exists {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) fn scratch_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nexus-store-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("nexus.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Task;

    /// A fresh database in its own directory. The directory name includes the process id so
    /// tests running in parallel cannot collide.
    fn scratch(name: &str) -> PathBuf {
        scratch_db(name)
    }

    fn sample(seq: u64) -> Task {
        Task::placeholder(
            seq,
            "https://example.com/a.bin".to_string(),
            "C:\tmp".to_string(),
        )
    }

    #[test]
    fn a_download_survives_a_reopen() {
        let path = scratch("roundtrip");
        {
            let store = Store::open(path.clone());
            assert!(store.enabled(), "database should open: {:?}", store.error());

            let mut task = sample(1);
            task.name = "a.bin".to_string();
            task.total = 4096;
            task.completed = 1024;
            task.status = Status::Active;
            task.connections = 8;
            store.upsert_download(&task, task.created_at);
        }

        let store = Store::open(path);
        let tasks = store.load_downloads();
        assert_eq!(tasks.len(), 1);
        let task = &tasks[0];
        assert_eq!(task.seq, 1);
        assert_eq!(task.name, "a.bin");
        assert_eq!(task.uris, vec!["https://example.com/a.bin".to_string()]);
        assert_eq!(task.total, 4096);
        assert_eq!(task.completed, 1024);
        assert_eq!(task.status, Status::Active);
        assert_eq!(task.connections, 8);
        // Restored rows never carry a gid: aria2 is restarted with the app.
        assert_eq!(task.gid, None);
    }

    #[test]
    fn an_imported_row_without_a_timestamp_gets_one() {
        let path = scratch("stamp");
        let store = Store::open(path);
        // This is what a row migrated from the old JSON store looks like: `serde(default)`
        // left `created_at` at zero.
        let mut task = sample(1);
        task.created_at = 0;
        store.upsert_download(&task, task.created_at);

        let tasks = store.load_downloads();
        assert!(
            tasks[0].created_at > 1_600_000_000,
            "expected a real timestamp, got {}",
            tasks[0].created_at
        );
    }

    #[test]
    fn upsert_updates_in_place_rather_than_appending() {
        let path = scratch("upsert");
        let store = Store::open(path);
        let mut task = sample(7);
        store.upsert_download(&task, task.created_at);

        task.completed = 512;
        task.status = Status::Complete;
        store.upsert_download(&task, task.created_at);

        assert_eq!(store.counts().0, 1);
        let tasks = store.load_downloads();
        assert_eq!(tasks[0].completed, 512);
        assert_eq!(tasks[0].status, Status::Complete);
    }

    #[test]
    fn deleting_a_download_hides_the_row_without_losing_it() {
        let path = scratch("delete");
        let store = Store::open(path);
        let task = sample(3);
        store.upsert_download(&task, task.created_at);
        store.log(Event::Added, Some(3), Some("a.bin"), None);
        store.log(Event::Removed, Some(3), Some("a.bin"), None);

        store.mark_deleted(3);

        assert!(store.load_downloads().is_empty(), "it leaves the list");
        assert_eq!(store.counts(), (1, 2), "but the row is still in the file");
        assert_eq!(store.highest_seq(), 3, "and its seq stays taken");
    }

    /// The reason for hiding rather than deleting: a removed download must not have its number
    /// handed to the next one, or the event log would attach new events to an old story.
    #[test]
    fn a_hidden_row_keeps_its_seq_out_of_circulation() {
        let path = scratch("seq-floor");
        let store = Store::open(path);
        store.upsert_download(&sample(7), 0);
        store.mark_deleted(7);
        assert!(store.load_downloads().is_empty());
        assert_eq!(store.highest_seq(), 7);
    }

    /// A database in the exact shape the previous release wrote: epoch-integer timestamps, and no
    /// foreign key on `events.seq`. Includes the case that actually occurs in the wild — an event
    /// whose download row was hard-deleted by a build that did not soft-delete — plus a row that
    /// is already hidden.
    fn previous_shape(db: &Path) {
        let conn = Connection::open(db).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE downloads (
                   seq INTEGER PRIMARY KEY, gid TEXT, uris TEXT NOT NULL, name TEXT NOT NULL,
                   dir TEXT NOT NULL, status TEXT NOT NULL,
                   total INTEGER NOT NULL DEFAULT 0, completed INTEGER NOT NULL DEFAULT 0,
                   connections INTEGER NOT NULL DEFAULT 0, error TEXT,
                   created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, finished_at INTEGER,
                   is_deleted INTEGER NOT NULL DEFAULT 0
               );
               CREATE TABLE events (
                   id INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL, seq INTEGER,
                   kind TEXT NOT NULL, name TEXT, detail TEXT
               );
               CREATE INDEX events_at ON events(at DESC);
               INSERT INTO downloads
                   (seq, uris, name, dir, status, created_at, updated_at, finished_at, is_deleted)
               VALUES (1, '["https://example.com/a.bin"]', 'a.bin', 'C:\dl', 'complete',
                       1791028339, 1791028349, 1791028349, 0),
                      (2, '["https://example.com/b.bin"]', 'b.bin', 'C:\dl', 'paused',
                       1791028400, 1791028400, NULL, 1);
               INSERT INTO events (at, seq, kind, name, detail) VALUES
                   (1791028339, 1, 'added', 'a.bin', NULL),
                   (1791028400, NULL, 'engine_ready', NULL, NULL),
                   (1791028500, 99, 'removed', 'ghost.bin', NULL);"#,
        )
        .unwrap();
    }

    /// A database in the shape the previous release wrote: text timestamps and the foreign key,
    /// but nothing enforcing the shape of a timestamp.
    fn shape_before_checks(db: &Path) {
        let conn = Connection::open(db).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE downloads (
                   seq INTEGER PRIMARY KEY, gid TEXT, uris TEXT NOT NULL, name TEXT NOT NULL,
                   dir TEXT NOT NULL, status TEXT NOT NULL,
                   total INTEGER NOT NULL DEFAULT 0, completed INTEGER NOT NULL DEFAULT 0,
                   connections INTEGER NOT NULL DEFAULT 0, error TEXT,
                   created_at TIMESTAMP NOT NULL, updated_at TIMESTAMP NOT NULL,
                   finished_at TIMESTAMP, is_deleted INTEGER NOT NULL DEFAULT 0
               );
               CREATE TABLE events (
                   id INTEGER PRIMARY KEY AUTOINCREMENT, at TIMESTAMP NOT NULL,
                   seq INTEGER REFERENCES downloads(seq), kind TEXT NOT NULL,
                   name TEXT, detail TEXT
               );
               INSERT INTO downloads
                   (seq, uris, name, dir, status, created_at, updated_at, finished_at)
               VALUES (1, '["https://example.com/a.bin"]', 'a.bin', 'C:\dl', 'paused',
                       '2021-01-02T03:04:05Z', '2021-01-02T03:04:05Z', NULL);"#,
        )
        .unwrap();
    }

    /// `2026-10-03T11:52:19Z` — checked by hand because the point is what is *stored*, which is
    /// exactly what the app does not see.
    fn is_iso_utc(text: &str) -> bool {
        let bytes = text.as_bytes();
        bytes.len() == 20
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes[10] == b'T'
            && bytes[13] == b':'
            && bytes[16] == b':'
            && bytes[19] == b'Z'
            && text
                .char_indices()
                .all(|(i, c)| [4, 7, 10, 13, 16, 19].contains(&i) || c.is_ascii_digit())
    }

    /// Rebuilding the tables is the riskiest thing in this file, so: nothing lost, the shape is
    /// the new one, and the timestamps still say what they said.
    #[test]
    fn an_old_file_is_rebuilt_without_losing_anything() {
        let path = scratch("rebuild");
        previous_shape(&path);
        let store = Store::open(path.clone());

        let tasks = store.load_downloads();
        assert_eq!(tasks.len(), 1, "the hidden row stays hidden");
        assert_eq!(tasks[0].seq, 1);
        assert_eq!(
            tasks[0].created_at, 1791028339,
            "the time survives the conversion"
        );
        assert_eq!(tasks[0].completed, 0);
        assert_eq!(
            store.counts(),
            (2, 3),
            "both hidden row and log are still there"
        );

        let conn = Connection::open(&path).unwrap();
        let (created, finished): (String, String) = conn
            .query_row(
                "SELECT created_at, finished_at FROM downloads WHERE seq = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(is_iso_utc(&created), "stored as text, got {created}");
        assert!(is_iso_utc(&finished), "stored as text, got {finished}");
        let (dangling,): (Option<i64>,) = conn
            .query_row(
                "SELECT finished_at FROM downloads WHERE seq = 2",
                [],
                |row| Ok((row.get(0)?,)),
            )
            .unwrap();
        assert_eq!(
            dangling, None,
            "a NULL stays NULL rather than becoming a date"
        );
    }

    /// The repair that makes the foreign key addable at all: `seq 99` never existed, and dropping
    /// the event would silently lose a line of the audit log. Clearing the reference keeps it.
    #[test]
    fn an_event_whose_download_is_gone_keeps_its_place() {
        let path = scratch("orphan");
        previous_shape(&path);
        let store = Store::open(path.clone());
        assert_eq!(store.counts().1, 3, "no event is thrown away");

        let conn = Connection::open(&path).unwrap();
        let (ghost, kind): (Option<i64>, String) = conn
            .query_row(
                "SELECT seq, kind FROM events WHERE name = 'ghost.bin'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "removed");
        assert_eq!(
            ghost, None,
            "the dangling reference is cleared, not the row"
        );
    }

    /// Adding the checks must not touch the values on the way past. Running the conversion over a
    /// value that is already ISO-8601 text yields NULL, and `stamp`'s fallback would then restamp
    /// every row in the file with "now" — a migration that looks like it worked.
    #[test]
    fn adding_the_checks_leaves_existing_timestamps_alone() {
        let path = scratch("checks-later");
        shape_before_checks(&path);
        let store = Store::open(path.clone());

        let tasks = store.load_downloads();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].created_at, 1609556645, "2021-01-02T03:04:05Z");

        let conn = Connection::open(&path).unwrap();
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'downloads'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("GLOB '"),
            "the check has to be there afterwards"
        );
        let created: String = conn
            .query_row("SELECT created_at FROM downloads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(created, "2021-01-02T03:04:05Z", "and the time is untouched");
    }

    /// The state a file lands in once a bug has already written a number: text in one column and
    /// an integer in another. Both have to come out as text — the one that is already a stamp
    /// untouched, the number converted rather than replaced by "now".
    #[test]
    fn a_file_mixed_between_text_and_numbers_is_rebuilt_whole() {
        let path = scratch("mixed");
        shape_before_checks(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute("UPDATE downloads SET updated_at = 1791029599", [])
                .unwrap();
        }

        let store = Store::open(path.clone());
        assert_eq!(store.error(), None, "the rebuild must not have failed");
        assert_eq!(store.counts(), (1, 0));

        let conn = Connection::open(&path).unwrap();
        let (created, updated): (String, String) = conn
            .query_row("SELECT created_at, updated_at FROM downloads", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(
            created, "2021-01-02T03:04:05Z",
            "already a stamp, left alone"
        );
        assert_eq!(
            updated, "2026-10-03T12:13:19Z",
            "the epoch integer is converted, not overwritten with now"
        );
    }

    /// The bug this exists for: `mark_deleted` passed epoch seconds straight into `updated_at`,
    /// and SQLite stored the integer without a word — in a column declared `TIMESTAMP`. Every
    /// write path is walked here because that is how the value got in: not through the statement
    /// anyone would have checked.
    #[test]
    fn every_timestamp_column_holds_text() {
        let path = scratch("typeof");
        let store = Store::open(path.clone());
        store.upsert_download(&sample(1), 0);
        store.log(Event::Added, Some(1), Some("a.bin"), None);
        store.mark_deleted(1);

        let conn = Connection::open(&path).unwrap();
        let (created, updated, finished, deleted, at): (String, String, String, i64, String) = conn
            .query_row(
                "SELECT typeof(created_at), typeof(updated_at), typeof(finished_at), is_deleted,
                        (SELECT typeof(at) FROM events LIMIT 1)
                 FROM downloads",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(created, "text");
        assert_eq!(updated, "text", "mark_deleted writes this one too");
        assert_eq!(
            finished, "null",
            "a download that never finished has no stamp"
        );
        assert_eq!(at, "text");
        // The write has to have landed. Asserting only the *type* would also pass when the
        // statement was rejected wholesale — which is exactly what the check above does to a
        // wrong-typed value — so the type on its own proves nothing about whether it was written.
        assert_eq!(deleted, 1, "the soft delete has to take effect");
    }

    /// And the column refuses the mistake outright, so no future write path can reintroduce it.
    #[test]
    fn a_timestamp_column_refuses_a_number() {
        let path = scratch("stamp-refused");
        let store = Store::open(path.clone());
        store.upsert_download(&sample(1), 0);

        let conn = Connection::open(&path).unwrap();
        let refused = conn.execute(
            "UPDATE downloads SET updated_at = 1791029599 WHERE seq = 1",
            [],
        );
        assert!(refused.is_err(), "an integer is not a timestamp");
    }

    /// Declaring the constraint is not enough — SQLite only honours foreign keys when they are
    /// switched on per connection, so this asserts the switch actually happened.
    #[test]
    fn an_event_cannot_point_at_a_download_that_is_not_there() {
        let path = scratch("fk");
        let store = Store::open(path.clone());
        store.upsert_download(&sample(1), 0);
        store.log(Event::Added, Some(1), Some("a.bin"), None);
        store.log(Event::Added, Some(99), Some("ghost"), None);

        let conn = Connection::open(&path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "the second event had no download to belong to");
    }

    /// An engine-wide event belongs to no download, so `seq` stays nullable.
    #[test]
    fn an_engine_event_needs_no_download() {
        let path = scratch("fk-null");
        let store = Store::open(path.clone());
        store.log(Event::EngineReady, None, None, None);

        let conn = Connection::open(&path).unwrap();
        let (seq, at): (Option<i64>, String) = conn
            .query_row("SELECT seq, at FROM events", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(seq, None);
        assert!(is_iso_utc(&at), "stored as text, got {at}");
    }

    #[test]
    fn settings_survive_a_reopen() {
        let path = scratch("settings");
        {
            let store = Store::open(path.clone());
            store.set_setting("theme", "dark");
            store.set_setting("theme", "light");
            store.set_setting("font", "MiSans");
        }
        let store = Store::open(path);
        assert_eq!(store.setting("theme").as_deref(), Some("light"));
        assert_eq!(store.setting("font").as_deref(), Some("MiSans"));
        assert_eq!(store.setting("missing"), None);
        assert!(store.has_settings());

        store.remove_setting("font");
        assert_eq!(store.setting("font"), None);
    }

    #[test]
    fn a_settings_batch_writes_and_removes_together() {
        let store = Store::open(scratch("settings-batch"));
        store.set_setting("font", "MiSans");
        store.set_setting("proxy", "http://127.0.0.1:7890");

        // One call does both halves of a save: values that changed and rows that are gone.
        store.update_settings(
            &[
                ("theme", "dark".to_string()),
                ("font", "Simsun".to_string()),
            ],
            &["proxy"],
        );

        assert_eq!(store.setting("theme").as_deref(), Some("dark"));
        assert_eq!(store.setting("font").as_deref(), Some("Simsun"));
        assert_eq!(store.setting("proxy"), None);
    }

    #[test]
    fn a_fresh_database_reports_no_settings() {
        let store = Store::open(scratch("no-settings"));
        assert!(!store.has_settings());
    }

    #[test]
    fn an_empty_database_is_what_triggers_a_legacy_import() {
        let path = scratch("legacy");
        let store = Store::open(path);
        assert!(store.is_empty());

        let task = sample(1);
        store.upsert_download(&task, task.created_at);
        assert!(!store.is_empty());
    }

    #[test]
    fn every_status_transition_maps_to_a_readable_event() {
        use Event::*;
        assert_eq!(Event::for_status(None, Status::Waiting), Some(Queued));
        assert_eq!(
            Event::for_status(Some(Status::Waiting), Status::Active),
            Some(Started)
        );
        assert_eq!(
            Event::for_status(Some(Status::Paused), Status::Active),
            Some(Resumed)
        );
        assert_eq!(
            Event::for_status(Some(Status::Active), Status::Paused),
            Some(Paused)
        );
        assert_eq!(
            Event::for_status(Some(Status::Active), Status::Complete),
            Some(Completed)
        );
        assert_eq!(
            Event::for_status(Some(Status::Active), Status::Error),
            Some(Failed)
        );

        // The stored spelling is what an external `sqlite3` client will see.
        assert_eq!(Completed.as_str(), "completed");
        assert_eq!(FilesDeleted.as_str(), "files_deleted");
    }

    #[test]
    fn a_corrupt_database_disables_persistence_instead_of_panicking() {
        let path = scratch("corrupt");
        std::fs::write(&path, b"this is not a database").unwrap();

        let store = Store::open(path);
        // Either the file was rejected outright, or SQLite treated the header as empty and
        // rebuilt it — both are fine, what matters is that no call panics.
        let task = sample(1);
        store.upsert_download(&task, task.created_at);
        store.log(Event::Added, Some(1), Some("a.bin"), None);
        let _ = store.load_downloads();
        let _ = store.counts();
        let _ = store.setting("ui");
    }
}
