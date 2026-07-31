mod app;
mod buffer;
mod config;
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
        let mut reader = BufReader::new(stdout);
        let mut buf: Vec<u8> = Vec::new();
        let mut local_dropped: u64 = 0;
        use std::io::Write;
        let log_line = |msg: &str| {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true).append(true).open("tlog-rejected.log")
            {
                let _ = writeln!(f, "[tlog] {}", msg);
            }
        };
        let mut last_reported_dropped: u64 = 0;

        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) => {
                    let _ = log_line("logcat reader: EOF (adb process exited)");
                    let _ = tx.send(Message::LogcatDied).await;
                    break;
                }
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(|c| c == '\n' || c == '\r');
                    // Try parsing as a log entry
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
                                break;
                            }
                        }
                    } else {
                        // Line didn't match expected format — append to rejected log.
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true).append(true).open("tlog-rejected.log")
                        {
                            if line.len() > 200 {
                                let _ = writeln!(f, "[unparsed len={}] {}...", line.len(), &line[..200]);
                            } else {
                                let _ = writeln!(f, "{}", line);
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = log_line(&format!("logcat reader: read error: {}", e));
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

    // Settings overlay captures all keys until closed (Esc / o / c).
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
        }
        // Settings overlay: display config + colors (o / c)
        KeyCode::Char('o') | KeyCode::Char('c') => {
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
            KeyCode::Esc | KeyCode::Char('o') | KeyCode::Char('c') => close_settings(app),
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
        KeyCode::Esc | KeyCode::Char('o') | KeyCode::Char('c') => close_settings(app),
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
    let Some(item) = app.color_editing else { return; };
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    /// Serializes tests that touch the process-global XDG_CONFIG_HOME.
    static CONFIG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Point XDG_CONFIG_HOME at a unique temp dir so saves land in an isolated
    /// file. Returns the dir plus the lock guard (held for the test's lifetime).
    fn isolate_config(
        tag: &str,
    ) -> (std::path::PathBuf, std::sync::MutexGuard<'static, ()>) {
        let guard = CONFIG_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "tlog-test-{}-{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: the lock guarantees no other test reads XDG_CONFIG_HOME while
        // this test owns it; each test uses its own unique dir.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        (dir, guard)
    }

    fn char_key(c: char) -> event::KeyEvent {
        event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn test_color_overlay_edit_commit_and_save() {
        let (dir, _guard) = isolate_config("edit");
        let mut app = App::new(i18n::Lang::En);

        // 'c' opens the settings overlay; it starts on the Display category.
        assert!(handle_logview_key(&mut app, char_key('c')));
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

        assert!(handle_logview_key(&mut app, char_key('c')));
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

        assert!(handle_logview_key(&mut app, char_key('c')));
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
        assert_eq!(
            app.color_error.as_deref(),
            Some(app.msgs.color_invalid_hex)
        );

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
