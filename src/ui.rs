use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use chrono::{Datelike, Timelike};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, DisplayConfig, Focus, SettingsCategory};
use crate::config::{ColorItem, ColorPalette, color_to_hex};
use crate::logcat::{LogEntry, LogLevel};
use crate::viewport::ViewportFrame;

/// Floor a byte offset to the nearest UTF-8 char boundary at or before it.
fn floor_char_boundary(s: &str, mut pos: usize) -> usize {
    pos = pos.min(s.len());
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Truncate a string to at most `max_cols` terminal columns.
fn truncate_columns(s: &str, max_cols: usize) -> &str {
    if max_cols == 0 {
        return "";
    }
    let mut cols = 0;
    for (idx, c) in s.char_indices() {
        let width = UnicodeWidthChar::width(c).unwrap_or(0);
        if cols + width > max_cols {
            return &s[..idx];
        }
        cols += width;
    }
    s
}

/// Round a desired scroll column down to a char-boundary column.
fn align_scroll_col(s: &str, desired: usize) -> usize {
    if desired == 0 {
        return 0;
    }
    let mut col = 0;
    for c in s.chars() {
        let width = UnicodeWidthChar::width(c).unwrap_or(0);
        if col + width > desired {
            return col;
        }
        col += width;
        if col >= desired {
            return col;
        }
    }
    col
}

/// Convert a char-boundary column to the byte offset at that boundary.
fn byte_at_col(s: &str, col: usize) -> usize {
    if col == 0 {
        return 0;
    }
    let mut cur = 0;
    for (idx, c) in s.char_indices() {
        cur += UnicodeWidthChar::width(c).unwrap_or(0);
        if cur > col {
            break;
        }
        if cur == col {
            return idx + c.len_utf8();
        }
    }
    s.len()
}

/// Return the visible slice of the filter input plus the cursor's column
/// within that slice, keeping the cursor inside the available width.
fn visible_filter_slice(input: &str, cursor: usize, max_cols: usize) -> (&str, usize) {
    let cursor = floor_char_boundary(input, cursor);
    if max_cols == 0 {
        return ("", 0);
    }

    let cursor_col = UnicodeWidthStr::width(&input[..cursor]);
    let total_cols = UnicodeWidthStr::width(input);
    if total_cols <= max_cols {
        return (truncate_columns(input, max_cols), cursor_col);
    }

    let desired_scroll = if cursor_col >= max_cols {
        cursor_col + 1 - max_cols
    } else {
        0
    };
    let scroll_col = align_scroll_col(input, desired_scroll);
    let scroll_byte = byte_at_col(input, scroll_col);
    let visible = truncate_columns(&input[scroll_byte..], max_cols);
    (visible, cursor_col.saturating_sub(scroll_col))
}

// ── Main render ──────────────────────────────────────────────────────────────

/// Render the bottom viewport: filter bar with embedded status,
/// and optionally the help/options overlay above it.
///
/// The viewport area layout (bottom to top):
/// - Filter input bar with status info in the border title (1 row + borders)
/// - Filter error (1 row, only when present)
/// - Help/Options overlay (N rows, only when toggled via h/o)
pub fn render_viewport(f: &mut ViewportFrame, app: &App) {
    let area = f.area();
    let overlay_rows = app.overlay_rows();
    let is_focused = app.focus == Focus::FilterInput;

    // Split area: overlay on top, filter bar on bottom
    let filter_area = Rect {
        x: area.x,
        y: area.y + overlay_rows,
        width: area.width,
        height: area.height.saturating_sub(overlay_rows),
    };

    // Render overlay if shown
    if overlay_rows > 0 {
        let overlay_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: overlay_rows,
        };
        render_overlay(f, app, overlay_area);
    }

    // Build status string
    let msgs = app.msgs;
    let status = msgs.status_bar(
        app.buffer.len(),
        app.buffer.filtered_len(),
        filter_status(app, msgs),
    );

    // Run-state indicator prepended to the filter title.
    let (indicator, indicator_style) = run_state_indicator(msgs, app.paused, app.running);
    let block = Block::default()
        .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
        .border_style(if is_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        })
        .title(Line::from(vec![
            Span::styled(format!("{} ", indicator), indicator_style),
            Span::raw(msgs.filter_title(&status)),
        ]));
    let inner = block.inner(filter_area);
    f.render_widget(block, filter_area);

    // Render filter input text. The visible slice is horizontally scrolled
    // around the cursor so arrow-key navigation stays visible on narrow
    // terminals.
    let prompt = if is_focused { "> " } else { "  " };
    let prompt_width = UnicodeWidthStr::width(prompt);
    let max_filter_cols = inner.width.saturating_sub(prompt_width as u16) as usize;
    let (visible_filter, cursor_col) =
        visible_filter_slice(&app.filter_input, app.filter_cursor, max_filter_cols);
    let display_text = format!("{}{}", prompt, visible_filter);

    let text = Paragraph::new(display_text).style(Style::default());
    f.render_widget(text, inner);

    // Cursor position for filter input (viewport-relative coordinates).
    if is_focused {
        let max_local_x = inner.width.saturating_sub(1) as usize;
        let local_x = (prompt_width + cursor_col).min(max_local_x);
        let rel_cx = (inner.x - area.x) + local_x as u16;
        let rel_cy = inner.y - area.y;
        f.set_cursor_position((rel_cx, rel_cy));
    }

    // Filter error below input
    if let Some(ref err) = app.filter_error
        && filter_area.height > 1
    {
        let error_area = Rect {
            x: filter_area.x + 1,
            y: inner.y + inner.height,
            width: filter_area.width.saturating_sub(2),
            height: 1,
        };
        let error_text = Span::styled(msgs.filter_error(err), Style::default().fg(Color::Red));
        f.render_widget(Paragraph::new(Line::from(error_text)), error_area);
    }
}

