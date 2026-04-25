# AGENT.md — RSSH 仓库导航

> **AGENT.md 是导航，不是真实来源；与代码冲突时以代码为准。**
> 行号、数量、签名都会变；不要把这里的字面量复制进 PR。每条声明都给了**核对命令**，自己跑一遍再用。
> 所有方案，必须是行业规范，不能走偏门，歪门邪道必须跟用户沟通后。

---

## 置顶规则（其他 AI 必读，违反就回滚）

这些是反复踩出来的硬约束。不是建议。

### R1. Tauri 事件命名：`<domain>:<event>:<sessionId>`

不带 sessionId 后缀的全局事件会在多 tab 共存时互相串话。新事件必须遵守这个三段式。

```bash
rg 'emit\(' src-tauri/src   # 看现有事件全是这个形态
```

### R2. CLI ↔ GUI 走 OSC 7337，不要再造 IPC

GUI 内嵌终端跑 `rssh-cli` 时，CLI 通过 OSC 7337 转义序列与 GUI 通信，xterm parser 解码后调 store。要扩通信，加个 OSC kind，**不要**起 socket / pipe / tauri event。

```bash
rg 'OSC_RSSH_ID|7337' src src-tauri/src/bin
```

### R3. 新增 `#[tauri::command]` 必须双注册

`commands/*.rs` 写函数 + `src-tauri/src/lib.rs` 的 `generate_handler!` 宏注册。漏一处 = 前端 "command not found"。

```bash
rg 'generate_handler!' src-tauri/src/lib.rs
```

### R4. Tab 内根容器三件套

`.pane` 是 `position: absolute; inset: 0; flex column`。**任何**直接放进 `.pane` 的根 `<div>` 必须：

```css
flex: 1;
overflow-y: auto;
min-height: 0;   /* 缺这条 flex 子元素不收缩，overflow 失效 */
```

漏了 → 内容溢出整块被裁，没法滚动。HomeScreen 是范例。

### R5. Secret 不进 DB 明文

走 `SecretStore` (`src-tauri/src/secret/`)。Keyring 在 macOS/Windows/Linux 用原生 backend，Android 自动降级到 DB（仍然是这一个抽象，不要绕开）。

```bash
rg 'secret_store|SecretStore' src-tauri/src
```

### R6. 不要建"分析文档" / "实施计划归档"

`IMPLEMENTATION_PLAN.md` 用完即删（CLAUDE.md 已规定）。不要写新的 `NOTES.md` / `ARCHITECTURE.md` / `DESIGN.md`。导航就这一份。

### R7. Svelte 5 runes only

`$state` / `$derived` / `$effect` / `$props`，事件 `onclick={fn}`。看到 `$:` / `export let` / `on:click` ——拒绝合并，让作者升级。

### R8. State 所有权在 `app.svelte.ts`

私有 `let _x = $state(...)` + 导出 getter 函数。**不要**导出裸 `$state` 对象，**不要**在组件里建跨页全局状态。

```bash
rg 'export function .* { return _' src/lib/stores/app.svelte.ts
```

### R9. 平台分支用 `cfg` / `app.isMobile`，不要运行时探测

Rust 端 `#[cfg(target_os = "android")]`，前端 `app.isMobile`（UA 嗅探，顶层 const）。

```bash
rg 'cfg\(target_os|isMobile' src src-tauri/src
```

### R10. 新增功能必须显式考虑三端：桌面 GUI / 移动 GUI / CLI

每个新 feature / UI 改动，PR 描述里至少写清三端各自怎么处理：

- **桌面 GUI**：默认目标，必须可用
- **移动 GUI**：`app.isMobile` 路径。没右键、没快捷键、没多窗口、屏幕窄。要么适配（`MobileKeybar` 加按钮、长按代替右键），要么显式声明"移动端不提供"
- **CLI**：`src-tauri/src/bin/rssh.rs`。CRUD 类操作大概率要补；纯 UI/可视化类可声明 N/A

允许的结论是 "三端都做" 或 "只在 X 端，因为 Y"。**不允许**的是没想过——上线后才发现移动端按钮够不着、CLI 改了 schema 但读不出新字段。

