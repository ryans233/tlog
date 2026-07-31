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
    pub status_bar: &'static str,       // "共{}条 | 显示{}条 | {}"
    pub filter_title: &'static str,      // " 过滤器 ({}) "
    pub filter_error_prefix: &'static str, // "错误: {}"

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

    pub fn status_bar(&self, total: usize, shown: usize, filter: &str) -> String {
        self.status_bar
            .replacen("{}", &total.to_string(), 1)
            .replacen("{}", &shown.to_string(), 1)
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

}

pub const EN: Messages = Messages {
    adb_not_found_prefix: "Error: '",
    adb_not_found_hint: "' command not found.\nPlease install Android SDK command-line tools:",
    adb_not_found_url: "https://developer.android.com/tools",

    status_bar: "{} total | {} shown | {}",
    filter_title: " Filter ({}) ",
    filter_error_prefix: "Error: {}",

    filter_bypassed: "Bypassed",
    filter_parse_error: "Parse error",
    filter_ok: "Filter OK",
    filter_none: "No filter",

    parse_error_prefix: "Parse error: {}",
    missing_close_paren: "Missing closing ')'",
    unexpected_token: "Unexpected token",
    age_needs_value: "age filter needs a value, e.g. age:5m",
    invalid_age_value: "Invalid age value: '{}'",

    help_title: "── Keybindings ───────────────────────────",
    help_quit: "Quit",
    help_show_help: "Show this help",
    help_pause: "Toggle pause/resume",
    help_clear: "Clear log buffer",
    help_settings: "Show settings",
    help_bypass: "Toggle filter bypass",
    help_tab: "Switch focus to filter",
    help_esc: "Cancel filter / return",

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

    status_bar: "共{}条 | 显示{}条 | {}",
    filter_title: " 过滤器 ({}) ",
    filter_error_prefix: "错误: {}",

    filter_bypassed: "已旁路",
    filter_parse_error: "解析错误",
    filter_ok: "过滤 OK",
    filter_none: "无过滤器",

    parse_error_prefix: "解析错误: {}",
    missing_close_paren: "缺少右括号 ')'",
    unexpected_token: "意外的 token",
    age_needs_value: "age 过滤器需要值，例如 age:5m",
    invalid_age_value: "无效的 age 值: '{}'",

    help_title: "── 快捷键 ──────────────────────────────",
    help_quit: "退出程序",
    help_show_help: "显示此帮助",
    help_pause: "切换暂停/恢复",
    help_clear: "清空日志缓冲区",
    help_settings: "打开设置",
    help_bypass: "临时禁用/恢复过滤器",
    help_tab: "切换焦点到过滤器",
    help_esc: "取消过滤/返回",

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
