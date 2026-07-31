use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use chrono::{Datelike, Timelike};

use crate::app::{App, DisplayConfig, Focus};
use crate::config::{color_to_hex, ColorItem, ColorPalette};
use crate::logcat::{LogEntry, LogLevel};
use crate::viewport::ViewportFrame;

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

    let block = Block::default()
        .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
        .border_style(if is_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        })
        .title(msgs.filter_title(&status));
    let inner = block.inner(filter_area);
    f.render_widget(block, filter_area);

    // Render filter input text
    let prompt = if is_focused { "> " } else { "  " };
    let display_text = format!("{}{}", prompt, app.filter_input);

    let text = Paragraph::new(display_text).style(Style::default());
    f.render_widget(text, inner);

    // Cursor position for filter input (viewport-relative coordinates).
    if is_focused {
        let rel_cx = (inner.x - area.x) + prompt.len() as u16
            + app.filter_input.len().min(inner.width.saturating_sub(1) as usize) as u16;
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
        let error_text = Span::styled(
            msgs.filter_error(err),
            Style::default().fg(Color::Red),
        );
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

// ── Entry formatting ─────────────────────────────────────────────────────────

/// Format a single log entry into a ratatui `Line`.
pub fn format_entry(entry: &LogEntry, config: &DisplayConfig, palette: &ColorPalette) -> Line<'static> {
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
        spans.push(Span::styled(
            tag_str,
            Style::default().fg(palette.tag),
        ));
    }

    let msg_style = if config.colorize {
        level_style
    } else {
        Style::default()
    };

    spans.push(Span::styled(
        entry.message.clone(),
        msg_style,
    ));

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


// ── Help / Options overlay ───────────────────────────────────────────────────

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
            Span::styled(msgs.help_options, desc),
        ]),
        Line::from(vec![
            Span::styled("  c           ", key_style),
            Span::styled(msgs.help_colors, desc),
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
        Line::from(vec![
            Span::styled("  1-6         ", key_style),
            Span::styled(msgs.help_display_opts, desc),
        ]),
        Line::default(),
    ]
}

fn options_lines(app: &App) -> Vec<Line<'static>> {
    let msgs = app.msgs;
    let header = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    fn status_label<'a>(enabled: bool, msgs: &'a crate::i18n::Messages) -> &'a str {
        if enabled { msgs.on_label } else { msgs.off_label }
    }
    fn style_for(enabled: bool) -> Style {
        if enabled {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    }

    vec![
        Line::from(Span::styled(msgs.opts_title, header)),
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
        Line::default(),
    ]
}

fn colors_lines(app: &App) -> Vec<Line<'static>> {
    let msgs = app.msgs;
    let header = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let key_style = Style::default().fg(Color::Cyan);

    let mut lines = vec![Line::from(Span::styled(msgs.color_title, header))];
    // Preset row
    lines.push(Line::from(vec![
        Span::styled(format!("  {}: ", msgs.color_preset_label), key_style),
        Span::styled(
            app.color_preset.label(msgs),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled("   [ ]  ", key_style),
    ]));
    // 8 item rows
    for (i, item) in ColorItem::ALL.iter().enumerate() {
        let n = i + 1;
        if app.color_editing == Some(*item) {
            let text = format!("  [{}] {}  #{}{}", n, item.label(msgs), app.color_input, "█");
            lines.push(Line::from(Span::styled(text, key_style)));
        } else {
            let color = item.color(&app.palette);
            let hex = color_to_hex(color);
            lines.push(Line::from(vec![
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
        lines.push(Line::from(Span::styled(
            format!("  {}", err),
            Style::default().fg(Color::Red),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("  {}", msgs.color_hint),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::default());
    lines
}

/// Render the help or options overlay in the given area.
fn render_overlay(f: &mut ViewportFrame, app: &App, area: Rect) {
    let lines: Vec<Line<'static>> = if app.show_help {
        help_lines(app.msgs)
    } else if app.show_options {
        options_lines(app)
    } else if app.show_colors {
        colors_lines(app)
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
        let mut config = DisplayConfig::default();
        config.colorize = false;
        let line = format_entry(&entry, &config, &ColorPalette::default());
        assert_eq!(line.spans[5].style.fg, None);
    }
}
