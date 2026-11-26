# agents.md — tlog 代理开发指南

本文件面向在 tlog 仓库中工作的编码代理（Claude Code / Cursor / Codex 等）。
阅读代码之前先读本文件：它汇总了架构、命令、不变量与常见任务，避免破坏终端渲染与数据流约定。

## 1. 项目概览

tlog 是一个用 Rust 编写的 Android Logcat TUI 查看器，解析 `adb logcat -v threadtime`
输出并在终端底部视口（viewport）上方以原生 scrollback 方式展示日志。

- 语言 / 版本：Rust，edition 2024（需要较新工具链），二进制 crate（无 lib.rs，模块在 `main.rs` 中声明）。
- 技术栈：ratatui 0.30 + crossterm 0.28（TUI）、tokio 1（异步）、pest 2（过滤器语法）、clap 4（CLI）、chrono 0.4（时间）、regex 1（生命周期正则）、color-eyre 0.6（错误）、which 7（查找 adb）。
- 运行前提：`adb` 必须在 `PATH` 上（`which::which(cmd[0])` 检查）。
- 许可证：MIT。文档：`README.md` / `README_ZH.md`，变更记录：`CHANGELOGS.md` / `CHANGELOGS_ZH.md`。

## 2. 常用命令

| 命令 | 说明 |
|------|------|
| `cargo run --release` | 以默认命令 `adb logcat -v threadtime` 运行 |
| `cargo run --release -- --filter 'tag:MainActivity & level:ERROR'` | 启动时预应用过滤器 |
| `cargo run --release -- --cmd logcat,-v,threadtime` | 自定义日志命令（逗号分隔） |
| `cargo run --release -- -s <serial> -b main,system -c` | 指定设备/缓冲区，并先清空设备日志 |
| `cargo run --release -- --reconnect` | logcat 断开后指数退避自动重连 |
| `cargo run --release -- --exit-on-eof` | logcat 结束时退出 |
| `cargo run --release -- --debug` | 将未解析行/读取诊断写入配置目录 `tlog-rejected.log` |
| `cargo run --release -- --file <log>` | 离线回放日志文件（无需 adb，EOF 自动退出） |
| `cargo run --release -- --lang zh` | 强制中文界面（`en` / `zh`） |
| `cargo test --all-targets` | 运行全部单元测试 |
| `cargo fmt --check` | 格式检查（仓库使用 rustfmt 默认配置） |
| `cargo clippy --all-targets -- -D warnings` | Clippy 检查 |
| `cargo bench --bench replay` | 运行热路径回归基准 |
| `cargo bench --bench replay -- --quick` | 基准冒烟（单次迭代，CI 使用） |
| `cargo install --path .` | 安装到本地 |

注意：测试都在 `#[cfg(test)] mod tests` 中，不需要真实 adb 或 Android 设备。

## 3. 架构

### 3.1 模块地图

