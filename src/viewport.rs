//! A minimal inline-viewport terminal for Ratatui + crossterm.
//!
//! Ported from Codex's `custom_terminal.rs` (MIT-licensed) and adapted for tlog.
//! This replaces `ratatui::Terminal` for our inline scrollback use case.
//!
//! Key differences from ratatui::Terminal:
//! - No alternate screen — works in the main screen buffer.
//! - `viewport_area` is not the full screen; it's a fixed region at the bottom.
//! - `autoresize()` only tracks screen size changes, never changes `viewport_area`.
//! - Diff-based rendering only touches cells within the viewport.

use std::io;

use ratatui::{
    backend::Backend,
    buffer::{Buffer, Cell},
    layout::{Position, Rect, Size},
    widgets::Widget,
};

// ── ViewportFrame ────────────────────────────────────────────────────────────

pub struct ViewportFrame<'a> {
    pub cursor_position: Option<Position>,
    pub viewport_area: Rect,
    pub buffer: &'a mut Buffer,
}

impl<'a> ViewportFrame<'a> {
    pub fn area(&self) -> Rect {
        self.viewport_area
    }

    pub fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, self.buffer);
    }

    pub fn set_cursor_position<P: Into<Position>>(&mut self, position: P) {
        self.cursor_position = Some(position.into());
    }
}


// ── ViewportTerminal ─────────────────────────────────────────────────────────

pub struct ViewportTerminal<B: Backend + io::Write> {
    backend: B,
    buffers: [Buffer; 2],
    current: usize,
    hidden_cursor: bool,
    /// The Ratatui-rendered region (bottom of screen).
    pub viewport_area: Rect,
    /// Last known full screen size (for resize detection).
    last_known_screen_size: Size,
    /// Last known cursor position (after flush).
    last_known_cursor_pos: Position,
    /// Tracks how many history rows have been inserted above the viewport.
    visible_history_rows: u16,
}

