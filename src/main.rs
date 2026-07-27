mod app;
mod buffer;
mod filter;
mod logcat;
mod scrollback;
mod i18n;
mod ui;
mod viewport;
use clap::Parser;
use color_eyre::Result;
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::layout::{Position, Rect};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time;

use app::{App, Focus};
use logcat::{Message, parse_line, parse_ps_line};
use scrollback::insert_log_lines;
use viewport::ViewportTerminal;
/// Android Logcat TUI viewer.
#[derive(Parser, Debug)]
#[command(name = "tlog", version, about)]
struct Cli {
    /// Override the logcat command (comma-separated args).
    /// Default: adb,logcat,-v,threadtime
    #[arg(long, value_delimiter = ',', default_values = ["adb", "logcat", "-v", "threadtime"])]
    cmd: Vec<String>,
    /// Pre-populate and apply a filter on startup.
    #[arg(long)]
    filter: Option<String>,
    /// UI language (en / zh). Auto-detected from LANG if not set.
    #[arg(long)]
    lang: Option<String>,
}
/// Run `adb shell ps` to seed the PID→package map.
async fn seed_pid_map(map: &mut std::collections::HashMap<u32, String>, adb: &str) {
    let result = tokio::process::Command::new(adb)
        .args(["shell", "ps", "-A"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;

    let output = match result {
        Ok(o) => o,
        Err(_) => {
            // Try without -A (older Android versions)
            let result2 = tokio::process::Command::new(adb)
                .args(["shell", "ps"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .await;
            match result2 {
                Ok(o) => o,
                Err(_) => return,
            }
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some((pid, name)) = parse_ps_line(line) {
            map.insert(pid, name);
        }
    }
}
/// Spawn the logcat reader task. Returns the subprocess handle and message receiver.
///
/// The reader task parses logcat stdout line by line and sends `Message::NewEntry`
/// via the bounded channel. When the channel is full, messages are dropped.
async fn start_logcat(cmd: &[String]) -> Result<(tokio::process::Child, mpsc::Receiver<Message>)> {
    let (tx, rx) = mpsc::channel::<Message>(1024);

    let mut command = Command::new(&cmd[0]);
    if cmd.len() > 1 {
        command.args(&cmd[1..]);
    }
    command.stdout(Stdio::piped());
    command.stderr(Stdio::null());
    command.kill_on_drop(true);

    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout should be piped");

    tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut local_dropped: u64 = 0;
        let mut last_reported_dropped: u64 = 0;

        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    // Try parsing as a log entry
                    if let Some(entry) = parse_line(&line) {
                        // Check for ActivityManager lifecycle events
                        if entry.tag == "ActivityManager"
                            && let Some(event) = logcat::parse_lifecycle(&entry.tag, &entry.message)
                        {
                            match event {
                                logcat::LifecycleEvent::Started { pid, package } => {
                                    let _ = tx.try_send(Message::ProcessStarted { pid, package });
                                }
                                logcat::LifecycleEvent::Died { pid } => {
                                    let _ = tx.try_send(Message::ProcessDied { pid });
                                }
                            }
                        }

                        match tx.try_send(Message::NewEntry(entry)) {
                            Ok(()) => {}
                            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                local_dropped += 1;
                                // Report dropped count periodically
                                if local_dropped - last_reported_dropped >= 1000 {
                                    let _ = tx.try_send(Message::Dropped(local_dropped));
                                    last_reported_dropped = local_dropped;
                                }
                            }
                            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                break;
                            }
                        }
                    }
                }
                Ok(None) => {
                    // EOF
                    let _ = tx.send(Message::LogcatDied).await;
                    break;
                }
                Err(_) => {
                    let _ = tx.send(Message::LogcatDied).await;
                    break;
                }
            }
        }
    });

    Ok((child, rx))
}
const VIEWPORT_HEIGHT: u16 = 3;
const MAX_REPLAY: usize = 10_000;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    let lang = i18n::resolve(cli.lang.as_deref());
    let msgs = i18n::messages(lang);

    // Check that the logcat binary is available.
    if which::which(&cli.cmd[0]).is_err() {
        eprintln!("{}", msgs.adb_not_found_full(&cli.cmd[0]));
        std::process::exit(1);
    }

    // ── Inline terminal init ──────────────────────────────────────────────
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(crossterm::event::EnableBracketedPaste)?;

    let (cursor_col, cursor_row) = crossterm::cursor::position()?;
    let (term_cols, term_rows) = crossterm::terminal::size()?;

    // If cursor is near top, push it down so the viewport has room.
    let target_row = term_rows.saturating_sub(VIEWPORT_HEIGHT + 1);
    if cursor_row < target_row {
        for _ in cursor_row..target_row {
            print!("\n");
        }
        // Re-probe cursor after pushing
        let (_, new_row) = crossterm::cursor::position()?;
        let cursor_pos = Position::new(0, new_row);

        let backend = ratatui::backend::CrosstermBackend::new(stdout);
        let mut terminal = ViewportTerminal::with_options_and_cursor_position(backend, cursor_pos)?;
        terminal.clear_scrollback_and_visible_screen_ansi()?;
        terminal.set_viewport_area(Rect::new(
            0,
            term_rows.saturating_sub(VIEWPORT_HEIGHT),
            term_cols,
            VIEWPORT_HEIGHT,
        ));
        run_app(&mut terminal, cli, lang).await?;
    } else {
        let cursor_pos = Position::new(cursor_col, cursor_row);
        let backend = ratatui::backend::CrosstermBackend::new(stdout);
        let mut terminal = ViewportTerminal::with_options_and_cursor_position(backend, cursor_pos)?;
        terminal.clear_scrollback_and_visible_screen_ansi()?;
        terminal.set_viewport_area(Rect::new(
            0,
            term_rows.saturating_sub(VIEWPORT_HEIGHT),
            term_cols,
            VIEWPORT_HEIGHT,
        ));
        run_app(&mut terminal, cli, lang).await?;
    }

    // ── Cleanup ───────────────────────────────────────────────────────────
    crossterm::terminal::disable_raw_mode()?;
    // Reset scroll region to full screen.
    print!("\x1b[r");

    Ok(())
}