fn filter_status<'a>(app: &App, msgs: &'a crate::i18n::Messages) -> &'a str {
    if app.filter_bypassed {
        msgs.filter_bypassed
    } else if app.filter_error.is_some() {
        msgs.filter_parse_error
    } else if app.active_filter.is_some() {
        msgs.filter_ok
    } else {
        msgs.filter_none
    }
}

/// (label, color) of the run-state indicator shown before the filter title.
fn run_state_indicator(
    msgs: &crate::i18n::Messages,
    paused: bool,
    running: bool,
) -> (&'static str, Color) {
    if paused {
        (msgs.status_paused, Color::Yellow)
    } else if running {
        (msgs.status_running, Color::Green)
    } else {
        (msgs.status_stopped, Color::Red)
    }
}

// ── Entry formatting ─────────────────────────────────────────────────────────

/// Format a single log entry into a ratatui `Line`.
pub fn format_entry(
    entry: &LogEntry,
    config: &DisplayConfig,
    palette: &ColorPalette,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();

    let level_style = level_style(entry.level, palette);

    if config.show_timestamp {
        let ts = entry.timestamp;
        let ts_str = format!(
            "{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            ts.month(),
            ts.day(),
            ts.hour(),
            ts.minute(),
            ts.second(),
            (ts.and_utc().timestamp_millis() % 1000) as u32,
        );
        spans.push(Span::styled(
            format!("{} ", ts_str),
            Style::default().fg(palette.timestamp),
        ));
    }

    if config.show_pid {
        spans.push(Span::styled(
            format!("{:>5} ", entry.pid),
            Style::default().fg(Color::DarkGray),
        ));
    }

    if config.show_tid {
        spans.push(Span::styled(
            format!("{:>5} ", entry.tid),
            Style::default().fg(Color::DarkGray),
        ));
    }

    if config.show_level {
        let level_char = entry.level.as_char();
        spans.push(Span::styled(
            format!("{} ", level_char),
            level_style.add_modifier(Modifier::BOLD),
        ));
    }

    if config.show_tag {
        let tag_str = if let Some(ref package) = entry.package {
            format!("{}/{}: ", package, entry.tag)
        } else {
            format!("{}: ", entry.tag)
        };
        spans.push(Span::styled(tag_str, Style::default().fg(palette.tag)));
    }

    let msg_style = if config.colorize {
        level_style
    } else {
        Style::default()
    };

    spans.push(Span::styled(entry.message.clone(), msg_style));

    Line::from(spans)
}

