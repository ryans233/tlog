use std::fmt;

/// Display language for user-facing strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    /// Detect language from environment (LANG / LC_ALL / LC_MESSAGES).
    pub fn detect() -> Self {
        for var in &["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(val) = std::env::var(var) {
                let lower = val.to_lowercase();
                if lower.starts_with("zh") || lower.contains("zh_cn") || lower.contains("zh-tw") {
                    return Lang::Zh;
                }
            }
        }
        Lang::En
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Lang::En => write!(f, "en"),
            Lang::Zh => write!(f, "zh"),
        }
    }
}

/// All localisable user-facing strings.
///
/// Two const instances are provided: [`EN`] and [`ZH`].
/// For messages containing a placeholder, call the corresponding method.
pub struct Messages {
    // ── Startup errors ──────────────────────────────────────────────────
    pub adb_not_found_prefix: &'static str,
    pub adb_not_found_hint: &'static str,
    pub adb_not_found_url: &'static str,

    // ── Status bar ──────────────────────────────────────────────────────
    pub status_bar: &'static str, // "共{}条 | 显示{}条 | 过滤掉{}条 | 丢弃{}条 | {}"
    pub filter_title: &'static str, // " 过滤器 ({}) "
    pub filter_error_prefix: &'static str, // "错误: {}"

    // ── Run-state indicator ─────────────────────────────────────────────
    pub status_running: &'static str,
    pub status_paused: &'static str,
    pub status_stopped: &'static str,

    // ── Filter status labels ────────────────────────────────────────────
    pub filter_bypassed: &'static str,
    pub filter_parse_error: &'static str,
    pub filter_ok: &'static str,
    pub filter_none: &'static str,

    // ── Filter parser errors ────────────────────────────────────────────
    pub parse_error_prefix: &'static str,
    pub missing_close_paren: &'static str,
    pub unexpected_token: &'static str,
    pub age_needs_value: &'static str,
    pub invalid_age_value: &'static str,
    pub invalid_regex: &'static str,
    pub unknown_filter_key: &'static str,
    pub unknown_named_filter: &'static str,
    pub recursive_named_filter: &'static str,

    // ── Search ──────────────────────────────────────────────────────────
    pub search_status: &'static str, // "Search: {}"
    pub search_hint: &'static str,

    // ── Named filter editor ─────────────────────────────────────────────
    pub filter_name_prompt: &'static str,
    pub filter_name_hint: &'static str,
    pub filter_name_invalid_name: &'static str,
    pub filter_name_invalid_expr: &'static str,
    pub filter_name_saved: &'static str, // "Saved filter '{}'"

    // ── Export ──────────────────────────────────────────────────────────
    pub export_saved: &'static str, // "Exported {} lines to {}"
    pub export_empty: &'static str,
    pub export_error_prefix: &'static str,

    // ── Help screen ─────────────────────────────────────────────────────
    pub help_title: &'static str,
    pub help_quit: &'static str,
    pub help_show_help: &'static str,
    pub help_pause: &'static str,
    pub help_clear: &'static str,
    pub help_settings: &'static str,
    pub help_bypass: &'static str,
    pub help_tab: &'static str,
    pub help_esc: &'static str,
    pub help_search: &'static str,
    pub help_export: &'static str,
    pub help_quick_level: &'static str,

    // ── Options screen ──────────────────────────────────────────────────
    pub opts_timestamp: &'static str,
    pub opts_pid: &'static str,
    pub opts_tid: &'static str,
    pub opts_tag: &'static str,
    pub opts_level: &'static str,
    pub opts_color: &'static str,
    pub on_label: &'static str,
    pub off_label: &'static str,

    // ── Settings screen ─────────────────────────────────────────────────
    pub settings_title: &'static str,
    pub settings_category_display: &'static str,
    pub settings_category_color: &'static str,
    pub settings_display_hint: &'static str,

    // ── Color config ────────────────────────────────────────────────────
    pub color_preset_label: &'static str,
    pub color_hint: &'static str,
    pub color_invalid_hex: &'static str,
    pub config_save_error: &'static str,
    pub color_level_verbose: &'static str,
    pub color_level_debug: &'static str,
    pub color_level_info: &'static str,
    pub color_level_warn: &'static str,
    pub color_level_error: &'static str,
    pub color_level_fatal: &'static str,
    pub color_tag: &'static str,
    pub color_timestamp: &'static str,
    pub preset_default: &'static str,
    pub preset_solarized: &'static str,
    pub preset_monokai: &'static str,
    pub preset_highcontrast: &'static str,
    pub preset_custom: &'static str,
}

impl Messages {
    pub fn adb_not_found_full(&self, cmd: &str) -> String {
        format!(
            "{}{}\n{}\n  {}",
            self.adb_not_found_prefix, cmd, self.adb_not_found_hint, self.adb_not_found_url
        )
    }

