use std::path::{Path, PathBuf};

use ratatui::style::Color;

use crate::app::DisplayConfig;
use crate::i18n::Messages;
use crate::logcat::LogLevel;

/// The colorable items of a log entry, ordered as shown in the overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorItem {
    Verbose,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
    Tag,
    Timestamp,
}

impl ColorItem {
    pub const ALL: [ColorItem; 8] = [
        ColorItem::Verbose,
        ColorItem::Debug,
        ColorItem::Info,
        ColorItem::Warn,
        ColorItem::Error,
        ColorItem::Fatal,
        ColorItem::Tag,
        ColorItem::Timestamp,
    ];
    /// '1'..='8' map to the items in `ALL` order.
    pub fn from_key(c: char) -> Option<ColorItem> {
        match c {
            '1' => Some(ColorItem::Verbose),
            '2' => Some(ColorItem::Debug),
            '3' => Some(ColorItem::Info),
            '4' => Some(ColorItem::Warn),
            '5' => Some(ColorItem::Error),
            '6' => Some(ColorItem::Fatal),
            '7' => Some(ColorItem::Tag),
            '8' => Some(ColorItem::Timestamp),
            _ => None,
        }
    }
    pub fn label(self, msgs: &Messages) -> &'static str {
        match self {
            ColorItem::Verbose => msgs.color_level_verbose,
            ColorItem::Debug => msgs.color_level_debug,
            ColorItem::Info => msgs.color_level_info,
            ColorItem::Warn => msgs.color_level_warn,
            ColorItem::Error => msgs.color_level_error,
            ColorItem::Fatal => msgs.color_level_fatal,
            ColorItem::Tag => msgs.color_tag,
            ColorItem::Timestamp => msgs.color_timestamp,
        }
    }
    pub fn color(self, p: &ColorPalette) -> Color {
        match self {
            ColorItem::Verbose => p.verbose,
            ColorItem::Debug => p.debug,
            ColorItem::Info => p.info,
            ColorItem::Warn => p.warn,
            ColorItem::Error => p.error,
            ColorItem::Fatal => p.fatal,
            ColorItem::Tag => p.tag,
            ColorItem::Timestamp => p.timestamp,
        }
    }
    pub fn set(self, p: &mut ColorPalette, c: Color) {
        match self {
            ColorItem::Verbose => p.verbose = c,
            ColorItem::Debug => p.debug = c,
            ColorItem::Info => p.info = c,
            ColorItem::Warn => p.warn = c,
            ColorItem::Error => p.error = c,
            ColorItem::Fatal => p.fatal = c,
            ColorItem::Tag => p.tag = c,
            ColorItem::Timestamp => p.timestamp = c,
        }
    }
}

/// Built-in palettes plus `Custom` (user has diverged from a preset).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Default,
    Solarized,
    Monokai,
    HighContrast,
    Custom,
}

/// Hex string → `Color::Rgb`. Optional leading `#`, exactly 6 hex digits, case-insensitive.
pub fn hex_to_color(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(Color::Rgb(
        u8::from_str_radix(&s[0..2], 16).ok()?,
        u8::from_str_radix(&s[2..4], 16).ok()?,
        u8::from_str_radix(&s[4..6], 16).ok()?,
    ))
}

/// `Color::Rgb` → uppercase `#RRGGBB`. Non-Rgb colors (should never occur in a palette) → `#808080`.
pub fn color_to_hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{:02X}{:02X}{:02X}", r, g, b),
        _ => "#808080".to_string(),
    }
}

/// Hex string → `Color::Rgb`; panics on invalid input (preset hexes are known-valid).
fn hex(s: &str) -> Color {
    hex_to_color(s).expect("preset hex is valid")
}

