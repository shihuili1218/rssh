# AGENT.md — RSSH 仓库导航

> **AGENT.md 是导航，不是真实来源；与代码冲突时以代码为准。**
> 行号、数量、签名都会变；不要把这里的字面量复制进 PR。每条声明都给了**核对命令**，自己跑一遍再用。
> 所有方案，必须是行业规范，不能走偏门，歪门邪道必须跟用户沟通后。

---

## 置顶规则（其他 AI 必读，违反就回滚）

这些是反复踩出来的硬约束。不是建议。

### R1. 后端事件命名：`<domain>:<event>:<instanceId>`

不带实例 ID 后缀的全局事件会在多 tab / 多传输并存时互相串话。第三段按事件实际作用域使用 session、prompt 或 transfer ID；不要一律假定它是 tab ID。

```bash
rg 'emit\(|format!\("[a-z]+:' src-tauri/src   # 核对最终 channel 的构造位置
```

### R2. CLI ↔ GUI 走 OSC 7337，不要再造 IPC

GUI 内嵌终端跑 `rssh-cli` 时，CLI 通过 OSC 7337 转义序列与 GUI 通信，xterm parser 解码后调 store。要扩通信，加个 OSC kind，**不要**起 socket / pipe / tauri event。

```bash
rg 'OSC_RSSH_ID|7337' src src-tauri/src/bin
```

### R3. 新增前端 command 必须检查两个后端适配器

Tauri 路径需要在 `commands/*.rs` 实现并加入 `src-tauri/src/lib.rs` 的 `generate_handler!`。同一前端功能若要在 JetBrains / browser headless 模式可用，还必须在 `src-tauri/src/server.rs` 的 dispatcher 中暴露同名 command，并尽量复用同一 domain helper。漏任一目标适配器，那个目标运行时就是 "command not found"。

```bash
rg 'generate_handler!' src-tauri/src/lib.rs
```

### R4. Tab 根容器必须明确尺寸与滚动所有权

`.pane.visible` 是 `position: absolute; inset: 0; display: flex; flex-direction: column`。直接子树必须填满它，并明确由哪一层滚动。普通滚动页面沿用 HomeScreen 的三件套：

```css
flex: 1;
overflow-y: auto;
min-height: 0;   /* 缺这条 flex 子元素不收缩，overflow 失效 */
```

终端、编辑器、Forward 这类固定画布可以用 `height: 100%`，把 overflow 留给内部 xterm / editor / list；不要盲目给根节点加 `overflow-y: auto`。共同不变量是：flex 子项需要收缩时必须有 `min-height: 0`，而且滚动只能有一个清晰 owner。

### R5. Secret 不进 DB 明文，只走统一的加密存储

调用方只能走 `SecretStore` (`src-tauri/src/secret/`)。`HybridStore` 用 ChaCha20-Poly1305 加密后把密文写入 DB `secrets` 表。首次选择 backend 时，系统 keychain 可用就把 master key 放 keychain，否则（包括 Android 和部分 headless 环境）放 data dir 下的 `master.key`；选择结果是 sticky 的，已选 keyring 后 keychain 失效必须硬失败，不能静默换新 file key。不要直接读写 `secrets` 表，也不要把原 secret 当成 keychain value 的当前架构。

```bash
rg 'secret_store|SecretStore' src-tauri/src
```

### R6. 不要建"分析文档" / "实施计划归档"

工作流要求临时 `IMPLEMENTATION_PLAN.md` 时，用完即删。不要把过程记录沉淀成新的 `NOTES.md` / `ARCHITECTURE.md` / `DESIGN.md`；除非用户明确要长期文档，仓库导航就维护这一份。

### R7. Svelte 5 runes only

`$state` / `$derived` / `$effect` / `$props`，事件 `onclick={fn}`。看到 `$:` / `export let` / `on:click` ——拒绝合并，让作者升级。

### R8. 跨页 state 放进对应 store，不放组件单例

