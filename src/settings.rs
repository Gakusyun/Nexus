//! User preferences, persisted alongside the download history.

use serde::{Deserialize, Serialize};

use crate::store::Store;

/// Keys in the `settings` table. One row per preference, so the table stays readable with any
/// SQLite client and adding a preference is a new field plus one line in `load`/`save`.
pub const KEY_THEME: &str = "theme";
pub const KEY_ACCENT: &str = "accent";
pub const KEY_LANGUAGE: &str = "language";
pub const KEY_FONT: &str = "font";
pub const KEY_DOWNLOAD_DIR: &str = "download_dir";
pub const KEY_USER_AGENT: &str = "user_agent";
pub const KEY_CONNECTIONS: &str = "connections";
pub const KEY_MAX_CONCURRENT: &str = "max_concurrent";
pub const KEY_SPEED_LIMIT: &str = "speed_limit";
pub const KEY_PROXY: &str = "proxy";
pub const KEY_MAX_TRIES: &str = "max_tries";
pub const KEY_TIMEOUT: &str = "timeout";
/// The single JSON blob the preferences lived in before they got their own rows.
const KEY_LEGACY: &str = "ui";

/// The accent Nexus is born with: the purple of the logo. The swatch grid marks it with a dot
/// and its reset button comes back here, so this one number is both the default and the promise.
pub const DEFAULT_ACCENT: u32 = 0x7c5cff;
/// Segments per download (and connections per server). aria2 caps the latter at 16, so the
/// picker never offers more.
pub const DEFAULT_CONNECTIONS: u32 = 16;
/// How many downloads run at once.
pub const DEFAULT_MAX_CONCURRENT: u32 = 5;
/// Retries per download; `0` means unlimited, as aria2 spells it.
pub const DEFAULT_MAX_TRIES: u32 = 5;
/// Seconds before an idle or unreachable server is given up on.
pub const DEFAULT_TIMEOUT: u32 = 60;

