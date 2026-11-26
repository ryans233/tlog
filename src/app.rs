use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use regex::Regex;
use tokio::sync::mpsc;

use crate::buffer::LogBuffer;
use crate::filter::{self, Expr};
use crate::logcat::Message;

/// Capacity of the feed→UI message channel. Sized to absorb short log bursts
/// while bounding the worst-case amount of queued entries.
pub const MSG_CHANNEL_CAPACITY: usize = 8192;

/// Maximum messages dispatched per event-loop iteration. Draining in batches
/// amortizes one scrollback insert and one viewport draw over a whole burst
/// instead of paying both per line.
pub const MSG_DRAIN_BATCH: usize = 512;

/// Outcome of one burst-drain pass over the feed channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedDrain {
    /// The channel may still have messages; keep polling.
    Continue,
    /// The feed is gone and will not reconnect; stop polling the channel.
    Closed,
    /// The UI must exit (`--exit-on-eof`, or offline playback reached EOF).
    Exit,
}

/// Maximum number of applied filter expressions kept for Up/Down history.
const MAX_FILTER_HISTORY: usize = 100;

/// How long a one-shot status notice (export/save result) stays visible.
const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);

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
    SearchInput,
    FilterName,
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
    /// Successfully applied filter expressions, oldest first (Up/Down history).
    pub filter_history: Vec<String>,
    /// `Some(i)` while navigating `filter_history`; `None` while editing freely.
    pub history_index: Option<usize>,
    /// Draft preserved when Up-arrow navigation starts, restored by Down-arrow.
    pub history_draft: String,
    /// Search query currently being typed (`/` mode).
    pub search_input: String,
    pub search_cursor: usize,
    /// Active search query for display (empty search clears highlighting).
    pub search_query: Option<String>,
    /// Case-insensitive matcher compiled once when the search is applied.
    pub search_regex: Option<Regex>,
    /// Saved named filters (name → filter expression), persisted next to config.
    pub named_filters: BTreeMap<String, String>,
    /// Name currently being typed in the save-filter-as mode.
    pub name_input: String,
    pub name_cursor: usize,
    /// Filter expression being saved by the save-filter-as mode.
    pub name_target: String,
    /// One-shot notice shown below the filter bar (export/save results).
    ///
    /// Cleared when [`Self::expire_status_message`] passes
    /// `status_message_deadline`, or by the input modes that replace it.
    pub status_message: Option<String>,
    /// When `status_message` stops being shown.
    pub status_message_deadline: Option<Instant>,
    pub status_is_error: bool,
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
            filter_history: Vec::new(),
            history_index: None,
            history_draft: String::new(),
            search_input: String::new(),
            search_cursor: 0,
            search_query: None,
            search_regex: None,
            named_filters: crate::named_filters::load(),
            name_input: String::new(),
            name_cursor: 0,
            name_target: String::new(),
            status_message: None,
            status_message_deadline: None,
            status_is_error: false,
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
        let mut app = Self::new_with(
            lang,
            crate::config::ColorPalette::default(),
            crate::config::Preset::Default,
            DisplayConfig::default(),
        );
        // Keep tests independent of the user's saved filters file.
        app.named_filters.clear();
        app
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
                    // `push` evaluates the filter once and returns the entry's
                    // position in the filtered view, so the line is formatted
                    // from the buffered entry instead of a clone.
                    if let Some(pos) = self.buffer.push(entry, eff_filter)
                        && let Some((_, entry)) = self.buffer.get_filtered(pos)
                    {
                        let line = crate::ui::format_entry(
                            entry,
                            &self.config,
                            &self.palette,
                            self.search_regex.as_ref(),
                        );
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
                // Tick drives frame rendering and expires one-shot notices.
                // Filter cutoffs are deliberately not refreshed (see `age:`).
                self.expire_status_message(Instant::now());
            }
        }
        true
    }

    /// Dispatch up to `batch` queued feed messages, starting with `first`.
    ///
    /// `first` is the message already received by the event loop's `select!`;
    /// `None` means the channel closed. Consuming a whole burst in one pass
    /// lets the caller write one scrollback batch and draw one frame for it,
    /// instead of paying a flush and a draw per log line.
    ///
    /// Messages are dispatched in channel order. `exit_on_eof` is the CLI/offline
    /// flag; it only takes effect when the message is `LogcatDied`.
    pub fn drain_feed(
        &mut self,
        rx: &mut mpsc::Receiver<Message>,
        first: Option<Message>,
        batch: usize,
        exit_on_eof: bool,
        reconnect: bool,
    ) -> FeedDrain {
        let mut next = first;
        let mut drained = 0usize;
        loop {
            match next {
                Some(message) => {
                    let feed_died = matches!(message, Message::LogcatDied);
                    self.dispatch(message);
                    if feed_died {
                        if exit_on_eof {
                            return FeedDrain::Exit;
                        }
                        if !reconnect {
                            return FeedDrain::Closed;
                        }
                    }
                }
                None => {
                    self.running = false;
                    return FeedDrain::Closed;
                }
            }
            drained += 1;
            if drained >= batch {
                return FeedDrain::Continue;
            }
            next = match rx.try_recv() {
                Ok(message) => Some(message),
                Err(mpsc::error::TryRecvError::Empty) => return FeedDrain::Continue,
                Err(mpsc::error::TryRecvError::Disconnected) => None,
            };
        }
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

        match filter::compile_with_named(&self.filter_input, self.msgs, &self.named_filters) {
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

    /// Move the filter cursor one character to the left.
    pub fn filter_move_left(&mut self) {
        input_move_left(&self.filter_input, &mut self.filter_cursor);
    }

    /// Move the filter cursor one character to the right.
    pub fn filter_move_right(&mut self) {
        input_move_right(&self.filter_input, &mut self.filter_cursor);
    }

    /// Move the filter cursor to the start (Home / Ctrl+A).
    pub fn filter_move_start(&mut self) {
        self.filter_cursor = 0;
    }

    /// Move the filter cursor to the end (End / Ctrl+E).
    pub fn filter_move_end(&mut self) {
        self.filter_cursor = self.filter_input.len();
    }

    /// Insert a character at the filter cursor.
    pub fn filter_insert_char(&mut self, c: char) {
        input_insert_char(&mut self.filter_input, &mut self.filter_cursor, c);
        self.note_filter_edited();
    }

    /// Delete the character before the filter cursor.
    pub fn filter_backspace(&mut self) {
        input_backspace(&mut self.filter_input, &mut self.filter_cursor);
        self.note_filter_edited();
    }

    /// Delete the character at the filter cursor.
    pub fn filter_delete(&mut self) {
        input_delete(&mut self.filter_input, &mut self.filter_cursor);
        self.note_filter_edited();
    }

    /// Delete the whitespace + word immediately before the filter cursor (Ctrl+W).
    pub fn filter_delete_word_before(&mut self) {
        input_delete_word_before(&mut self.filter_input, &mut self.filter_cursor);
        self.note_filter_edited();
    }

    /// Clear the filter input and reset the cursor.
    pub fn filter_clear(&mut self) {
        self.filter_input.clear();
        self.filter_cursor = 0;
        self.note_filter_edited();
    }

    /// Any manual edit exits history navigation so Down can later restore
    /// the pre-navigation draft exactly once.
    fn note_filter_edited(&mut self) {
        self.history_index = None;
    }

    /// Record an applied non-empty filter for Up/Down history (dedupes the
    /// most recent entry only, mirroring shell behaviour).
    pub fn record_filter_history(&mut self) {
        let input = self.filter_input.trim();
        if input.is_empty() {
            self.history_index = None;
            return;
        }
        let input = input.to_string();
        if self.filter_history.last() != Some(&input) {
            self.filter_history.push(input);
            if self.filter_history.len() > MAX_FILTER_HISTORY {
                let excess = self.filter_history.len() - MAX_FILTER_HISTORY;
                self.filter_history.drain(0..excess);
            }
        }
        self.history_index = None;
    }

    /// Move to the previous applied filter expression (Up arrow).
    pub fn filter_history_up(&mut self) {
        if self.filter_history.is_empty() {
            return;
        }
        let next = match self.history_index {
            None => {
                self.history_draft = self.filter_input.clone();
                self.filter_history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.restore_history_entry(next);
    }

    /// Move to the next expression, restoring the draft after the newest one.
    pub fn filter_history_down(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.filter_history.len() {
            self.restore_history_entry(index + 1);
        } else {
            self.filter_input = std::mem::take(&mut self.history_draft);
            self.filter_cursor = self.filter_input.len();
            self.history_index = None;
        }
    }

    fn restore_history_entry(&mut self, index: usize) {
        self.filter_input = self.filter_history[index].clone();
        self.filter_cursor = self.filter_input.len();
        self.history_index = Some(index);
    }

    /// Open search mode with the current query pre-filled.
    pub fn begin_search(&mut self) {
        self.search_input = self.search_query.clone().unwrap_or_default();
        self.search_cursor = self.search_input.len();
        let closing_help = self.show_help;
        self.show_help = false;
        self.status_message = None;
        self.status_message_deadline = None;
        self.status_is_error = false;
        // Closing the help overlay changes the viewport height; re-home the
        // bottom bar before the search query is applied. When help was already
        // closed there is nothing to repaint yet.
        self.needs_replay |= closing_help;
        self.focus = Focus::SearchInput;
    }

    /// Apply the search query and return to the log view.
    ///
    /// An empty query clears highlighting. The query is escaped and wrapped in
    /// `(?i)`, so it is always a valid case-insensitive literal regex.
    pub fn apply_search(&mut self) {
        let query = self.search_input.trim();
        if query.is_empty() {
            self.search_query = None;
            self.search_regex = None;
        } else {
            let pattern = format!("(?i){}", regex::escape(query));
            let compiled = Regex::new(&pattern).expect("regex::escape output must always compile");
            self.search_query = Some(query.to_string());
            self.search_regex = Some(compiled);
        }
        self.needs_replay = true;
    }

    pub fn search_move_left(&mut self) {
        input_move_left(&self.search_input, &mut self.search_cursor);
    }

    pub fn search_move_right(&mut self) {
        input_move_right(&self.search_input, &mut self.search_cursor);
    }

    pub fn search_move_start(&mut self) {
        self.search_cursor = 0;
    }

    pub fn search_move_end(&mut self) {
        self.search_cursor = self.search_input.len();
    }

    pub fn search_insert_char(&mut self, c: char) {
        input_insert_char(&mut self.search_input, &mut self.search_cursor, c);
    }

    pub fn search_backspace(&mut self) {
        input_backspace(&mut self.search_input, &mut self.search_cursor);
    }

    pub fn search_delete(&mut self) {
        input_delete(&mut self.search_input, &mut self.search_cursor);
    }

    pub fn search_delete_word_before(&mut self) {
        input_delete_word_before(&mut self.search_input, &mut self.search_cursor);
    }

    pub fn search_clear(&mut self) {
        self.search_input.clear();
        self.search_cursor = 0;
    }

    /// Open the save-filter-as prompt for the current filter input.
    pub fn begin_filter_save(&mut self) {
        self.name_target = self.filter_input.clone();
        self.name_input.clear();
        self.name_cursor = 0;
        self.status_message = None;
        self.status_message_deadline = None;
        self.status_is_error = false;
        self.focus = Focus::FilterName;
    }

    /// Validate and persist the filter currently targeted by the name prompt.
    ///
    /// On success the status line reports the saved name; on failure the user
    /// stays in the name prompt with the reason shown in the status line.
    pub fn save_named_filter(&mut self) {
        let name = self.name_input.trim().to_string();
        let expr = self.name_target.trim().to_string();
        if name.is_empty() || name.contains(['\t', '\n', '\r']) {
            self.set_status(self.msgs.filter_name_invalid_name.to_string(), true);
            return;
        }
        if expr.is_empty() {
            self.set_status(self.msgs.filter_name_invalid_expr.to_string(), true);
            return;
        }
        self.named_filters.insert(name.clone(), expr);
        match crate::named_filters::save(&self.named_filters) {
            Ok(()) => self.set_status(self.msgs.filter_name_saved(&name), false),
            Err(e) => self.set_status(format!("{}: {}", self.msgs.config_save_error, e), true),
        }
    }

    pub fn name_move_left(&mut self) {
        input_move_left(&self.name_input, &mut self.name_cursor);
    }

    pub fn name_move_right(&mut self) {
        input_move_right(&self.name_input, &mut self.name_cursor);
    }

    pub fn name_move_start(&mut self) {
        self.name_cursor = 0;
    }

    pub fn name_move_end(&mut self) {
        self.name_cursor = self.name_input.len();
    }

    pub fn name_insert_char(&mut self, c: char) {
        input_insert_char(&mut self.name_input, &mut self.name_cursor, c);
    }

    pub fn name_backspace(&mut self) {
        input_backspace(&mut self.name_input, &mut self.name_cursor);
    }

    pub fn name_delete(&mut self) {
        input_delete(&mut self.name_input, &mut self.name_cursor);
    }

    pub fn name_delete_word_before(&mut self) {
        input_delete_word_before(&mut self.name_input, &mut self.name_cursor);
    }

    pub fn name_clear(&mut self) {
        self.name_input.clear();
        self.name_cursor = 0;
    }

    /// One-key level filter (`v`/`d`/`i`/`w`/`e`).
    ///
    /// Appends `level:X` with an explicit AND, so pressing several keys
    /// narrows to the strictest selected level. The exact token is deduped.
    pub fn quick_level(&mut self, level: char) {
        let token = format!("level:{}", level.to_ascii_uppercase());
        if self
            .filter_input
            .split_whitespace()
            .any(|part| part == token)
        {
            return;
        }
        let trimmed = self.filter_input.trim();
        if trimmed.is_empty() {
            self.filter_input = token;
        } else {
            self.filter_input = format!("{} & {}", trimmed, token);
        }
        self.filter_cursor = self.filter_input.len();
        self.apply_filter();
        self.record_filter_history();
    }

    /// Export the current filtered view to a timestamped file in the CWD.
    pub fn export_filtered(&mut self) {
        if self.buffer.filtered_len() == 0 {
            self.set_status(self.msgs.export_empty.to_string(), false);
            return;
        }
        let base = crate::export::default_export_path();
        let path = crate::export::available_export_path(&base);
        match crate::export::write_filtered(&self.buffer, &path) {
            Ok(count) => self.set_status(
                self.msgs.export_saved(count, &path.display().to_string()),
                false,
            ),
            Err(e) => self.set_status(self.msgs.export_error(&e.to_string()), true),
        }
    }

    fn set_status(&mut self, message: String, is_error: bool) {
        self.status_message = Some(message);
        self.status_is_error = is_error;
        self.status_message_deadline = Some(Instant::now() + STATUS_MESSAGE_TTL);
    }

    /// Clear the one-shot status notice once its TTL has elapsed.
    ///
    /// Takes `now` explicitly so the behaviour is testable without sleeping.
    pub fn expire_status_message(&mut self, now: Instant) {
        if let Some(deadline) = self.status_message_deadline
            && now >= deadline
        {
            self.status_message = None;
            self.status_message_deadline = None;
            self.status_is_error = false;
        }
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
            13 // title + 11 keybindings + blank line
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

/// Floor a cursor to the nearest UTF-8 char boundary at or before it.
fn clamp_input_cursor(input: &str, cursor: &mut usize) {
    if *cursor > input.len() {
        *cursor = input.len();
    }
    while *cursor > 0 && !input.is_char_boundary(*cursor) {
        *cursor -= 1;
    }
}

/// Byte offset of the char immediately before `cursor`.
fn input_move_left(input: &str, cursor: &mut usize) {
    clamp_input_cursor(input, cursor);
    if *cursor == 0 {
        return;
    }
    let mut prev = 0;
    for (i, _) in input.char_indices() {
        if i >= *cursor {
            break;
        }
        prev = i;
    }
    *cursor = prev;
}

/// Move the cursor one UTF-8 character to the right.
fn input_move_right(input: &str, cursor: &mut usize) {
    clamp_input_cursor(input, cursor);
    if *cursor == input.len() {
        return;
    }
    if let Some(c) = input[*cursor..].chars().next() {
        *cursor += c.len_utf8();
    }
}

/// Insert a char at the cursor, keeping the cursor on a char boundary.
fn input_insert_char(input: &mut String, cursor: &mut usize, c: char) {
    clamp_input_cursor(input, cursor);
    input.insert(*cursor, c);
    *cursor += c.len_utf8();
}

/// Delete the char immediately before the cursor.
fn input_backspace(input: &mut String, cursor: &mut usize) {
    clamp_input_cursor(input, cursor);
    if *cursor == 0 {
        return;
    }
    let mut prev = 0;
    for (i, _) in input.char_indices() {
        if i >= *cursor {
            break;
        }
        prev = i;
    }
    input.drain(prev..*cursor);
    *cursor = prev;
}

/// Delete the char at the cursor.
fn input_delete(input: &mut String, cursor: &mut usize) {
    clamp_input_cursor(input, cursor);
    if *cursor == input.len() {
        return;
    }
    let next = input[*cursor..]
        .chars()
        .next()
        .map(|c| *cursor + c.len_utf8())
        .unwrap_or(input.len());
    input.drain(*cursor..next);
}

/// Readline-style Ctrl+W: delete whitespace, then the word before the cursor.
///
/// Cursor lands at the start of the removed run. If the cursor is at 0 this is
/// a no-op.
fn input_delete_word_before(input: &mut String, cursor: &mut usize) {
    clamp_input_cursor(input, cursor);
    if *cursor == 0 {
        return;
    }
    let mut start = *cursor;
    let mut chars = input[..*cursor].char_indices().rev().peekable();
    while let Some((idx, c)) = chars.peek().copied() {
        if c.is_whitespace() {
            start = idx;
            chars.next();
        } else {
            break;
        }
    }
    for (idx, c) in chars {
        if c.is_whitespace() {
            start = idx + c.len_utf8();
            break;
        }
        start = idx;
    }
    input.drain(start..*cursor);
    *cursor = start;
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

    #[test]
    fn test_filter_delete_word_before_is_readline_style() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        for c in "tag:Foo bar".chars() {
            app.filter_insert_char(c);
        }
        app.filter_delete_word_before();
        assert_eq!(app.filter_input, "tag:Foo ");
        assert_eq!(app.filter_cursor, "tag:Foo ".len());

        // Deleting again removes the trailing whitespace + the previous
        // whitespace-delimited token.
        app.filter_delete_word_before();
        assert_eq!(app.filter_input, "");
        assert_eq!(app.filter_cursor, 0);
    }

    #[test]
    fn test_filter_history_navigates_and_restores_draft() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.filter_input = "tag:A".to_string();
        app.record_filter_history();
        app.filter_input = "tag:B".to_string();
        app.record_filter_history();
        app.filter_input = "tag:C".to_string();
        app.record_filter_history();
        assert_eq!(app.filter_history, ["tag:A", "tag:B", "tag:C"]);

        // Up enters history from the newest entry; the draft is preserved.
        app.filter_history_up();
        assert_eq!(app.filter_input, "tag:C");
        app.filter_history_up();
        assert_eq!(app.filter_input, "tag:B");
        app.filter_history_up();
        assert_eq!(app.filter_input, "tag:A");
        // At the oldest entry another Up is a no-op.
        app.filter_history_up();
        assert_eq!(app.filter_input, "tag:A");

        // Down walks forward, then restores the pre-navigation draft.
        app.filter_history_down();
        assert_eq!(app.filter_input, "tag:B");
        app.filter_history_down();
        assert_eq!(app.filter_input, "tag:C");
        app.filter_history_down();
        assert_eq!(app.filter_input, "tag:C");
        assert_eq!(app.history_index, None);

        // Manual editing aborts history navigation.
        app.filter_history_up();
        assert_eq!(app.history_index, Some(2));
        app.filter_insert_char('X');
        assert_eq!(app.history_index, None);
    }

    #[test]
    fn test_filter_history_is_bounded() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        for i in 0..(MAX_FILTER_HISTORY + 5) {
            app.filter_input = format!("tag:F{i}");
            app.record_filter_history();
        }
        assert_eq!(app.filter_history.len(), MAX_FILTER_HISTORY);
        assert_eq!(app.filter_history[0], "tag:F5");
        assert_eq!(
            app.filter_history.last().unwrap(),
            &format!("tag:F{}", MAX_FILTER_HISTORY + 4)
        );
    }

    #[test]
    fn test_quick_level_appends_dedupes_and_applies() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.quick_level('e');
        assert_eq!(app.filter_input, "level:E");
        assert!(app.active_filter.is_some());
        assert_eq!(app.filter_history, ["level:E"]);

        // A second level is ANDed; a duplicate of the same level is ignored.
        app.quick_level('w');
        assert_eq!(app.filter_input, "level:E & level:W");
        app.quick_level('e');
        assert_eq!(app.filter_input, "level:E & level:W");

        // Existing filters are preserved with explicit AND.
        app.filter_input = "tag:Main".to_string();
        app.quick_level('i');
        assert_eq!(app.filter_input, "tag:Main & level:I");
    }

    #[test]
    fn test_search_apply_is_case_insensitive_literal() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.search_input = "Main(Activity)".to_string();
        app.apply_search();
        let re = app.search_regex.as_ref().expect("search regex compiled");
        assert!(re.is_match("prefix MAIN(ACTIVITY) suffix"));
        assert!(!re.is_match("MainActivity"));
        assert!(app.needs_replay);

        // Empty query clears the search.
        app.search_input.clear();
        app.apply_search();
        assert!(app.search_regex.is_none());
        assert!(app.search_query.is_none());
    }

    #[test]
    fn test_begin_search_prefills_current_query() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.show_help = true;
        app.search_query = Some("oom".to_string());
        app.begin_search();
        assert_eq!(app.focus, Focus::SearchInput);
        assert_eq!(app.search_input, "oom");
        assert_eq!(app.search_cursor, 3);
        assert!(!app.show_help);
    }

    #[test]
    fn test_save_named_filter_rejects_empty_expression_without_io() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.begin_filter_save();
        app.name_input = "errors".to_string();
        app.save_named_filter();
        assert!(app.status_is_error);
        assert_eq!(
            app.status_message.as_deref(),
            Some(app.msgs.filter_name_invalid_expr)
        );
        assert!(app.named_filters.is_empty());
    }

    #[test]
    fn test_status_message_expires_after_ttl() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.set_status("exported".to_string(), false);
        assert_eq!(app.status_message.as_deref(), Some("exported"));

        // Not due yet: the notice stays visible.
        app.expire_status_message(Instant::now());
        assert!(app.status_message.is_some());

        app.expire_status_message(Instant::now() + STATUS_MESSAGE_TTL + Duration::from_millis(1));
        assert!(app.status_message.is_none());
        assert!(app.status_message_deadline.is_none());
        assert!(!app.status_is_error);
    }

    #[test]
    fn test_tick_expires_overdue_status_message() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.set_status("saved".to_string(), false);
        // Force the deadline into the past so the wiring is checked without
        // sleeping for the real TTL.
        app.status_message_deadline = Some(Instant::now() - Duration::from_secs(1));
        app.dispatch(Message::Tick);
        assert!(app.status_message.is_none());
    }

    #[test]
    fn test_dispatch_buffers_and_queues_only_matching_entries() {
        use crate::logcat::{LogEntry, LogLevel};

        fn entry(tag: &str, message: &str) -> LogEntry {
            LogEntry {
                timestamp: chrono::NaiveDateTime::parse_from_str(
                    "2024-07-27 10:15:30.123",
                    "%Y-%m-%d %H:%M:%S%.3f",
                )
                .expect("fixed timestamp"),
                pid: 1234,
                tid: 5678,
                level: LogLevel::Info,
                tag: tag.to_string(),
                message: message.to_string(),
                package: None,
            }
        }

        let mut app = App::new_for_tests(crate::i18n::Lang::En);
        app.filter_input = "tag:Keep".to_string();
        app.apply_filter();

        app.dispatch(Message::NewEntry(entry("Keep", "one")));
        app.dispatch(Message::NewEntry(entry("Drop", "two")));

        assert_eq!(app.buffer.len(), 2);
        assert_eq!(app.buffer.filtered_len(), 1);
        // Only the matching entry reaches scrollback, and it is rendered from
        // the entry stored in the buffer.
        assert_eq!(app.pending_log_lines.len(), 1);
        let rendered: String = app.pending_log_lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(rendered.contains("one"), "{rendered}");
        assert!(!rendered.contains("two"), "{rendered}");

        // Paused entries are discarded, not buffered or queued.
        app.paused = true;
        app.dispatch(Message::NewEntry(entry("Keep", "three")));
        assert_eq!(app.buffer.len(), 2);
        assert_eq!(app.pending_log_lines.len(), 1);
    }

    #[test]
    fn test_drain_feed_dispatches_a_burst_in_channel_order() {
        use crate::logcat::{LogEntry, LogLevel};

        fn entry(message: &str) -> LogEntry {
            LogEntry {
                timestamp: chrono::NaiveDateTime::parse_from_str(
                    "2024-07-27 10:15:30.123",
                    "%Y-%m-%d %H:%M:%S%.3f",
                )
                .expect("fixed timestamp"),
                pid: 1234,
                tid: 5678,
                level: LogLevel::Info,
                tag: "Burst".to_string(),
                message: message.to_string(),
                package: None,
            }
        }

        let (tx, mut rx) = mpsc::channel::<Message>(16);
        for text in ["one", "two", "three", "four"] {
            tx.try_send(Message::NewEntry(entry(text)))
                .expect("burst must fit the channel");
        }
        let mut app = App::new_for_tests(crate::i18n::Lang::En);

        // The batch cap bounds one pass even when more messages are queued.
        let first = rx.try_recv().ok();
        assert_eq!(
            app.drain_feed(&mut rx, first, 2, false, false),
            FeedDrain::Continue
        );
        assert_eq!(app.buffer.len(), 2);
        assert_eq!(app.pending_log_lines.len(), 2);

        let first = rx.try_recv().ok();
        assert_eq!(
            app.drain_feed(&mut rx, first, MSG_DRAIN_BATCH, false, false),
            FeedDrain::Continue
        );
        assert_eq!(app.buffer.len(), 4);
        assert_eq!(app.pending_log_lines.len(), 4);

        let messages: Vec<&str> = (0..app.buffer.filtered_len())
            .filter_map(|pos| {
                app.buffer
                    .get_filtered(pos)
                    .map(|(_, entry)| entry.message.as_str())
            })
            .collect();
        assert_eq!(messages, ["one", "two", "three", "four"]);
    }

    #[test]
    fn test_drain_feed_reports_exit_and_closed() {
        let mut app = App::new_for_tests(crate::i18n::Lang::En);

        // `LogcatDied` with EOF exit semantics stops the UI loop.
        let (tx, mut rx) = mpsc::channel::<Message>(4);
        tx.try_send(Message::LogcatDied).expect("queued");
        let first = rx.try_recv().ok();
        assert_eq!(
            app.drain_feed(&mut rx, first, MSG_DRAIN_BATCH, true, false),
            FeedDrain::Exit
        );

        // Without reconnect the channel is abandoned so the loop stops polling.
        let (tx, mut rx) = mpsc::channel::<Message>(4);
        tx.try_send(Message::LogcatDied).expect("queued");
        let first = rx.try_recv().ok();
        assert_eq!(
            app.drain_feed(&mut rx, first, MSG_DRAIN_BATCH, false, false),
            FeedDrain::Closed
        );

        // With reconnect the loop keeps polling for the restarted stream.
        let (tx, mut rx) = mpsc::channel::<Message>(4);
        tx.try_send(Message::LogcatDied).expect("queued");
        let first = rx.try_recv().ok();
        assert_eq!(
            app.drain_feed(&mut rx, first, MSG_DRAIN_BATCH, false, true),
            FeedDrain::Continue
        );

        // A disconnected channel reports `Closed` and marks the feed stopped.
        let (tx, mut rx) = mpsc::channel::<Message>(4);
        drop(tx);
        let first = rx.try_recv().ok();
        assert_eq!(
            app.drain_feed(&mut rx, first, MSG_DRAIN_BATCH, false, true),
            FeedDrain::Closed
        );
        assert!(!app.running);
    }
}