async fn run_app(
    terminal: &mut ViewportTerminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    cli: Cli,
    lang: i18n::Lang,
) -> Result<()> {
    let mut app = App::new(lang);

    // Seed PID→package map from `adb shell ps`
    seed_pid_map(&mut app.pid_package_map, &cli.cmd[0]).await;

    // Apply startup filter if provided
    if let Some(ref f) = cli.filter {
        app.filter_input = f.clone();
        app.apply_filter();
        // Replay all entries into scrollback (none yet, but init the viewport).
        replay_filtered(terminal, &app)?;
    }

    let (child, mut msg_rx) = start_logcat(&cli.cmd).await?;
    app.running = true;
    let mut child_handle = Some(child);

    let mut ticker = time::interval(Duration::from_millis(250));
    let mut event_stream = event::EventStream::new();

    let result: Result<()> = loop {
        tokio::select! {
            Some(Ok(event)) = event_stream.next() => {
                match event {
                    Event::Key(key) if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat => {
                        if !handle_key(&mut app, key) {
                            break Ok(());
                        }
                    }
                    Event::Resize(cols, rows) => {
                        let vh = app.viewport_height();
                        let new_area = Rect::new(0, rows.saturating_sub(vh), cols, vh);
                        terminal.clear_scrollback_and_visible_screen_ansi()?;
                        terminal.set_viewport_area(new_area);
                        replay_filtered(terminal, &app)?;
                    }
                    _ => {}
                }
            }
            msg = msg_rx.recv() => {
                match msg {
                    Some(Message::NewEntry(entry)) => {
                        app.dispatch(Message::NewEntry(entry));
                    }
                    Some(m) => {
                        if !app.dispatch(m) {
                            break Ok(());
                        }
                    }
                    None => {
                        app.running = false;
                    }
                }
            }

            _ = ticker.tick() => {
                app.dispatch(Message::Tick);
            }
        }
        // Handle filter change: clear scrollback and replay.
        if app.needs_replay {
            terminal.clear_scrollback_and_visible_screen_ansi()?;
            // Recompute viewport position after clear.
            let size = terminal.size()?;
            let vh = app.viewport_height();
            terminal.set_viewport_area(Rect::new(
                0,
                size.height.saturating_sub(vh),
                size.width,
                vh,
            ));
            replay_filtered(terminal, &app)?;
            app.needs_replay = false;
        }

        // Flush pending log lines into scrollback.
        if !app.pending_log_lines.is_empty() {
            insert_log_lines(terminal, &app.pending_log_lines)?;
            app.pending_log_lines.clear();
        }

        // Render the viewport (bottom bar).
        terminal.draw(|f| ui::render_viewport(f, &app))?;
    };

    // Cleanup
    if let Some(mut child) = child_handle.take() {
        child.start_kill().ok();
        let _ = child.wait().await;
    }

    result
}