/// Which palette to paint. `System` follows the platform's light/dark setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];

    /// How the mode is spelled in the database. Lowercase so the row reads naturally in an
    /// `sqlite3` session.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
            ThemeMode::System => "system",
        }
    }

    /// `None` for anything unrecognised, so a hand-edited row falls back to the default
    /// instead of taking the whole configuration down.
    pub fn parse(raw: &str) -> Option<ThemeMode> {
        match raw {
            "light" => Some(ThemeMode::Light),
            "dark" => Some(ThemeMode::Dark),
            "system" => Some(ThemeMode::System),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    En,
    Zh,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::En, Language::Zh];

    pub fn as_str(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Zh => "zh",
        }
    }

    pub fn parse(raw: &str) -> Option<Language> {
        match raw {
            "en" => Some(Language::En),
            "zh" => Some(Language::Zh),
            _ => None,
        }
    }

    /// The language's own name, which is what a language picker should show.
    pub fn label(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::Zh => "简体中文",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeMode,
    /// The accent everything tinted is painted with, `0xRRGGBB`.
    ///
    /// Kept in the same form the library takes it — a plain integer — because that is what
    /// `Look::accent` holds and what the swatches hand back; the database spells it as six hex
    /// digits so a row reads as a colour in any client.
    pub accent: u32,
    pub language: Language,
    /// The UI font family list, in CSS `font-family` order: `"Times New Roman", Simsun`.
    /// `None` (or a list nothing matches) keeps the platform default. See [`font_stack`].
    pub font: Option<String>,
    /// Where new downloads are written. `None` means the platform's Downloads folder.
    pub download_dir: Option<String>,
    // ---- engine preferences (see `aria2::EngineOptions`) ----
    /// `None` keeps the built-in user agent.
    pub user_agent: Option<String>,
    pub connections: u32,
    pub max_concurrent: u32,
    /// Bytes per second overall; `0` is unlimited.
    pub speed_limit: u64,
    pub proxy: Option<String>,
    pub max_tries: u32,
    pub timeout: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::default(),
            accent: DEFAULT_ACCENT,
            language: Language::default(),
            font: None,
            download_dir: None,
            user_agent: None,
            connections: DEFAULT_CONNECTIONS,
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            speed_limit: 0,
            proxy: None,
            max_tries: DEFAULT_MAX_TRIES,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// Split a font family list into names, dropping the quotes and any empty entry.
///
/// Quoting is what keeps a name with spaces together, exactly as in CSS and VS Code's
/// `editor.fontFamily`: `"Times New Roman", Simsun` is two families, not three.
pub fn font_stack(list: &str) -> Vec<String> {
    list.split(',')
        .map(|name| name.trim().trim_matches(['"', '\'']).trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// Read a colour the way a person types one: `#7c5cff` or `7c5cff`, either case.
///
/// `None` for anything else, including the short `#fff` form — expanding it would mean inventing
/// three digits the user never typed, and the box always shows the six anyway.
pub fn parse_accent(raw: &str) -> Option<u32> {
    let raw = raw.trim().trim_start_matches('#');
    if raw.len() != 6 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(raw, 16).ok()
}

/// Read a numeric preference. A missing or unreadable row is `None`, so the caller keeps its
/// default instead of silently getting a zero.
fn number<T: std::str::FromStr>(store: &Store, key: &str) -> Option<T> {
    store.setting(key).and_then(|raw| raw.parse().ok())
}

impl Settings {
    /// Read the preferences key by key. A missing or unreadable row keeps its default, so a
    /// partially written database still starts with a usable configuration.
    pub fn load(store: &Store) -> Self {
        let mut settings = Settings::default();
        if let Some(raw) = store.setting(KEY_THEME) {
            settings.theme = ThemeMode::parse(&raw).unwrap_or_default();
        }
        settings.accent = store
            .setting(KEY_ACCENT)
            .and_then(|raw| parse_accent(&raw))
            .unwrap_or(DEFAULT_ACCENT);
        if let Some(raw) = store.setting(KEY_LANGUAGE) {
            settings.language = Language::parse(&raw).unwrap_or_default();
        }
        settings.font = store.setting(KEY_FONT).filter(|font| !font.is_empty());
        settings.download_dir = store
            .setting(KEY_DOWNLOAD_DIR)
            .filter(|dir| !dir.is_empty());
        settings.user_agent = store.setting(KEY_USER_AGENT).filter(|ua| !ua.is_empty());
        settings.proxy = store.setting(KEY_PROXY).filter(|proxy| !proxy.is_empty());
        settings.connections = number(store, KEY_CONNECTIONS).unwrap_or(DEFAULT_CONNECTIONS);
        settings.max_concurrent =
            number(store, KEY_MAX_CONCURRENT).unwrap_or(DEFAULT_MAX_CONCURRENT);
        settings.speed_limit = number(store, KEY_SPEED_LIMIT).unwrap_or(0);
        settings.max_tries = number(store, KEY_MAX_TRIES).unwrap_or(DEFAULT_MAX_TRIES);
        settings.timeout = number(store, KEY_TIMEOUT).unwrap_or(DEFAULT_TIMEOUT);
        settings
    }

    /// Write every preference back out, as one transaction. An unset value removes its row rather
    /// than storing an empty string, so "not set" has exactly one on-disk spelling.
    pub fn save(&self, store: &Store) {
        let mut writes: Vec<(&str, String)> = vec![
            (KEY_THEME, self.theme.as_str().to_string()),
            // Six lowercase hex digits, no `#`: the row then reads as a colour in a client and
            // parses back through [`parse_accent`], which accepts both spellings.
            (KEY_ACCENT, format!("{:06x}", self.accent)),
            (KEY_LANGUAGE, self.language.as_str().to_string()),
            (KEY_CONNECTIONS, self.connections.to_string()),
            (KEY_MAX_CONCURRENT, self.max_concurrent.to_string()),
            (KEY_SPEED_LIMIT, self.speed_limit.to_string()),
            (KEY_MAX_TRIES, self.max_tries.to_string()),
            (KEY_TIMEOUT, self.timeout.to_string()),
        ];
        let mut removes: Vec<&str> = Vec::new();
        for (key, value) in [
            (KEY_FONT, &self.font),
            (KEY_DOWNLOAD_DIR, &self.download_dir),
            (KEY_USER_AGENT, &self.user_agent),
            (KEY_PROXY, &self.proxy),
        ] {
            match value.as_deref().filter(|value| !value.is_empty()) {
                Some(value) => writes.push((key, value.to_string())),
                None => removes.push(key),
            }
        }
        store.update_settings(&writes, &removes);
    }

    /// Fold the old `ui` JSON blob into the per-key rows, once. Idempotent: the blob is
    /// removed on success, so later calls find nothing left to migrate.
    pub fn absorb_legacy(store: &Store) {
        let Some(raw) = store.setting(KEY_LEGACY) else {
            return;
        };
        if let Ok(settings) = serde_json::from_str::<Settings>(&raw) {
            settings.save(store);
            store.remove_setting(KEY_LEGACY);
        }
        // A blob we cannot parse is left in place: the defaults are used, and the row still
        // counts as "preferences present" — the same call the old build would have made,
        // rather than destroying the user's data silently.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::scratch_db;

    #[test]
    fn a_font_list_splits_the_way_css_does() {
        assert_eq!(font_stack("MiSans"), ["MiSans"]);
        assert_eq!(
            font_stack("\"Times New Roman\", Simsun , 'KaiTi'"),
            ["Times New Roman", "Simsun", "KaiTi"]
        );
        // Internal spaces survive; stray ones and empty entries do not.
        assert_eq!(font_stack(" , MiSans ,, "), ["MiSans"]);
        assert_eq!(font_stack("Microsoft YaHei UI").len(), 1);
        assert!(font_stack("   ").is_empty());
    }

    #[test]
    fn an_accent_round_trips_as_six_hex_digits() {
        let path = scratch_db("accent");
        let store = Store::open(path.clone());
        Settings {
            accent: 0x00ff7f,
            ..Settings::default()
        }
        .save(&store);
        // Spelled the way any client would read it: no `#`, lowercase.
        assert_eq!(store.setting(KEY_ACCENT).as_deref(), Some("00ff7f"));
        assert_eq!(Settings::load(&store).accent, 0x00ff7f);

        // Both spellings a person might type parse; a short form or a typo never becomes a colour.
        assert_eq!(parse_accent("#7C5CFF"), Some(0x7c5cff));
        assert_eq!(parse_accent("7c5cff"), Some(0x7c5cff));
        assert_eq!(parse_accent("  #7c5cff  "), Some(0x7c5cff));
        assert_eq!(parse_accent("#fff"), None, "no short form to expand");
        assert_eq!(parse_accent("#7c5cf"), None);
        assert_eq!(parse_accent("#7c5czz"), None);
        assert_eq!(parse_accent(""), None);

        // A hand-edited row keeps the logo's purple instead of taking the app down.
        store.set_setting(KEY_ACCENT, "not-a-colour");
        assert_eq!(Settings::load(&store).accent, DEFAULT_ACCENT);
    }

    #[test]
    fn preferences_round_trip_through_their_own_rows() {
        let path = scratch_db("prefs");
        {
            let store = Store::open(path.clone());
            let settings = Settings {
                theme: ThemeMode::Dark,
                language: Language::Zh,
                font: Some("MiSans".to_string()),
                download_dir: Some(r"C:\Temp".to_string()),
                ..Settings::default()
            };
            settings.save(&store);
            // The values are spelled out in the table, not wrapped in JSON.
            assert_eq!(store.setting(KEY_THEME).as_deref(), Some("dark"));
            assert_eq!(store.setting(KEY_LANGUAGE).as_deref(), Some("zh"));
        }

        let store = Store::open(path);
        let settings = Settings::load(&store);
        assert_eq!(settings.theme, ThemeMode::Dark);
        assert_eq!(settings.language, Language::Zh);
        assert_eq!(settings.font.as_deref(), Some("MiSans"));
        assert_eq!(settings.download_dir.as_deref(), Some(r"C:\Temp"));
    }

    #[test]
    fn engine_preferences_round_trip_and_fall_back() {
        let path = scratch_db("engine-prefs");
        {
            let store = Store::open(path.clone());
            Settings {
                user_agent: Some("Nexus/9".to_string()),
                connections: 4,
                max_concurrent: 3,
                speed_limit: 2_097_152,
                proxy: Some("http://127.0.0.1:7890".to_string()),
                max_tries: 0,
                timeout: 30,
                ..Settings::default()
            }
            .save(&store);
            assert_eq!(store.setting(KEY_CONNECTIONS).as_deref(), Some("4"));
            assert_eq!(store.setting(KEY_SPEED_LIMIT).as_deref(), Some("2097152"));
        }

        let store = Store::open(path);
        let settings = Settings::load(&store);
        assert_eq!(settings.user_agent.as_deref(), Some("Nexus/9"));
        assert_eq!(settings.connections, 4);
        assert_eq!(settings.max_concurrent, 3);
        assert_eq!(settings.speed_limit, 2_097_152);
        assert_eq!(settings.proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(settings.max_tries, 0, "0 is a real value, not 'unset'");
        assert_eq!(settings.timeout, 30);

        // A junk row keeps the default rather than producing a zero the engine would choke on.
        store.set_setting(KEY_CONNECTIONS, "many");
        store.set_setting(KEY_TIMEOUT, "");
        let settings = Settings::load(&store);
        assert_eq!(settings.connections, DEFAULT_CONNECTIONS);
        assert_eq!(settings.timeout, DEFAULT_TIMEOUT);

        // Clearing the optional text boxes removes their rows instead of storing an empty string.
        Settings::default().save(&store);
        assert_eq!(store.setting(KEY_USER_AGENT), None);
        assert_eq!(store.setting(KEY_PROXY), None);
    }

    #[test]
    fn clearing_a_preference_removes_its_row() {
        let store = Store::open(scratch_db("prefs-unset"));
        Settings {
            font: Some("MiSans".to_string()),
            ..Settings::default()
        }
        .save(&store);
        assert!(store.setting(KEY_FONT).is_some());

        Settings::default().save(&store);
        assert_eq!(store.setting(KEY_FONT), None);
        assert_eq!(store.setting(KEY_DOWNLOAD_DIR), None);
        // The enums always have a value, so their rows stay.
        assert_eq!(store.setting(KEY_THEME).as_deref(), Some("system"));
    }

    #[test]
    fn a_junk_value_falls_back_to_the_default_instead_of_failing() {
        let store = Store::open(scratch_db("prefs-junk"));
        store.set_setting(KEY_THEME, "sepia");
        store.set_setting(KEY_LANGUAGE, "");
        // An empty string is how "unset" is spelled on disk for the optional fields.
        store.set_setting(KEY_FONT, "");

        let settings = Settings::load(&store);
        assert_eq!(settings.theme, ThemeMode::System);
        assert_eq!(settings.language, Language::En);
        assert_eq!(settings.font, None);
    }

    #[test]
    fn the_old_json_blob_is_migrated_once() {
        let path = scratch_db("prefs-legacy");
        {
            let store = Store::open(path.clone());
            store.set_setting(
                KEY_LEGACY,
                r#"{"theme":"Dark","language":"Zh","font":"MiSans"}"#,
            );
        }

        let store = Store::open(path);
        Settings::absorb_legacy(&store);
        assert_eq!(store.setting(KEY_LEGACY), None);
        let settings = Settings::load(&store);
        assert_eq!(settings.theme, ThemeMode::Dark);
        assert_eq!(settings.language, Language::Zh);
        assert_eq!(settings.font.as_deref(), Some("MiSans"));

        // Running it again must not disturb what is already there.
        Settings::absorb_legacy(&store);
        assert_eq!(Settings::load(&store).theme, ThemeMode::Dark);
    }

    #[test]
    fn an_unreadable_blob_is_left_alone() {
        let store = Store::open(scratch_db("prefs-broken"));
        store.set_setting(KEY_LEGACY, "{not json");
        Settings::absorb_legacy(&store);
        assert_eq!(store.setting(KEY_LEGACY).as_deref(), Some("{not json"));
    }
}
