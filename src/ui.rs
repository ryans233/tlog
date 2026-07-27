use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use chrono::{Datelike, Timelike};

use crate::app::{App, DisplayConfig, Focus};
use crate::logcat::{LogEntry, LogLevel};
use crate::viewport::ViewportFrame;

// ── Main render ──────────────────────────────────────────────────────────────

/// Render the bottom viewport: filter bar with embedded status.
///
/// The viewport covers only the bottom 2 rows:
/// - Row 0: Filter input bar with status info in the border title
/// - Row 1: Filter error (only when present)
pub fn render_viewport(f: &mut ViewportFrame, app: &App) {
    let area = f.area();
    let is_focused = app.focus == Focus::FilterInput;

    // Build status string
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
    let inner = block.inner(area);
    f.render_widget(block, area);

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

    // Filter error below input (absolute screen coordinates — buffer handles offset).
    if let Some(ref err) = app.filter_error
        && area.height > 1
    {
        let error_area = Rect {
            x: area.x + 1,
            y: inner.y + inner.height,
            width: area.width.saturating_sub(2),
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
pub fn format_entry(entry: &LogEntry, config: &DisplayConfig) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();

    let level_style = level_style(entry.level);

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
            Style::default().fg(Color::DarkGray),
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
            Style::default().fg(Color::Cyan),
        ));
    }

    let msg_style = if config.colorize {
        level_style
    } else {
        Style::default()
    };

    spans.push(Span::styled(
        truncate_line(&entry.message, 512),
        msg_style,
    ));

    Line::from(spans)
}

fn level_style(level: LogLevel) -> Style {
    match level {
        LogLevel::Verbose => Style::default().fg(Color::DarkGray),
        LogLevel::Debug => Style::default().fg(Color::Cyan),
        LogLevel::Info => Style::default().fg(Color::Green),
        LogLevel::Warn => Style::default().fg(Color::Yellow),
        LogLevel::Error => Style::default().fg(Color::Red),
        LogLevel::Fatal => Style::default().fg(Color::Red).bg(Color::DarkGray),
    }
}

/// Truncate a string to at most `max_chars` characters on a valid UTF-8 boundary.
fn truncate_line(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect::<String>() + "..."
    }
}