| 文件 | 职责 |
|------|------|
| `src/main.rs` | CLI 解析（clap）、设备/缓冲区/清空/EOF/重连/`--file` 参数解析、终端初始化/恢复、后台 `adb shell ps` 种子、logcat/文件读取任务、`tokio::select!` 事件循环、按键分发、分段回放（`ReplayState`/`REPLAY_CHUNK`）、Resize 防抖 |
| `src/rejected.rs` | 可选的 `tlog-rejected.log`：`--debug`/`TLOG_DEBUG` 启用，写入配置目录，1 MiB 轮转 |
| `src/logcat.rs` | `threadtime` 固定列偏移解析、`LogLevel` / `LogEntry` / `Message` 类型、`LogEntry::threadtime_string` 导出格式、ActivityManager 生命周期正则（`LazyLock`）、`adb shell ps` 行解析、字段截断 |
| `src/filter.rs` + `src/filter.pest` | 过滤器语法（pest 语法文件）、`compile_with_named` 三阶段管道（见 4 节）、`Expr` AST（含 `Not` 节点）与求值、正则预编译与求值、`name:` 命名过滤器内联 |
| `src/named_filters.rs` | 命名过滤器持久化：`<config-dir>/tlog/filters.conf`，每行 `name<TAB>expr`，容错加载 |
| `src/export.rs` | `E` 导出当前过滤视图：`tlog-export-<时间戳>.log`，规范 threadtime 格式，不覆盖已有文件 |
| `src/buffer.rs` | `LogBuffer` 环形缓冲区（100k 上限）+ `filtered: Vec<usize>` 零拷贝过滤视图 |
| `src/app.rs` | `App` 全局状态、`DisplayConfig`、`Focus`（LogView/FilterInput/SearchInput/FilterName）、`SettingsCategory`、`dispatch`、`drain_feed`（每轮批量消费 feed 通道，`MSG_CHANNEL_CAPACITY` / `MSG_DRAIN_BATCH`）、过滤器应用/旁路、搜索/命名过滤器/历史/快速等级逻辑、显示与颜色设置保存 |
| `src/config.rs` | 调色板 `ColorPalette`、预设 `Preset`、颜色项 `ColorItem`、hex 转换、`config.conf` 路径与读写 |
| `src/ui.rs` | 底部视口渲染（过滤器栏 + 状态）、`format_entry` 日志行格式化与搜索高亮、帮助/设置 overlay 渲染 |
| `src/i18n.rs` | `Lang` 检测、`Messages` 结构体与 `EN` / `ZH` 两个常量实例、占位符方法 |
| `src/scrollback.rs` | `DECSTBM` 滚动区域命令、`insert_log_lines` 将日志写入视口上方的终端原生 scrollback、`write_spans` ANSI 样式输出 |
| `src/viewport.rs` | `ViewportTerminal` / `ViewportFrame`：不使用备用屏（alternate screen）的 ratatui 终端，在屏幕底部固定区域做 diff 渲染 |
| `benches/replay.rs` | 无 lib.rs 的回归基准（通过 `#[path]` 引入源码模块）：100k push/驱逐、过滤重建、10k 回放格式化（普通/搜索高亮）、突发摄取（逐行 vs 批量 drain，对比 msg/s），`--quick` 供 CI 冒烟 |

### 3.2 数据流

1. **启动**：`Cli::parse` → 解析设备/缓冲区/EOF/重连/`--file` 参数并生成 logcat 命令 → `i18n::resolve` → 非文件模式 `which::which(cmd[0])` 检查 adb → `enable_raw_mode` → 计算视口区域（屏幕底部，基础高度 `VIEWPORT_HEIGHT = 3`）→ `run_app`。`--serial` 优先于 `ANDROID_SERIAL`；`--cmd` 是完整命令覆盖，与 `--serial/--buffer/--clear` 冲突；`--file` 与 `--cmd/--serial/--buffer/--clear/--reconnect` 冲突，到 EOF 自动退出且不需要 adb。
2. **运行循环**（`run_app`）：`tokio::select!` 五路并发：
   - crossterm `EventStream` 键盘/Resize 事件 → `handle_key`；Resize 先立即重新定位视口区域，再经 75ms 防抖后清屏重放；
   - 后台 `seed_pid_map` 任务（3 秒超时）完成后合并 PID→包名，并回填缓冲中缺失的包名（文件模式为空的占位任务）；
   - `mpsc::channel(MSG_CHANNEL_CAPACITY = 8192)` 收到 `Message` → `App::drain_feed` 每轮最多批量消费 `MSG_DRAIN_BATCH = 512` 条（新条目先入缓冲并攒进 `pending_log_lines`，整批只写一次 scrollback）；
   - 250ms `ticker` → `App::dispatch(Message::Tick)`；
   - resize 防抖 `Sleep`（75ms，合并窗口拖动风暴）。
   每轮循环后统一处理：`needs_replay` → 清空 `pending_log_lines` + 清屏 + 重置视口 + 建立 `ReplayState`；随后每轮最多格式化/写入 `REPLAY_CHUNK = 128` 行；`replay_state.is_none()` 且 `pending_log_lines` 非空 → `insert_log_lines`（回放期间新行暂存，保证时间顺序）；最后 `terminal.draw(ui::render_viewport)`。
