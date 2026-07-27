//! Terminal scrollback management for tlog.
//!
//! Provides ANSI scroll-region commands (`DECSTBM`) and the `insert_log_lines`
//! function that writes log entries into the terminal's native scrollback buffer
//! above the Ratatui viewport.

use std::io;

use crossterm::Command;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::viewport::ViewportTerminal;

// ── Scroll Region Commands ───────────────────────────────────────────────────

/// Set the terminal scroll region to `rows`.
///
/// Emits `CSI <start>;<end> r` (DECSTBM).
/// Rows are 1-based, inclusive.
pub struct SetScrollRegion(pub std::ops::Range<u16>);

impl Command for SetScrollRegion {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        write!(f, "\x1b[{};{}r", self.0.start + 1, self.0.end)
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Reset the terminal scroll region to full screen.
///
/// Emits `CSI r`.
pub struct ResetScrollRegion;

impl Command for ResetScrollRegion {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        write!(f, "\x1b[r")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        Ok(())
    }
}

// ── insert_log_lines ─────────────────────────────────────────────────────────

/// Write styled log lines above the viewport into the terminal's scrollback.
///
/// After this call, the viewport may have moved if there was room below it.
pub fn insert_log_lines<B>(
    terminal: &mut ViewportTerminal<B>,
    lines: &[Line<'_>],
) -> io::Result<()>
where
    B: ratatui::backend::Backend + io::Write,
    <B as ratatui::backend::Backend>::Error: Into<io::Error>,
{
    if lines.is_empty() {
        return Ok(());
    }

    let viewport_top = terminal.viewport_area.top();

    if viewport_top == 0 {
        // Viewport at top — nowhere to insert above.
        let backend = terminal.backend_mut();
        for (i, line) in lines.iter().enumerate() {
            crossterm::execute!(backend, crossterm::cursor::MoveTo(0, i as u16))?;
            write_spans(backend, &line.spans)?;
            io::Write::flush(backend)?;
        }
        return Ok(());
    }

    {
        let backend = terminal.backend_mut();

        // Set scroll region to everything above the viewport.
        // This ensures \n at the boundary scrolls only the log area, not the viewport.
        crossterm::execute!(
            backend,
            SetScrollRegion(0..viewport_top),
        )?;

        // Move cursor just above the viewport (last row of scroll region).
        let write_row = viewport_top.saturating_sub(1);
        crossterm::execute!(
            backend,
            crossterm::cursor::MoveTo(0, write_row),
        )?;

        for line in lines {
            // \r\n at the bottom of the scroll region scrolls the region up by 1,
            // leaving the cursor at the same row (now blank).
            write!(backend, "\r\n")?;
            write_spans(backend, &line.spans)?;
            // Flush after each line so the terminal processes the scroll.
            io::Write::flush(backend)?;
        }

        crossterm::execute!(backend, ResetScrollRegion)?;
    }

    terminal.note_history_rows_inserted(lines.len() as u16);

    Ok(())
}

// ── write_spans ──────────────────────────────────────────────────────────────

/// Write styled spans to a writer, emitting ANSI SGR sequences for colors and
/// modifiers.
///
/// This is a simplified version of Codex's `write_spans` — log lines are plain
/// text so we only need basic color support.
pub fn write_spans<W: io::Write>(w: &mut W, spans: &[Span<'_>]) -> io::Result<()> {
    let mut current_style = Style::default();

    for span in spans {
        let style = span.style;

        // Emit style changes.
        emit_style_change(w, current_style, style)?;
        current_style = style;

        // Write the text.
        write!(w, "{}", span.content)?;
    }

    // Reset style at end.
    if current_style != Style::default() {
        emit_style_change(w, current_style, Style::default())?;
    }

    Ok(())
}

fn emit_style_change<W: io::Write>(
    w: &mut W,
    from: Style,
    to: Style,
) -> io::Result<()> {
    if from == to {
        return Ok(());
    }

    let mut params: Vec<String> = Vec::new();

    // Reset all attributes, then apply new ones.
    params.push("0".into());

    let m = to.add_modifier;
    if m.contains(Modifier::BOLD) {
        params.push("1".into());
    }
    if m.contains(Modifier::DIM) {
        params.push("2".into());
    }
    if m.contains(Modifier::ITALIC) {
        params.push("3".into());
    }
    if m.contains(Modifier::UNDERLINED) {
        params.push("4".into());
    }
    if m.contains(Modifier::SLOW_BLINK) {
        params.push("5".into());
    }
    if m.contains(Modifier::RAPID_BLINK) {
        params.push("6".into());
    }
    if m.contains(Modifier::REVERSED) {
        params.push("7".into());
    }
    if m.contains(Modifier::HIDDEN) {
        params.push("8".into());
    }
    if m.contains(Modifier::CROSSED_OUT) {
        params.push("9".into());
    }

    // Foreground color.
    if let Some(fg) = to.fg {
        params.push(color_ansi_fg(fg));
    }
    // Background color.
    if let Some(bg) = to.bg {
        params.push(color_ansi_bg(bg));
    }

    write!(w, "\x1b[{}m", params.join(";"))?;
    Ok(())
}

fn color_ansi_fg(color: Color) -> String {
    match color {
        Color::Reset => "39".into(),
        Color::Black => "30".into(),
        Color::Red => "31".into(),
        Color::Green => "32".into(),
        Color::Yellow => "33".into(),
        Color::Blue => "34".into(),
        Color::Magenta => "35".into(),
        Color::Cyan => "36".into(),
        Color::Gray => "37".into(),
        Color::DarkGray => "90".into(),
        Color::LightRed => "91".into(),
        Color::LightGreen => "92".into(),
        Color::LightYellow => "93".into(),
        Color::LightBlue => "94".into(),
        Color::LightMagenta => "95".into(),
        Color::LightCyan => "96".into(),
        Color::White => "97".into(),
        Color::Rgb(r, g, b) => format!("38;2;{};{};{}", r, g, b),
        Color::Indexed(i) => format!("38;5;{}", i),
    }
}

fn color_ansi_bg(color: Color) -> String {
    match color {
        Color::Reset => "49".into(),
        Color::Black => "40".into(),
        Color::Red => "41".into(),
        Color::Green => "42".into(),
        Color::Yellow => "43".into(),
        Color::Blue => "44".into(),
        Color::Magenta => "45".into(),
        Color::Cyan => "46".into(),
        Color::Gray => "47".into(),
        Color::DarkGray => "100".into(),
        Color::LightRed => "101".into(),
        Color::LightGreen => "102".into(),
        Color::LightYellow => "103".into(),
        Color::LightBlue => "104".into(),
        Color::LightMagenta => "105".into(),
        Color::LightCyan => "106".into(),
        Color::White => "107".into(),
        Color::Rgb(r, g, b) => format!("48;2;{};{};{}", r, g, b),
        Color::Indexed(i) => format!("48;5;{}", i),
    }
}