impl Preset {
    pub const ALL: [Preset; 4] = [
        Preset::Default,
        Preset::Solarized,
        Preset::Monokai,
        Preset::HighContrast,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Preset::Default => "default",
            Preset::Solarized => "solarized",
            Preset::Monokai => "monokai",
            Preset::HighContrast => "high-contrast",
            Preset::Custom => "custom",
        }
    }
    /// Unknown id → Default.
    pub fn from_id(s: &str) -> Preset {
        match s {
            "default" => Preset::Default,
            "solarized" => Preset::Solarized,
            "monokai" => Preset::Monokai,
            "high-contrast" => Preset::HighContrast,
            "custom" => Preset::Custom,
            _ => Preset::Default,
        }
    }
    pub fn label(self, msgs: &Messages) -> &'static str {
        match self {
            Preset::Default => msgs.preset_default,
            Preset::Solarized => msgs.preset_solarized,
            Preset::Monokai => msgs.preset_monokai,
            Preset::HighContrast => msgs.preset_highcontrast,
            Preset::Custom => msgs.preset_custom,
        }
    }
    /// Cycling wraps inside `ALL`; from Custom, '[' / ']' start at HighContrast/Default.
    pub fn next(self) -> Preset {
        match self {
            Preset::Default => Preset::Solarized,
            Preset::Solarized => Preset::Monokai,
            Preset::Monokai => Preset::HighContrast,
            Preset::HighContrast => Preset::Default,
            Preset::Custom => Preset::Default,
        }
    }
    pub fn prev(self) -> Preset {
        match self {
            Preset::Default => Preset::HighContrast,
            Preset::Solarized => Preset::Default,
            Preset::Monokai => Preset::Solarized,
            Preset::HighContrast => Preset::Monokai,
            Preset::Custom => Preset::HighContrast,
        }
    }
    /// Overwrite the palette with this preset's colors. `Custom` is a no-op.
    pub fn apply(self, p: &mut ColorPalette) {
        match self {
            Preset::Default => {
                p.verbose = hex("#808080");
                p.debug = hex("#00FFFF");
                p.info = hex("#00FF00");
                p.warn = hex("#FFFF00");
                p.error = hex("#FF0000");
                p.fatal = hex("#FF0000");
                p.tag = hex("#00FFFF");
                p.timestamp = hex("#808080");
            }
            Preset::Solarized => {
                p.verbose = hex("#586E75");
                p.debug = hex("#268BD2");
                p.info = hex("#859900");
                p.warn = hex("#B58900");
                p.error = hex("#DC322F");
                p.fatal = hex("#CB4B16");
                p.tag = hex("#2AA198");
                p.timestamp = hex("#657B83");
            }
            Preset::Monokai => {
                p.verbose = hex("#75715E");
                p.debug = hex("#66D9EF");
                p.info = hex("#A6E22E");
                p.warn = hex("#E6DB74");
                p.error = hex("#F92672");
                p.fatal = hex("#FD971F");
                p.tag = hex("#AE81FF");
                p.timestamp = hex("#75715E");
            }
            Preset::HighContrast => {
                p.verbose = hex("#A9A9A9");
                p.debug = hex("#00BFFF");
                p.info = hex("#32CD32");
                p.warn = hex("#FFD700");
                p.error = hex("#FF3030");
                p.fatal = hex("#FF4500");
                p.tag = hex("#00E5FF");
                p.timestamp = hex("#C0C0C0");
            }
            Preset::Custom => {}
        }
    }
}

/// The full user palette. `Default` reproduces the current hardcoded colors.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorPalette {
    pub verbose: Color,
    pub debug: Color,
    pub info: Color,
    pub warn: Color,
    pub error: Color,
    pub fatal: Color,
    pub tag: Color,
    pub timestamp: Color,
}

