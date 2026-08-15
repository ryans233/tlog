use std::collections::HashMap;

use crate::buffer::LogBuffer;
use crate::filter::{self, Expr};
use crate::logcat::Message;

/// Controls which fields are shown in the log display.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayConfig {
    pub show_timestamp: bool,
    pub show_pid: bool,
    pub show_tid: bool,
    pub show_tag: bool,
    pub show_level: bool,
    pub colorize: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            show_timestamp: true,
            show_pid: true,
            show_tid: true,
            show_tag: true,
            show_level: true,
            colorize: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    LogView,
    FilterInput,
}

/// A tab of the settings screen. Adding a new category only requires
/// extending this enum, `ALL`, and the label match in `ui.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsCategory {
    Display,
    Color,
}

impl SettingsCategory {
    pub const ALL: [SettingsCategory; 2] = [SettingsCategory::Display, SettingsCategory::Color];

    pub fn next(self) -> SettingsCategory {
        match self {
            SettingsCategory::Display => SettingsCategory::Color,
            SettingsCategory::Color => SettingsCategory::Display,
        }
    }

    pub fn prev(self) -> SettingsCategory {
        match self {
            SettingsCategory::Display => SettingsCategory::Color,
            SettingsCategory::Color => SettingsCategory::Display,
        }
    }

    pub fn label(self, msgs: &crate::i18n::Messages) -> &'static str {
        match self {
            SettingsCategory::Display => msgs.settings_category_display,
            SettingsCategory::Color => msgs.settings_category_color,
        }
    }
}
pub struct App {
    pub buffer: LogBuffer,
    pub filter_input: String,
    /// Cursor byte offset inside `filter_input`; always kept on a UTF-8 char boundary.
    pub filter_cursor: usize,
    pub active_filter: Option<Expr>,
    pub filter_error: Option<String>,
    pub running: bool,
    pub paused: bool,
    pub config: DisplayConfig,
    /// User-customisable color palette for log entries.
    pub palette: crate::config::ColorPalette,
    /// Active preset name (display only; the palette itself is authoritative).
    pub color_preset: crate::config::Preset,
    /// Item currently being edited (hex input active), if any.
    pub color_editing: Option<crate::config::ColorItem>,
    /// Hex digits typed so far for the active edit (without '#')
    pub color_input: String,
    /// Color-config error shown in the overlay (invalid hex / save failure).
    pub color_error: Option<String>,
    pub dropped_messages: u64,
    pub focus: Focus,
    /// Maps PID → package name (populated from `adb shell ps` and lifecycle events).
    pub pid_package_map: HashMap<u32, String>,
    /// Log lines waiting to be written into scrollback.
    pub pending_log_lines: Vec<ratatui::text::Line<'static>>,
    /// Set to true when filter changes; triggers scrollback clear+replay.
    pub needs_replay: bool,
    /// When true, the active filter is temporarily disabled (g key toggle).
    pub filter_bypassed: bool,
    /// When true, the help overlay (h key) is shown between filter and log area.
    pub show_help: bool,
    /// When true, the settings overlay (o / c keys) is shown between filter and log area.
    pub show_settings: bool,
    /// Active tab of the settings overlay.
    pub settings_category: SettingsCategory,
    /// Localised messages.
    pub msgs: &'static crate::i18n::Messages,
}
impl App {
    pub fn new(lang: crate::i18n::Lang) -> Self {
        let (palette, color_preset, config) = crate::config::load();
        Self::new_with(lang, palette, color_preset, config)
    }

    fn new_with(
        lang: crate::i18n::Lang,
        palette: crate::config::ColorPalette,
        color_preset: crate::config::Preset,
        config: DisplayConfig,
    ) -> Self {
        Self {
            buffer: LogBuffer::new(),
            filter_input: String::new(),
            filter_cursor: 0,
            active_filter: None,
            filter_error: None,
            running: false,
            paused: false,
            config,
            palette,
            color_preset,
            color_editing: None,
            color_input: String::new(),
            color_error: None,
            dropped_messages: 0,
            focus: Focus::LogView,
            pid_package_map: HashMap::new(),
            pending_log_lines: Vec::new(),
            needs_replay: false,
            filter_bypassed: false,
            show_help: false,
            show_settings: false,
            settings_category: SettingsCategory::Display,
            msgs: crate::i18n::messages(lang),
        }
    }

    /// Test constructor that skips the process-global config path, keeping
    /// tests independent of `XDG_CONFIG_HOME`.
    #[cfg(test)]
    pub(crate) fn new_for_tests(lang: crate::i18n::Lang) -> Self {
        Self::new_with(
            lang,
            crate::config::ColorPalette::default(),
            crate::config::Preset::Default,
            DisplayConfig::default(),
        )
    }