3. **日志管道**：logcat 子进程 stdout 或 `--file` 文件 → `read_until_limited`（保留上限 64 KiB，超长行丢弃到下一个换行符）→ `process_log_line`（两源共用）→ `parse_line` → `LogEntry` → 若 `tag == "ActivityManager"` 先解析生命周期事件（`ProcessStarted` / `ProcessDied`）→ `try_send(Message::NewEntry)`。Channel 满时**直接丢弃**并累计，每 1000 条发一次 `Message::Dropped`；未解析行在启用 `--debug`/`TLOG_DEBUG` 时写入配置目录 `tlog-rejected.log`；EOF/读取错误发 `Message::LogcatDied`。`--reconnect` 由 logcat 读取任务自己重启子进程（指数退避 1s..30s）并先发 `Message::LogcatStarted`；不重连时主循环用 `feed_closed` 禁用已关闭的 channel 分支，避免忙轮询。
4. **入缓冲**：`App::dispatch(NewEntry)` 解析 PID→包名，未暂停时 `buffer.push(entry, eff_filter)`。`push` 只求值一次谓词并返回条目在过滤视图中的位置（`Option<usize>`，`None` 表示被过滤掉），`ui::format_entry(..., search_regex)` 用该位置通过 `get_filtered` 回取缓冲中的条目格式化，**不要**在 push 前 clone 条目或再求值一次谓词。
5. **过滤/搜索/设置变更**：`compile_with_named`（pest 解析 → `collect_tokens` → `merge_same_key_or` → `apply_implicit_and` → `build_ast`，内联 `name:`）或 `App::apply_search` / 设置切换 → `buffer.rebuild_filtered` / 更新 `search_regex` → `needs_replay = true` → 主循环清屏并分段重放最新 `MAX_REPLAY = 10_000` 条。
6. **退出**：abort 读取任务（子进程设置了 `kill_on_drop`）→ `restore_terminal`（DisableBracketedPaste、ResetColor、光标移到底行、Clear All + Purge）→ `disable_raw_mode` → 打印 `"\x1b[r"` 复位滚动区域。启动时的 `EnableBracketedPaste` 必须在这里配对关闭，否则退出后终端残留 bracketed paste 模式。

## 4. 关键不变量与陷阱

这些是修改代码时最容易出错的地方。

