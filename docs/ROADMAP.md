# tlog 项目分析与迭代路线图

> 本文件基于对 tlog 代码库（源码约 5.0k 行）的完整阅读与实测检查整理，记录当前状态、问题清单与版本化迭代建议。
>
> **状态更新（2026-08-16）**：P0 已在 v1.1.1 完成；P1 已在 v1.2.0 完成；P2 体验项已在 v1.3.0 完成；P3 性能项与 P4 发布/社区项已在 v1.4.0 完成（全屏浏览模式按用户要求保留至 v2.0）。v1.4.1 已修复 Shift+E 导出按键识别与底部状态行越界渲染。最新复检通过 `cargo test --all-targets`（107 个测试）、`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo bench --bench replay -- --quick`。

> **v1.4.2（2026-09-10，本批次）**：修复 `tlog-rejected.log` 多字节截断 panic、退出未关闭 bracketed paste、发布校验和被构建矩阵覆盖（改为每平台 `SHA256SUMS.<target>` + 发布时聚合）、一次性提示不消失，并新增 tag 与 `Cargo.toml` 版本一致性校验；性能上把 scrollback 写入合并为**单次** `write_all`（此前 stdout 行缓冲导致逐行 flush），`LogBuffer::push` 只求值一次过滤谓词并返回过滤视图位置（去掉每条日志一次 clone 与重复求值）。复检 `cargo test --all-targets` 为 114 个测试。仍未处理：回放期 `ReplayState` 的 generation 守卫、PID→包名映射刷新、Windows/macOS CI 矩阵、`--locked`/Dependabot、lib.rs 抽取与 `viewport.rs` 测试。

## 总体结论

架构清晰、文档质量高（尤其 `AGENTS.md`）、内存与背压设计克制，是一个完成度不错的小型 TUI 工具。v1.1.1 修复了分组否定 bug 并落地 CI 质量门禁；v1.2.0 已补齐多设备、启动并行、EOF/重连、日志防线与状态统计（P1）；v1.3.0 已完成搜索高亮、过滤器编辑/命名过滤、快速等级键、导出与离线回放及 Resize 防抖（P2）；v1.4.0 已完成分段回放、回归基准、tag 质量门禁、SHA256/标准归档与 Homebrew/Scoop/AUR 打包模板（P3/P4）。剩余路线只有 v2.0 的全屏浏览模式（含内部历史分页）。

## 当前检查结果（v1.4.1 复检，2026-08-16）

| 检查项 | 结果 |
| --- | --- |
| `cargo test --all-targets` | ✅ 107 个测试全部通过 |
| `cargo fmt --check` | ✅ 无 diff |
| `cargo clippy --all-targets -- -D warnings` | ✅ 无警告 |
| `cargo bench --bench replay -- --quick` | ✅ 基准冒烟可运行（100k push/驱逐、过滤重建、10k 回放格式化） |
| CI 是否跑 test/fmt/clippy/bench | ✅ 可复用 `quality.yml` 在 push/PR 运行 fmt、clippy、test 与 bench 冒烟；tag 发布先过同一门禁 |
| 发布资产 | ✅ 原始二进制 + `tlog-<target>.tar.gz`/`.zip` + `SHA256SUMS`；`packaging/` 提供 Homebrew/Scoop/AUR 模板 |

---

## P0：正确性与质量门禁（✅ 已于 v1.1.1 完成）

1. **修复过滤器「分组否定」语义 bug（已验证）**

   `filter.rs` 的 `not_expr` 处理 `-(...)` 时，只是把括号内每个谓词的 `negate` 置 true，运算符保持不变。实测：

   `-(tag:foo | tag:bar)` 对 foo、bar、baz 三条日志**全部返回 true**，等于恒真。

   正确语义是 `NOT(A | B) = NOT A & NOT B`。v1.1.1 已引入 `Expr::Not(Box<Expr>)` 节点，并补齐显式括号与分组否定的测试。

2. **正则表达式不要在求值时反复编译**

   `evaluate_predicate` 中每个条目、每个 regex 谓词都调用 `Regex::new`。一个 `tag~:Foo` 过滤器回放 10k 条日志就是 10k 次正则编译，过滤风暴场景会非常慢。v1.1.1 已在 `compile()` 阶段编译并缓存到 `Predicate.regex`，无效正则会在应用过滤器时作为 `filter_error` 报错，而不是静默返回 false。

