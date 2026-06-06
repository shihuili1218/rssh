# 命令黑名单 CRUD —— 实施计划

> 注：根目录 `IMPLEMENTATION_PLAN.md` 是 JetBrains 插件的在途计划，本任务单独成文，互不干扰。

## 术语（仅本任务内）
- **黑名单**：安全类五张表 `DESTRUCTIVE` / `WRITE_VERBS` / `INTERPRETERS_DENIED` / `DEFERRED_EXEC` / `COMMAND_FORWARDERS`。可 CRUD，AI 页面每类一行可编辑。
- **可用性过滤名单**：`INTERACTIVE_BARE` / `COUNTED_LOOP`。AI 页面只读说明展示。

## 模型决策（已与 Linus 对齐）
- 采用 redact rule 同款 **C 模型**：const 仅作 seed 真值 + 出厂兜底；首次建表 seed 进 DB；之后无 builtin 概念，统一 CRUD；**空表 = 用户显式放行该类**。
- **不变量（正确性，非偏好）**：空黑名单只能来自用户显式删除。seed 绑 migration v14；`load(db)` 失败 fail-closed 上抛（会话起不来），绝不退化成空集放行。
- **范围外（直接定）**：per-command 形态规则（`chmod -R` / `tail -f` / `sed -i` / `find -exec` / `touch -t` / redirect 白名单）保持硬编码；`COMMAND_ALIASES` 不开放编辑；可用性过滤名单只读。

## 数据结构
```rust
enum BlCategory { Destructive, WriteVerb, Interpreter, DeferredExec, Forwarder }
struct Blacklist(HashMap<String, BlCategory>);   // 一个命令只属一类
// check_head: 一次 get + match → 对应 ShapeError；None → Ok
// Blacklist::builtin() 从 5 张 const 表构造；DB load 从表构造；check_head 唯一
```
DB 表（schema v14，照抄 v13 seed 块）：
```sql
CREATE TABLE ai_command_blacklist ( name TEXT PRIMARY KEY, category TEXT NOT NULL );
```

---

## 第 1 阶段：后端数据结构重构（零行为变化）
**目标**：引入 `Blacklist` / `BlCategory` / `check_head`（HashMap）；`validate` 拆成 `validate(cmd)`=builtin wrapper + `validate_with(cmd, &Blacklist)`；const 5 张表保留为 `builtin()` 数据源。
**成功标准**：所有现有 `shape_*` 测试零改动且全绿（证明出厂行为完全不变）。
**测试**：现有 sanitize 测试套件 + `builtin()` 内容 == 5 张 const 表。
**状态**：完成。`cargo test --lib sanitize` → 68 passed / 0 failed（原 65 零改动 + 新增 3：空黑名单放行命令头 / 自定义黑名单拦新命令头 / builtin 覆盖五表且无重叠）。`validate` = `validate_with(cmd, &Blacklist::builtin())` wrapper，`session.rs:1022` 未动。

## 第 2 阶段：DB 层 + seed + 迁移
**目标**：`db/ai_command_blacklist.rs`（list / replace_category 整类替换事务 / load）；`schema.rs` v14 建表 + seed 5 张表带 category；`ai/command_blacklist.rs` 业务层（命令名校验 fail-fast + `load(db)->Blacklist` fail-closed）。
**成功标准**：漂移守卫单测 `seed_matches_builtin` 绿；load 在 DB 损坏时 Err、空表时返回空 Blacklist。
**测试**：seed==builtin、整类替换、空表=空名单、load fail-closed。
**状态**：完成。`db/ai_command_blacklist.rs`（list / replace_category 事务）+ `schema.rs` v14（建表 + seed 39 条）+ `ai/command_blacklist.rs`（list_grouped / replace_category 白名单校验 / load fail-closed）。全 lib **374 passed / 0 failed**，零新增 warning。seed 用硬编码 SQL（db 层不反向依赖 ai），靠 `seed_matches_builtin` 钉死漂移。

## 第 3 阶段：接线 + Tauri commands
**目标**：session 建立时 `load(db)` 物化 Blacklist 存入 session，`session.rs:1022` 改 `validate_with`；新增 `ai_list_command_blacklist` / `ai_replace_command_blacklist` commands。
**成功标准**：端到端 —— 删空 destructive 后 `rm` 放行；DB 错误时会话建立失败而非放行。
**测试**：集成测试覆盖"删空某类→放行"与"load 失败→会话 Err"。
**状态**：完成。`SessionConfig` 加 `blacklist`（启动时 `command_blacklist::load` 一次性物化、fail-closed）；`session.rs:1022` 改 `validate_with(&cmd, &self.cfg.blacklist)`；新增 `ai_list_command_blacklist` / `ai_replace_command_blacklist` 两命令并注册进 lib.rs。端到端测试 `end_to_end_emptying_category_allows_those_commands` 绿；全目标 `cargo check --all-targets` 通过。

## 迁移后清理（收尾）
**触发**：Linus 问"迁移到 DB 后原代码清理了吗"。审计结论：
- `check_command_head` 重构时已删（→ `Blacklist::check_head`）。
- `validate`(builtin wrapper) 生产零调用、却是 `pub` 脚枪（误用会绕过 DB 黑名单）→ 标 `#[cfg(test)]`，从生产 build 物理移除。
- 连带暴露：5 张 const 表 + `builtin`/`iter`/`len`/`is_empty` 迁移后已是 test-only → 全标 `#[cfg(test)]`，与 redact 的 `default_rules()`（本就 `#[cfg(test)]`）同例。
- 保留（生产路径，正确）：`from_entries`/`check_head`、`BlCategory::{ALL,as_str,from_db_str}`、硬编码的 per-command 规则 + `INTERACTIVE_BARE`/`COUNTED_LOOP`/`WRAPPERS`/`COMMAND_ALIASES`（未迁 DB，仍活跃）。
- 验证：生产 `cargo check` 仅剩 2 个预存无关警告（`decode_lossy`/`cli_status_headless`）；`cargo test --lib` 375 passed。

## 第 4 阶段：前端
**目标**：AiSettings 黑名单五行可编辑（整类 textarea，照抄 redact manager 骨架）+ 可用性过滤名单只读说明区；store / types / i18n(zh,en)。
**成功标准**：五类增删改保存后即时生效；可用性说明文案讲清"为何拦 + 改用什么"。
**测试**：手动验证五类 CRUD + 可用性说明展示。
**状态**：完成（代码层）。`types.ts` 加 `CategoryGroup`；`store.svelte.ts` 加 `listCommandBlacklist`/`replaceCommandBlacklist`；`AiSettings.svelte` 加黑名单五行整类编辑（textarea，空格/逗号分隔）+ 可用性只读说明区 + 样式；en/zh i18n 各加一段。`npm run build` 通过、`npm test` **232 passed**。**待 Linus 在 app 里目视验证 UI 渲染/交互**（本环境无浏览器）。
