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
pub struct App {
    pub buffer: LogBuffer,
    pub filter_input: String,
    pub active_filter: Option<Expr>,
    pub filter_error: Option<String>,
    pub running: bool,
    pub paused: bool,
    pub config: DisplayConfig,
    /// User-customisable color palette for log entries.
    pub palette: crate::config::ColorPalette,
    /// Active preset name (display only; the palette itself is authoritative).
    pub color_preset: crate::config::Preset,
    /// When true, the color config overlay (c key) is shown between filter and log area.
    pub show_colors: bool,
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
    /// When true, the options overlay (o key) is shown between filter and log area.
    pub show_options: bool,
    /// Localised messages.
    pub msgs: &'static crate::i18n::Messages,
}
impl App {
    pub fn new(lang: crate::i18n::Lang) -> Self {
        let (palette, color_preset, config) = crate::config::load();
        Self {
            buffer: LogBuffer::new(),
            filter_input: String::new(),
            active_filter: None,
            filter_error: None,
            running: false,
            paused: false,
            config,
            palette,
            color_preset,
            show_colors: false,
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
            show_options: false,
            msgs: crate::i18n::messages(lang),
        }
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
                    let eff_filter: Option<&Expr> = if self.filter_bypassed { None } else { self.active_filter.as_ref() };
                    self.buffer.push(entry.clone(), eff_filter);
                    // Only format and queue for scrollback if the entry passes the filter.
                    if eff_filter.map_or(true, |f| f.evaluate(&entry)) {
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
            Message::Tick => {
                // Tick is only used for frame rendering; no filter refresh needed.
            }
        }
        true
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

    /// Toggle filter bypass (g key). Keeps filter_input intact.
    pub fn toggle_filter_bypass(&mut self) {
        self.filter_bypassed = !self.filter_bypassed;
        let f = if self.filter_bypassed { None } else { self.active_filter.as_ref() };
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
            13 // title + 11 keybindings + blank line
        } else if self.show_options {
            9 // title + 7 configs + blank line
        } else if self.show_colors {
            12 // title + preset + 8 items + hint + blank line
        } else {
            0
        }
    }

    /// Total viewport height including filter bar and overlay.
    pub fn viewport_height(&self) -> u16 {
        3 + self.overlay_rows()
    }
}
