# IMPLEMENTATION_PLAN：RSSH 嵌入 JetBrains IDE

## 背景与目标
让用户在 JetBrains IDE（首发 IntelliJ IDEA）的工具窗里直接使用完整 RSSH
（终端 + 色条 + AI 面板 + SFTP 面板），**无需手动启动任何 RSSH 进程**。

原理：前端是 Web（Svelte + xterm.js），**UI 在任何浏览器里都能渲染**——开 Vite 的
devUrl（`localhost:1420`）在 Chrome 里就能看到界面，这是已验证的事实。所以难点不在
UI 渲染，而在**数据通道**：90 处 `invoke` / 25 处 `listen` 全靠 Tauri 注入的
`window.__TAURI_INTERNALS__`（`@tauri-apps/api` core.js:202），裸浏览器里这个全局不存在；
且后端没有任何给前端的 inbound HTTP/ws server（全仓唯一的 `TcpListener` 在
`ssh/forward.rs`，是 SSH 端口转发，不是 IPC）。所以 Chrome 里打开 = **能渲染、连不上
后端的死壳**。把这条数据通道接上（垫片 → ws → headless）就是全部的活——正是阶段一要做的。

## 架构决定（已由用户确认，后续不得擅自推翻）
1. **代码共用**：同一套 Rust 引擎源码，零重写。从中抽出一个"传输无关的核心"，
   上面挂两个适配器：
   - 桌面 app —— 现有 Tauri 壳，行为不变、不破坏；
   - headless ws server —— 新增构建目标。
2. **运行时独立**：桌面 app 与插件后端是**两个独立进程**，互不依赖，各自能单独跑。
   "共用 engine" 指代码层，不是运行时同一个进程。
3. **共享数据目录**：两个后端指向同一数据目录 → 主机/密钥/设置共享；活会话各自独立
   （IDEA 里连的和 app 里连的是两条独立连接）。并发由 SQLite WAL 兜底；常态下
   "只用 IDEA" 时只有一个进程在跑，无并发。

## 关键不变量
- **INV-1** 前端业务代码（34 个 `import @tauri-apps` 的文件、90 处 `invoke`、
  25 处 `listen`）**零改动**。所有适配收敛到**一个** IPC 垫片模块（一条缝，不是 34 条；
  Tauri 自带 `mockIPC` 即证明这是单一全局）。
- **INV-2** 桌面 app 行为不退化（Never break userspace）。
- **INV-3** headless server 只绑 `127.0.0.1`，每次启动随机端口 + 随机 token；
  无 token 一律拒绝。
- **INV-4** 引擎逻辑（ssh / pty / sftp / db / crypto）全局只有一份实现。
- **INV-5** `rssh open` 在嵌入态必须仍"开新 tab"（而非直连 ssh）。本质要求：headless 必须经
  共享 `pty.rs` 拉 shell（保 `RSSH_APP=1`）+ 前端 OSC handler 不变。详见下节。

## 关键集成：rssh CLI ↔ GUI（嵌入态必须一致）
`rssh open <name>` 的判定靠两段**共享代码**自动成立，无需任何 IDEA 专属逻辑：
1. **探测**：`terminal/pty.rs:329` 拉 shell 时注入 `RSSH_APP=1`；CLI
   `bin/rssh/commands/open.rs:13 in_rssh_app()` 读它。
2. **动作**：在 GUI 内 → `open.rs:23` 打印 `OSC 7337 ; open:<name>`（纯 stdout 字节，
   零 IPC、零 Tauri）；前端 `osc/handler.ts` 的 OSC 7337 handler catch → `app.addTab`
   开新 tab。不在 GUI → 直连 ssh（`cmd_open_ssh`）。

因为 OSC 只是终端字节流，走 Tauri IPC 还是走 ws 毫无区别 → 嵌入态**自动继承**，零额外代码。
路由也天然正确：OSC 顺哪个 PTY 打出，就回到哪个宿主的前端，两进程不串台。

**反证（为何"代码共用"是对的）**：若当初选 🔴"JVM 重写后端 / pty4j 拉 shell"，shell 不带
`RSSH_APP` → CLI 误判"不在 GUI" → 退化成直连 ssh，**静默破坏**此行为。代码共用恰好消灭了
这个特殊情况——这正是该决定的价值。

## 风险（最危险的假设排最前）
- **R1（致命）** 前端能否脱离 Tauri、在裸 Chromium 里活下来？→ 阶段一专门证伪。
- **R2** JCEF 无原生多窗口，`open_tab_in_new_window` / AI handoff 失效 → 降级为开新 tab。（这个功能，在插件上可以隐藏）
- **R3** macOS 下 JVM spawn 未公证的原生 helper 可能撞 Gatekeeper → 阶段三处理。
- **R4** 插件需长期追 IDEA API 漂移 → 接受为维护成本。