/// Replay all filtered entries into scrollback (for filter changes / resize).
fn replay_filtered(
    terminal: &mut ViewportTerminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &App,
) -> std::io::Result<()> {
    let total = app.buffer.filtered_len();
    if total == 0 {
        return Ok(());
    }

    // Only replay the most recent MAX_REPLAY entries.
    let start = total.saturating_sub(MAX_REPLAY);
    let mut batch: Vec<ratatui::text::Line<'static>> = Vec::with_capacity(256);

    for pos in start..total {
        if let Some((_, entry)) = app.buffer.get_filtered(pos) {
            batch.push(ui::format_entry(entry, &app.config));
        }
        if batch.len() >= 256 {
            insert_log_lines(terminal, &batch)?;
            batch.clear();
        }
    }
    if !batch.is_empty() {
        insert_log_lines(terminal, &batch)?;
    }

    Ok(())
}

fn handle_key(app: &mut App, key: event::KeyEvent) -> bool {
    match app.focus {
        Focus::LogView => handle_logview_key(app, key),
        Focus::FilterInput => handle_filter_key(app, key),
    }
}

fn handle_logview_key(app: &mut App, key: event::KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') => return false,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return false,

        // Pause/resume
        KeyCode::Char('p') | KeyCode::Char(' ') => {
            app.paused = !app.paused;
        }
        // Clear buffer and scrollback
        KeyCode::Char('C') => {
            app.buffer.clear();
            app.pending_log_lines.clear();
            app.needs_replay = true;
        }
        // Toggle filter bypass (g key)
        KeyCode::Char('g') => {
            app.toggle_filter_bypass();
        }
        KeyCode::Tab => {
            app.focus = Focus::FilterInput;
        }
        // Display config toggles (1-6)
        KeyCode::Char('1') => app.toggle_config('1'),
        KeyCode::Char('2') => app.toggle_config('2'),
        KeyCode::Char('3') => app.toggle_config('3'),
        KeyCode::Char('4') => app.toggle_config('4'),
        KeyCode::Char('5') => app.toggle_config('5'),
        KeyCode::Char('6') => app.toggle_config('6'),
        // Options overlay: toggle display config status
        KeyCode::Char('o') => {
            app.show_options = !app.show_options;
            app.show_help = false;
            app.needs_replay = true;
        }
        // Help overlay: toggle keybindings
        KeyCode::Char('h') => {
            app.show_help = !app.show_help;
            app.show_options = false;
            app.needs_replay = true;
        }
        _ => {}
    }
    true
}

fn handle_filter_key(app: &mut App, key: event::KeyEvent) -> bool {
    // Ctrl+U must be checked before Char(c) to avoid unreachable pattern
    if key.code == KeyCode::Char('u') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.filter_input.clear();
        return true;
    }

    match key.code {
        KeyCode::Esc => {
            app.focus = Focus::LogView;
            app.filter_error = None;
        }
        KeyCode::Tab => {
            app.focus = Focus::LogView;
        }
        KeyCode::Enter => {
            app.apply_filter();
            app.focus = Focus::LogView;
        }
        KeyCode::Char(c) => {
            app.filter_input.push(c);
        }
        KeyCode::Backspace => {
            app.filter_input.pop();
        }
        KeyCode::Delete if !app.filter_input.is_empty() => {
            app.filter_input.remove(0);
        }
        _ => {}
    }
    true
}
