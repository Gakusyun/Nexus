//! Every string the interface can show, in each supported language.
//!
//! A struct rather than a key-value lookup: the compiler then refuses to build a language
//! that is missing a string, which is the one mistake a translation table makes silently.

use crate::settings::Language;

pub struct Strings {
    // Window chrome.
    pub app_name: &'static str,

    // Command bar.
    pub placeholder: &'static str,
    pub add: &'static str,

    // Filters and the strip beside them.
    pub filter_all: &'static str,
    pub filter_active: &'static str,
    pub filter_finished: &'static str,
    pub clear_finished: &'static str,
    pub one_active: &'static str,
    pub many_active: &'static str,

    // Status badges.
    pub status_active: &'static str,
    pub status_waiting: &'static str,
    pub status_paused: &'static str,
    pub status_complete: &'static str,
    pub status_error: &'static str,

    // Task rows.
    pub fetching_metadata: &'static str,
    pub queued: &'static str,
    pub time_left: &'static str,
    pub one_connection: &'static str,
    pub many_connections: &'static str,
    pub paused_at: &'static str,
    pub download_failed: &'static str,

    // Empty states.
    pub empty_all_title: &'static str,
    pub empty_all_hint: &'static str,
    pub empty_active_title: &'static str,
    pub empty_active_hint: &'static str,
    pub empty_finished_title: &'static str,
    pub empty_finished_hint: &'static str,

    // Settings page.
    pub settings_title: &'static str,
    pub save: &'static str,
    pub section_downloads: &'static str,
    pub downloads_summary: &'static str,
    pub download_folder: &'static str,
    pub open_folder: &'static str,
    pub change: &'static str,
    pub folder_note: &'static str,
    pub section_appearance: &'static str,
    pub appearance_summary: &'static str,
    pub theme: &'static str,
    pub theme_note: &'static str,
    pub theme_system: &'static str,
    pub theme_light: &'static str,
    pub theme_dark: &'static str,
    pub font: &'static str,
    pub font_placeholder: &'static str,
    pub font_note: &'static str,
    pub font_using: &'static str,
    pub font_missing: &'static str,
    pub font_search_placeholder: &'static str,
    pub font_no_match: &'static str,
    pub section_language: &'static str,
    pub language_summary: &'static str,
    pub language: &'static str,
    pub choose_folder: &'static str,
    pub section_data: &'static str,
    pub data_summary: &'static str,
    pub database: &'static str,
    pub records_one: &'static str,
    pub records_many: &'static str,
    pub log_entries: &'static str,
    pub storage_disabled: &'static str,

    // Engine settings.
    pub section_engine: &'static str,
    pub engine_summary: &'static str,
    pub subsection_concurrency: &'static str,
    pub user_agent: &'static str,
    pub user_agent_placeholder: &'static str,
    pub user_agent_note: &'static str,
    pub connections: &'static str,
    pub connections_note: &'static str,
    pub max_concurrent: &'static str,
    pub max_concurrent_note: &'static str,
    pub speed_limit: &'static str,
    pub speed_limit_note: &'static str,
    pub unlimited: &'static str,
    pub proxy: &'static str,
    pub proxy_placeholder: &'static str,
    pub proxy_note: &'static str,
    pub max_tries: &'static str,
    pub max_tries_note: &'static str,
    pub timeout: &'static str,
    pub timeout_note: &'static str,
    pub engine_note: &'static str,

    // Add-download dialog.
    pub add_title: &'static str,
    pub download_link: &'static str,
    pub save_to: &'static str,
    pub choose: &'static str,
    pub dir_placeholder: &'static str,
    pub file_name: &'static str,
    pub file_name_placeholder: &'static str,
    pub referer: &'static str,
    pub referer_placeholder: &'static str,
    pub start_download: &'static str,
    pub per_download_speed_limit: &'static str,
    pub per_download_note: &'static str,

    // Confirmation dialog.
    pub remove_one: &'static str,
    pub remove_many: &'static str,
    pub one_file_on_disk: &'static str,
    pub many_files_on_disk: &'static str,
    pub cancel: &'static str,
    pub keep_file: &'static str,
    pub delete_file: &'static str,

    // Notices.
    pub bad_uri: &'static str,
    pub engine_unavailable: &'static str,
    pub engine_starting: &'static str,
    pub add_failed: &'static str,
    pub pause_failed: &'static str,
    pub picker_failed: &'static str,
    pub delete_failed: &'static str,
}