impl ColorPalette {
    /// Same colors as the current hardcoded defaults.
    pub fn default() -> Self {
        Self {
            verbose: hex("#808080"),
            debug: hex("#00FFFF"),
            info: hex("#00FF00"),
            warn: hex("#FFFF00"),
            error: hex("#FF0000"),
            fatal: hex("#FF0000"),
            tag: hex("#00FFFF"),
            timestamp: hex("#808080"),
        }
    }
    pub fn level_color(&self, level: LogLevel) -> Color {
        match level {
            LogLevel::Verbose => self.verbose,
            LogLevel::Debug => self.debug,
            LogLevel::Info => self.info,
            LogLevel::Warn => self.warn,
            LogLevel::Error => self.error,
            LogLevel::Fatal => self.fatal,
        }
    }
}

/// Resolve the config file path: `$XDG_CONFIG_HOME` first, else per-OS fallback.
pub fn config_path() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let dir = xdg.or_else(|| {
        #[cfg(target_os = "windows")]
        {
            std::env::var_os("APPDATA").map(PathBuf::from).or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(|u| PathBuf::from(u).join("AppData").join("Roaming"))
            })
        }
        #[cfg(target_os = "macos")]
        {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join("Library").join("Application Support"))
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
        }
    })?;
    Some(dir.join("tlog").join("config.conf"))
}

fn load_from(path: &Path) -> (ColorPalette, Preset, DisplayConfig) {
    let mut p = ColorPalette::default();
    let mut preset = Preset::Default;
    let mut display = DisplayConfig::default();
    let Ok(text) = std::fs::read_to_string(path) else {
        return (p, preset, display);
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        let set_color = |slot: &mut Color, v: &str| {
            if let Some(c) = hex_to_color(v) {
                *slot = c;
            }
        };
        match k {
            "preset" => preset = Preset::from_id(v),
            "verbose" => set_color(&mut p.verbose, v),
            "debug" => set_color(&mut p.debug, v),
            "info" => set_color(&mut p.info, v),
            "warn" => set_color(&mut p.warn, v),
            "error" => set_color(&mut p.error, v),
            "fatal" => set_color(&mut p.fatal, v),
            "tag" => set_color(&mut p.tag, v),
            "timestamp" => set_color(&mut p.timestamp, v),
            "show_timestamp" => set_bool(&mut display.show_timestamp, v),
            "show_pid" => set_bool(&mut display.show_pid, v),
            "show_tid" => set_bool(&mut display.show_tid, v),
            "show_tag" => set_bool(&mut display.show_tag, v),
            "show_level" => set_bool(&mut display.show_level, v),
            "colorize" => set_bool(&mut display.colorize, v),
            _ => {}
        }
    }
    (p, preset, display)
}

/// `"true"`/`"false"` → `Some`; anything else → `None` (keep the default).
fn set_bool(slot: &mut bool, v: &str) {
    match v {
        "true" => *slot = true,
        "false" => *slot = false,
        _ => {}
    }
}

fn save_to(
    path: &Path,
    palette: &ColorPalette,
    preset: &Preset,
    display: &DisplayConfig,
) -> std::io::Result<()> {
    let body = format!(
        "preset = {}\nverbose = {}\ndebug = {}\ninfo = {}\nwarn = {}\nerror = {}\nfatal = {}\ntag = {}\ntimestamp = {}\nshow_timestamp = {}\nshow_pid = {}\nshow_tid = {}\nshow_tag = {}\nshow_level = {}\ncolorize = {}\n",
        preset.id(),
        color_to_hex(palette.verbose),
        color_to_hex(palette.debug),
        color_to_hex(palette.info),
        color_to_hex(palette.warn),
        color_to_hex(palette.error),
        color_to_hex(palette.fatal),
        color_to_hex(palette.tag),
        color_to_hex(palette.timestamp),
        display.show_timestamp,
        display.show_pid,
        display.show_tid,
        display.show_tag,
        display.show_level,
        display.colorize,
    );
    std::fs::write(path, body)
}

pub fn load() -> (ColorPalette, Preset, DisplayConfig) {
    match config_path() {
        Some(p) => load_from(&p),
        None => (ColorPalette::default(), Preset::Default, DisplayConfig::default()),
    }
}