impl<B> ViewportTerminal<B>
where
    B: Backend + io::Write,
    <B as Backend>::Error: Into<io::Error>,
{
    /// Create a new ViewportTerminal at a specific cursor position.
    pub fn with_options_and_cursor_position(
        backend: B,
        cursor_pos: Position,
    ) -> io::Result<Self> {
        let screen_size = backend.size().map_err(Into::into)?;
        let viewport_area = Rect::new(0, cursor_pos.y, 0, 0);
        let buffer = Buffer::empty(viewport_area);

        Ok(Self {
            backend,
            buffers: [buffer.clone(), buffer],
            current: 0,
            hidden_cursor: false,
            viewport_area,
            last_known_screen_size: screen_size,
            last_known_cursor_pos: cursor_pos,
            visible_history_rows: 0,
        })
    }

    /// Set the viewport area and resize both buffers.
    pub fn set_viewport_area(&mut self, area: Rect) {
        self.viewport_area = area;
        for buf in &mut self.buffers {
            buf.resize(area);
            buf.reset();
        }
        self.visible_history_rows = self.visible_history_rows.min(area.top());
    }

    /// Get a ViewportFrame for rendering into the current buffer.
    pub fn get_frame(&mut self) -> ViewportFrame<'_> {
        ViewportFrame {
            cursor_position: None,
            viewport_area: self.viewport_area,
            buffer: self.current_buffer_mut(),
        }
    }

    /// Render using Ratatui's diff-and-flush pipeline.
    pub fn draw<F>(&mut self, render_callback: F) -> io::Result<()>
    where
        F: FnOnce(&mut ViewportFrame),
    {
        self.autoresize()?;

        let cursor_position;
        {
            let mut frame = self.get_frame();
            render_callback(&mut frame);
            cursor_position = frame.cursor_position;
        }

        self.apply_buffer_with_cursor(cursor_position)
    }

    /// Apply the buffer diff to the backend.
    fn apply_buffer_with_cursor(
        &mut self,
        cursor_position: Option<Position>,
    ) -> io::Result<()> {
        self.flush_diff()?;

        match cursor_position {
            None => {
                if !self.hidden_cursor {
                    crossterm::execute!(self.backend, crossterm::cursor::Hide)?;
                    self.hidden_cursor = true;
                }
            }
            Some(pos) => {
                let abs_pos = Position::new(
                    self.viewport_area.x + pos.x,
                    self.viewport_area.y + pos.y,
                );
                if self.hidden_cursor {
                    crossterm::execute!(self.backend, crossterm::cursor::Show)?;
                    self.hidden_cursor = false;
                }
                crossterm::execute!(
                    self.backend,
                    crossterm::cursor::MoveTo(abs_pos.x, abs_pos.y)
                )?;
            }
        }

        self.swap_buffers();
        // Flush backend I/O (not the ratatui Backend::flush).
        io::Write::flush(&mut self.backend)?;

        Ok(())
    }

    /// Diff current buffer against previous and write only changed cells.
    fn flush_diff(&mut self) -> io::Result<()> {
        let previous_buffer = &self.buffers[1 - self.current];
        let current_buffer = &self.buffers[self.current];
        let area = self.viewport_area;

        let mut last_pos: Option<Position> = None;

        for y in 0..area.height {
            for x in 0..area.width {
                let idx = (y as usize) * (area.width as usize) + (x as usize);
                let prev = &previous_buffer.content[idx];
                let curr = &current_buffer.content[idx];

                if prev == curr {
                    continue;
                }

                let abs_col = area.x + x;
                let abs_row = area.y + y;
                last_pos = Some(Position::new(x, y));

                // Move cursor
                crossterm::execute!(
                    self.backend,
                    crossterm::cursor::MoveTo(abs_col, abs_row)
                )?;

                // Emit style + color changes via raw ANSI.
                write_cell_style(&mut self.backend, curr)?;

                // Write character.
                write!(self.backend, "{}", curr.symbol())?;
            }
        }

        if let Some(pos) = last_pos {
            self.last_known_cursor_pos = pos;
        }

        Ok(())
    }

    /// Track terminal size changes. NEVER changes `viewport_area`.
    pub fn autoresize(&mut self) -> io::Result<()> {
        let size = self.backend.size().map_err(Into::into)?;
        if size != self.last_known_screen_size {
            self.last_known_screen_size = size;
        }
        Ok(())
    }

    fn swap_buffers(&mut self) {
        self.buffers[1 - self.current].reset();
        self.current = 1 - self.current;
    }

    /// Reset the back buffer to force full repaint next draw.
    pub fn invalidate_viewport(&mut self) {
        self.buffers[1 - self.current].reset();
    }


    pub fn clear_scrollback_and_visible_screen_ansi(&mut self) -> io::Result<()> {
        crossterm::execute!(
            self.backend,
            crossterm::style::ResetColor,
            crossterm::cursor::MoveTo(0, 0),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::Purge),
            crossterm::cursor::MoveTo(0, 0),
        )?;
        self.visible_history_rows = 0;
        self.invalidate_viewport();
        Ok(())
    }

    pub fn note_history_rows_inserted(&mut self, n: u16) {
        self.visible_history_rows = self
            .visible_history_rows
            .saturating_add(n)
            .min(self.viewport_area.top());
    }


    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn size(&self) -> io::Result<Size> {
        self.backend.size().map_err(Into::into)
    }



    fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }
}

impl<B: Backend + io::Write> Drop for ViewportTerminal<B> {
    fn drop(&mut self) {
        if self.hidden_cursor {
            let _ = crossterm::execute!(self.backend, crossterm::cursor::Show);
        }
    }
}

// ── Cell style writer (raw ANSI for simplicity) ─────────────────────────────

