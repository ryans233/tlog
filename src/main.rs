mod app;
mod buffer;
mod config;
mod filter;
mod i18n;
mod logcat;
mod rejected;
mod scrollback;
mod ui;
mod viewport;
use clap::Parser;
use color_eyre::Result;
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::layout::{Position, Rect};
use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader};
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
    #[arg(long, value_delimiter = ',')]
    cmd: Option<Vec<String>>,
    /// Target a device by serial number (passed as `adb -s <serial>`).
    #[arg(long, short = 's', conflicts_with = "cmd")]
    serial: Option<String>,
    /// Log buffer(s) to pass to `adb logcat -b` (repeatable or comma-separated).
    #[arg(long, short = 'b', value_delimiter = ',', conflicts_with = "cmd")]
    buffer: Vec<String>,
    /// Clear the device log before streaming (`adb logcat -c`).
    #[arg(long, short = 'c', conflicts_with = "cmd")]
    clear: bool,
    /// Exit when the logcat feed ends instead of leaving the UI open.
    #[arg(long, conflicts_with = "reconnect")]
    exit_on_eof: bool,
    /// Reconnect with exponential backoff when the logcat feed ends.
    #[arg(long, conflicts_with = "exit_on_eof")]
    reconnect: bool,
    /// Write rejected lines and reader diagnostics to the config-dir rejected log.
    #[arg(long)]
    debug: bool,
    /// Pre-populate and apply a filter on startup.
    #[arg(long)]
    filter: Option<String>,
    /// UI language (en / zh). Auto-detected from LANG if not set.
    #[arg(long)]
    lang: Option<String>,
}

impl Cli {
    /// Whether the rejected log is enabled via `--debug` or `TLOG_DEBUG`.
    fn debug_enabled(&self) -> bool {
        self.debug || env_flag_enabled("TLOG_DEBUG")
    }

    /// Resolve the device serial from `--serial`, a custom `--cmd`'s `-s`
    /// argument, or `ANDROID_SERIAL`.
    fn resolved_serial(&self) -> Option<String> {
        if let Some(serial) = &self.serial {
            return Some(serial.clone());
        }
        if let Some(cmd) = &self.cmd {
            let mut args = cmd.iter();
            while let Some(arg) = args.next() {
                if arg == "-s" || arg == "--serial" {
                    return args.next().cloned();
                }
            }
        }
        std::env::var("ANDROID_SERIAL")
            .ok()
            .filter(|s| !s.is_empty())
    }

    /// The logcat command passed to `Command::new`.
    ///
    /// `--cmd` is a full override; otherwise tlog builds
    /// `adb [-s SERIAL] logcat [-b BUF]... -v threadtime`.
    fn logcat_args(&self) -> Vec<String> {
        if let Some(cmd) = &self.cmd {
            return cmd.clone();
        }
        let mut args = vec!["adb".to_string()];
        if let Some(serial) = self.resolved_serial() {
            args.push("-s".to_string());
            args.push(serial);
        }
        args.push("logcat".to_string());
        for buffer in &self.buffer {
            args.push("-b".to_string());
            args.push(buffer.clone());
        }
        args.extend(["-v".to_string(), "threadtime".to_string()]);
        args
    }

    /// Command for clearing the device log, if `--clear` was requested.
    fn clear_args(&self) -> Option<Vec<String>> {
        if !self.clear || self.cmd.is_some() {
            return None;
        }
        let mut args = vec!["adb".to_string()];
        if let Some(serial) = self.resolved_serial() {
            args.push("-s".to_string());
            args.push(serial);
        }
        args.extend(["logcat".to_string(), "-c".to_string()]);
        Some(args)
    }
}

fn env_flag_enabled(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "on" | "ON" | "yes" | "YES")
    )
}
const SEED_TIMEOUT: Duration = Duration::from_secs(3);
const CLEAR_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_LOG_LINE_BYTES: usize = 64 * 1024;