1. **固定列偏移解析**：`parse_line` 按 `threadtime` 格式的字节偏移逐位校验（`MM-DD HH:MM:SS.mmm  PID  TID L TAG: MESSAGE`，PID/TID 为 5 字符右对齐）。行长度 `< 37` 直接拒绝。改动解析格式必须同步更新 `logcat.rs` 测试与 README。
2. **时间戳无年份**：logcat 不输出年份；`parse_line` 按 `now` 的日期做 ±183 天推断，自动选择当年/上一年/下一年。修改阈值或候选年份时要同步更新 `logcat.rs` 的跨年测试。
3. **字段截断**：tag ≤ 256 字节、message ≤ 4096 字节，且必须落在 UTF-8 字符边界（`truncate_to_boundary`）。
4. **环形缓冲与索引失效**：`LogBuffer` 满（100,000）时先 `drain(0..EVICT_BATCH)`（20,000）并 `shrink_to_fit()`，然后**必须** `rebuild_filtered`，因为 `filtered` 存的是 `entries` 的索引，驱逐后索引全部失效。
5. **有界通道 + try_send + 批量 drain**：logcat 读取任务用 `try_send` 而非 `await`，保证实时性；channel（`MSG_CHANNEL_CAPACITY = 8192`）满时丢弃新日志而不是阻塞。不要在 logcat 读取任务里改成阻塞发送。消费端由 `App::drain_feed` 每轮最多取 `MSG_DRAIN_BATCH = 512` 条，让一次 scrollback 写入与一帧绘制覆盖整批日志，而不是逐行各来一次；批内顺序必须保持 channel 顺序，`FeedDrain::{Exit, Closed}` 的退出/停止轮询语义不能丢。唯一例外是 `--file` 离线回放（`run_file_feed`）用 `send().await` 做有界背压——文件不会持续产生新数据，丢行反而会破坏回放完整性。
6. **回放上限**：过滤变更 / resize / 清屏后只重放最新 10,000 条；不会全量重放历史。
7. **`needs_replay` 与分段回放协议**：置位后由主循环统一执行：清空 `pending_log_lines` → `clear_scrollback_and_visible_screen_ansi` + `set_viewport_area` → `replay_state_for(filtered_len)` + 复位标志。之后每轮只处理 `next_replay_chunk_end` 界定的一个 `REPLAY_CHUNK = 128` 行块；`pending_log_lines` 在回放期间暂存，`replay_state` 清空后才能写入，否则新行会插到旧回放块中间破坏时间顺序。再次置位会丢弃当前进度并重新快照。新功能需要重绘 scrollback 时置 `needs_replay = true`，不要自行直接渲染。
8. **无备用屏**：`ViewportTerminal` 工作在主屏幕缓冲区（不是 ratatui 默认的 alternate screen）。退出清理必须在 `main` 中执行 `restore_terminal` + `disable_raw_mode` + `"\x1b[r"`，否则终端画面与滚动区域会残留。
9. **scrollback 插入**：`insert_log_lines` 先经 `insert_batch_ansi` 把整批渲染进一个 `Vec<u8>`（`DECSTBM` 把滚动区域设为 `0..viewport_top`，在 `viewport_top - 1` 行写 `\r\n` 触发滚动，再写样式化文本，最后 `ResetScrollRegion`），然后用**一次** `write_all` + `flush` 写出。stdout 是行缓冲的，逐行 `write!` 会退化成逐行 flush（每行一次 syscall），不要在循环里逐行写。若 `viewport_top == 0`（视口已到顶），退化为直接覆盖前几行。插入后调用 `note_history_rows_inserted`。
10. **`age:` 是编译时刻的固定窗口（已写入 README）**：`make_predicate` 在 compile 时把 cutoff 固化为 `now - duration`，否定形式 `-age:5m` 用 `Expr::Not(AgeSince)` 表达「比该固定窗口更旧」；`Message::Tick` 只驱动帧渲染与一次性提示过期，不刷新 cutoff。`age:5m` 表示「过滤条件编译时刻起算的最近 5 分钟」，不是随运行持续滑动的滚动窗口。若要实现真正每 tick 刷新的滚动窗口，需在 Tick 时更新 `Expr::AgeSince` 的 cutoff 或重编译过滤器，并处理由此产生的 scrollback 重放开销。
11. **`package:mine` 恒真已实现**：`evaluate_predicate` 对 `Key::Package` 且非正则、值等于 `mine` 的谓词直接返回 `!negate`（即恒真，`-package:mine` 恒假）。该特例在包名解析与否时行为一致，且只影响 `package:`，不影响 `process:`。
12. **配置文件容错**：`config.conf` 为 `key = value` 行；未知 key、畸形值、缺失文件都按默认值处理。`save_config` 失败不崩溃，错误写入 `App.color_error` 并在 overlay 显示。
13. **i18n 必须双语言同步**：新增用户可见字符串要同时改 `Messages` 结构体、`EN` 和 `ZH` 两个常量；含占位符的字符串在 `Messages` 上提供 `replacen("{}", ...)` 方法（如 `status_bar`、`filter_title`、`parse_error`、`invalid_age_value`）。
14. **Overlay 行数强约定**：`App::overlay_rows()` 返回 Help 13 行、Display 10 行、Color 13 行，`ui` 的对应测试断言这些行数。增删 overlay 行时必须同步改 `overlay_rows()` 和对应测试，否则视口高度计算错误。
15. **测试环境变量隔离**：`main.rs` 的测试通过 `static CONFIG_LOCK` + `isolate_config` 修改进程级 `XDG_CONFIG_HOME`（`unsafe set_var`）。新增任何读取配置路径的测试都必须先获取同一把锁，避免与并行测试竞争。
16. **`tlog-rejected.log`**：默认关闭；只有 `--debug` 或 `TLOG_DEBUG=1/true/on/yes` 时才写入配置目录（`config.conf` 同目录），超过 1 MiB 轮转为 `.old.log`。长度 > 200 字节的未解析行经 `logcat::truncate_to_boundary` 在 UTF-8 字符边界上截断（**不要**用 `&line[..200]` 这类字节切片，多字节字符会 panic 掉读取任务）。不要把它当成错误——divider 行（`--------- beginning of ...`）本就会被拒绝；未启用时这些行直接丢弃。
17. **level 语义**：`level:INFO` 是 `>=`（匹配 INFO、WARN、ERROR、FATAL）；`S`（Silent）映射为 `Fatal`。
18. **过滤器优先级**：`&` 高于 `|`；相邻同 key 非否定谓词合并为隐式 OR；相邻不同 key 谓词为隐式 AND（见 `filter.rs` 的三个阶段与 README 示例）。
19. **显式括号必须保留**：`collect_tokens` 会把输入中的 `(...)` 转成 `OpenParen`/`CloseParen` token，再交给优先级解析。不要删除或吞掉括号 token，否则 `(a | b) c` 会退化成 `a | (b & c)`。
20. **分组否定用 `Expr::Not`**：`-(...)` 必须包裹整个组（`Not` token + 括号），不能只翻转组内每个谓词的 `negate`，否则 `-(a | b)` 会被错误地变成 `-a | -b`（德摩根错误）。单个谓词 `-tag:foo` 仍直接翻转 `negate` 标志。
21. **正则只编译一次**：regex 修饰符在 `compile_with_named()` 阶段编译并存入 `Predicate.regex`（`Option<Regex>`），求值路径禁止再调用 `Regex::new`。无效正则必须在应用过滤器时作为 `filter_error` 报错，不能静默当作不匹配。
22. **搜索高亮也走 `needs_replay`**：`App::apply_search` 用 `regex::escape` + `(?i)` 编译一次 `search_regex`，然后置 `needs_replay = true`；`format_entry` 只接收已编译的 `Option<&Regex>`，不允许在格式化路径里重新编译。搜索是字面量、不区分大小写，匹配 Tag/包名/消息文本；空查询清除高亮。
23. **`name:` 内联与循环保护**：`compile_with_named` 递归编译 `name:<saved>` 引用的表达式，栈深度上限 8 且同栈内重名报「递归过深/循环」错误；未知名称是过滤器错误，不再是恒真 no-op（空 `name:""` 仍保持恒真）。保存名含 `-` 时用户必须写成 `name:"foo-bar"`（语法中 `-` 是取反前缀）。
24. **过滤器历史键位**：`↑`/`↓` 是历史导航（不再跳行首/行尾）；`Home`/`End` 与 `Ctrl+A`/`Ctrl+E` 才是行首/行尾。任何手动编辑调用 `note_filter_edited` 退出历史导航，越过最新项后 `↓` 恢复进入历史前的草稿。
25. **Resize 防抖**：`Event::Resize` 先立即 `set_viewport_area`（保证 draw 缓冲区与终端尺寸一致），把清屏+回放延迟 75ms（`RESIZE_DEBOUNCE`）合并到最终尺寸；不要在事件分支直接清屏回放。
26. **离线文件回放**：`--file` 走 `run_file_feed`，与 logcat 共用 `process_log_line`/`read_until_limited`/`Message` 协议；不检查 adb、跳过 ps 种子，并强制在 EOF 退出。`LogEntry::threadtime_string` 是导出的规范格式，必须能通过 `parse_line` 回读（有往返测试）。
27. **无 lib.rs 的 benchmark**：`benches/replay.rs` 用 `#[path]` 直接引入 `src/` 模块并设 `harness = false`。不要为 benchmark 新增 lib.rs；bench crate 内需要 `#![allow(dead_code, unused_imports)]`，否则引入的 TUI 渲染函数会被 -D warnings 判定为未使用。新增热路径基准时补充到同一 bench 文件，CI 用 `--quick` 冒烟。
28. **暂停即丢弃**：`App::dispatch(NewEntry)` 在 `paused` 时既不 `push` 也不格式化，日志永久丢失且不计入 `total_parsed`（只有 channel 溢出才会走 `Message::Dropped`）。README 已声明该语义；若要改成暂停缓冲，必须同时处理恢复后的写入顺序与上限。
29. **一次性提示有 TTL**：`status_message` 必须是"一次性"的——`set_status` 同时写入 `status_message_deadline`（`STATUS_MESSAGE_TTL = 5s`），`Message::Tick` 调 `expire_status_message(Instant::now())` 清理，`begin_search`/`begin_filter_save` 也会清空。新增提示一律走 `set_status`，不要直接给 `status_message` 赋值（否则永不消失）。
30. **`process:` 与 `package:` 同义**：`evaluate_predicate` 的 `Key::Package | Key::Process` 共用一个分支——优先匹配解析出的包名，包名未解析时回退匹配 PID 字符串；`package:mine` 恒真特例只作用于 `package:`。README 的键列表与语法示例必须同时列出两者。