```bash
rg 'isMobile' src/lib/components       # 看现有移动端分支怎么写
rg '#\[cfg\(' src-tauri/src/commands   # 看 command 层平台分支
```

---

## 事实（Facts）— 文件 + 概念 + 核对命令

### 二进制与 crate 布局

| 概念 | 在哪 | 怎么验证 |
|---|---|---|
| GUI 二进制 `rssh` | `src-tauri/src/main.rs` | `cat src-tauri/Cargo.toml` 看 `[[bin]]` |
| CLI 二进制 `rssh-cli` | `src-tauri/src/bin/rssh.rs`，gated by feature `cli` | 同上 |
| 共享 lib `rssh_lib` | `src-tauri/src/lib.rs`，`[lib] name="rssh_lib"` | CLI 的 `use rssh_lib::*;` |

### 前端

| 概念 | 在哪 | 怎么验证 |
|---|---|---|
| 全局 store | `src/lib/stores/app.svelte.ts` | 单文件，搜 `export function` |
| Tab 渲染分发 | `src/lib/components/AppShell.svelte` | 搜 `tab.type === ` |
| 终端层 | `src/lib/components/TerminalPane.svelte` | 单文件，xterm + 高亮 + auth 全在内 |
| OSC 解码 | `src/lib/osc/handler.ts` | `registerRsshOscHandlers` |
| 键盘注册表 | `src/lib/keyboard/registry.ts` | `attachShortcuts` |
| i18n | `src/lib/i18n/index.svelte.ts` + `locales/{en,zh}.ts` | `t('key')` |
| 设计令牌 | `src/styles/global.css` | `--bg --accent --raised --pressed` |

### 后端

| 概念 | 在哪 | 怎么验证 |
|---|---|---|
| Tauri command 模块 | `src-tauri/src/commands/*.rs` | `ls src-tauri/src/commands` |
| Command 注册总表 | `src-tauri/src/lib.rs` 的 `generate_handler!` | 见 R3 |
| 全局运行态 | `src-tauri/src/state.rs` `AppState` | `rg 'AppState' src-tauri/src` |
| SSH 客户端 | `src-tauri/src/ssh/client.rs` | russh wrapper |
| SFTP | `src-tauri/src/ssh/sftp.rs` | russh-sftp wrapper |
| 端口转发 | `src-tauri/src/ssh/forward.rs` | local/remote/dynamic |
| PTY（仅桌面） | `src-tauri/src/terminal/pty.rs`，`#[cfg(not(target_os="android"))]` | portable-pty |
| 数据库 | `src-tauri/src/db/`，rusqlite bundled | 看 `db/schema.rs` |
| 密钥抽象 | `src-tauri/src/secret/`，trait + keyring/db 两实现 | 见 R5 |
| 错误类型 | `src-tauri/src/error.rs` `AppError` | thiserror 派生 |
| GitHub 同步 | `src-tauri/src/sync/github.rs` | `commands/sync.rs` 入口 |

### Tab ID 形态

| Tab type | 格式 | 出处 |
|---|---|---|
| `home` | 字面量 `"home"`，唯一固定，不可关闭 | `app.svelte.ts` 初始 `_tabs` |
| `ssh` / `local` / `edit` | `"<type>:<uuid>"` | `crypto.randomUUID()` 调用点 |
| `forward` | `"fwd:<forward_id>:<timestamp>"` | `HomeScreen` / `osc/handler.ts` |

```bash
rg 'crypto\.randomUUID' src/lib   # 看 tab id 怎么造
```

### 构建命令

```bash
npm run build                     # Vite 编前端
cd src-tauri && cargo check       # Rust 类型检查
./build-mac.sh                    # macOS aarch64 .dmg
./build-android.sh                # APK + AAB（需 ANDROID_HOME/NDK）
npm run tauri dev                 # 本地跑
```

无 lint，无 unit test。验证靠编译 + 手动点。

---

## 坑（Pitfalls）— 非显然，会咬人

### P1. 多窗口 `reconcile_sessions` 陷阱

启动时若不是克隆窗口，调 `reconcile_sessions(activeIds=[])` 让后端清孤儿。**克隆窗口必须跳过**——它们与父窗共享 `AppState.sessions`，传空列表会把别窗口的 session 全杀。判定靠 `window.__rssh_clone` 标志，由 `open_tab_in_new_window` 注入。

