# tlog — Android Logcat TUI

A terminal-based Android logcat viewer written in Rust.

[English](README.md) | [中文](README_ZH.md)

<img width="906" height="593" alt="image" src="https://github.com/user-attachments/assets/8777e329-fecb-4a17-94ce-fd2c06c57a68" />

## Features

- **Real-time log stream** — reads and parses `adb logcat -v threadtime` output
- **Android Studio-style filter** — supports `tag:`, `level:`, `package:`, `message:`, `age:`, `is:` key-value filters with regex, negation, and boolean operators
- **Package name resolution** — auto-resolves PID to package name from ActivityManager lifecycle events (inspired by pidcat)
- **Color highlighting** — color-coded by log level (V=gray D=cyan I=green W=yellow E=red F=red-bg), fully customizable in Settings
- **Display options** — set which fields show (timestamp, PID, TID, tag, level, color) in Settings (`o`)
- **Pause/resume** — freeze log output for careful inspection
- **Filter bypass** — temporarily disable the active filter with `g`, keeping the input intact
- **Search & highlight** — `/` opens a search box; matching tag/package/message text is highlighted in the current filter view
- **Quick level filters** — `v` / `d` / `i` / `w` / `e` add `level:` filters instantly
- **Filter editor upgrades** — readline-style `Ctrl+A/E/W`, applied-filter history (`↑`/`↓`), and saved named filters (`Ctrl+S`, then use `name:<name>`)
- **Export & offline playback** — `E` writes the current filtered view to a timestamped file; `tlog --file <log>` replays a log file with the same UI pipeline
- **Multi-language** — UI strings in English and Chinese, auto-detected from `LANG` or set via `--lang`
- **Multi-device** — select a device with `-s` / `--serial` or `ANDROID_SERIAL`, and choose log buffers with `-b` / `--buffer`
- **Resilient feed** — `--clear` flushes the device log first, `--exit-on-eof` quits when logcat stops, and `--reconnect` restarts it with exponential backoff
- **Opt-in diagnostics** — unparsed lines go to `tlog-rejected.log` in the config directory only when `--debug` (or `TLOG_DEBUG`) is set, with automatic rotation
- **Ring buffer** — 100,000 entries hard cap, memory-safe

## Installation

```bash
cargo install --path .
```

Or run directly:

```bash
cargo run --release
```