## 5. 代码约定

- 注释与 doc comment 使用英文；用户可见字符串通过 `i18n.rs` 提供中英双语。
- 每个模块底部有 `#[cfg(test)] mod tests`；新增逻辑应补测试。唯一例外是 `viewport.rs`（依赖真实 TTY 才能有意义地断言 diff 输出），`scrollback.rs` 已通过 `insert_batch_ansi` 覆盖 ANSI 字节。
- 使用 Rust 2024 特性，如 let-chains（`if let Some(...) = ... && let Some(...) = ...`）。
- 正则用 `std::sync::LazyLock` 编译一次（见 `logcat.rs` 的生命周期正则）。
- 模块顶部用 `//!` 写模块级文档（`scrollback.rs`、`viewport.rs`）。
- 没有自定义 rustfmt / clippy 配置；提交前应通过 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings`。
- 类型导出：`LogEntry`、`Message` 等公共类型直接通过 `use crate::...` 引用；不要新增 `mod.rs` / `lib.rs`，二进制 crate 结构保持不变。

## 6. 测试

测试为纯单元测试，不依赖 adb 或设备：

- `src/logcat.rs`：`threadtime` 解析（正常、零 PID、大 PID、空 message、F/S level）、跨年推断、拒绝 divider/垃圾行、字节边界截断、ActivityManager 生命周期 5 类事件、`adb shell ps` 行解析、`threadtime_string` 导出往返。
- `src/filter.rs`：tag、否定、优先级、显式括号、分组否定（`-(a | b)`）、隐式 OR/AND、level >=、否定正则、无效正则报错、age、is:stacktrace、is:crash、`package:mine`、引号值、`name:""` 恒真、命名过滤器解析/嵌套/否定/未知名称/循环、错误输入。
- `src/named_filters.rs`：save/load 往返（含 `=`、`|` 等特殊字符）、畸形行容错、缺失文件、非法条目跳过。
- `src/export.rs`：导出规范 threadtime 行可回读、唯一路径不覆盖。
- `src/buffer.rs`：push/get、push 返回过滤视图位置（过滤掉返回 `None`）、驱逐、clear、包名回填。
- `src/config.rs`：hex 往返与非法输入、preset apply、save/load 往返、Custom 往返、缺失/部分配置。
- `src/ui.rs`：`format_entry` 颜色与 `colorize=false`、搜索高亮 span、运行状态指示、help/settings 行数结构、`visible_filter_slice`（宽字符、零宽字符、光标跟随滚动、短输入全显）。
- `src/app.rs`：过滤游标/UTF-8/退格删除、Ctrl+W 删词、历史导航与草稿恢复、历史上限、快速等级追加去重、搜索应用/清空/预填、命名过滤器空表达式拒绝、一次性提示 TTL 过期与 Tick 清理、`drain_feed` 批内顺序/批上限/`Exit`/`Closed`/断连停轮询。
- `src/main.rs`：CLI 参数组装（serial/buffer/clear/EOF/`--file` 冲突）、重连退避、有界行读取、分段回放边界（`replay_state_for`/`next_replay_chunk_end`）、过滤器方向键/`Ctrl+A/E/W`/历史、搜索与命名过滤器输入模式、快速等级与导出键、命名过滤器持久化与 `name:` 解析、文件 feed 解析/打开失败、颜色编辑提交保存、预设循环保存、非法 hex 取消、显示开关持久化、`restore_terminal` ANSI 序列（含 DisableBracketedPaste）、启动时调色板重载。
- `src/rejected.rs`：禁用时零写入、超长截断（含多字节字符不 panic）、1 MiB 轮转。
- `src/scrollback.rs`：`insert_batch_ansi` 的滚动区域/逐行 CRLF/颜色 ANSI 字节、视口到顶时退化为覆盖前几行。
- `src/i18n.rs`：状态栏过滤/丢弃统计占位符。

运行：`cargo test --all-targets`。若增加与配置路径相关的测试，务必使用 `main.rs` 中 `isolate_config` 的锁模式。

## 7. CI/CD

`.github/workflows/quality.yml`（可复用质量门禁）：

- 由 `ci.yml` 在 push 到 `main` / PR 到 `main` 时调用；`release.yml` 在 `v*` tag 上先调用它，构建 job 依赖其成功结果。
- ubuntu 上执行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets`、`cargo bench --bench replay -- --quick`。