fn write_cell_style(w: &mut impl io::Write, cell: &Cell) -> io::Result<()> {
    use ratatui::style::Modifier;

    // Reset all attributes.
    write!(w, "\x1b[0")?;

    let m = cell.modifier;
    if m.contains(Modifier::BOLD) {
        write!(w, ";1")?;
    }
    if m.contains(Modifier::DIM) {
        write!(w, ";2")?;
    }
    if m.contains(Modifier::ITALIC) {
        write!(w, ";3")?;
    }
    if m.contains(Modifier::UNDERLINED) {
        write!(w, ";4")?;
    }
    if m.contains(Modifier::SLOW_BLINK) {
        write!(w, ";5")?;
    }
    if m.contains(Modifier::RAPID_BLINK) {
        write!(w, ";6")?;
    }
    if m.contains(Modifier::REVERSED) {
        write!(w, ";7")?;
    }
    if m.contains(Modifier::HIDDEN) {
        write!(w, ";8")?;
    }
    if m.contains(Modifier::CROSSED_OUT) {
        write!(w, ";9")?;
    }

    // Foreground color.
    match cell.fg {
        ratatui::style::Color::Reset => write!(w, ";39")?,
        ratatui::style::Color::Black => write!(w, ";30")?,
        ratatui::style::Color::Red => write!(w, ";31")?,
        ratatui::style::Color::Green => write!(w, ";32")?,
        ratatui::style::Color::Yellow => write!(w, ";33")?,
        ratatui::style::Color::Blue => write!(w, ";34")?,
        ratatui::style::Color::Magenta => write!(w, ";35")?,
        ratatui::style::Color::Cyan => write!(w, ";36")?,
        ratatui::style::Color::Gray => write!(w, ";37")?,
        ratatui::style::Color::DarkGray => write!(w, ";90")?,
        ratatui::style::Color::LightRed => write!(w, ";91")?,
        ratatui::style::Color::LightGreen => write!(w, ";92")?,
        ratatui::style::Color::LightYellow => write!(w, ";93")?,
        ratatui::style::Color::LightBlue => write!(w, ";94")?,
        ratatui::style::Color::LightMagenta => write!(w, ";95")?,
        ratatui::style::Color::LightCyan => write!(w, ";96")?,
        ratatui::style::Color::White => write!(w, ";97")?,
        ratatui::style::Color::Rgb(r, g, b) => write!(w, ";38;2;{};{};{}", r, g, b)?,
        ratatui::style::Color::Indexed(i) => write!(w, ";38;5;{}", i)?,
    }

    // Background color.
    match cell.bg {
        ratatui::style::Color::Reset => write!(w, ";49")?,
        ratatui::style::Color::Black => write!(w, ";40")?,
        ratatui::style::Color::Red => write!(w, ";41")?,
        ratatui::style::Color::Green => write!(w, ";42")?,
        ratatui::style::Color::Yellow => write!(w, ";43")?,
        ratatui::style::Color::Blue => write!(w, ";44")?,
        ratatui::style::Color::Magenta => write!(w, ";45")?,
        ratatui::style::Color::Cyan => write!(w, ";46")?,
        ratatui::style::Color::Gray => write!(w, ";47")?,
        ratatui::style::Color::DarkGray => write!(w, ";100")?,
        ratatui::style::Color::LightRed => write!(w, ";101")?,
        ratatui::style::Color::LightGreen => write!(w, ";102")?,
        ratatui::style::Color::LightYellow => write!(w, ";103")?,
        ratatui::style::Color::LightBlue => write!(w, ";104")?,
        ratatui::style::Color::LightMagenta => write!(w, ";105")?,
        ratatui::style::Color::LightCyan => write!(w, ";106")?,
        ratatui::style::Color::White => write!(w, ";107")?,
        ratatui::style::Color::Rgb(r, g, b) => write!(w, ";48;2;{};{};{}", r, g, b)?,
        ratatui::style::Color::Indexed(i) => write!(w, ";48;5;{}", i)?,
    }

    write!(w, "m")?;
    Ok(())
}
