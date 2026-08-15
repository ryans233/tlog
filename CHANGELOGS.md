# Changelog

All notable changes to tlog are documented in this file.

## [1.4.1] - 2026-08-16

### Fixed
- Shift+E export now accepts both `Char('E')` and `Char('e') + SHIFT` key
  reports, so CapsLock/Shift combinations can no longer fall through to the
  quick `level:E` filter instead of exporting.
- Export/save notices and filter errors are now rendered on the visible second
  row of the bottom viewport; previously they were drawn one row outside the
  viewport and silently disappeared.

## [1.4.0] - 2026-08-16

### Added
- Segmented replay: filter/search/settings changes now repaint the latest
  10,000 entries in 128-line chunks, yielding to the event loop between chunks
  and deferring newly arrived lines until the replay finishes so scrollback
  order stays chronological.
- Regression benchmark target `benches/replay.rs` (`cargo bench --bench replay`)
  covering 100k-entry pushes, eviction, filter rebuilds, and 10k-entry replay
  formatting with and without search highlighting. `--quick` runs one
  iteration for CI smoke testing.
- Reusable quality-gates workflow (`.github/workflows/quality.yml`) used by CI
  and tag releases; tag releases now run format/lint/test/bench smoke before
  any release build.
- Release artifacts now include SHA256 checksums and standard
  `tlog-<target>.tar.gz` / `.zip` archives that `cargo-binstall` can consume
  directly from GitHub Releases.
- Ready-to-publish Homebrew, Scoop, and AUR templates under `packaging/`.
- Illustrative demo GIF plus the full `tlog --help` reference in both READMEs.
- Package metadata (`repository`, `license`, `description`) for installers and
  package managers.

### Changed
- CI now delegates its checks to the reusable quality workflow and additionally
  runs the benchmark smoke test.
- Release builds keep the existing raw binary names for backwards
  compatibility while adding standard archive names and checksum files.

### Performance
- Filter changes on a full buffer no longer block the UI for the entire
  replay; each iteration repaints at most 128 lines before processing keys,
  resize, ticks, and channel messages again.

## [1.3.0] - 2026-08-16

### Added
- Search and highlight: `/` opens the search input, and a case-insensitive
  literal query highlights matches in the displayed tag/package/message text of
  the current filtered view (including newly arriving entries). Submitting an
  empty query clears the highlight.
- Quick level filters: `v` / `d` / `i` / `w` / `e` append `level:V/D/I/W/E`
  with an explicit AND, so combined keys narrow to the strictest level.
- Filter history: successfully applied filters are recorded and navigated with
  `Up` / `Down`; the pre-navigation draft is restored after the newest entry.
- Readline-style filter editing: `Ctrl+A` / `Ctrl+E` jump to start/end and
  `Ctrl+W` deletes the previous whitespace-delimited word.
- Saved named filters: `Ctrl+S` saves the current filter expression to
  `<config-dir>/tlog/filters.conf`; `name:<saved>` inlines it during filter
  compilation with negation support, nested references, and cycle/depth guards.
- Export: `E` writes the current filtered view to a timestamped
  `tlog-export-<timestamp>.log` in the working directory (canonical threadtime
  format, no overwrite), and reports the result in the filter bar.
- Offline playback: `tlog --file <log>` replays a log file through the same
  parser/UI pipeline without requiring adb and exits automatically at EOF.
- Resize debounce: terminal resize bursts are coalesced for 75 ms before the
  scrollback is cleared and replayed.

### Changed
- `Up` / `Down` in the filter input now navigate applied-filter history;
  `Home` / `End` (and the new `Ctrl+A` / `Ctrl+E`) cover start/end movement.
- `name:` now resolves against saved filters and reports unknown/missing names
  as a filter error instead of silently matching everything.
- The help overlay lists the new search, export, and quick-level keys.

### Fixed
- The logcat and file readers now share one line-processing path, so offline
  playback gets identical lifecycle parsing and backpressure behaviour.

## [1.2.0] - 2026-08-15

### Added
- Device selection with `-s` / `--serial`; `ANDROID_SERIAL` is used as a
  fallback, and `-s` inside a custom `--cmd` is extracted for the `adb shell ps`
  seed command.
- `-b` / `--buffer` for choosing logcat buffers and `-c` / `--clear` to flush
  the device log before streaming.
- `--exit-on-eof` exits when the logcat feed ends, and `--reconnect` restarts
  the feed with exponential backoff (1s..30s). The default keeps the UI open in
  `STOPPED` state without busy-polling the closed channel.
- Status bar now shows how many entries were filtered out and how many incoming
  messages were dropped due to channel backpressure.
- Opt-in rejected log: `--debug` or `TLOG_DEBUG=1` writes unparsed lines and
  reader diagnostics to `tlog-rejected.log` in the config directory, rotating
  it at 1 MiB.

### Changed
- The startup `adb shell ps` seed now runs concurrently with logcat startup and
  is bounded by a 3-second timeout; the PID map is merged when it completes.
- Log lines are read with a 64 KiB retained-size cap; overlong lines are
  discarded through the next newline so parsing stays aligned.
- Scrollback insertion now flushes once per batch instead of once per line.
- Paused input semantics are documented: logs arriving while paused are
  discarded, not buffered.

### Fixed
- Logcat timestamps near the New Year now infer the previous/next year instead
  of always using the current year.
- `age:-5m` and `age:0m` are rejected instead of compiling into nonsensical
  fixed windows.
- The UI no longer spin-loops after the logcat receiver closes.

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
