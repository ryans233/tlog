# tlog — Android Logcat TUI

用 Rust 写的终端 Android logcat 查看器。

## 功能

- **实时日志流** — 通过 `adb logcat -v threadtime` 读取并解析日志
- **Android Studio 风格过滤器** — 支持 `tag:`、`level:`、`package:`、`message:`、`age:`、`is:` 等键值过滤，支持正则、否定和布尔运算
- **包名解析** — 自动从 ActivityManager 生命周期事件解析 PID→包名映射（参考 pidcat）
- **颜色高亮** — 按日志等级着色（V=灰 D=青 I=绿 W=黄 E=红 F=红底）
- **显示选项切换** — 按 `1`–`6` 开关时间戳、PID、TID、Tag、等级、颜色的显示
- **暂停/恢复** — 冻结日志输出以便仔细查看
- **过滤器旁路** — 按 `g` 临时禁用过滤器（保留输入内容，再按恢复）
- **多语言** — 界面文本支持中文和英文，根据 `LANG` 环境变量自动检测，也可通过 `--lang` 指定
- **环形缓冲区** — 10 万条硬上限，内存安全

## 安装

```bash
cargo install --path .
```

或直接运行：

```bash
cargo run --release
```

**前置条件：** 需安装 [Android SDK 命令行工具](https://developer.android.com/tools?hl=zh-cn)，且 `adb` 在 `PATH` 中可用。

## 用法

```bash
# 默认：adb logcat -v threadtime
tlog

# 启动时预置过滤器
tlog --filter 'tag:MainActivity & level:ERROR'

# 自定义命令（Termux 等环境）
tlog --cmd logcat,-v,threadtime

# 指定语言
tlog --lang zh
tlog --lang en
```

### 快捷键

| 键 | 功能 |
|----|------|
| `q` / `Ctrl+C` | 退出 |
| `p` / `Space` | 切换暂停/恢复 |
| `C` | 清空日志缓冲区 |
| `g` | 临时旁路过滤器（保留输入内容，再按恢复） |
| `o` | 输出当前显示配置到日志 |
| `h` | 显示快捷键帮助 |
| `1`–`6` | 切换显示选项（时间戳/PID/TID/Tag/等级/颜色） |
| `Tab` | 切换到过滤器编辑 |
| `Esc` | 返回日志视图（过滤器编辑时） |
| `Enter` | 应用过滤器（过滤器编辑时） |

### 过滤器语法

```
# 键值过滤（子串匹配）
tag:MainActivity
level:ERROR        # >= 语义，匹配 ERROR 和 FATAL

# 正则（~ 修饰符）
tag~:My.*Tag

# 否定
-tag:Debug

# 逻辑组合
tag:foo & level:ERROR    # AND（& 优先于 |）
tag:foo | tag:bar        # OR
tag:foo tag:bar          # 同键隐式 OR
tag:foo level:ERROR      # 不同键隐式 AND

# 特殊过滤器
age:5m                   # 最近 5 分钟
age:1h                   # 最近 1 小时
is:crash                 # FATAL EXCEPTION
is:stacktrace            # 堆栈延续行
package:com.example      # 包名过滤（需先解析到）
package:mine             # 恒真（无项目上下文）
```

## 架构

```
┌─ main.rs ─── 事件循环 (tokio::select!) ─┐
│  ├─ crossterm 键盘事件                    │
│  ├─ logcat 子进程 stdout → channel       │
│  └─ 250ms tick（age 过滤器刷新）         │
├─ logcat.rs ─ 日志解析 + 进程生命周期 ────┤
├─ filter.rs ─ pest 语法 → AST → 求值 ────┤
├─ buffer.rs ─ 环形缓冲区（100k 硬上限）───┤
├─ app.rs ──── 全局状态 + 消息分发 ────────┤
├─ ui.rs ───── ratatui 渲染 ──────────────┤
├─ i18n.rs ─── 多语言消息 ────────────────┤
├─ scrollback.rs ─ 回滚缓冲区 ─────────────┤
└─ viewport.rs ── 底部视口管理 ───────────┘
```

## 内存策略

| 机制 | 详情 |
|------|------|
| 环形缓冲区 | 100,000 条硬上限，满时淘汰最旧 20,000 条 |
| 有界 channel | `mpsc::channel(1024)`，满时丢弃而非堆积 |
| 消息截断 | 单条 message ≤ 4096 字节，tag ≤ 256 字节 |
| 零拷贝视图 | `filtered: Vec<usize>` 存索引，渲染时临时格式化 |
| 定期收缩 | 淘汰后 `shrink_to_fit()` |

## 技术栈

| 用途 | Crate |
|------|-------|
| TUI | ratatui 0.30 + crossterm 0.28 |
| 异步 | tokio 1.x |
| 过滤器语法 | pest 2.x |
| 命令行 | clap 4.x |
| 时间 | chrono 0.4 |
| 正则 | regex 1.x |
| 错误 | color-eyre 0.6 |
| 二进制查找 | which 7 |

## 致谢

包名解析方案参考了 [JakeWharton/pidcat](https://github.com/JakeWharton/pidcat)。

## License

MIT