```bash
rg '__rssh_clone|reconcile_sessions' src src-tauri/src
```

### P2. Linux CLI shadow GUI

`/usr/local/bin/rssh`（CLI）会 shadow `/usr/bin/rssh`（GUI）。CLI 在无 subcommand + 有 `DISPLAY`/`WAYLAND_DISPLAY` 时主动 fork GUI 二进制。改 CLI 启动逻辑必须保留 `canonicalize` 自循环检测。

```bash
rg 'try_launch_gui|RSSH_APP' src-tauri/src/bin
```

### P3. Highlight 注入是 stateful lexer

`TerminalPane.svelte` 把用户配的 keyword regex 插入 ANSI 24-bit 转义到 stdin 流。**必须保留已有 ANSI 序列**——别用朴素 replace。改这块前先理解现有 lexer 状态机。

### P4. Keyboard-interactive auth 走 oneshot channel

`AppState.auth_waiters: HashMap<tab_id, oneshot::Sender<Vec<String>>>`。事件 `ssh:auth_prompt:{tabId}` → 前端模态 → `ssh_auth_respond` command → channel send。Tab 中途关闭要清 waiter，否则 leak。

### P5. CLI 直接读写 DB，不经 Tauri command

改表结构 / 改 SecretStore key 命名时，**同时审 CLI 路径**。CLI 不会自动跟随 command 层的逻辑变更。

```bash
rg 'db::|secret_store' src-tauri/src/bin/rssh.rs
```

### P6. `save_to_remote` 决定 secret 是否上 GitHub

Credential 上的开关，`config push` / `commands/sync.rs` 据此过滤。改同步逻辑必须看两处。

### P7. `isMobile` 是 const，不响应窗口缩放

UA 嗅探，启动一次。需要响应式断点 → 自己加 `$state` + `resize` 监听，别误以为现成。

### P8. Tauri command 改名是破坏性变更

前端 `invoke("name")` 字符串硬编码。Rust 端改函数名 = 前端运行时炸。要改 grep 全局 `invoke("` 同步。

```bash
rg 'invoke\("' src/lib
```

---

## 偏好（Preferences）— 风格 / 流程

### Pr1. 命名

- Rust：snake_case 函数与字段，PascalCase 类型
- TypeScript：camelCase 函数与变量，PascalCase 类型与组件
- State getter：动词短语 `tabs()` `activeTab()` `settingsActive()`，不带 `get` 前缀
- 错误消息：中文，面向用户

### Pr2. 错误处理

- Rust：`AppError` enum + `AppResult<T>`，命令返回它，自动序列化成字符串
- 前端：`try { await invoke(...) } catch (e) { app.toast(...) }`，无全局 boundary
- **不要**静默吞错（`.catch(() => {})`）除非确认是清理路径

### Pr3. CSS

用 `src/styles/global.css` 的变量与现有 `.neu-*` / `.btn*` 类。**不要**自己挑十六进制色——主题切换会破。

### Pr4. 改动节奏

- 渐进式 > 大爆炸
- 一个 PR 一件事，不要顺手重构无关代码
- UI 改动跑 `npm run tauri dev` 实际点一遍，类型通过 ≠ 功能正确

### Pr5. 提交前自查

1. `npm run build` 通过
2. `cd src-tauri && cargo check` 通过
3. 改了 command？检查 `lib.rs` 的 `generate_handler!`（R3）
4. 改了事件名？前后端同步 grep `<domain>:`（R1）
5. 改了 schema？审 `db/schema.rs` migration + CLI 路径（P5）
6. 改了 UI？跑 dev 点过

---

## 不要做的事（速查）

- 创建分析/计划文档（R6）
- 起新 IPC 通道（R3 / R2）
- 在组件里建跨页全局状态（R8）
- 把 secret 写 DB 明文（R5）
- 用 `--no-verify` 跳 hook
- 给 tab 根容器忘记 `flex:1; overflow-y:auto; min-height:0`（R4）
- 复制本文行号字面量进代码或 PR 描述（开头免责声明）
- 只为单一平台写功能而没声明其他两端的处理（R10）
