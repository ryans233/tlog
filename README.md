# tlog — Android Logcat TUI

A terminal-based Android logcat viewer written in Rust.

[English](README.md) | [中文](README_ZH.md)

<img width="906" height="593" alt="image" src="https://github.com/user-attachments/assets/8777e329-fecb-4a17-94ce-fd2c06c57a68" />

## Features

- **Real-time log stream** — reads and parses `adb logcat -v threadtime` output
- **Android Studio-style filter** — supports `tag:`, `level:`, `package:`, `message:`, `age:`, `is:` key-value filters with regex, negation, and boolean operators
- **Package name resolution** — auto-resolves PID to package name from ActivityManager lifecycle events (inspired by pidcat)
- **Color highlighting** — color-coded by log level (V=gray D=cyan I=green W=yellow E=red F=red-bg)
- **Display toggles** — press `1`–`6` to show/hide timestamp, PID, TID, tag, level, and color
- **Pause/resume** — freeze log output for careful inspection
- **Filter bypass** — temporarily disable the active filter with `g`, keeping the input intact
- **Multi-language** — UI strings in English and Chinese, auto-detected from `LANG` or set via `--lang`
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
| `o` | Print display config to scrollback |
| `h` | Show keybindings help |
| `1`–`6` | Toggle display options (timestamp/PID/TID/tag/level/color) |
| `Tab` | Switch focus to filter input |
| `Esc` | Return to log view (when editing filter) |
| `Enter` | Apply filter (when editing filter) |

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
age:5m                   # Last 5 minutes
age:1h                   # Last 1 hour
is:crash                 # FATAL EXCEPTION
is:stacktrace            # Stack trace continuation lines
package:com.example      # Package name filter (requires resolved PID)
package:mine             # Always true (no project context)
```

## Architecture

```
┌─ main.rs ─── Event loop (tokio::select!) ─┐
│  ├─ crossterm keyboard events              │
│  ├─ logcat child process stdout → channel  │
│  └─ 250ms tick (age filter refresh)        │
├─ logcat.rs ─ Log parsing + process lifecycle ─┤
├─ filter.rs ─ pest grammar → AST → evaluation ─┤
├─ buffer.rs ─ Ring buffer (100k hard cap) ─────┤
├─ app.rs ──── Global state + message dispatch ─┤
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

## License

MIT