pub const EN: Strings = Strings {
    app_name: "Nexus",

    placeholder: "Paste a link or magnet link",
    add: "Add",

    filter_all: "All",
    filter_active: "Active",
    filter_finished: "Finished",
    clear_finished: "Clear finished",
    one_active: "1 active",
    many_active: "active",

    status_active: "Downloading",
    status_waiting: "Waiting",
    status_paused: "Paused",
    status_complete: "Complete",
    status_error: "Failed",

    fetching_metadata: "Fetching metadata…",
    queued: "Queued",
    time_left: "left",
    one_connection: "1 connection",
    many_connections: "connections",
    paused_at: "Paused at",
    download_failed: "Download failed",

    empty_all_title: "Nothing here yet",
    empty_all_hint: "Paste a link above to start",
    empty_active_title: "Nothing in flight",
    empty_active_hint: "Everything queued has finished",
    empty_finished_title: "No finished downloads",
    empty_finished_hint: "Completed downloads will appear here",

    settings_title: "Settings",
    save: "Save",
    section_downloads: "Downloads",
    downloads_summary: "Where files are saved.",
    download_folder: "Download folder",
    open_folder: "Open",
    change: "Change…",
    folder_note: "Applies to new downloads only.",
    section_appearance: "Appearance",
    appearance_summary: "Theme and font.",
    theme: "Theme",
    theme_note: "Follows the Windows light/dark setting.",
    theme_system: "System",
    theme_light: "Light",
    theme_dark: "Dark",
    font: "Font",
    font_placeholder: "\"Times New Roman\", Simsun",
    font_note: "Comma-separated; first installed wins. Quote names with spaces.",
    font_using: "Using: ",
    font_missing: "Not installed: ",
    font_search_placeholder: "Filter fonts…",
    font_no_match: "No matching font",
    section_language: "Language",
    language_summary: "",
    language: "Language",
    choose_folder: "Use this folder",
    section_data: "Data",
    data_summary: "History and logs.",
    database: "History database",
    records_one: "download",
    records_many: "downloads",
    log_entries: "log entries",
    storage_disabled: "Not saving — database could not be opened",

    section_engine: "Download engine",
    engine_summary: "How aria2 fetches each download.",
    subsection_concurrency: "Concurrency & speed",
    user_agent: "User agent",
    user_agent_placeholder: "Empty uses the built-in UA",
    user_agent_note: "Sent as the User-Agent header.",
    connections: "Connections",
    connections_note: "aria2 caps this at 16.",
    max_concurrent: "Parallel downloads",
    max_concurrent_note: "How many run at once.",
    speed_limit: "Speed limit",
    speed_limit_note: "Shared by all downloads.",
    unlimited: "Unlimited",
    proxy: "Proxy",
    proxy_placeholder: "http://127.0.0.1:7890",
    proxy_note: "Empty means a direct connection.",
    max_tries: "Retries",
    max_tries_note: "Attempts before giving up.",
    timeout: "Timeout",
    timeout_note: "Give up after this long with no data.",
    engine_note: "Applies to new downloads; parallel downloads and speed limit are immediate.",

    add_title: "New download",
    download_link: "Link",
    save_to: "Save to",
    choose: "Choose…",
    dir_placeholder: "Default folder if empty",
    file_name: "File name",
    file_name_placeholder: "Server's name if empty",
    referer: "Referer",
    referer_placeholder: "Optional Referer header",
    start_download: "Start",
    per_download_speed_limit: "Speed limit",
    per_download_note: "Applies to this download only.",

    remove_one: "Remove this download?",
    remove_many: "Remove finished downloads?",
    one_file_on_disk: "1 file on disk",
    many_files_on_disk: "files on disk",
    cancel: "Cancel",
    keep_file: "Keep file",
    delete_file: "Delete file",

    bad_uri: "Not a valid link or magnet link",
    engine_unavailable: "Engine unavailable",
    engine_starting: "Engine starting",
    add_failed: "Could not add download",
    pause_failed: "Could not pause",
    picker_failed: "Could not open the folder picker",
    delete_failed: "Could not delete",
};