/// Run `adb shell ps` to seed the PID→package map.
///
/// Tries `ps -A` first, then falls back to `ps` for older Android versions.
async fn seed_pid_map(map: &mut HashMap<u32, String>, adb: &str, serial: Option<&str>) {
    let attempts: [&[&str]; 2] = [&["shell", "ps", "-A"], &["shell", "ps"]];
    for args in attempts {
        let mut command = Command::new(adb);
        if let Some(serial) = serial {
            command.arg("-s").arg(serial);
        }
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let Ok(output) = command.output().await else {
            continue;
        };
        let before = map.len();
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some((pid, name)) = parse_ps_line(line) {
                map.insert(pid, name);
            }
        }
        if map.len() > before {
            return;
        }
        // `ps -A` succeeded but yielded no parseable rows (e.g. an older
        // Android format); fall through and try plain `ps`.
    }
}

/// Run a short adb command (used for `logcat -c`) and report success.
async fn run_adb_command(args: &[String]) -> bool {
    let mut command = Command::new(&args[0]);
    if args.len() > 1 {
        command.args(&args[1..]);
    }
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    matches!(command.status().await, Ok(status) if status.success())
}

/// Read one line, bounding the retained buffer at `max` bytes.
///
/// If the line exceeds `max`, the remainder is discarded through the next
/// newline so the stream stays aligned and memory use stays bounded.
async fn read_until_limited<R>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    delim: u8,
    max: usize,
) -> std::io::Result<usize>
where
    R: AsyncBufRead + Unpin,
{
    buf.clear();
    let mut limited = (&mut *reader).take(max as u64);
    let n = limited.read_until(delim, buf).await?;
    if n == 0 {
        return Ok(0);
    }
    let truncated = n >= max && buf.last().is_none_or(|last| *last != delim);
    if truncated {
        discard_until(reader, delim).await?;
    }
    Ok(n)
}

/// Discard buffered input through the next `delim` (or EOF) without retaining it.
async fn discard_until<R>(reader: &mut R, delim: u8) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
{
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(());
        }
        if let Some(pos) = available.iter().position(|byte| *byte == delim) {
            reader.consume(pos + 1);
            return Ok(());
        }
        let len = available.len();
        reader.consume(len);
    }
}

struct LogcatFeed {
    task: tokio::task::JoinHandle<()>,
    rx: mpsc::Receiver<Message>,
}

/// Spawn the logcat reader task and return its message receiver.
///
/// The task owns the subprocess so it can restart it for `--reconnect`.
/// Aborting the task kills the current child (`kill_on_drop`).
fn spawn_logcat_feed(
    cmd: &[String],
    reconnect: bool,
    rejected: rejected::RejectedLog,
) -> LogcatFeed {
    let (tx, rx) = mpsc::channel::<Message>(1024);
    let cmd = cmd.to_vec();
    let task = tokio::spawn(async move {
        run_logcat_feed(cmd, reconnect, rejected, tx).await;
    });
    LogcatFeed { task, rx }
}

/// Own the logcat subprocess lifecycle: parse stdout, report status changes,
/// and optionally reconnect with exponential backoff.
async fn run_logcat_feed(
    cmd: Vec<String>,
    reconnect: bool,
    rejected: rejected::RejectedLog,
    tx: mpsc::Sender<Message>,
) {
    if cmd.is_empty() {
        rejected.log("logcat reader: empty command");
        let _ = tx.send(Message::LogcatDied).await;
        return;
    }
    let mut local_dropped: u64 = 0;
    let mut last_reported_dropped: u64 = 0;
    let mut attempt: u32 = 0;

    loop {
        let mut command = Command::new(&cmd[0]);
        if cmd.len() > 1 {
            command.args(&cmd[1..]);
        }
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                rejected.log(&format!("logcat spawn failed: {}", err));
                if tx.send(Message::LogcatDied).await.is_err() {
                    return;
                }
                if !reconnect {
                    return;
                }
                time::sleep(reconnect_delay(attempt)).await;
                attempt = attempt.saturating_add(1);
                continue;
            }
        };

        if tx.send(Message::LogcatStarted).await.is_err() {
            let _ = child.start_kill();
            return;
        }

        let stdout = child.stdout.take().expect("stdout should be piped");
        let mut reader = BufReader::new(stdout);
        let mut buf: Vec<u8> = Vec::new();

        loop {
            match read_until_limited(&mut reader, &mut buf, b'\n', MAX_LOG_LINE_BYTES).await {
                Ok(0) => {
                    rejected.log("logcat reader: EOF (adb process exited)");
                    break;
                }
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\n', '\r']);
                    if let Some(entry) = parse_line(line) {
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
                                if local_dropped - last_reported_dropped >= 1000 {
                                    let _ = tx.try_send(Message::Dropped(local_dropped));
                                    last_reported_dropped = local_dropped;
                                }
                            }
                            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                return;
                            }
                        }
                    } else {
                        // Line didn't match expected format — append to the
                        // opt-in rejected log.
                        rejected.unparsed(line);
                    }
                }
                Err(err) => {
                    rejected.log(&format!("logcat reader: read error: {}", err));
                    break;
                }
            }
        }

        let _ = child.wait().await;
        if tx.send(Message::LogcatDied).await.is_err() {
            return;
        }
        if !reconnect {
            return;
        }
        time::sleep(reconnect_delay(attempt)).await;
        attempt = attempt.saturating_add(1);
    }
}