3. **修 fmt/clippy，并把检查纳入 CI**

   这是成本最低的一批：`field_reassign_with_default`、`let_unit_value`、`manual_pattern_char_comparison`、`print_with_newline`、`map_or`、未使用的 `Preset::ALL` 等。v1.1.1 已全部修复；`ci.yml` 现在独立运行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets`。

4. **补齐解析器与生命周期的测试盲区（已完成）**

   v1.1.1 已覆盖 `parse_lifecycle` 的 5 种 ActivityManager 事件（新版/旧版 `Start proc`、`Killing`、`Process ... has died`、`No longer want`）与 `parse_ps_line` 的正常行/垃圾行拒绝路径。真实 Android 5.x~15 设备样本的 fixture 回归集仍可作为后续 P1/P3 的加固项。

5. **收口已知的文档/实现不一致**

   `AGENTS.md` 当时诚实记录了两处不一致。v1.1.1 的落地结果：

   - `age:` 已把文档明确为「过滤器编译时刻起算的固定窗口」，与实现的 `AgeSince.cutoff` 一致；若将来要真正的滚动窗口，可在 Tick 时更新 cutoff 并按分钟粒度触发 `rebuild_filtered`。
   - `package:mine` 已实现为恒真（`-package:mine` 恒假），与 README / Android Studio 语义一致。

---

## P1：可靠性与运维体验（✅ 已于 v1.2.0 完成）

1. **多设备支持**：新增 `-s` / `--serial`，未指定时回退到 `ANDROID_SERIAL`；`seed_pid_map` 与 logcat 命令共享同一 serial。自定义 `--cmd` 中的 `-s <serial>` 也会被提取用于 ps 种子；同时新增 `-b` / `--buffer` 与 `-c` / `--clear`。`--cmd` 与 `-s/-b/-c` 互斥（命令覆盖模式）。

2. **启动并行 + 超时**：`adb shell ps` 种子改为后台任务，与 logcat 启动并行，3 秒超时；完成后回填缓冲中已有条目的 package，并在包名过滤器激活时重建过滤视图。

3. **日志读取防线**：新增 `read_until_limited`，单行保留上限 64 KiB，超长行丢弃到下一个换行符以保持流对齐。`tlog-rejected.log` 改为默认关闭，仅 `--debug` / `TLOG_DEBUG` 开启，写入配置目录并按 1 MiB 轮转为 `.old.log`。

4. **EOF/重连策略**：默认保持 UI 在 `STOPPED` 状态（且不再忙轮询已关闭的 channel）；`--exit-on-eof` 在 feed 结束时退出；`--reconnect` 由读取任务自动重启子进程，指数退避 1s..30s，重启后通过 `Message::LogcatStarted` 恢复 `RUNNING` 状态。

5. **暂停语义**：保持「暂停即丢弃」的实现，并在 README/AGENTS 中明确：暂停期间新到达的日志不会进入缓冲区。

6. **小正确性项**：跨年时间戳按 ±183 天推断上一年/下一年；`age:-5m`、`age:0m` 在 `parse_duration` 阶段直接拒绝。

---

## P2：功能与体验迭代（✅ 已于 v1.3.0 完成）

1. **搜索与高亮（✅ v1.3.0）**

   `/` 打开搜索并高亮当前过滤视图中匹配的条目；查询为不区分大小写的字面量（`regex::escape` + `(?i)` 编译一次），匹配 Tag/包名/消息文本，空查询清除高亮。选中、跳转和复制仍可作为后续增强（更自然的载体是 P3 的全屏模式）。

2. **过滤器编辑器升级（✅ v1.3.0）**

   新增 `Ctrl+A/E/W`；`↑/↓` 改为浏览已应用过滤器历史（`Home/End` 仍为首尾跳转）；`Ctrl+S` 把当前表达式保存为命名过滤器（写入 `<config-dir>/tlog/filters.conf`），`name:<saved>` 在编译时内联展开，支持否定、嵌套引用与循环/深度保护（上限 8）。

3. **快速等级过滤键（✅ v1.3.0）**

   `v/d/i/w/e` 一键追加 `level:V/D/I/W/E`，使用显式 AND（多次按下按最严格等级收敛），与导出键 `E`（大写）不冲突。

4. **状态栏展示丢数与过滤统计（✅ 已于 v1.2.0 完成）**

   状态栏现在显示 `total / shown / filtered out / dropped` 四项与过滤状态；`status_bar` 为 5 占位符并保持中英双语同步。

5. **导出与离线回放（✅ v1.3.0）**

   - `E` 把当前过滤视图导出为工作目录下 `tlog-export-<时间戳>.log`（规范 threadtime 格式，已有文件自动编号不覆盖）；
   - `tlog --file <log>` 离线回放，复用 `process_log_line`/解析/渲染管道，无需 adb 且 EOF 自动退出，适合 CI 演示和录屏。

6. **Resize 防抖（✅ v1.3.0）**

   `Event::Resize` 先立即重定位视口区域，清屏+回放经 `RESIZE_DEBOUNCE`（75ms）合并到最终尺寸，Windows Terminal 拖拽窗口不再触发回放风暴。

---

## P3：性能与架构演进

1. **合并 scrollback 写入的 flush（✅ 已于 v1.2.0 完成；2026-09 补丁批次才真正生效）**

   `insert_log_lines` 现在先经 `insert_batch_ansi` 把整批渲染进一个 `Vec<u8>`，再用一次 `write_all` 写出。此前的实现虽然在函数末尾只显式 flush 一次，但后端 stdout 是行缓冲的，循环里的逐行 `\r\n` 已经触发了逐行 flush。

2. **让回放更便宜（✅ 已于 v1.4.0 完成）**

   回放改为分段执行：`needs_replay` 建立 `ReplayState`，每轮事件循环最多格式化/写入 `REPLAY_CHUNK = 128` 行，回放期间新到达的日志先暂存在 `pending_log_lines`，待最后一个块完成后再按时间顺序写入，避免旧块与新行交错。整段 10k 回放不再阻塞按键、Resize 与日志消息。

3. **为基准场景建立 benchmark（✅ 已于 v1.4.0 完成）**

   新增无 lib.rs 的 `benches/replay.rs`（`harness = false` + `#[path]` 引入源码模块），覆盖 100k push、120k 驱逐、100k 过滤重建、10k 回放格式化（普通/搜索高亮）；`--quick` 单次迭代接入 CI 冒烟，防止热路径回归。本机参考：push 100k 约 29ms、过滤重建 100k 约 2.5ms、10k 回放格式化约 7ms（随机器波动）。