pub const ZH: Strings = Strings {
    app_name: "Nexus",

    placeholder: "粘贴链接或磁力链接",
    add: "添加",

    filter_all: "全部",
    filter_active: "进行中",
    filter_finished: "已完成",
    clear_finished: "清除已完成",
    one_active: "1 个进行中",
    many_active: "个进行中",

    status_active: "下载中",
    status_waiting: "等待中",
    status_paused: "已暂停",
    status_complete: "已完成",
    status_error: "失败",

    fetching_metadata: "正在获取元数据…",
    queued: "排队中",
    time_left: "剩余",
    one_connection: "1 个连接",
    many_connections: "个连接",
    paused_at: "暂停于",
    download_failed: "下载失败",

    empty_all_title: "还没有任务",
    empty_all_hint: "在上方粘贴链接即可开始",
    empty_active_title: "没有进行中的任务",
    empty_active_hint: "排队的任务都已完成",
    empty_finished_title: "还没有已完成的下载",
    empty_finished_hint: "完成的任务会出现在这里",

    settings_title: "设置",
    save: "保存",
    section_downloads: "下载",
    downloads_summary: "文件的保存位置。",
    download_folder: "下载目录",
    open_folder: "打开",
    change: "更改…",
    folder_note: "仅对新的下载生效。",
    section_appearance: "外观",
    appearance_summary: "主题与字体。",
    theme: "主题",
    theme_note: "跟随 Windows 的明暗设置。",
    theme_system: "系统",
    theme_light: "浅色",
    theme_dark: "深色",
    font: "字体",
    font_placeholder: "\"Times New Roman\", Simsun",
    font_note: "逗号分隔，取第一个已安装的；含空格的字体名加引号。",
    font_using: "正在使用：",
    font_missing: "未安装：",
    font_search_placeholder: "筛选字体…",
    font_no_match: "没有匹配的字体",
    section_language: "语言",
    language_summary: "",
    language: "语言",
    choose_folder: "使用此文件夹",
    section_data: "数据",
    data_summary: "历史与日志。",
    database: "历史数据库",
    records_one: "条下载",
    records_many: "条下载",
    log_entries: "条日志",
    storage_disabled: "未能保存：无法打开数据库",

    section_engine: "下载引擎",
    engine_summary: "aria2 如何获取每个下载。",
    subsection_concurrency: "并发与限速",
    user_agent: "用户代理 (UA)",
    user_agent_placeholder: "留空使用内置 UA",
    user_agent_note: "作为 User-Agent 头发送。",
    connections: "连接数",
    connections_note: "aria2 的上限为 16。",
    max_concurrent: "同时下载数",
    max_concurrent_note: "同时进行的任务数。",
    speed_limit: "全局限速",
    speed_limit_note: "所有任务共享此上限。",
    unlimited: "不限",
    proxy: "代理",
    proxy_placeholder: "http://127.0.0.1:7890",
    proxy_note: "留空表示直连。",
    max_tries: "重试次数",
    max_tries_note: "失败重试的上限。",
    timeout: "连接超时",
    timeout_note: "无数据多久后放弃。",
    engine_note: "对新任务生效；同时下载数与全局限速立即生效。",

    add_title: "新建下载",
    download_link: "链接",
    save_to: "保存到",
    choose: "选择…",
    dir_placeholder: "留空使用默认目录",
    file_name: "文件名",
    file_name_placeholder: "留空用服务器文件名",
    referer: "引用页",
    referer_placeholder: "作为 Referer 头发送",
    start_download: "开始",
    per_download_speed_limit: "限速",
    per_download_note: "仅对本次下载生效。",

    remove_one: "要移除这个下载吗？",
    remove_many: "要移除已完成的下载吗？",
    one_file_on_disk: "1 个文件",
    many_files_on_disk: "个文件",
    cancel: "取消",
    keep_file: "保留文件",
    delete_file: "删除文件",

    bad_uri: "不是有效的链接或磁力链接",
    engine_unavailable: "下载引擎不可用",
    engine_starting: "下载引擎正在启动",
    add_failed: "无法添加下载",
    pause_failed: "无法暂停",
    picker_failed: "无法打开文件夹选择器",
    delete_failed: "无法删除文件",
};

impl Strings {
    pub fn get(language: Language) -> &'static Strings {
        match language {
            Language::En => &EN,
            Language::Zh => &ZH,
        }
    }

    /// "3 connections" / "1 connection", and the languages that just append a counter.
    pub fn connections(&self, count: u64) -> String {
        if count == 1 {
            self.one_connection.to_string()
        } else {
            format!("{count} {}", self.many_connections)
        }
    }

    pub fn active_count(&self, count: u64) -> String {
        if count == 1 {
            self.one_active.to_string()
        } else {
            format!("{count} {}", self.many_active)
        }
    }

    pub fn files_on_disk(&self, count: usize, size: &str) -> String {
        let files = if count == 1 {
            self.one_file_on_disk.to_string()
        } else {
            format!("{count} {}", self.many_files_on_disk)
        };
        if size.is_empty() {
            files
        } else {
            format!("{files} · {size}")
        }
    }

    /// "12 downloads · 40 log entries".
    pub fn records(&self, downloads: i64, events: i64) -> String {
        let downloads = if downloads == 1 {
            format!("1 {}", self.records_one)
        } else {
            format!("{downloads} {}", self.records_many)
        };
        format!("{downloads} · {events} {}", self.log_entries)
    }
}
