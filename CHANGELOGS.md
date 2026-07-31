# Changelog

All notable changes to tlog are documented in this file.

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