`.github/workflows/release.yml`：

- 触发：push 到 `main`、`v*` tag、PR 到 `main`；tag 上先跑质量门禁，失败则不构建。
- 矩阵：ubuntu x86_64、windows x86_64、macos x86_64、macos aarch64；`cargo build --release --target`。
- 每平台产出：原有 `tlog-<os>-<arch>[.exe]` 兼容名 + `tlog-<target>.tar.gz`/`.zip`（供 cargo-binstall）+ `SHA256SUMS`。
- 打 tag 时 `softprops/action-gh-release` 自动创建 GitHub Release 并附带全部 artifacts。
- 新增平台目标时，同时更新矩阵、归档名与 `packaging/` 模板；发布后把实际 SHA256 回填到模板占位符。

## 8. 常见任务指南

### 8.1 新增过滤器键

1. `src/filter.pest`：在 `key` 规则中加入新字面量。
2. `src/filter.rs`：`Key` 枚举加变体；`Key::from_str` 与 `Display` 增加映射；`evaluate_predicate` 增加匹配分支（regex 谓词使用 `p.regex.as_ref()`，不要重新编译）。
3. 若需要特殊节点（如 `age:` 的 `AgeSince`），在 `make_predicate` 中特殊处理。
4. 在 `src/filter.rs` 的 `tests` 中补测试；更新 `README.md` 的 Filter syntax 小节。