---

## 第 0 阶段：架构决定与边界冻结
**目标**：把上面"架构决定 + 不变量"定为后续所有阶段的契约。
**成功标准**：三条决定（代码共用 / 运行时独立 / 共享数据目录）+ 四条不变量白纸黑字、获确认。
**测试**：评审通过；无代码。
**状态**：完成（本文件即阶段零产物，决定已由用户确认）

## 第 1 阶段：前端脱离 Tauri（去风险，和 IDE 无关）★最关键
**目标**：证明前端能在裸浏览器里、经 ws 连 headless 后端正常工作。
- 从 Rust 引擎抽出传输无关核心 + headless ws server，先实现**最小子集**：
  pty 开/写/关、`ssh:data` 事件、ssh 连接/断开。
- 写前端 IPC 垫片：检测无 `window.__TAURI_INTERNALS__` → 把 `invoke` / `listen` 路由到 ws。
**成功标准**：Chrome 里打开 RSSH 前端 → 连 headless → 开终端、输入、看到输出；
**色条照常渲染**；AI 面板、SFTP 面板能开并工作。
**测试**：
- 本地 shell：开 tab、`ls`、色条按命令分块；
- CLI 一致性：终端里 `rssh open <profile>` → **开新 tab**（断言走 OSC 7337，不是 spawn ssh）；
- 远程：连一台 SSH、跑命令、断开；
- SFTP：列目录、下载一个文件；
- AI：开面板、发一条消息、看到流式返回；
- 垫片单测：invoke 往返、listen 订阅/退订、缺 token 被拒。
**状态**：完成（headless 全栈接线 + 前端垫片全覆盖）。**前端 invoke 的 92 条命令零"unknown command"死壳**
（机械求差集核对，含多行 `invoke(` —— 首轮单行正则漏了 `forward_stats`，codex 独立复审抓出并补齐；
剩余全由垫片就地服务或诚实报错）。独立 codex 复审：3 处问题（forward_stats 漏接 / SFTP cancel flag
panic 泄漏 / 垫片 close 后 invoke 永挂）已全部修复 + 确认无新 bug。
**架构**：`Host`（`enum Tauri|Sink`，`emitter.rs`）替代穿过引擎的 `AppHandle`——统一 `emit` /
`state()` / 开窗三能力；`Host::Tauri` 与原桌面逐字节相同（INV-2）。`ssh/{client,auth,prompt}` +
`ai/session` + `ssh/sftp`（流式 download/upload）已 Host 化；`ai_session_start/settings_get/settings_set/
list_models/remote_shell_probe`、`forward_start`、`github_push/pull`、`export/import_config`、
`list_recordings/read_recording` 抽 `_impl` 让命令与 server 共用。异步 dispatch + 逐消息并发。
**server 已接（全部引擎命令）**：pty；profile/credential/group/forward/settings/snippets/highlights 全 CRUD；
SSH（connect + 认证往返 + write/resize/disconnect，**录制已接**）；SFTP（核心 + **流式 download_to/upload_from**
+ cancel）；AI（settings get/**set**、**list_models**、session 全生命周期、skills、audit_save、
**cache_remote_shell**、**remote_shell_probe 真实化**）；config（export/import）、github（push/pull）、
forward（start/stop）、ssh-config（read/import）、recordings（list/read）、cli_status、update。
**垫片就地服务（浏览器即宿主）**：clipboard（navigator.clipboard）、open_external_url（window.open）、
open_tab_in_new_window（开新窗 + localStorage 交接 clone）、config 导出/导入（Blob 下载 / `<input file>`）、
ai_audit_save_pick（下载）、sftp_pick_*（IDEA 插件 `__RSSH_PICK__` 桥；裸浏览器诚实报错）。
**已验证**：`cargo check` 双绿（lib + server bin，0 error）；`cargo test --lib` **347 passed**；
`npm test` **226 passed**（含新增垫片单测 11 条）；`npm run build` 通过；Node e2e 对真实 server：
HTTP 吐前端 + 9 条命令返真数据/诚实错误（含负向控制 unknown command），export_config 仅因 mac keychain
非交互取消（与桌面同款行为，非 wiring 缺陷）。
**真正的平台缺口（仅裸浏览器）**：sftp_pick_folder/open_files 需真实本地路径，浏览器沙箱给不了 →
诚实报错；IDEA 插件侧用 `RsshBridge`（`FileChooser`）补上。cli_install 嵌入态不适用 → 诚实报错。

## 第 2 阶段：IDEA 插件外壳（先连手动启动的 headless）
**目标**：rssh 前端跑进 IDEA 工具窗。
- Kotlin 插件：注册一个 Tool Window，内嵌 `JBCefBrowser` 加载前端 bundle；
  ws 地址 + token 先靠配置/环境变量从**手动启动**的 headless 传入。
- 多窗口降级：`open_tab_in_new_window` / AI handoff → 改映射为开新 tab。
**成功标准**：IDEA 里打开该工具窗 → 完整 rssh，终端 / AI / SFTP 都能用；
"开新窗口" 动作不再报错（已降级）。
**测试**：阶段一用例在 IDEA 工具窗里重跑通过；触发"新窗口"动作 → 落为新 tab、不崩。
**状态**：代码已写，**未在本环境构建/运行**（无 IntelliJ SDK）。`idea-plugin/`：Gradle 工程 +
`plugin.xml`(ToolWindow) + `RsshToolWindowFactory`(JBCefBrowser) + `RsshServerProcess`(spawn 二进制、
读 `{port,token}`、退出杀进程) + **`RsshBridge`**（`JBCefJSQuery` 注入 `__RSSH_PICK__`，背靠 IntelliJ
`FileChooser`，让 SFTP 落盘传输在插件里可用）。**关键简化**：`rssh-server` 已自包含——前端 `include_dir!`
编进二进制，同端口 peek 区分 HTTP(吐前端) / ws(IPC)，已 curl + ws 双验证 PASS。插件只需 spawn 二进制 +
`JBCefBrowser` 指向 `http://127.0.0.1:<port>/`。dev 用 `RSSH_SERVER_BIN`，详见 `idea-plugin/README.md`。
**已构建**：`./gradlew buildPlugin` 对本机 **IntelliJ IDEA CE 2026.1 / build 261** 编译通过，产出
`idea-plugin/build/distributions/rssh-idea-0.1.0.zip`（~8MB，内含 21MB `rssh-server` 二进制 +
patched plugin.xml，since-build 242 可装进 261）。**Kotlin（含 `RsshBridge` 的 `JBCefJSQuery`）对真实
261 平台类编译通过——API 没猜错**。构建关键：`local(...)` 用本机 IDE 免下载、`jvmToolchain(21)`、
`-Xskip-metadata-version-check`（261 平台类是更新的 Kotlin 编的）、`buildSearchableOptions=false`、
wrapper 锁 Gradle 8.10.2（IJ 插件 2.1.0 不支持 Gradle 9）。
**待用户（运行时，无法在无头环境验）**：IDEA 里装 zip 或 `runIde` → 工具窗渲染 / `RsshBridge` 的 JS 注入 /
SFTP 选路径 这几条运行时行为；mac 首启 keychain 一次性授权。剩多平台二进制 + 公证。

## 第 3 阶段：自包含（插件捆二进制 + 代管生命周期）★达成用户目标
**目标**：装插件即用，零手动启动。
- 各平台 headless 二进制打进插件包；插件**懒启动**（首次开工具窗才 spawn）、
  选随机空闲端口、每次生成 token、退 IDE 时杀进程、孤儿清理、unix 解压补 `+x`。
- 处理 macOS 公证 / Gatekeeper。
- headless 指向与桌面 app **同一数据目录**；SQLite 开 WAL。
- headless 把自身所在目录 prepend 进所 spawn shell 的 PATH → 嵌入态终端里 `rssh` 命令可用，
  无需用户单独装 CLI（复用 `pty.rs` 已有的 PATH 处理）。
**成功标准**：干净环境**只装插件**（不装/不开桌面 app）→ 开工具窗 → 一切可用，
全程不敲一条命令。
**测试**：三平台冒烟；杀 IDE 进程后无残留 rssh 进程；与桌面 app 同开时配置共享、数据不坏；
纯插件环境（未单独装 rssh CLI）里 `rssh open <profile>` 仍开新 tab。
**状态**：未开始

## 第 4 阶段：分发与安装入口（含 RSSH GUI 一键）
**目标**：插件上架，RSSH 里一键直达安装。
- 插件发布到 JetBrains Marketplace。
- RSSH GUI 加"安装 IDE 插件"按钮 → 复用现有 `open_external_url` 打开插件的
  Marketplace 页面，由 JetBrains 的 "Install to IDE" 流程接手。
  **推荐理由**：零 IDE 发现逻辑、跨所有 JetBrains IDE、不随安装路径改版而坏。
- **不做**：扫描本地 IDE + shell `installPlugins` 的静默自动装——IDE 发现是个
  永远在坏的启发式黑洞，为省两次点击不值。
**成功标准**：RSSH 里点按钮 → 浏览器打开 Marketplace 插件页 → 能装进 IDE。
**测试**：点击打开正确 URL；Marketplace 安装流程走通。
**状态**：未开始

---
（全部阶段完成后删除本文件。）