    /// Dispatch a message from the logcat reader or timer.
    /// Returns `false` if the app should exit.
    pub fn dispatch(&mut self, msg: Message) -> bool {
        match msg {
            Message::NewEntry(mut entry) => {
                // Resolve package name from PID→package map
                if entry.package.is_none() {
                    entry.package = self.pid_package_map.get(&entry.pid).cloned();
                }
                if !self.paused {
                    let eff_filter: Option<&Expr> = if self.filter_bypassed {
                        None
                    } else {
                        self.active_filter.as_ref()
                    };
                    self.buffer.push(entry.clone(), eff_filter);
                    // Only format and queue for scrollback if the entry passes the filter.
                    if eff_filter.is_none_or(|f| f.evaluate(&entry)) {
                        let line = crate::ui::format_entry(&entry, &self.config, &self.palette);
                        self.pending_log_lines.push(line);
                    }
                }
            }
            Message::Dropped(count) => {
                self.dropped_messages = count;
            }
            Message::ProcessStarted { pid, package } => {
                self.pid_package_map.insert(pid, package);
            }
            Message::ProcessDied { pid } => {
                self.pid_package_map.remove(&pid);
            }
            Message::LogcatDied => {
                self.running = false;
            }
            Message::LogcatStarted => {
                self.running = true;
            }
            Message::Tick => {
                // Tick is only used for frame rendering; no filter refresh needed.
            }
        }
        true
    }

    /// Merge a `adb shell ps` seed map and backfill buffered entries.
    ///
    /// Called once after the startup seed task completes. If a package-aware
    /// filter is active, rebuild the filtered view so entries that arrived
    /// before the seed finished are re-evaluated with the resolved packages.
    pub fn apply_seed_package_map(&mut self, map: HashMap<u32, String>) {
        if map.is_empty() {
            return;
        }
        self.pid_package_map.extend(map);
        self.buffer.resolve_packages(&self.pid_package_map);
        if !self.filter_bypassed
            && let Some(filter) = self.active_filter.clone()
        {
            self.buffer.rebuild_filtered(Some(&filter));
            self.needs_replay = true;
        }
    }

    /// Apply the current filter input, recompiling and rebuilding.
    pub fn apply_filter(&mut self) {
        if self.filter_input.trim().is_empty() {
            self.active_filter = None;
            self.filter_error = None;
            self.buffer.rebuild_filtered(None);
            self.needs_replay = true;
            return;
        }

        match filter::compile(&self.filter_input, self.msgs) {
            Ok(expr) => {
                self.filter_error = None;
                self.buffer.rebuild_filtered(Some(&expr));
                self.active_filter = Some(expr);
            }
            Err(e) => {
                self.filter_error = Some(e);
                // Keep old filter
            }
        }

        self.needs_replay = true;
    }

    /// Clamp the filter cursor onto the nearest valid UTF-8 char boundary.
    fn clamp_filter_cursor(&mut self) {
        if self.filter_cursor > self.filter_input.len() {
            self.filter_cursor = self.filter_input.len();
        }
        while self.filter_cursor > 0 && !self.filter_input.is_char_boundary(self.filter_cursor) {
            self.filter_cursor -= 1;
        }
    }

    /// Move the filter cursor one character to the left.
    pub fn filter_move_left(&mut self) {
        self.clamp_filter_cursor();
        if self.filter_cursor == 0 {
            return;
        }
        let mut prev = 0;
        for (i, _) in self.filter_input.char_indices() {
            if i >= self.filter_cursor {
                break;
            }
            prev = i;
        }
        self.filter_cursor = prev;
    }

    /// Move the filter cursor one character to the right.
    pub fn filter_move_right(&mut self) {
        self.clamp_filter_cursor();
        if self.filter_cursor == self.filter_input.len() {
            return;
        }
        if let Some(c) = self.filter_input[self.filter_cursor..].chars().next() {
            self.filter_cursor += c.len_utf8();
        }
    }

    /// Move the filter cursor to the start (also used by the Up arrow).
    pub fn filter_move_start(&mut self) {
        self.filter_cursor = 0;
    }

    /// Move the filter cursor to the end (also used by the Down arrow).
    pub fn filter_move_end(&mut self) {
        self.filter_cursor = self.filter_input.len();
    }

    /// Insert a character at the filter cursor.
    pub fn filter_insert_char(&mut self, c: char) {
        self.clamp_filter_cursor();
        self.filter_input.insert(self.filter_cursor, c);
        self.filter_cursor += c.len_utf8();
    }

    /// Delete the character before the filter cursor.
    pub fn filter_backspace(&mut self) {
        self.clamp_filter_cursor();
        if self.filter_cursor == 0 {
            return;
        }
        let mut prev = 0;
        for (i, _) in self.filter_input.char_indices() {
            if i >= self.filter_cursor {
                break;
            }
            prev = i;
        }
        self.filter_input.drain(prev..self.filter_cursor);
        self.filter_cursor = prev;
    }