### 8.2 新增显示开关（Display setting）

1. `src/app.rs`：`DisplayConfig` 加字段并给默认值。
2. `src/config.rs`：`save_to` 输出、`load_from` 的 `match k` 分支加 `set_bool` 键。
3. `src/ui.rs`：`settings_display_rows` 增加一行（`[7]` 等），并调整 `App::overlay_rows()` 的 Display 行数。
4. `src/main.rs`：`toggle_config` 增加按键映射（若提供）。
5. 更新 `ui.rs` 测试中的行数断言与 `main.rs` 的持久化测试；更新 README。

### 8.3 新增颜色项 / 颜色预设

颜色项：`ColorItem` 枚举、`ALL`、`from_key`、`label`、`color`、`set` 都要同步；overlay 行数（Color 13 行）与 `settings_color_rows` 同步；测试更新。
颜色预设：`Preset` 枚举、`ALL`、`id` / `from_id` / `label` / `next` / `prev` / `apply` 全部同步；`config.rs` 中补 roundtrip 测试。

### 8.4 新增用户可见字符串

1. `src/i18n.rs`：`Messages` 结构体加字段；`EN` 与 `ZH` 两个常量同步赋值。
2. 需要插值时在 `Messages` 的 `impl` 中加 `replacen("{}", ...)` 方法。
3. 在 `ui.rs` 或对应位置调用 `app.msgs.xxx`，不要硬编码英文/中文。

### 8.5 修改日志解析格式

1. 在 `src/logcat.rs` 的 `parse_line` 中修改固定偏移逻辑；新增格式差异时先加测试样本。
2. 同步更新 `parse_line` 的 doc comment、README 中的格式说明与 `tlog-rejected.log` 的触发条件。
3. 生命周期解析只处理 `tag == "ActivityManager"` 的行，格式变更时确认这些正则仍匹配。

### 8.6 修改环形缓冲区大小

修改 `src/buffer.rs` 的 `RING_CAP` / `EVICT_BATCH`（`EVICT_BATCH = RING_CAP / 5`），同步更新 `README.md` 的 Memory strategy 表格与 `buffer.rs` 的驱逐测试预期。

### 8.7 新增保存/读取类配置

1. 路径解析尽量复用 `config::config_path()` 的父目录（见 `named_filters.rs` / `rejected.rs`）。
2. 文件格式要容错：缺失文件、空行、注释、畸形行都按默认处理，不能 panic。
3. 写入时跳过会破坏格式的字段（tab/换行），并在本模块补 save/load 往返测试。
4. 涉及 `XDG_CONFIG_HOME` 的集成测试放在 `main.rs` 测试并先取 `CONFIG_LOCK`。

### 8.8 发版 / 打包

1. 版本同时改 `Cargo.toml` 与 `Cargo.lock` 的 tlog 条目。
2. tag 使用 `v<version>`；release.yml 会先跑质量门禁，再构建矩阵与生成 SHA256。
3. 新增平台时同步更新 release 矩阵、`tlog-<target>.tar.gz`/`.zip` 归档名与 `packaging/` 模板。
4. Release 发布后，把各平台 `SHA256SUMS` 回填到 Homebrew/Scoop/AUR 模板的 `REPLACE_WITH_*_SHA256` 占位符；cargo-binstall 用户可用 `cargo binstall --repo ryans233/tlog tlog`。
5. `scripts/generate_demo.py` 可重新生成 `assets/demo.gif`；改动 UI 布局后应重新生成。