fn level_style(level: LogLevel, palette: &ColorPalette) -> Style {
    let fg = palette.level_color(level);
    if level == LogLevel::Fatal {
        Style::default().fg(fg).bg(Color::DarkGray) // Fatal keeps its fixed darkgray background
    } else {
        Style::default().fg(fg)
    }
}

// ── Help / Settings overlay ─────────────────────────────────────────────────

fn help_lines(msgs: &crate::i18n::Messages) -> Vec<Line<'static>> {
    let header = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let key_style = Style::default().fg(Color::Cyan);
    let desc = Style::default();

    vec![
        Line::from(Span::styled(msgs.help_title, header)),
        Line::from(vec![
            Span::styled("  q / Ctrl+C  ", key_style),
            Span::styled(msgs.help_quit, desc),
        ]),
        Line::from(vec![
            Span::styled("  h           ", key_style),
            Span::styled(msgs.help_show_help, desc),
        ]),
        Line::from(vec![
            Span::styled("  p / Space   ", key_style),
            Span::styled(msgs.help_pause, desc),
        ]),
        Line::from(vec![
            Span::styled("  C           ", key_style),
            Span::styled(msgs.help_clear, desc),
        ]),
        Line::from(vec![
            Span::styled("  o           ", key_style),
            Span::styled(msgs.help_settings, desc),
        ]),
        Line::from(vec![
            Span::styled("  g           ", key_style),
            Span::styled(msgs.help_bypass, desc),
        ]),
        Line::from(vec![
            Span::styled("  Tab         ", key_style),
            Span::styled(msgs.help_tab, desc),
        ]),
        Line::from(vec![
            Span::styled("  Esc         ", key_style),
            Span::styled(msgs.help_esc, desc),
        ]),
        Line::default(),
    ]
}

/// The settings screen: shared title + category row, then the active
/// category's rows. New categories plug into `SettingsCategory` and the
/// match in `settings_lines`.
fn settings_lines(app: &App) -> Vec<Line<'static>> {
    let header = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let mut lines = vec![Line::from(Span::styled(app.msgs.settings_title, header))];
    lines.push(settings_category_row(app));
    match app.settings_category {
        SettingsCategory::Display => lines.extend(settings_display_rows(app)),
        SettingsCategory::Color => lines.extend(settings_color_rows(app)),
    }
    lines
}

/// Category tabs; the active one is highlighted, `[Tab]` cycles.
/// Iterates `SettingsCategory::ALL` so a new category only needs a variant,
/// a `label` arm, i18n strings, and its rows fn.
fn settings_category_row(app: &App) -> Line<'static> {
    let msgs = app.msgs;
    let key_style = Style::default().fg(Color::Cyan);
    let active = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let inactive = Style::default().fg(Color::DarkGray);
    let mut spans = vec![Span::styled("  ", inactive)];
    for (i, cat) in SettingsCategory::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  |  ", inactive));
        }
        let style = if *cat == app.settings_category {
            active
        } else {
            inactive
        };
        spans.push(Span::styled(cat.label(msgs), style));
    }
    spans.push(Span::styled("   [Tab]  ", key_style));
    Line::from(spans)
}