4. **架构选择的提前思考：原生 scrollback vs 全屏 TUI（⏭ 保留至 v2.0，按用户要求本次不做）**

   原生 scrollback 是 tlog 的差异化亮点（日志融入终端历史，可配合终端搜索/选择），但也受终端能力限制。建议不要推翻，而是在 v2.0 规划可选的 full-screen 模式（ratatui `List` + 内置搜索/分页），做成启动 flag 或按键切换，而不是二选一。

---

## P4：发布与社区（✅ 已于 v1.4.0 完成）

- **质量门禁与 release 挂钩（✅ v1.4.0）**：质量检查抽成可复用 `.github/workflows/quality.yml`；CI 在 push/PR 调用，`release.yml` 在 `v*` tag 上先跑同套 fmt/clippy/test/bench 冒烟，通过后才开始构建；产物新增 `SHA256SUMS`。
- **降低安装门槛（✅ v1.4.0）**：Release 除原有原始二进制外，新增 `tlog-<target>.tar.gz`/`.zip` 标准归档，可直接被 `cargo-binstall` 使用；`packaging/` 提供 Homebrew formula、Scoop manifest 与 AUR PKGBUILD 模板（校验和占位符在发布后回填）。`cargo-dist`/`taiki-e` 可继续评估，当前发布链路已不依赖它们。
- **README 演示与帮助（✅ v1.4.0）**：双语文档新增示意性演示 GIF（`assets/demo.gif`，由 `scripts/generate_demo.py` 可复现）与完整 `tlog --help` 输出。

---

## 建议的迭代路线

| 版本 | 主题 | 核心内容 |
| --- | --- | --- |
| v1.1.1 | 质量门禁 | ✅ 已完成：fmt/clippy/CI、分组否定 bug、正则预编译、生命周期测试、文档对齐、过滤器游标编辑 |
| v1.2 | 可靠性 | ✅ 已完成：`--serial`/设备选择、启动并行+超时、EOF/重连策略、rejected.log 治理、状态栏统计、批量 flush |
| v1.3 | 体验 | ✅ 已完成：搜索/高亮、过滤器历史与命名过滤（Ctrl+A/E/W、↑/↓、Ctrl+S/`name:`）、快速等级键（v/d/i/w/e）、导出（E）与 `--file` 离线回放、Resize 防抖 |
| v1.4 | 性能与发布 | ✅ 已完成：128 行/轮分段回放、`cargo bench --bench replay` 回归基准、可复用质量门禁、tag 发布先过 checks、SHA256/标准归档、cargo-binstall 兼容、Homebrew/Scoop/AUR 模板、演示 GIF 与 `--help` 文档 |
| v1.4.1 | 补丁 | ✅ 已完成：Shift+E 导出按键识别（`e`+SHIFT 协议）、导出/错误提示行越界修复 |
| v1.4.2 | 补丁 | ✅ 已完成：`tlog-rejected.log` 多字节截断 panic、退出关闭 bracketed paste、发布校验和按平台命名+聚合、tag↔版本校验、一次性提示 TTL、`process:` 文档对齐、scrollback 单次 `write_all`、`push` 单次求值返回过滤位置 |
| v2.0 | 架构 | ⏭ 仅剩：可选全屏浏览模式（含内部历史分页），按用户要求不在 v1.4 实现 |

---

## 结语

这个项目最宝贵的资产是清晰的架构约定和严谨的 `AGENTS.md`，迭代时继续保持「不变量文档先行」的风格即可。P0（v1.1.1）、P1（v1.2.0）、P2（v1.3.0）、P3 性能项与 P4 发布项（v1.4.0）均已收敛；路线图只剩 v2.0 的可选全屏浏览模式（含内部历史分页）。