Tab、导航和连接会话协调在 `app.svelte.ts`；AI、主题、快捷键、传输等分别在既有 domain store。沿用私有 `$state` + 导出 getter / action 的边界，**不要**导出裸 `$state` 对象，**不要**在组件里新建跨页全局状态，也不要把所有领域硬塞回 `app.svelte.ts`。

```bash
rg '^let _.*\$state|^export function' src/lib/stores src/lib/ai src/lib/themes
```

### R9. 平台条件统一走 `cfg` / `app.isMobile`

OS / 设备形态分支：Rust 端用 `#[cfg(...)]`，前端用 `app.isMobile`（UA 嗅探，顶层 const），不要在各组件重复造一套判断。Docker CLI、kubectl、keychain 等外部能力是否可用，仍应在运行时真实探测。

```bash
rg 'cfg\(target_os|isMobile' src src-tauri/src
```

### R10. 新增功能必须显式考虑四条入口

每个新 feature / UI 改动，PR 描述里至少写清以下入口各自怎么处理：

- **桌面 GUI**：默认目标，必须可用
- **移动 GUI**：`app.isMobile` 路径。没右键、没快捷键、没多窗口、屏幕窄。要么适配（`MobileKeybar` 加按钮、长按代替右键），要么显式声明"移动端不提供"
- **CLI**：`src-tauri/src/bin/rssh/`。CRUD 类操作大概率要补；纯 UI/可视化类可声明 N/A
- **Headless / JetBrains**：`src-tauri/src/server.rs` + `src/lib/ipc-shim.ts`。复用同一前端，但 command / event 需要 server adapter 支持

允许的结论是“全部支持”或“只在 X 入口，因为 Y”。**不允许**的是没想过——上线后才发现移动端按钮够不着、CLI 改了 schema 但读不出新字段、JetBrains 页面只能报 unknown command。

```bash
rg 'isMobile' src/lib/components       # 看现有移动端分支怎么写
rg '#\[cfg\(' src-tauri/src/commands   # 看 command 层平台分支
rg '"[a-z_]+" =>' src-tauri/src/server.rs  # 看 headless dispatcher
```

### R11. Transport session 必须经过 lifecycle registry

SSH / PTY / serial / Telnet / SFTP / forward 的 open 路径必须先在 `commands/lifecycle.rs` 预留规范 UUID、绑定 `SessionOwner` 与 nonce，再把 Ready handle 激活进 typed map。取消、关闭、窗口销毁和 reload reconcile 都依赖这份 registry；不要绕过它直接向 `sessions` / `pty_sessions` 等 map 插 handle。

```bash
rg 'reserve_resource|\.activate\(|reconcile_owner|close_owner' src-tauri/src/commands src-tauri/src/server.rs
```

### R12. 动态发现只持久化 source，不持久化结果

`dynamic_discovery_sources` 是用户配置，进入 DB 与 sync；Docker container / K8s pod 的发现结果是瞬时 launch target，只能转成 connector-backed PTY tab。不要偷偷写成 Profile，也不要让已消失的容器在 Home 留陈旧数据。

```bash
rg 'dynamic_discovery_sources|DynamicDiscoveredTarget|connectDynamicTarget' src src-tauri/src
```

---

## 事实（Facts）— 文件 + 概念 + 核对命令

### 二进制与 crate 布局

| 概念 | 在哪 | 怎么验证 |
|---|---|---|
| GUI 二进制 `rssh` | `src-tauri/src/main.rs` | `cat src-tauri/Cargo.toml` 看 `[[bin]]` |
| CLI 二进制 `rssh-cli` | `src-tauri/src/bin/rssh/main.rs` + `commands/`，gated by feature `cli` | 同上 |
| Headless 二进制 `rssh-server` | `src-tauri/src/server_main.rs`，gated by feature `server` | 同上；给 browser / JetBrains 提供 embedded HTTP + WebSocket IPC |
| 共享 lib `rssh_lib` | `src-tauri/src/lib.rs`，`[lib] name="rssh_lib"` | `rg 'rssh_lib::' src-tauri/src/bin/rssh` |

### 前端