    pub fn status_bar(
        &self,
        total: usize,
        shown: usize,
        filtered_out: usize,
        dropped: u64,
        filter: &str,
    ) -> String {
        self.status_bar
            .replacen("{}", &total.to_string(), 1)
            .replacen("{}", &shown.to_string(), 1)
            .replacen("{}", &filtered_out.to_string(), 1)
            .replacen("{}", &dropped.to_string(), 1)
            .replacen("{}", filter, 1)
    }

    pub fn filter_title(&self, status: &str) -> String {
        self.filter_title.replacen("{}", status, 1)
    }

    pub fn filter_error(&self, err: &str) -> String {
        self.filter_error_prefix.replacen("{}", err, 1)
    }

    pub fn parse_error(&self, err: &str) -> String {
        self.parse_error_prefix.replacen("{}", err, 1)
    }

    pub fn invalid_age_value(&self, s: &str) -> String {
        self.invalid_age_value.replacen("{}", s, 1)
    }

    pub fn invalid_regex(&self, s: &str) -> String {
        self.invalid_regex.replacen("{}", s, 1)
    }

    pub fn unknown_named_filter(&self, s: &str) -> String {
        self.unknown_named_filter.replacen("{}", s, 1)
    }

    pub fn recursive_named_filter(&self, s: &str) -> String {
        self.recursive_named_filter.replacen("{}", s, 1)
    }

    pub fn search_status(&self, s: &str) -> String {
        self.search_status.replacen("{}", s, 1)
    }

    pub fn filter_name_saved(&self, s: &str) -> String {
        self.filter_name_saved.replacen("{}", s, 1)
    }

    pub fn export_saved(&self, count: usize, path: &str) -> String {
        self.export_saved
            .replacen("{}", &count.to_string(), 1)
            .replacen("{}", path, 1)
    }

    pub fn export_error(&self, s: &str) -> String {
        self.export_error_prefix.replacen("{}", s, 1)
    }
}

pub const EN: Messages = Messages {
    adb_not_found_prefix: "Error: '",
    adb_not_found_hint: "' command not found.\nPlease install Android SDK command-line tools:",
    adb_not_found_url: "https://developer.android.com/tools",

    status_bar: "{} total | {} shown | {} filtered out | {} dropped | {}",
    filter_title: " Filter ({}) ",
    filter_error_prefix: "Error: {}",

    status_running: "RUNNING",
    status_paused: "PAUSED",
    status_stopped: "STOPPED",

    filter_bypassed: "Bypassed",
    filter_parse_error: "Parse error",
    filter_ok: "Filter OK",
    filter_none: "No filter",

    parse_error_prefix: "Parse error: {}",
    missing_close_paren: "Missing closing ')'",
    unexpected_token: "Unexpected token",
    age_needs_value: "age filter needs a value, e.g. age:5m",
    invalid_age_value: "Invalid age value: '{}'",
    invalid_regex: "Invalid regex: {}",
    unknown_filter_key: "Unknown filter key",
    unknown_named_filter: "Unknown saved filter: {}",
    recursive_named_filter: "Recursive or too deeply nested saved filter: {}",

    search_status: "Search: {}",
    search_hint: "Type query | Enter to highlight | Esc to cancel",

    filter_name_prompt: "save as: ",
    filter_name_hint: "Enter save | Esc cancel",
    filter_name_invalid_name: "Filter name cannot be empty or contain tabs/newlines",
    filter_name_invalid_expr: "Filter expression is empty",
    filter_name_saved: "Saved filter '{}'",

    export_saved: "Exported {} lines to {}",
    export_empty: "Nothing to export",
    export_error_prefix: "Export failed: {}",

    help_title: "── Keybindings ───────────────────────────",
    help_quit: "Quit",
    help_show_help: "Show this help",
    help_pause: "Toggle pause/resume",
    help_clear: "Clear log buffer",
    help_settings: "Show settings",
    help_bypass: "Toggle filter bypass",
    help_tab: "Switch focus to filter",
    help_esc: "Cancel filter / return",
    help_search: "Search and highlight",
    help_export: "Export filtered view",
    help_quick_level: "Quick level filter",

    opts_timestamp: "Timestamp",
    opts_pid: "PID",
    opts_tid: "TID",
    opts_tag: "Tag",
    opts_level: "Level",
    opts_color: "Color",
    on_label: "ON ",
    off_label: "OFF",

    settings_title: "── Settings ────────────",
    settings_category_display: "Display",
    settings_category_color: "Colors",
    settings_display_hint: "1-6 toggle | Tab switch | Esc close",

    color_preset_label: "Preset",
    color_hint: "1-8 edit hex | [ ] preset | Tab switch | Esc close",
    color_invalid_hex: "Invalid hex, expected #RRGGBB",
    config_save_error: "Failed to save config file",
    color_level_verbose: "Verbose",
    color_level_debug: "Debug",
    color_level_info: "Info",
    color_level_warn: "Warn",
    color_level_error: "Error",
    color_level_fatal: "Fatal",
    color_tag: "Tag",
    color_timestamp: "Timestamp",
    preset_default: "Default",
    preset_solarized: "Solarized Dark",
    preset_monokai: "Monokai",
    preset_highcontrast: "High Contrast",
    preset_custom: "Custom",
};