fn settings_display_rows(app: &App) -> Vec<Line<'static>> {
    let msgs = app.msgs;
    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    fn status_label(enabled: bool, msgs: &crate::i18n::Messages) -> &str {
        if enabled {
            msgs.on_label
        } else {
            msgs.off_label
        }
    }
    fn style_for(enabled: bool) -> Style {
        if enabled {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    }

    let mut rows = vec![
        Line::from(vec![
            Span::styled("  [1] ", key_style),
            Span::styled(msgs.opts_timestamp, key_style),
            Span::styled(": ", key_style),
            Span::styled(
                status_label(app.config.show_timestamp, msgs),
                style_for(app.config.show_timestamp),
            ),
        ]),
        Line::from(vec![
            Span::styled("  [2] ", key_style),
            Span::styled(msgs.opts_pid, key_style),
            Span::styled(":    ", key_style),
            Span::styled(
                status_label(app.config.show_pid, msgs),
                style_for(app.config.show_pid),
            ),
        ]),
        Line::from(vec![
            Span::styled("  [3] ", key_style),
            Span::styled(msgs.opts_tid, key_style),
            Span::styled(":    ", key_style),
            Span::styled(
                status_label(app.config.show_tid, msgs),
                style_for(app.config.show_tid),
            ),
        ]),
        Line::from(vec![
            Span::styled("  [4] ", key_style),
            Span::styled(msgs.opts_tag, key_style),
            Span::styled(":    ", key_style),
            Span::styled(
                status_label(app.config.show_tag, msgs),
                style_for(app.config.show_tag),
            ),
        ]),
        Line::from(vec![
            Span::styled("  [5] ", key_style),
            Span::styled(msgs.opts_level, key_style),
            Span::styled(":   ", key_style),
            Span::styled(
                status_label(app.config.show_level, msgs),
                style_for(app.config.show_level),
            ),
        ]),
        Line::from(vec![
            Span::styled("  [6] ", key_style),
            Span::styled(msgs.opts_color, key_style),
            Span::styled(":   ", key_style),
            Span::styled(
                status_label(app.config.colorize, msgs),
                style_for(app.config.colorize),
            ),
        ]),
    ];
    rows.push(Line::from(Span::styled(
        format!("  {}", msgs.settings_display_hint),
        Style::default().fg(Color::DarkGray),
    )));
    rows.push(Line::default());
    rows
}

fn settings_color_rows(app: &App) -> Vec<Line<'static>> {
    let msgs = app.msgs;
    let key_style = Style::default().fg(Color::Cyan);

    let mut rows = vec![Line::from(vec![
        Span::styled(format!("  {}: ", msgs.color_preset_label), key_style),
        Span::styled(
            app.color_preset.label(msgs),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled("   [ ]  ", key_style),
    ])];
    // 8 item rows
    for (i, item) in ColorItem::ALL.iter().enumerate() {
        let n = i + 1;
        if app.color_editing == Some(*item) {
            let text = format!(
                "  [{}] {}  #{}{}",
                n,
                item.label(msgs),
                app.color_input,
                "█"
            );
            rows.push(Line::from(Span::styled(text, key_style)));
        } else {
            let color = item.color(&app.palette);
            let hex = color_to_hex(color);
            rows.push(Line::from(vec![
                Span::styled(
                    format!("  [{}] {}  {}  ", n, item.label(msgs), hex),
                    Style::default().fg(color),
                ),
                Span::styled("    ", Style::default().bg(color)), // color swatch
            ]));
        }
    }
    // Hint or error row
    if let Some(ref err) = app.color_error {
        rows.push(Line::from(Span::styled(
            format!("  {}", err),
            Style::default().fg(Color::Red),
        )));
    } else {
        rows.push(Line::from(Span::styled(
            format!("  {}", msgs.color_hint),
            Style::default().fg(Color::DarkGray),
        )));
    }
    rows.push(Line::default());
    rows
}

