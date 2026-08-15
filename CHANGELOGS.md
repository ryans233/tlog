# Changelog

All notable changes to tlog are documented in this file.

## [1.1.1] - 2026-08-15

### Added
- CI quality gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  and `cargo test --all-targets` now run on every push and pull request.
- UTF-8-safe cursor editing in the filter input: `Left`/`Right` move by character,
  `Home`/`Up` jump to the start, `End`/`Down` jump to the end, and
  insert/backspace/delete operate at the cursor; the input scrolls horizontally
  on narrow terminals.
- Unit tests for ActivityManager lifecycle parsing (all five event patterns) and
  `adb shell ps` line parsing.
- Filter tests for explicit parentheses, negated groups, `package:mine`, and
  invalid regex rejection.

### Fixed
- Group negation semantics: `-(tag:foo | tag:bar)` now means
  `NOT (foo OR bar)` and is evaluated as `-foo AND -bar`; it previously
  compiled incorrectly and could match everything.
- Explicit parentheses are preserved during filter compilation, so
  `(a | b) c` no longer degrades to `a | (b & c)`.
- Regex predicates are compiled once when the filter is applied instead of
  recompiling `Regex` for every log entry during evaluation.
- Invalid regex patterns now surface as a filter error immediately instead of
  silently matching nothing.
- `package:mine` is implemented as always-true (`-package:mine` always-false),
  matching the README and Android Studio semantics.
- All rustfmt and clippy warnings fixed.

### Changed
- Documented `age:` as a fixed window anchored at filter-compile time
  (previously documented as a per-tick rolling window).

## [1.1.0] - 2026-08-01

### Added
- Customizable log-entry colors. `o` opens a Settings screen; each colorable item
  (6 log levels, tag, timestamp) can be edited with a hex value (`1`–`8`), and
  presets cycle with `[` / `]` (Default / Solarized Dark / Monokai / High Contrast).
- Settings persistence: colors, active preset, and display toggles are saved to
  `<config-dir>/tlog/config.conf` on every change and reloaded on startup.
- Run-state indicator before the filter title: `RUNNING` (green), `PAUSED`
  (yellow, toggled with `p` / Space), `STOPPED` (red, when the logcat feed exits).
- Log messages are no longer truncated to 512 characters for display.

### Changed
- Display config and color config merged into a single Settings screen with
  category tabs (`Tab` / `Shift+Tab` switches between Display and Colors).
  `o` is the only entry key; `c` no longer opens it.
- Main-window `1`–`6` display quick-toggles removed — those options are now set
  inside Settings → Display.
- Display setting changes now re-render the existing scrollback immediately
  (previously only new entries picked them up).

### Fixed
- A manually edited palette reloaded as `Custom` instead of falling back to
  `Default` on restart.
- The terminal is cleared (visible screen + scrollback) on exit, so no tlog
  content remains in Windows Terminal after quitting.

## [1.0.0] - 2026-07-27

### Added
- Initial release: Android logcat TUI viewer (`-v threadtime` parsing) built on
  ratatui + crossterm — inline scrollback viewport, filter bar with live status,
  pause/clear/filter-bypass controls, help and display-config overlays, and
  PID→package name resolution from `adb shell ps`.
- Multi-platform CI build workflow.