    /// Delete the character at the filter cursor.
    pub fn filter_delete(&mut self) {
        self.clamp_filter_cursor();
        if self.filter_cursor == self.filter_input.len() {
            return;
        }
        let next = self.filter_input[self.filter_cursor..]
            .chars()
            .next()
            .map(|c| self.filter_cursor + c.len_utf8())
            .unwrap_or(self.filter_input.len());
        self.filter_input.drain(self.filter_cursor..next);
    }

    /// Clear the filter input and reset the cursor.
    pub fn filter_clear(&mut self) {
        self.filter_input.clear();
        self.filter_cursor = 0;
    }

    /// Toggle filter bypass (g key). Keeps filter_input intact.
    pub fn toggle_filter_bypass(&mut self) {
        self.filter_bypassed = !self.filter_bypassed;
        let f = if self.filter_bypassed {
            None
        } else {
            self.active_filter.as_ref()
        };
        self.buffer.rebuild_filtered(f);
        self.needs_replay = true;
    }

    pub fn toggle_config(&mut self, key: char) {
        match key {
            '1' => self.config.show_timestamp = !self.config.show_timestamp,
            '2' => self.config.show_pid = !self.config.show_pid,
            '3' => self.config.show_tid = !self.config.show_tid,
            '4' => self.config.show_tag = !self.config.show_tag,
            '5' => self.config.show_level = !self.config.show_level,
            '6' => self.config.colorize = !self.config.colorize,
            _ => return,
        }
        self.needs_replay = true; // scrollback must re-render with the new display options
        self.save_config();
    }

    /// Persist palette, preset, and display config to the user config file.
    /// Save failures degrade gracefully: the error is surfaced in the color
    /// overlay, nothing crashes, and the in-session state stays live.
    pub fn save_config(&mut self) {
        if crate::config::save(&self.palette, &self.color_preset, &self.config).is_err() {
            self.color_error = Some(self.msgs.config_save_error.to_string());
        }
    }

    /// Number of overlay rows to show above the filter bar (0 if none).
    pub fn overlay_rows(&self) -> u16 {
        if self.show_help {
            10 // title + 8 keybindings + blank line
        } else if self.show_settings {
            match self.settings_category {
                SettingsCategory::Display => 10, // title + category + 6 toggles + hint + blank
                SettingsCategory::Color => 13, // title + category + preset + 8 items + hint + blank
            }
        } else {
            0
        }
    }

    /// Total viewport height including filter bar and overlay.
    pub fn viewport_height(&self) -> u16 {
        3 + self.overlay_rows()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_cursor_moves_and_inserts() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.filter_insert_char('a');
        app.filter_insert_char('b');
        app.filter_insert_char('c');
        assert_eq!(app.filter_input, "abc");
        assert_eq!(app.filter_cursor, 3);

        app.filter_move_left();
        assert_eq!(app.filter_cursor, 2);
        app.filter_insert_char('X');
        assert_eq!(app.filter_input, "abXc");
        assert_eq!(app.filter_cursor, 3);

        app.filter_move_start();
        app.filter_move_right();
        assert_eq!(app.filter_cursor, 1);
        app.filter_move_end();
        assert_eq!(app.filter_cursor, 4);
    }

    #[test]
    fn test_filter_backspace_and_delete_respect_cursor() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.filter_insert_char('a');
        app.filter_insert_char('b');
        app.filter_insert_char('c');

        app.filter_move_left();
        app.filter_backspace();
        assert_eq!(app.filter_input, "ac");
        assert_eq!(app.filter_cursor, 1);

        app.filter_move_start();
        app.filter_delete();
        assert_eq!(app.filter_input, "c");
        assert_eq!(app.filter_cursor, 0);
    }

    #[test]
    fn test_filter_cursor_handles_utf8() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        for c in "héllo".chars() {
            app.filter_insert_char(c);
        }
        assert_eq!(app.filter_input, "héllo");
        assert_eq!(app.filter_cursor, "héllo".len());

        app.filter_move_left();
        assert_eq!(app.filter_cursor, 5); // before the final 'o'
        app.filter_move_right();
        assert_eq!(app.filter_cursor, 6);
        app.filter_backspace();
        assert_eq!(app.filter_input, "héll");
        assert_eq!(app.filter_cursor, 5);

        app.filter_move_start();
        app.filter_move_right();
        assert_eq!(app.filter_cursor, 1); // after 'h', before the 2-byte 'é'
        app.filter_delete();
        assert_eq!(app.filter_input, "hll");
    }

    #[test]
    fn test_filter_clear_resets_cursor() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.filter_insert_char('a');
        app.filter_clear();
        assert_eq!(app.filter_input, "");
        assert_eq!(app.filter_cursor, 0);
    }
}