/// Exponential backoff: 1s, 2s, 4s, 8s, 16s, capped at 30s.
fn reconnect_delay(attempt: u32) -> Duration {
    let shift = attempt.min(5);
    Duration::from_secs(1u64 << shift).min(Duration::from_secs(30))
}
const VIEWPORT_HEIGHT: u16 = 3;
const MAX_REPLAY: usize = 10_000;

/// Restore the terminal after tlog exits: reset styles, clear the visible
/// screen and the scrollback the session filled, and park the cursor on the
/// bottom row so the shell prompt lands in a clean terminal.
fn restore_terminal<W: std::io::Write>(w: &mut W, term_rows: u16) -> std::io::Result<()> {
    crossterm::execute!(
        w,
        crossterm::style::ResetColor,
        crossterm::cursor::MoveTo(0, term_rows.saturating_sub(1)),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::Purge),
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    let lang = i18n::resolve(cli.lang.as_deref());
    let msgs = i18n::messages(lang);

    let logcat_args = cli.logcat_args();
    let logcat_bin = logcat_args
        .first()
        .cloned()
        .unwrap_or_else(|| "adb".to_string());

    // Check that the logcat binary is available.
    if which::which(&logcat_bin).is_err() {
        eprintln!("{}", msgs.adb_not_found_full(&logcat_bin));
        std::process::exit(1);
    }

    let rejected_log = rejected::RejectedLog::new(cli.debug_enabled());

    // ── Inline terminal init ──────────────────────────────────────────────
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(crossterm::event::EnableBracketedPaste)?;

    let (cursor_col, cursor_row) = crossterm::cursor::position()?;
    let (term_cols, term_rows) = crossterm::terminal::size()?;

    // If cursor is near top, push it down so the viewport has room.
    let target_row = term_rows.saturating_sub(VIEWPORT_HEIGHT + 1);
    let result = if cursor_row < target_row {
        for _ in cursor_row..target_row {
            println!();
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
        run_app(&mut terminal, cli, lang, rejected_log).await
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
        run_app(&mut terminal, cli, lang, rejected_log).await
    };

    // ── Cleanup ───────────────────────────────────────────────────────────
    // Restore the terminal before propagating any error, so nothing tlog
    // drew remains on screen or in the scrollback after quitting.
    if let Ok((_, term_rows)) = crossterm::terminal::size() {
        let mut stdout = std::io::stdout();
        let _ = restore_terminal(&mut stdout, term_rows);
    }
    crossterm::terminal::disable_raw_mode()?;
    // Reset scroll region to full screen.
    print!("\x1b[r");

    result?;
    Ok(())
}

async fn run_app(
    terminal: &mut ViewportTerminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    cli: Cli,
    lang: i18n::Lang,
    rejected_log: rejected::RejectedLog,
) -> Result<()> {
    let mut app = App::new(lang);
    let logcat_args = cli.logcat_args();
    let adb = logcat_args
        .first()
        .cloned()
        .unwrap_or_else(|| "adb".to_string());
    let serial = cli.resolved_serial();

    // Clear the device log before streaming. This is intentionally bounded so
    // a stuck adb call cannot hang startup forever.
    if let Some(clear_args) = cli.clear_args() {
        let _ = time::timeout(CLEAR_TIMEOUT, run_adb_command(&clear_args)).await;
    }

    // Seed the PID→package map in the background with a timeout. The UI and
    // logcat stream start immediately; the map is merged when the task lands.
    let seed_adb = adb.clone();
    let seed_serial = serial.clone();
    let mut seed_task = tokio::spawn(async move {
        let mut map = HashMap::new();
        let _ = time::timeout(
            SEED_TIMEOUT,
            seed_pid_map(&mut map, &seed_adb, seed_serial.as_deref()),
        )
        .await;
        map
    });
    let mut seed_done = false;

    // Apply startup filter if provided
    if let Some(ref filter) = cli.filter {
        app.filter_input = filter.clone();
        app.filter_move_end();
        app.apply_filter();
        // Replay all entries into scrollback (none yet, but init the viewport).
        replay_filtered(terminal, &app)?;
    }

    let feed = spawn_logcat_feed(&logcat_args, cli.reconnect, rejected_log);
    let mut feed_task = Some(feed.task);
    let mut msg_rx = feed.rx;
    let mut feed_closed = false;
    app.running = true;

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
            seed = &mut seed_task, if !seed_done => {
                seed_done = true;
                if let Ok(map) = seed {
                    app.apply_seed_package_map(map);
                }
            }
            msg = msg_rx.recv(), if !feed_closed => {
                match msg {
                    Some(Message::NewEntry(entry)) => {
                        app.dispatch(Message::NewEntry(entry));
                    }
                    Some(message) => {
                        let feed_died = matches!(message, Message::LogcatDied);
                        let exit_on_eof = feed_died && cli.exit_on_eof;
                        app.dispatch(message);
                        if feed_died && !cli.reconnect {
                            // Keep the UI open in STOPPED state without busy-polling
                            // the closed channel.
                            feed_closed = true;
                        }
                        if exit_on_eof {
                            break Ok(());
                        }
                    }
                    None => {
                        feed_closed = true;
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
            // Replay covers every entry already in the buffer; pending lines
            // would otherwise be inserted after the replay and duplicate them.
            app.pending_log_lines.clear();
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

    // Cleanup: abort the feed task so its `kill_on_drop` child is terminated.
    if let Some(task) = feed_task.take() {
        task.abort();
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
            batch.push(ui::format_entry(entry, &app.config, &app.palette));
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
        _ => {}
    }

    // Settings overlay captures all keys until closed (Esc / o).
    if app.show_settings {
        return handle_settings_key(app, key);
    }

    match key.code {
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
            app.filter_move_end();
        }
        // Settings overlay: display config + colors (o)
        KeyCode::Char('o') => {
            app.show_settings = !app.show_settings;
            app.show_help = false;
            app.needs_replay = true;
        }
        // Help overlay: toggle keybindings
        KeyCode::Char('h') => {
            app.show_help = !app.show_help;
            app.show_settings = false;
            app.needs_replay = true;
        }
        _ => {}
    }
    true
}

/// Keys while the settings overlay is open. Display category: 1-6 toggle the
/// display options; Color category: 1-8 edit hex, [ ] cycle presets; Tab
/// switches category; Esc / o / c close.
fn handle_settings_key(app: &mut App, key: event::KeyEvent) -> bool {
    use app::SettingsCategory;

    if app.settings_category == SettingsCategory::Color {
        // While editing an item, all keys belong to the hex input.
        if app.color_editing.is_some() {
            handle_color_edit_key(app, key);
            return true;
        }
        match key.code {
            KeyCode::Char(c) if crate::config::ColorItem::from_key(c).is_some() => {
                if let Some(item) = crate::config::ColorItem::from_key(c) {
                    app.color_editing = Some(item);
                    app.color_input = String::new();
                    app.color_error = None;
                }
            }
            KeyCode::Char('[') => apply_preset(app, app.color_preset.prev()),
            KeyCode::Char(']') => apply_preset(app, app.color_preset.next()),
            KeyCode::Tab => switch_category(app, app.settings_category.next()),
            KeyCode::BackTab => switch_category(app, app.settings_category.prev()),
            KeyCode::Esc | KeyCode::Char('o') => close_settings(app),
            _ => {}
        }
        return true;
    }

    // Display category.
    match key.code {
        KeyCode::Char('1') => app.toggle_config('1'),
        KeyCode::Char('2') => app.toggle_config('2'),
        KeyCode::Char('3') => app.toggle_config('3'),
        KeyCode::Char('4') => app.toggle_config('4'),
        KeyCode::Char('5') => app.toggle_config('5'),
        KeyCode::Char('6') => app.toggle_config('6'),
        KeyCode::Tab => switch_category(app, app.settings_category.next()),
        KeyCode::BackTab => switch_category(app, app.settings_category.prev()),
        KeyCode::Esc | KeyCode::Char('o') => close_settings(app),
        _ => {}
    }
    true
}

/// Hex-input sub-state of the Color category.
fn handle_color_edit_key(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Char(c) if c.is_ascii_hexdigit() && app.color_input.len() < 6 => {
            app.color_input.push(c);
        }
        KeyCode::Backspace => {
            app.color_input.pop();
        }
        KeyCode::Enter => commit_color_edit(app),
        KeyCode::Esc => {
            app.color_editing = None;
            app.color_error = None;
        }
        _ => {}
    }
}

fn switch_category(app: &mut App, category: app::SettingsCategory) {
    app.settings_category = category;
    app.needs_replay = true;
}

fn close_settings(app: &mut App) {
    app.show_settings = false;
    app.needs_replay = true;
}

fn commit_color_edit(app: &mut App) {
    let Some(item) = app.color_editing else {
        return;
    };
    match crate::config::hex_to_color(&app.color_input) {
        Some(c) => {
            item.set(&mut app.palette, c);
            app.color_editing = None;
            app.color_error = None;
            app.color_preset = crate::config::Preset::Custom;
            app.needs_replay = true; // scrollback must re-render with the new color
            app.save_config();
        }
        None => {
            app.color_error = Some(app.msgs.color_invalid_hex.to_string());
        }
    }
}

fn apply_preset(app: &mut App, preset: crate::config::Preset) {
    if preset == crate::config::Preset::Custom {
        return;
    }
    preset.apply(&mut app.palette);
    app.color_preset = preset;
    app.color_error = None;
    app.needs_replay = true;
    app.save_config();
}

fn handle_filter_key(app: &mut App, key: event::KeyEvent) -> bool {
    // Ctrl+U must be checked before Char(c) to avoid unreachable pattern
    if key.code == KeyCode::Char('u') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.filter_clear();
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
            app.filter_insert_char(c);
        }
        KeyCode::Backspace => {
            app.filter_backspace();
        }
        KeyCode::Delete => {
            app.filter_delete();
        }
        KeyCode::Left => app.filter_move_left(),
        KeyCode::Right => app.filter_move_right(),
        KeyCode::Up | KeyCode::Home => app.filter_move_start(),
        KeyCode::Down | KeyCode::End => app.filter_move_end(),
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    /// Serializes tests that touch the process-global XDG_CONFIG_HOME.
    static CONFIG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Point XDG_CONFIG_HOME at a unique temp dir so saves land in an isolated
    /// file. Returns the dir plus the lock guard (held for the test's lifetime).
    fn isolate_config(tag: &str) -> (std::path::PathBuf, std::sync::MutexGuard<'static, ()>) {
        let guard = CONFIG_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("tlog-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: the lock guarantees no other test reads XDG_CONFIG_HOME while
        // this test owns it; each test uses its own unique dir.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        (dir, guard)
    }

    fn char_key(c: char) -> event::KeyEvent {
        event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn code_key(code: KeyCode) -> event::KeyEvent {
        event::KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_cli_builds_serial_buffer_and_clear_args() {
        let cli = Cli::try_parse_from([
            "tlog",
            "--serial",
            "ABC123",
            "--buffer",
            "main,system",
            "--clear",
        ])
        .expect("cli parses");
        assert_eq!(
            cli.logcat_args(),
            [
                "adb",
                "-s",
                "ABC123",
                "logcat",
                "-b",
                "main",
                "-b",
                "system",
                "-v",
                "threadtime"
            ]
        );
        assert_eq!(
            cli.clear_args().expect("clear requested"),
            ["adb", "-s", "ABC123", "logcat", "-c"]
        );
        assert_eq!(cli.resolved_serial().as_deref(), Some("ABC123"));
    }

    #[test]
    fn test_cli_custom_cmd_extracts_serial_for_seed() {
        let cli = Cli::try_parse_from(["tlog", "--cmd", "adb,-s,XYZ,logcat,-v,threadtime"])
            .expect("cli parses");
        assert_eq!(
            cli.logcat_args(),
            ["adb", "-s", "XYZ", "logcat", "-v", "threadtime"]
        );
        assert_eq!(cli.resolved_serial().as_deref(), Some("XYZ"));
        assert_eq!(cli.clear_args(), None, "--clear conflicts with --cmd");
    }

    #[test]
    fn test_cli_eof_options_conflict() {
        assert!(Cli::try_parse_from(["tlog", "--exit-on-eof", "--reconnect"]).is_err());
        assert!(Cli::try_parse_from(["tlog", "--reconnect"]).is_ok());
        assert!(Cli::try_parse_from(["tlog", "--exit-on-eof"]).is_ok());
    }

    #[test]
    fn test_reconnect_delay_backoff() {
        assert_eq!(reconnect_delay(0), Duration::from_secs(1));
        assert_eq!(reconnect_delay(1), Duration::from_secs(2));
        assert_eq!(reconnect_delay(4), Duration::from_secs(16));
        assert_eq!(reconnect_delay(5), Duration::from_secs(30));
        assert_eq!(reconnect_delay(100), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_read_until_limited_reads_short_lines() {
        let data = b"one\ntwo
";
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();

        assert_eq!(
            read_until_limited(&mut reader, &mut buf, b'\n', 1024)
                .await
                .unwrap(),
            4
        );
        assert_eq!(buf, b"one\n");
        assert_eq!(
            read_until_limited(&mut reader, &mut buf, b'\n', 1024)
                .await
                .unwrap(),
            4
        );
        assert_eq!(buf, b"two\n");
        assert_eq!(
            read_until_limited(&mut reader, &mut buf, b'\n', 1024)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn test_read_until_limited_discards_overflow_and_stays_aligned() {
        let mut data = vec![b'a'; MAX_LOG_LINE_BYTES + 5];
        data.push(b'\n');
        data.extend_from_slice(b"tail\n");
        let mut reader = BufReader::new(data.as_slice());
        let mut buf = Vec::new();

        let n = read_until_limited(&mut reader, &mut buf, b'\n', MAX_LOG_LINE_BYTES)
            .await
            .expect("read should succeed");
        assert_eq!(n, MAX_LOG_LINE_BYTES);
        assert_eq!(buf.len(), MAX_LOG_LINE_BYTES);
        assert!(buf.iter().all(|byte| *byte == b'a'));

        // The stream stays aligned: the next line is the one after the
        // discarded overflow.
        assert_eq!(
            read_until_limited(&mut reader, &mut buf, b'\n', 1024)
                .await
                .unwrap(),
            5
        );
        assert_eq!(buf, b"tail\n");
    }

    #[test]
    fn test_filter_arrow_keys_move_cursor() {
        let mut app = App::new_for_tests(i18n::Lang::En);
        app.focus = Focus::FilterInput;
        app.filter_input = "ab".to_string();
        app.filter_cursor = 0;

        // Right moves one character at a time; Up/Down jump to start/end.
        assert!(handle_filter_key(&mut app, code_key(KeyCode::Right)));
        assert_eq!(app.filter_cursor, 1);
        assert!(handle_filter_key(&mut app, code_key(KeyCode::Down)));
        assert_eq!(app.filter_cursor, 2);
        assert!(handle_filter_key(&mut app, code_key(KeyCode::Left)));
        assert_eq!(app.filter_cursor, 1);
        assert!(handle_filter_key(&mut app, code_key(KeyCode::Up)));
        assert_eq!(app.filter_cursor, 0);

        // Home/End map to the same start/end movement.
        assert!(handle_filter_key(&mut app, code_key(KeyCode::End)));
        assert_eq!(app.filter_cursor, 2);
        assert!(handle_filter_key(&mut app, code_key(KeyCode::Home)));
        assert_eq!(app.filter_cursor, 0);

        // Insertion and deletion happen at the cursor, not always at the ends.
        assert!(handle_filter_key(&mut app, code_key(KeyCode::Right)));
        assert!(handle_filter_key(&mut app, char_key('X')));
        assert_eq!(app.filter_input, "aXb");
        assert_eq!(app.filter_cursor, 2);
        assert!(handle_filter_key(&mut app, code_key(KeyCode::Backspace)));
        assert_eq!(app.filter_input, "ab");
        assert_eq!(app.filter_cursor, 1);
    }

    #[test]
    fn test_color_overlay_edit_commit_and_save() {
        let (dir, _guard) = isolate_config("edit");
        let mut app = App::new(i18n::Lang::En);

        // 'o' opens the settings overlay; it starts on the Display category.
        assert!(handle_logview_key(&mut app, char_key('o')));
        assert!(app.show_settings);
        assert_eq!(app.settings_category, app::SettingsCategory::Display);

        // Tab switches to the Color category.
        assert!(handle_logview_key(
            &mut app,
            event::KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
        ));
        assert_eq!(app.settings_category, app::SettingsCategory::Color);

        // '1' starts editing Verbose.
        assert!(handle_logview_key(&mut app, char_key('1')));
        assert_eq!(app.color_editing, Some(crate::config::ColorItem::Verbose));
        assert_eq!(app.color_input, "");

        // Hex digits accumulate (max 6).
        for c in ['f', 'f', '8', '8', '0', '0'] {
            assert!(handle_logview_key(&mut app, char_key(c)));
        }
        assert_eq!(app.color_input, "ff8800");
        // A 7th digit is rejected.
        assert!(handle_logview_key(&mut app, char_key('1')));
        assert_eq!(app.color_input, "ff8800");

        // Enter commits, flips preset to Custom, clears edit state.
        assert!(handle_logview_key(
            &mut app,
            event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ));
        assert_eq!(app.color_editing, None);
        assert_eq!(app.color_preset, crate::config::Preset::Custom);
        assert_eq!(app.palette.verbose, Color::Rgb(0xFF, 0x88, 0x00));
        assert_eq!(app.color_error, None);

        // Persisted immediately.
        let conf = crate::config::config_path().expect("config path");
        let text = std::fs::read_to_string(&conf).expect("config file written");
        assert!(text.contains("preset = custom"), "got: {text}");
        assert!(text.contains("verbose = #FF8800"), "got: {text}");

        // Esc closes the settings overlay.
        assert!(handle_logview_key(
            &mut app,
            event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert!(!app.show_settings);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_color_overlay_preset_cycle_and_save() {
        let (dir, _guard) = isolate_config("preset");
        let mut app = App::new(i18n::Lang::En);

        assert!(handle_logview_key(&mut app, char_key('o')));
        assert!(handle_logview_key(
            &mut app,
            event::KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
        ));
        // From Default, '[' goes to HighContrast.
        assert!(handle_logview_key(&mut app, char_key('[')));
        assert_eq!(app.color_preset, crate::config::Preset::HighContrast);
        assert_eq!(app.palette.verbose, Color::Rgb(0xA9, 0xA9, 0xA9));
        // ']' from HighContrast wraps to Default.
        assert!(handle_logview_key(&mut app, char_key(']')));
        assert_eq!(app.color_preset, crate::config::Preset::Default);
        assert_eq!(app.palette.verbose, Color::Rgb(0x80, 0x80, 0x80));

        // Persisted after the last switch.
        let conf = crate::config::config_path().expect("config path");
        let text = std::fs::read_to_string(&conf).expect("config file written");
        assert!(text.contains("preset = default"), "got: {text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_color_overlay_invalid_hex_and_cancel() {
        let (dir, _guard) = isolate_config("invalid");
        let mut app = App::new(i18n::Lang::En);

        assert!(handle_logview_key(&mut app, char_key('o')));
        assert!(handle_logview_key(
            &mut app,
            event::KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
        ));
        assert!(handle_logview_key(&mut app, char_key('1')));
        // Too-short hex: commit shows the invalid-hex error and keeps editing.
        for c in ['f', 'f'] {
            assert!(handle_logview_key(&mut app, char_key(c)));
        }
        assert!(handle_logview_key(
            &mut app,
            event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ));
        assert_eq!(app.color_editing, Some(crate::config::ColorItem::Verbose));
        assert_eq!(app.color_error.as_deref(), Some(app.msgs.color_invalid_hex));

        // Esc cancels the edit without saving.
        assert!(handle_logview_key(
            &mut app,
            event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert_eq!(app.color_editing, None);
        assert_eq!(app.color_error, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_display_toggle_persists() {
        let (dir, _guard) = isolate_config("display");
        let mut app = App::new(i18n::Lang::En);

        // Main-window quick toggles are gone: '2' does nothing here.
        assert!(handle_logview_key(&mut app, char_key('2')));
        assert!(app.config.show_pid);
        // 'c' no longer opens settings; only 'o' is the entry key.
        assert!(handle_logview_key(&mut app, char_key('c')));
        assert!(!app.show_settings);

        // 'o' opens settings; Display is the default category, so '2' toggles
        // the PID row through the real handler.
        assert!(handle_logview_key(&mut app, char_key('o')));
        assert!(app.show_settings);
        app.needs_replay = false;
        assert!(handle_logview_key(&mut app, char_key('2')));
        assert!(!app.config.show_pid);
        assert!(app.config.show_timestamp);
        // Display toggles re-render the scrollback immediately (like colors).
        assert!(app.needs_replay, "display toggle must request a replay");

        // Toggle persisted immediately.
        let conf = crate::config::config_path().expect("config path");
        let text = std::fs::read_to_string(&conf).expect("config file written");
        assert!(text.contains("show_pid = false"), "got: {text}");
        assert!(text.contains("show_timestamp = true"), "got: {text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_restore_terminal_clears_screen_and_scrollback() {
        let mut out = Vec::new();
        restore_terminal(&mut out, 24).expect("restore should succeed");
        let bytes = String::from_utf8(out).expect("ansi bytes");
        assert!(
            bytes.starts_with("\x1b[0m"),
            "resets colors first: {bytes:?}"
        );
        assert!(
            bytes.contains("\x1b[24;1H"),
            "cursor parked on bottom row: {bytes:?}"
        );
        assert!(
            bytes.ends_with("\x1b[2J\x1b[3J"),
            "clears screen + scrollback: {bytes:?}"
        );
    }

    #[test]
    fn test_palette_reloads_from_config_on_startup() {
        let (dir, _guard) = isolate_config("reload");
        let conf_dir = dir.join("tlog");
        std::fs::create_dir_all(&conf_dir).expect("create conf dir");
        std::fs::write(
            conf_dir.join("config.conf"),
            "preset = solarized\nverbose = #112233\ntag = #AABBCC\nshow_pid = false\ncolorize = false\n",
        )
        .expect("write config");

        let app = App::new(i18n::Lang::En);
        assert_eq!(app.color_preset, crate::config::Preset::Solarized);
        assert_eq!(app.palette.verbose, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(app.palette.tag, Color::Rgb(0xAA, 0xBB, 0xCC));
        // Unlisted keys keep defaults.
        assert_eq!(app.palette.debug, Color::Rgb(0x00, 0xFF, 0xFF));
        // Display settings load from the same file.
        assert!(!app.config.show_pid);
        assert!(!app.config.colorize);
        assert!(app.config.show_timestamp);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