/// Render the help or settings overlay in the given area.
fn render_overlay(f: &mut ViewportFrame, app: &App, area: Rect) {
    let lines: Vec<Line<'static>> = if app.show_help {
        help_lines(app.msgs)
    } else if app.show_settings {
        settings_lines(app)
    } else {
        return;
    };

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = Paragraph::new(lines);
    f.render_widget(text, inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(level: LogLevel, tag: &str, msg: &str) -> LogEntry {
        LogEntry {
            timestamp: chrono::NaiveDateTime::parse_from_str(
                "2024-07-27 10:15:30.123",
                "%Y-%m-%d %H:%M:%S%.3f",
            )
            .expect("test timestamp should parse"),
            pid: 1234,
            tid: 5678,
            level,
            tag: tag.to_string(),
            message: msg.to_string(),
            package: None,
        }
    }

    #[test]
    fn test_format_entry_custom_colors() {
        let mut palette = ColorPalette::default();
        palette.error = Color::Rgb(0xFF, 0x30, 0x30);
        palette.tag = Color::Rgb(0x12, 0x34, 0x56);
        palette.timestamp = Color::Rgb(0x65, 0x43, 0x21);
        let entry = make_entry(LogLevel::Error, "MyTag", "boom");
        let line = format_entry(&entry, &DisplayConfig::default(), &palette);
        // Span order with all toggles on: 0=timestamp, 1=pid, 2=tid, 3=level, 4=tag, 5=message.
        assert_eq!(line.spans[0].style.fg, Some(Color::Rgb(0x65, 0x43, 0x21)));
        assert_eq!(line.spans[3].style.fg, Some(Color::Rgb(0xFF, 0x30, 0x30)));
        assert_eq!(line.spans[4].style.fg, Some(Color::Rgb(0x12, 0x34, 0x56)));
        assert_eq!(line.spans[5].style.fg, Some(Color::Rgb(0xFF, 0x30, 0x30)));
    }

    #[test]
    fn test_format_entry_colorize_off() {
        let entry = make_entry(LogLevel::Info, "MyTag", "hello");
        let config = DisplayConfig {
            colorize: false,
            ..Default::default()
        };
        let line = format_entry(&entry, &config, &ColorPalette::default());
        assert_eq!(line.spans[5].style.fg, None);
    }

    fn line_text(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn test_run_state_indicator() {
        let msgs = crate::i18n::messages(crate::i18n::Lang::En);
        assert_eq!(
            run_state_indicator(msgs, false, true),
            (msgs.status_running, Color::Green)
        );
        assert_eq!(
            run_state_indicator(msgs, true, true),
            (msgs.status_paused, Color::Yellow)
        );
        assert_eq!(
            run_state_indicator(msgs, false, false),
            (msgs.status_stopped, Color::Red)
        );
    }

    #[test]
    fn test_settings_lines_structure() {
        let app = App::new(crate::i18n::Lang::En);

        // Display category: title + category row + 6 toggles + hint + blank.
        let lines = settings_lines(&app);
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0].spans[0].content.as_ref(), app.msgs.settings_title);
        assert!(line_text(&lines[1]).contains(app.msgs.settings_category_display));
        assert!(line_text(&lines[1]).contains(app.msgs.settings_category_color));

        // Color category: title + category row + preset + 8 items + hint + blank.
        let mut app = app;
        app.settings_category = SettingsCategory::Color;
        let lines = settings_lines(&app);
        assert_eq!(lines.len(), 13);
        assert!(line_text(&lines[1]).contains(app.msgs.settings_category_color));
    }

    #[test]
    fn test_visible_filter_slice_shows_full_short_input() {
        let (visible, cursor_col) = visible_filter_slice("abc", 3, 10);
        assert_eq!(visible, "abc");
        assert_eq!(cursor_col, 3);
    }

    #[test]
    fn test_visible_filter_slice_scrolls_to_keep_cursor_visible() {
        let input = "abcdefghij";
        // Cursor after "f" (column 6) with only 4 columns available: the
        // window should scroll so the cursor stays inside the visible area.
        let (visible, cursor_col) = visible_filter_slice(input, 6, 4);
        assert_eq!(cursor_col, 3);
        assert_eq!(visible, "defg");
        assert_eq!(cursor_col, UnicodeWidthStr::width(visible) - 1);
    }

    #[test]
    fn test_visible_filter_slice_handles_wide_chars() {
        let input = "中文abc";
        let cursor = input.len();
        let (visible, cursor_col) = visible_filter_slice(input, cursor, 4);
        // The window starts at column 4 (after the two wide chars), leaving
        // the three ASCII chars visible and the cursor on the last column.
        assert_eq!(visible, "abc");
        assert_eq!(UnicodeWidthStr::width(visible), 3);
        assert_eq!(cursor_col, 3);
    }

    #[test]
    fn test_visible_filter_slice_handles_zero_width() {
        let (visible, cursor_col) = visible_filter_slice("abc", 1, 0);
        assert_eq!(visible, "");
        assert_eq!(cursor_col, 0);
    }
}