| 概念 | 在哪 | 怎么验证 |
|---|---|---|
| Tab / 导航 / 会话协调 store | `src/lib/stores/app.svelte.ts` | 搜 `export function` |
| 领域 store | `src/lib/{ai,themes}/` + `src/lib/stores/` | AI、主题、快捷键、传输等各自维护 |
| Tab 渲染分发 | `src/lib/components/AppShell.svelte` | 搜 `tab.type === ` |
| 终端层 | `src/lib/components/TerminalPane.svelte` | 单文件，xterm + 高亮 + auth 全在内 |
| OSC 解码 | `src/lib/osc/handler.ts` | `registerRsshOscHandlers` |
| Headless IPC shim | `src/lib/ipc-shim.ts` | Tauri 外安装兼容 `invoke` / `listen` 的 WebSocket adapter |
| 键盘注册表 | `src/lib/keyboard/registry.ts` | `attachShortcuts` |
| i18n | `src/lib/i18n/index.svelte.ts` + `locales/{en,zh}.ts` | `t('key')` |
| 设计令牌 | `src/styles/global.css` | `--bg --accent --raised --pressed` |

### 后端

| 概念 | 在哪 | 怎么验证 |
|---|---|---|
| Tauri command 模块 | `src-tauri/src/commands/*.rs` | `ls src-tauri/src/commands` |
| Command 注册总表 | `src-tauri/src/lib.rs` 的 `generate_handler!` | 见 R3 |
| Headless command dispatcher | `src-tauri/src/server.rs` | 见 R3；只暴露明确支持的同名 command |
| 全局运行态 | `src-tauri/src/state.rs` `AppState` | `rg 'AppState' src-tauri/src` |
| Session lifecycle / ownership | `src-tauri/src/commands/lifecycle.rs` | reserve → activate → owner-scoped close/reconcile，见 R11 |
| 跨宿主事件抽象 | `src-tauri/src/emitter.rs` `Host` | Tauri emit 与 headless WebSocket sink 共用入口 |
| SSH 客户端 | `src-tauri/src/ssh/client.rs` | russh wrapper |
| SSH 认证 / 算法策略 | `src-tauri/src/ssh/auth.rs` + `ssh/algorithms.rs` | password/key/agent/kbd-interactive + per-profile algorithms |
| Telnet | `src-tauri/src/terminal/telnet.rs` + `commands/telnet.rs` | 跨平台 TCP transport + profile CRUD |
| 串口 | `src-tauri/src/terminal/serial.rs` + `commands/serial.rs` | desktop-only serialport transport |
| Docker / K8s 动态发现 | `src-tauri/src/commands/discovery.rs` | source 持久化；发现结果临时；connector PTY 见 `commands/pty.rs` |
| SFTP | `src-tauri/src/ssh/sftp.rs` | russh-sftp wrapper |
| 端口转发 | `src-tauri/src/ssh/forward.rs` | local/remote/dynamic |
| PTY（仅桌面） | `src-tauri/src/terminal/pty.rs`，`#[cfg(not(target_os="android"))]` | portable-pty |
| 数据库 | `src-tauri/src/db/`，rusqlite bundled | 看 `db/schema.rs` |
| 数据迁移 | `src-tauri/src/migration/` + `db/schema.rs` | GUI / CLI 启动都必须走幂等 migration |
| Secret 抽象 | `src-tauri/src/secret/`，HybridStore + master-key backend + 加密 DB | 见 R5 |
| 错误类型 | `src-tauri/src/error.rs` `AppError` | thiserror 派生 |
| GitHub 同步 | `src-tauri/src/sync/github.rs` | `commands/sync.rs` 入口 |
| WebDAV 同步 | `src-tauri/src/sync/webdav.rs` | `commands/sync.rs` + CLI `rssh config webdav {set,push,pull}` |

### Tab ID 形态

| Tab type | 格式 | 出处 |
|---|---|---|
| `home` | 字面量 `"home"`，唯一固定，不可关闭 | `app.svelte.ts` 初始 `_tabs` |
| `ssh` / `local` / `serial` / `telnet` / `docker_exec` / `kubectl_exec` / `edit` | `"<type>:<uuid>"` | `crypto.randomUUID()` 调用点 |
| `forward` | `"fwd:<forward_id>:<timestamp>"` | `HomeScreen` / `osc/handler.ts` |