pub const ZH: Messages = Messages {
    adb_not_found_prefix: "错误: 找不到 '",
    adb_not_found_hint: "' 命令。\n请先安装 Android SDK 命令行工具:",
    adb_not_found_url: "https://developer.android.com/tools?hl=zh-cn",

    status_bar: "共{}条 | 显示{}条 | 过滤掉{}条 | 丢弃{}条 | {}",
    filter_title: " 过滤器 ({}) ",
    filter_error_prefix: "错误: {}",

    status_running: "运行中",
    status_paused: "已暂停",
    status_stopped: "已停止",

    filter_bypassed: "已旁路",
    filter_parse_error: "解析错误",
    filter_ok: "过滤 OK",
    filter_none: "无过滤器",

    parse_error_prefix: "解析错误: {}",
    missing_close_paren: "缺少右括号 ')'",
    unexpected_token: "意外的 token",
    age_needs_value: "age 过滤器需要值，例如 age:5m",
    invalid_age_value: "无效的 age 值: '{}'",
    invalid_regex: "无效的正则表达式: {}",
    unknown_filter_key: "未知过滤器键",
    unknown_named_filter: "未知的已保存过滤器: {}",
    recursive_named_filter: "已保存过滤器递归过深或存在循环: {}",

    search_status: "搜索: {}",
    search_hint: "输入关键字 | Enter 高亮 | Esc 取消",

    filter_name_prompt: "另存为: ",
    filter_name_hint: "Enter 保存 | Esc 取消",
    filter_name_invalid_name: "过滤器名称不能为空，且不能包含制表符或换行",
    filter_name_invalid_expr: "过滤器表达式为空",
    filter_name_saved: "已保存过滤器 '{}'",

    export_saved: "已导出 {} 条到 {}",
    export_empty: "没有可导出的日志",
    export_error_prefix: "导出失败: {}",

    help_title: "── 快捷键 ──────────────────────────────",
    help_quit: "退出程序",
    help_show_help: "显示此帮助",
    help_pause: "切换暂停/恢复",
    help_clear: "清空日志缓冲区",
    help_settings: "打开设置",
    help_bypass: "临时禁用/恢复过滤器",
    help_tab: "切换焦点到过滤器",
    help_esc: "取消过滤/返回",
    help_search: "搜索并高亮匹配",
    help_export: "导出当前过滤视图",
    help_quick_level: "快速等级过滤",

    opts_timestamp: "时间戳",
    opts_pid: "PID",
    opts_tid: "TID",
    opts_tag: "Tag",
    opts_level: "等级",
    opts_color: "颜色",
    on_label: "ON ",
    off_label: "OFF",

    settings_title: "── 设置 ────────────────",
    settings_category_display: "显示",
    settings_category_color: "颜色",
    settings_display_hint: "1-6 切换 | Tab 切换分类 | Esc 关闭",

    color_preset_label: "预设",
    color_hint: "1-8 编辑颜色 | [ ] 预设 | Tab 切换分类 | Esc 关闭",
    color_invalid_hex: "无效的hex颜色，应为 #RRGGBB",
    config_save_error: "保存配置文件失败",
    color_level_verbose: "冗长",
    color_level_debug: "调试",
    color_level_info: "信息",
    color_level_warn: "警告",
    color_level_error: "错误",
    color_level_fatal: "致命",
    color_tag: "Tag",
    color_timestamp: "时间戳",
    preset_default: "默认",
    preset_solarized: "Solarized Dark",
    preset_monokai: "Monokai",
    preset_highcontrast: "高对比度",
    preset_custom: "自定义",
};

/// Resolve the active language from CLI override or environment.
pub fn resolve(cli_lang: Option<&str>) -> Lang {
    match cli_lang {
        Some("zh") => Lang::Zh,
        Some("en") => Lang::En,
        Some(_) => Lang::detect(),
        None => Lang::detect(),
    }
}

/// Get messages for a given language.
pub fn messages(lang: Lang) -> &'static Messages {
    match lang {
        Lang::En => &EN,
        Lang::Zh => &ZH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_bar_includes_drop_and_filter_stats() {
        let en = EN.status_bar(100, 42, 58, 7, "Filter OK");
        assert_eq!(
            en,
            "100 total | 42 shown | 58 filtered out | 7 dropped | Filter OK"
        );
        let zh = ZH.status_bar(100, 42, 58, 7, "过滤 OK");
        assert_eq!(zh, "共100条 | 显示42条 | 过滤掉58条 | 丢弃7条 | 过滤 OK");
    }
}