pub fn save(
    palette: &ColorPalette,
    preset: &Preset,
    display: &DisplayConfig,
) -> std::io::Result<()> {
    let Some(p) = config_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory",
        ));
    };
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    save_to(&p, palette, preset, display)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_conf(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tlog-test-{}-{}.conf", std::process::id(), name))
    }

    #[test]
    fn test_hex_roundtrip() {
        assert_eq!(color_to_hex(Color::Rgb(0xFF, 0x88, 0x00)), "#FF8800");
        assert_eq!(
            hex_to_color("#FF8800"),
            Some(Color::Rgb(0xFF, 0x88, 0x00))
        );
        assert_eq!(
            hex_to_color("ff8800"),
            Some(Color::Rgb(0xFF, 0x88, 0x00))
        );
    }

    #[test]
    fn test_hex_invalid() {
        assert_eq!(hex_to_color(""), None);
        assert_eq!(hex_to_color("#12345"), None);
        assert_eq!(hex_to_color("#1234567"), None);
        assert_eq!(hex_to_color("#GGGGGG"), None);
    }

    #[test]
    fn test_preset_apply() {
        let mut p = ColorPalette::default();
        Preset::Monokai.apply(&mut p);
        assert_eq!(p.verbose, Color::Rgb(0x75, 0x71, 0x5E));
        assert_eq!(p.error, Color::Rgb(0xF9, 0x26, 0x72));
        assert_eq!(p.tag, Color::Rgb(0xAE, 0x81, 0xFF));
    }

    #[test]
    fn test_save_load_roundtrip() {
        let path = temp_conf("roundtrip");
        let mut palette = ColorPalette::default();
        Preset::Solarized.apply(&mut palette);
        let preset = Preset::Solarized;
        let mut display = DisplayConfig::default();
        display.show_pid = false;
        display.colorize = false;
        save_to(&path, &palette, &preset, &display).expect("save should succeed");
        let (loaded_p, loaded_s, loaded_d) = load_from(&path);
        assert_eq!(loaded_p, palette);
        assert_eq!(loaded_s, preset);
        assert_eq!(loaded_d, display);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_custom_preset_roundtrip() {
        // A user-edited palette saves as `custom`; it must reload as Custom
        // (not fall back to Default) so the overlay shows the right label.
        let path = temp_conf("custom");
        let mut palette = ColorPalette::default();
        palette.verbose = Color::Rgb(0xFF, 0x88, 0x00);
        let display = DisplayConfig::default();
        save_to(&path, &palette, &Preset::Custom, &display).expect("save should succeed");
        let (loaded_p, loaded_s, loaded_d) = load_from(&path);
        assert_eq!(loaded_p, palette);
        assert_eq!(loaded_s, Preset::Custom);
        assert_eq!(loaded_d, display);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_missing_and_partial() {
        // Nonexistent path → defaults.
        let missing = temp_conf("missing");
        let _ = std::fs::remove_file(&missing);
        let (p, s, d) = load_from(&missing);
        assert_eq!(p, ColorPalette::default());
        assert_eq!(s, Preset::Default);
        assert_eq!(d, DisplayConfig::default());

        // Partial file: preset + one color applied, others default;
        // malformed bool values keep the default.
        let path = temp_conf("partial");
        std::fs::write(
            &path,
            "preset = monokai\nverbose = #112233\nbogus = #ffffff\nshow_pid = false\nshow_tag = bogus\n",
        )
        .expect("write should succeed");
        let (p, s, d) = load_from(&path);
        assert_eq!(s, Preset::Monokai);
        assert_eq!(p.verbose, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(p.debug, ColorPalette::default().debug);
        assert!(!d.show_pid);
        assert!(d.show_tag, "malformed bool keeps the default");
        assert!(d.show_timestamp);
        let _ = std::fs::remove_file(&path);
    }
}