```bash
rg 'crypto\.randomUUID' src/lib   # 看 tab id 怎么造
```

### 构建命令

```bash
npm test                                                        # Vitest；CI gate
npm run build                                                   # Vite production build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml                  # Rust tests；CI gate
cargo check --manifest-path src-tauri/Cargo.toml                 # 默认 GUI/lib
cargo check --manifest-path src-tauri/Cargo.toml --features cli --bin rssh-cli
cargo check --manifest-path src-tauri/Cargo.toml --features server --bin rssh-server
npm run tauri dev                                               # 本地 GUI
./build-mac.sh                                                  # macOS aarch64 .dmg
./build-android.sh                                              # APK + AAB（需 Android SDK/NDK）
```

当前没有 frontend lint script；有 Vitest 与 Rust unit/integration tests。UI、真实 SSH/Telnet/serial、Docker/K8s 等环境行为仍要按改动范围手测，不能拿编译替代。

---

## 坑（Pitfalls）— 非显然，会咬人

### P1. 启动 reconcile 与资源创建的顺序

主窗口必须先完成 owner-scoped `reconcile_sessions(activeIds=[])`，再放开 `resourcePanesAllowed` 让 Terminal / Forward / SFTP 创建后端资源；否则同一 owner 的新 session 会被启动清理误判成孤儿。Clone 与 AI handoff 窗口跳过这套主窗口初始化。后端现在按 `SessionOwner` 隔离，**不要再沿用“空列表会杀掉其他窗口所有 session”的旧解释**；真正不能破坏的是 owner 边界和 reconcile-before-create barrier。

```bash
rg '__rssh_clone|__rssh_ai_handoff|resourcePanesAllowed|reconcile_sessions' src src-tauri/src
```

### P2. Linux CLI shadow GUI

`/usr/local/bin/rssh`（CLI）会 shadow `/usr/bin/rssh`（GUI）。CLI 在无 subcommand + 有 `DISPLAY`/`WAYLAND_DISPLAY` 时主动 fork GUI 二进制。改 CLI 启动逻辑必须保留 `canonicalize` 自循环检测。

```bash
rg 'try_launch_gui|RSSH_APP' src-tauri/src/bin/rssh
```

### P3. Highlight 是 xterm decoration，原始字节不可改

`TerminalPane.svelte` 把后端字节原样写入 xterm；`terminal/highlight-decorations.ts` 在 xterm 已解析的 cell grid 上注册 decoration。旧版“往输出流注入 ANSI”的方案会破坏 OSC/CSI 和原始颜色，已经废弃，不能复活。改这块要同时验证宽字符、组合字符、scrollback、resize/reflow、alternate buffer 和规则热更新。

### P4. SSH prompt waiter 带 owner 与 nonce

Keyboard-interactive、私钥 passphrase、host-key TOFU 分别使用 `auth_waiters` / `passphrase_waiters` / `host_key_waiters`；值是 `OwnedWaiter { owner, nonce, sender }`，key 是 prompt ID。`ssh::prompt::prompt_oneshot` 负责注册、emit、RAII 清理，respond/cancel 与 lifecycle 按 owner/nonce 防止旧 attempt 接管新 waiter。新增 prompt 类型不要另写一套裸 oneshot map。

### P5. CLI 直接读写 DB，不经 Tauri command

改表结构 / 改 SecretStore key 命名时，**同时审 CLI 路径**。CLI 不经过 Tauri command，但应复用 `db` / `secret` / `sync::config` 等共享 domain helper；只改 command wrapper 不会自动覆盖 CLI。

```bash
rg 'db::|secret_store|sync::config' src-tauri/src/bin/rssh
```

### P6. 本地备份与远程 push 的 secret 过滤不同