**Prerequisite:** [Android SDK command-line tools](https://developer.android.com/tools) must be installed and `adb` available on `PATH`.

## Usage

```bash
# Default: adb logcat -v threadtime
tlog

# Start with a pre-applied filter
tlog --filter 'tag:MainActivity & level:ERROR'

# Custom command (e.g. Termux)
tlog --cmd logcat,-v,threadtime
# `--cmd` is a full override and cannot be combined with -s/-b/-c.

# Select a device and buffers, clearing the device log first
tlog -s emulator-5554 -b main,system -c

# Exit when logcat stops / keep reconnecting when it drops
tlog --exit-on-eof
tlog --reconnect

# Write rejected lines and reader diagnostics to the config directory
tlog --debug

# Offline playback of a saved/exported log file (exits at EOF, no adb needed)
tlog --file crash.log
tlog --file crash.log --filter 'level:ERROR'

# Force language
tlog --lang en
tlog --lang zh
```

### Keybindings

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Quit |
| `p` / `Space` | Toggle pause/resume |
| `C` | Clear log buffer |
| `g` | Toggle filter bypass (keeps input, press again to restore) |
| `o` | Open settings (display options + colors) |
| `h` | Show keybindings help |
| `/` | Search and highlight matches in the current filter view |
| `E` | Export the current filtered view to `tlog-export-<timestamp>.log` |
| `v` / `d` / `i` / `w` / `e` | Add `level:V` / `level:D` / `level:I` / `level:W` / `level:E` to the filter |
| `Tab` | Switch focus to filter input |
| `Esc` | Return to log view (when editing filter / search) |
| `Enter` | Apply filter or search (when editing) |
| `←` / `→` | Move the filter cursor by character (when editing filter) |
| `Home` / `Ctrl+A` | Jump to the start of the filter input |
| `End` / `Ctrl+E` | Jump to the end of the filter input |
| `↑` / `↓` | Previous / next applied filter (history; restores the draft after the newest entry) |
| `Backspace` / `Delete` | Delete before / at the cursor (when editing filter) |
| `Ctrl+W` | Delete the word before the cursor (when editing filter) |
| `Ctrl+U` | Clear the filter input (when editing filter) |
| `Ctrl+S` | Save the current filter under a name; use it later with `name:<name>` |

> While paused, incoming log lines are not buffered and are discarded.
> Search is a case-insensitive literal match against the displayed tag/package
> and message text; press `/` again and submit an empty query to clear it.

### Settings screen (`o`)

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Switch category (Display / Colors) |
| `1`–`6` | Toggle display options (Display category) |
| `1`–`8` | Edit an item's color (type hex, `Enter` commits; Colors category) |
| `[` / `]` | Cycle color presets |
| `Esc` / `o` | Close settings |

### Filter syntax

```
# Key-value filter (substring match)
tag:MainActivity
level:ERROR        # >= semantics, matches ERROR and FATAL

# Regex (~ modifier)
tag~:My.*Tag

# Negation
-tag:Debug

# Boolean logic
tag:foo & level:ERROR    # AND (& binds tighter than |)
tag:foo | tag:bar        # OR
tag:foo tag:bar          # Same-key implicit OR
tag:foo level:ERROR      # Different-key implicit AND

# Special filters
age:5m                   # Last 5 minutes (window fixed at filter compile time)
age:1h                   # Last 1 hour (window fixed at filter compile time)
is:crash                 # FATAL EXCEPTION
is:stacktrace            # Stack trace continuation lines
package:com.example      # Package name filter (requires resolved PID)
package:mine             # Always true (no project context)
name:errors              # Saved filter (created with Ctrl+S; quote names with `-`)
```

### Search, export, and offline playback

- `/` enters search mode. `Enter` applies a **case-insensitive literal** search;
  all matching entries in the current filtered view are highlighted (tag,
  package, and message text). New entries are highlighted as they arrive.
  Submitting an empty query clears the highlight.
- `E` writes every entry in the current filtered view (up to the 100k ring
  buffer) to `tlog-export-<timestamp>.log` in the working directory, in the
  canonical `-v threadtime` format.
- `tlog --file <log>` replays an existing log file through the same parser and
  UI. It does not require `adb` and exits automatically at EOF, which is handy
  for CI demos and recordings.
- Terminal resize bursts are debounced for 75 ms before the scrollback is
  cleared and replayed.

## Architecture

```
┌─ main.rs ─── Event loop (tokio::select!) ─────┐
│  ├─ crossterm keyboard events                 │
│  ├─ logcat child process stdout → channel     │
│  └─ 250ms tick (periodic frame rendering)     │
├─ logcat.rs ─ Log parsing + process lifecycle ─┤
├─ filter.rs ─ pest grammar → AST → evaluation ─┤
├─ buffer.rs ─ Ring buffer (100k hard cap) ─────┤
├─ app.rs ──── Global state + message dispatch ─┤
├─ config.rs ── Colors + display persistence ───┤
├─ named_filters.rs ─ Saved filter persistence ─┤
├─ export.rs ── Filtered-view export ───────────┤
├─ ui.rs ───── ratatui rendering ───────────────┤
├─ i18n.rs ─── Multi-language messages ─────────┤
├─ scrollback.rs ─ Scrollback buffer ───────────┤
└─ viewport.rs ── Bottom viewport management ───┘
```

## Memory strategy

| Mechanism | Detail |
|-----------|--------|
| Ring buffer | 100,000 entries hard cap, evicts oldest 20,000 when full |
| Bounded channel | `mpsc::channel(1024)`, drops on overflow instead of queuing |
| Message truncation | Single message ≤ 4096 bytes, tag ≤ 256 bytes |
| Zero-copy view | `filtered: Vec<usize>` stores indices, formatting is done on-the-fly |
| Periodic shrink | `shrink_to_fit()` after eviction |

## Tech stack

| Purpose | Crate |
|---------|-------|
| TUI | ratatui 0.30 + crossterm 0.28 |
| Async | tokio 1.x |
| Filter grammar | pest 2.x |
| CLI | clap 4.x |
| Time | chrono 0.4 |
| Regex | regex 1.x |
| Error | color-eyre 0.6 |
| Binary lookup | which 7 |

## Acknowledgments

Package name resolution inspired by [JakeWharton/pidcat](https://github.com/JakeWharton/pidcat).

## Configuration

Settings are saved to `config.conf` on every change and loaded on startup. Colors
(6 log levels, tag, timestamp), the active preset, and the display options are
stored as `key = value` lines, e.g. `preset = default`, `verbose = #808080`,
`show_pid = true`.

The file location depends on the platform:

| Platform | Path |
|----------|------|
| Any (if `XDG_CONFIG_HOME` is set) | `$XDG_CONFIG_HOME/tlog/config.conf` |
| Linux / BSD | `~/.config/tlog/config.conf` |
| macOS | `~/Library/Application Support/tlog/config.conf` |
| Windows | `%APPDATA%\tlog\config.conf` (falls back to `%USERPROFILE%\AppData\Roaming\tlog\config.conf`) |

Unknown or malformed lines are ignored per-key; missing keys fall back to
defaults.

Saved filters live next to `config.conf` in `filters.conf`, one
tab-separated `name<TAB>expression` line per filter.

## Changelog

See [CHANGELOGS.md](CHANGELOGS.md) for release notes.

## License

MIT