本地 export 是全量备份；远程 `rssh config github push` / `rssh config webdav push` 与 GUI push 才按同步类别、分组及 secret opt-in 过滤。Credential 用 `save_to_remote`，Telnet 登录脚本用 `save_script_to_remote`。改同步逻辑必须统一走 `sync::config::build_payload`，不能让 GUI / CLI / GitHub / WebDAV 四条路径各写一份规则。

### P7. `isMobile` 是 const，不响应窗口缩放

UA 嗅探，启动一次。需要响应式断点 → 自己加 `$state` + `resize` 监听，别误以为现成。

### P8. Command 名是跨适配器 wire contract

前端 `invoke("name")`、Tauri `generate_handler!` 与 headless `server.rs` dispatcher 都按字符串对接。Rust 函数或 wire 字段改名会让某个入口运行时炸；要改就全局 grep，并补 desktop + ipc-shim/headless 覆盖。

```bash
rg 'invoke\("|generate_handler!|"[a-z_]+" =>' src/lib src-tauri/src/lib.rs src-tauri/src/server.rs
```

---

## 偏好（Preferences）— 风格 / 流程

### Pr1. 命名

- Rust：snake_case 函数与字段，PascalCase 类型
- TypeScript：camelCase 函数与变量，PascalCase 类型与组件
- State getter：动词短语 `tabs()` `activeTab()` `settingsActive()`，不带 `get` 前缀
- 错误：后端给稳定 code + params，前端按当前 locale 渲染中英文；不要在 Rust 里硬编码单一语言的用户文案

### Pr2. 错误处理

- Rust：`AppError` + `AppResult<T>`；业务错误用 `CodedMsg(code, params)`，序列化为 `__rssh_err__|{...}` wire format
- 前端：`catch (e) { toast.error(errMsg(e)) }`；不要直接把 coded wire 或裸 `String(e)` 展示给用户
- Headless：错误也要保持与 Tauri 相同的 wire shape，保证同一个 `errMsg()` 可本地化
- **不要**静默吞错（`.catch(() => {})`）除非确认是清理路径

### Pr3. CSS

- 用 `src/styles/global.css` 的变量与现有 `.neu-*` / `.btn*` 类。**不要**自己挑十六进制色——主题切换会破
- 图标自己画，别用emoji

### Pr4. 改动节奏

- 渐进式 > 大爆炸
- 一个 PR 一件事，不要顺手重构无关代码
- UI 改动跑 `npm run tauri dev` 实际点一遍，类型通过 ≠ 功能正确
- 所有分支禁止 force push；需要更新 PR 时追加正常 commit + 普通 `git push`
- 新分支先看现有命名，沿用 `feat/`、`fix/`、`chore/`、`perf/`、`refactor/`、`doc/` 前缀

### Pr5. 提交前自查

1. `npm test` + `npm run build` 通过
2. `cargo fmt --manifest-path src-tauri/Cargo.toml --check` + `cargo test --manifest-path src-tauri/Cargo.toml` 通过；按改动入口补 CLI 或 server 的 feature check（见“构建命令”）
3. 改了 command？检查 `lib.rs` handler；支持 headless 时同步 `server.rs` dispatcher（R3）
4. 改了事件名？前后端同步 grep `<domain>:`（R1）
5. 改了 schema？审 migration、sync import/export、CLI 共享路径（P5/P6）
6. 改了 UI？跑 dev 实际点；涉及 JetBrains/headless 时也走 `scripts/dev-browser.mjs`

---

## 不要做的事（速查）

- 创建分析/计划文档（R6）
- 起新 IPC 通道（R3 / R2）
- 在组件里建跨页全局状态（R8）
- 把 secret 写 DB 明文（R5）
- 绕过 lifecycle registry 直接注册 transport handle（R11）
- 把动态发现结果保存成 Profile 或其他持久数据（R12）
- 对任何分支 force push（Pr4）
- 用 `--no-verify` 跳 hook
- 让 tab 根容器尺寸不明确或出现双层滚动（R4）
- 复制本文行号字面量进代码或 PR 描述（开头免责声明）
- 只为单一入口写功能而没声明其他入口的处理（R10）
