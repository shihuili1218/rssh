//! AI 模块的 Tauri 命令入口。仅前端 ↔ Rust 桥，不引入新 IPC 协议。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use serde_json::json;

use crate::error::{locked, AppError, AppResult};
use crate::secret::setting_key;
use crate::state::AppState;

use super::llm;
use super::redact_rules::{self, RedactRuleRecord};
use super::sanitize;
use super::session::{self, DiagnoseSession, UserAction};
use super::skills::{self, SkillRecord};

// ─── BYOK 设置存储键 ────────────────────────────────────────────────
//
// 分两类存储：
//   - API key：真 secret，走 state.secret_store（= HybridStore，ChaCha20-Poly1305
//     加密后落 DB.secrets 表；主密钥在 keychain 或 master.key 文件），带 "setting:"
//     命名空间前缀。
//   - 其它（provider/model/endpoint/danger_mode/auto_*）：行为偏好，走 DB.settings 表
//     （明文裸 key），跟 locale / theme / verbose_log 同档次。
//
// 之前历史版本所有 ai_* 都塞进 keychain，把 keychain 当通用键值库用 ——
// 滥用 keychain 容量、增加 OS 解锁次数（mac Touch ID 弹窗）、语义错乱（行为偏好
// 不是 secret）。PR #59 把行为偏好迁出 keychain；PR #60 把 secret 统一走 HybridStore
// 加密 DB 解决 Windows Credential Manager 2560 字节硬限。

fn key_provider() -> String {
    "ai_provider".into()
}
fn key_model(provider: &str) -> String {
    format!("ai_{provider}_model")
}
fn key_endpoint(provider: &str) -> String {
    format!("ai_{provider}_endpoint")
}
/// API key 走 keychain —— 唯一真 secret，命名空间带 "setting:" 前缀以跟其它
/// secret（cred:* 等）隔离。
fn key_api_key(provider: &str) -> String {
    setting_key(&format!("ai_{provider}_key"))
}
/// 危险模式（全局，不分 provider）：总闸；开启后才允许下面的 per-tool 自动批准生效。
/// 这是 issue #39 的明确需求——用户在受控环境（隔离 VM、靶机）里期望像 Claude Code
/// 一样无打扰自主跑，但要分粒度——文件改动比命令风险高一档，得让用户自己决定。
fn key_danger_mode() -> String {
    "ai_danger_mode".into()
}
/// per-tool 自动批准开关。仅当 danger_mode=on 时生效。
/// run_command / match_file 默认 true（向后兼容旧 danger_mode 全开的行为），
/// 其它默认 false（写动作 / 大副作用，明确表示需要新设默认就开）。
fn key_auto(name: &str) -> String {
    format!("ai_auto_{name}")
}
/// 远端 shell 自动探测开关。off（默认）时远端 shell 假设 POSIX（覆盖 ~99% Linux/macOS
/// 远端，零探测开销保留现有行为）；on 时 AI panel 打开后台发一行 echo 探针，靠
/// `$PSEdition`/`$$` 解析结果在三类 shell（POSIX / cmd.exe / PowerShell）间分类。
/// 跟 per-tool `auto_*`（自动批准）语义不同，独立 key。
fn key_auto_detect_remote_shell() -> String {
    "ai_auto_detect_remote_shell".into()
}

// ─── 命令 ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct AiSessionInfo {
    /// Tab 身份。actor 跟 tab 同寿命 —— SSH 断了重连，前端用 tab_id 仍能定位到
    /// 同一个 actor，历史就回来了。
    pub tab_id: String,
    /// 当前绑定的 SSH/PTY session_id（重连时由 rebind 更新）。
    pub target_id: String,
    pub skill: String,
    pub model: String,
    pub provider: String,
}

impl From<&DiagnoseSession> for AiSessionInfo {
    fn from(s: &DiagnoseSession) -> Self {
        Self {
            tab_id: s.tab_id.clone(),
            target_id: s.target_id.clone(),
            skill: s.skill.clone(),
            model: s.model.clone(),
            provider: s.provider.clone(),
        }
    }
}

/// What an AI session is attached to. An enum — not a `(kind: String, id: String)`
/// pair — precisely because the kind is closed ("ssh" | "local"): the type makes
/// the old stringly-typed `_ => unknown_target_kind` runtime error (which was
/// duplicated across session start, rebind, and the headless rebind) impossible
/// to reach. Wire shape: `{ "kind": "ssh"|"local", "id": "<session/pty id>" }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum AiTarget {
    Ssh(String),
    Local(String),
}

impl AiTarget {
    /// The underlying SSH session / local-PTY id, regardless of kind.
    pub fn id(&self) -> &str {
        match self {
            AiTarget::Ssh(id) | AiTarget::Local(id) => id,
        }
    }
}

#[tauri::command]
pub async fn ai_list_skills(state: State<'_, AppState>) -> AppResult<Vec<SkillRecord>> {
    skills::list_all(&state.db)
}

#[tauri::command]
pub async fn ai_get_skill(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<SkillRecord>> {
    skills::get(&state.db, &id)
}

#[tauri::command]
pub async fn ai_save_skill(
    state: State<'_, AppState>,
    id: String,
    name: String,
    description: String,
    content: String,
) -> AppResult<()> {
    skills::save_user(
        &state.db,
        &SkillRecord {
            id,
            name,
            description,
            content,
            builtin: false,
        },
    )
}

#[tauri::command]
pub async fn ai_delete_skill(state: State<'_, AppState>, id: String) -> AppResult<()> {
    skills::delete_user(&state.db, &id)
}

// ─── 脱敏规则管理 ────────────────────────────────────────────────────
// 规则变更只对**新会话**生效（同 skill：建会话时 snapshot，会话期间不重读 DB）。

#[tauri::command]
pub async fn ai_list_redact_rules(
    state: State<'_, AppState>,
) -> AppResult<Vec<RedactRuleRecord>> {
    redact_rules::list(&state.db)
}

#[tauri::command]
pub async fn ai_save_redact_rule(
    state: State<'_, AppState>,
    id: String,
    pattern: String,
    replacement: String,
) -> AppResult<()> {
    redact_rules::save(
        &state.db,
        &RedactRuleRecord {
            id,
            pattern,
            replacement,
        },
    )
}

#[tauri::command]
pub async fn ai_delete_redact_rule(state: State<'_, AppState>, id: String) -> AppResult<()> {
    redact_rules::delete(&state.db, &id)
}

/// 把前端 locale code 映射为给 LLM 的语言名称（用于 prompt 末尾的 "Respond in X"）。
fn locale_label(locale: &str) -> &'static str {
    match locale {
        "zh" | "zh-CN" | "zh-Hans" => "Chinese (Simplified)",
        "zh-TW" | "zh-Hant" => "Chinese (Traditional)",
        _ => "English",
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ai_session_start(
    app: AppHandle,
    state: State<'_, AppState>,
    tab_id: String,
    target: AiTarget,
    skill: String,
    provider: String,
    model: String,
    locale: Option<String>,
) -> AppResult<AiSessionInfo> {
    ai_session_start_impl(
        &state,
        crate::emitter::Host::Tauri(app),
        tab_id,
        target,
        skill,
        provider,
        model,
        locale,
    )
    .await
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
#[allow(clippy::too_many_arguments)]
pub async fn ai_session_start_impl(
    state: &AppState,
    host: crate::emitter::Host,
    tab_id: String,
    target: AiTarget,
    skill: String,
    provider: String,
    model: String,
    locale: Option<String>,
) -> AppResult<AiSessionInfo> {
    {
        let g = locked(&state.ai_sessions)?;
        // 一个 tab 至多一个 actor。前端 ensureSession 已经做了"有 session 就复用"的判断，
        // 但并发请求还是可能撞同一个 tab_id —— 这里 fail-fast，不静默覆盖。
        // 注意：此处只是早退化检查；下面有 await 会释放锁，所以 insert 时还要再查一次。
        if g.contains_key(&tab_id) {
            return Err(AppError::other(
                "session_already_exists",
                json!({ "tab_id": tab_id }),
            ));
        }
    }

    // 1. API key（走 keychain）+ endpoint（走 DB settings 明文）
    let api_key = state
        .secret_store
        .get(&key_api_key(&provider))?
        .ok_or_else(|| AppError::config("api_key_missing", json!({ "provider": provider })))?;
    let endpoint = crate::db::settings::get(&state.db, &key_endpoint(&provider))?;

    // 2. 校验 target 存在 + 抓 SSH handle（给 download_file 工具复用）+ 推断初始 shell。
    //
    // Shell 决策 3 路径：
    //   - local PTY: PtyHandle 记得用户选的 shell path → 映射 ShellKind，零探测。
    //   - remote SSH + auto_detect off: POSIX 兜底（保护 99% Linux/macOS 现状）。
    //   - remote SSH + auto_detect on:
    //       cache 命中（连接时探测已写入）→ 用缓存值。
    //       cache 未命中（探测未完成 / 失败 / 未开 auto_detect）→ POSIX 兜底。
    //   探测本身在 SSH 连接成功后跑（见前端 TerminalPane + ai_remote_shell_probe_needed /
    //   ai_cache_remote_shell），结果只落 profile 缓存，与 AI 会话生命周期解耦。
    let auto_detect = crate::db::settings::get(&state.db, &key_auto_detect_remote_shell())?
        .map(|v| v == "1")
        .unwrap_or(false);
    let mut initial_shell = super::shell::ShellKind::default();
    let ssh_handle = match &target {
        AiTarget::Ssh(target_id) => {
            let (handle, profile_id) = {
                let g = locked(&state.sessions)?;
                let h = g
                    .get(target_id)
                    .ok_or_else(|| AppError::not_found("ssh_session_not_found", json!({})))?;
                (h.ssh_handle().clone(), h.profile_id().to_string())
            };
            if auto_detect {
                if let Some(k) = locked(&state.ai_remote_shell_cache)?.get(&profile_id).copied() {
                    initial_shell = k; // cache 命中：用连接时探测的结果
                }
                // cache 未命中：保持 POSIX 兜底（探测还没跑完或失败，不阻塞启动）。
            }
            Some(handle)
        }
        #[cfg(not(target_os = "android"))]
        AiTarget::Local(target_id) => {
            let g = locked(&state.pty_sessions)?;
            let pty = g
                .get(target_id)
                .ok_or_else(|| AppError::not_found("local_pty_not_found", json!({})))?;
            initial_shell = super::shell::ShellKind::from_local_path(pty.shell_path());
            None
        }
        #[cfg(target_os = "android")]
        AiTarget::Local(_) => {
            return Err(AppError::not_found("local_pty_not_found", json!({})))
        }
    };

    let client = llm::build_client(&provider, api_key, endpoint)?;

    // system prompt = 内置 general 规则集 + user-skill 目录（id + description）。
    // user-skill 详细内容走 load_skill 工具按需加载（claude skills 模式），
    // 用户写多个 skill 也不会让启动 prompt 爆炸。
    let _ = skill; // 前端不再选；保留参数兼容
    let locale_lbl = locale_label(locale.as_deref().unwrap_or("en"));
    // 移动端硬阻碍：analyze_locally（spawn window）+ download_file（rfd dialog）
    // 都在本端无解（见 session.rs:442 / commands.rs:281）。给 LLM 注入声明，
    // 让它直接引导用户切桌面端，不要在远端硬扛 dump/分析。
    let is_mobile = cfg!(target_os = "android") || cfg!(target_os = "ios");
    let system_prompt = skills::build_catalog_prompt(&state.db, locale_lbl, is_mobile)?;
    let user_skills_cache = skills::list_user(&state.db)?;

    let cfg = session::SessionConfig {
        tab_id: tab_id.clone(),
        target_id: target.id().to_string(),
        skill: "general".to_string(),
        system_prompt,
        user_skills_cache,
        model,
        client,
        // DB 读规则失败直接向上抛 —— fail-closed。脱敏策略读不出来就让会话起步失败、
        // 报错给用户，绝不静默回退默认：用户可能删过 / 改严过默认规则，回退会悄悄套用
        // 一套不同的（更弱的）脱敏策略，给"我的配置生效了"的假象，比报错危险得多。
        redact_rules: redact_rules::compiled(&state.db)?,
        max_output_bytes: sanitize::DEFAULT_MAX_OUTPUT_BYTES,
        ssh_handle,
        data_dir: state.data_dir.clone(),
        // 本地 PTY: 上面从 PtyHandle.shell_path 推断（确定值）；
        // 远端 SSH: 连接时探测命中过则用缓存值，否则 POSIX 默认（写一次，会话期不变）。
        shell_kind: initial_shell,
    };

    // 并发 start 防御：上方的 contains_key 检查到这里的 insert 之间夹着
    // 几个 await（api_key / target 校验 / llm client 构造），锁会释放，两个
    // 并发请求都可能通过第一轮检查。
    //
    // `session::start` 现在返回 `PendingSession`（actor **未 spawn**）—— 在
    // 锁下再查一遍，撞了直接 return Err，PendingSession 就地 drop，actor
    // 从未运行过、不会 emit `ai:session_ended:<tab_id>` 污染赢家的事件流。
    let pending = session::start(cfg, host)?;
    let info = AiSessionInfo::from(pending.info());
    {
        let mut g = locked(&state.ai_sessions)?;
        if g.contains_key(&tab_id) {
            return Err(AppError::other(
                "session_already_exists",
                json!({ "tab_id": tab_id }),
            ));
        }
        g.insert(tab_id, pending.launch());
    }
    Ok(info)
}

#[tauri::command]
pub async fn ai_user_message(
    state: State<'_, AppState>,
    tab_id: String,
    text: String,
) -> AppResult<()> {
    let tx = locked(&state.ai_sessions)?
        .get(&tab_id)
        .map(|s| s.action_tx.clone())
        .ok_or_else(|| AppError::not_found("ai_session_not_found", json!({})))?;
    tx.send(UserAction::Message(text))
        .map_err(|_| AppError::other("ai_session_stopped", json!({})))?;
    Ok(())
}

#[tauri::command]
pub async fn ai_command_result(
    state: State<'_, AppState>,
    tab_id: String,
    tool_call_id: String,
    exit_code: i32,
    output: String,
    timed_out: bool,
    early_terminated: Option<bool>,
) -> AppResult<()> {
    let tx = locked(&state.ai_sessions)?
        .get(&tab_id)
        .map(|s| s.action_tx.clone())
        .ok_or_else(|| AppError::not_found("ai_session_not_found", json!({})))?;
    tx.send(UserAction::CommandResult {
        tool_call_id,
        exit_code,
        output,
        timed_out,
        early_terminated: early_terminated.unwrap_or(false),
    })
    .map_err(|_| AppError::other("ai_session_stopped", json!({})))?;
    Ok(())
}

#[tauri::command]
pub async fn ai_command_reject(
    state: State<'_, AppState>,
    tab_id: String,
    tool_call_id: String,
    reason: String,
) -> AppResult<()> {
    let tx = locked(&state.ai_sessions)?
        .get(&tab_id)
        .map(|s| s.action_tx.clone())
        .ok_or_else(|| AppError::not_found("ai_session_not_found", json!({})))?;
    tx.send(UserAction::RejectCommand {
        tool_call_id,
        reason,
    })
    .map_err(|_| AppError::other("ai_session_stopped", json!({})))?;
    Ok(())
}

/// 销毁 actor。前端在 tab close 时调（panel close 只隐藏 UI，不调这个）。
#[tauri::command]
pub async fn ai_session_stop(state: State<'_, AppState>, tab_id: String) -> AppResult<()> {
    let session = locked(&state.ai_sessions)?
        .remove(&tab_id)
        .ok_or_else(|| AppError::not_found("ai_session_not_found", json!({})))?;
    let _ = session.action_tx.send(UserAction::Stop);
    Ok(())
}

/// 清空 actor 的 history（保留 audit log）。
/// 用户手动按"清理上下文"按钮触发。actor 不死，下条 message 进来时是全新对话。
#[tauri::command]
pub async fn ai_session_clear_context(
    state: State<'_, AppState>,
    tab_id: String,
) -> AppResult<()> {
    let tx = locked(&state.ai_sessions)?
        .get(&tab_id)
        .map(|s| s.action_tx.clone())
        .ok_or_else(|| AppError::not_found("ai_session_not_found", json!({})))?;
    tx.send(UserAction::ClearContext)
        .map_err(|_| AppError::other("ai_session_stopped", json!({})))?;
    Ok(())
}

/// 连接时探测前的门控查询：这个 target 现在需要跑 shell 探测吗？
///
/// true 仅当三条同时成立：auto_detect 开 + target 是已知 SSH 会话 + 该 profile 进程
/// 缓存未命中。缓存命中（同 profile 之前探过）或 auto_detect 关 → false，于是连接 /
/// 重连都不会重复刷探针。本地 PTY 的 session_id 不在 `state.sessions` 里 → 自然返回
/// false（本地 shell 已知，无需探测）。
#[tauri::command]
pub async fn ai_remote_shell_probe_needed(
    state: State<'_, AppState>,
    target_id: String,
) -> AppResult<bool> {
    ai_remote_shell_probe_needed_impl(&state, target_id)
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub fn ai_remote_shell_probe_needed_impl(state: &AppState, target_id: String) -> AppResult<bool> {
    let auto_detect = crate::db::settings::get(&state.db, &key_auto_detect_remote_shell())?
        .map(|v| v == "1")
        .unwrap_or(false);
    if !auto_detect {
        return Ok(false);
    }
    let profile_id = match locked(&state.sessions)?.get(&target_id) {
        Some(h) => h.profile_id().to_string(),
        None => return Ok(false), // 本地 PTY 或会话已断：不探
    };
    Ok(!locked(&state.ai_remote_shell_cache)?.contains_key(&profile_id))
}

/// 前端在 SSH 连接成功后探测远端 shell，把结果写进程级 cache（key=profile_id）。
///
/// 不需要 AI 会话 —— shell 种类是 profile 级事实，AI session 启动时（ai_session_start）
/// 从这个缓存读初始 shell。同 profile 的下一次连接 / AI 会话直接命中，不再重探。
///
/// `target_id` 是发起探测时的 SSH session_id；探针刚回来 SSH 就断（极少）→ 查不到
/// profile_id，静默跳过写入（下次连接会重探），不当错误抛。
#[tauri::command]
pub async fn ai_cache_remote_shell(
    state: State<'_, AppState>,
    target_id: String,
    shell: super::shell::ShellKind,
) -> AppResult<()> {
    if let Some(profile_id) = locked(&state.sessions)?
        .get(&target_id)
        .map(|h| h.profile_id().to_string())
    {
        locked(&state.ai_remote_shell_cache)?.insert(profile_id, shell);
    }
    Ok(())
}

/// SSH 重连后重新绑定 target。actor 内部更新 target_id + ssh_handle，让
/// 后续 file_ops / SFTP 走新连接。history / audit / pending 全保留。
///
/// 错误模型：target_id 必须在 state.sessions / pty_sessions 里存在，否则
/// rebind 后立即触发的 file_ops 会拿到孤儿 handle 失败。前端通常在 SSH 重连
/// 成功后立刻调，target 存在是常态。
#[tauri::command]
pub async fn ai_session_rebind_target(
    state: State<'_, AppState>,
    tab_id: String,
    target: AiTarget,
) -> AppResult<()> {
    // 重新抓 ssh_handle（local 是 None）—— 复用 ai_session_start 的同款校验。
    let ssh_handle = match &target {
        AiTarget::Ssh(target_id) => {
            let g = locked(&state.sessions)?;
            let h = g
                .get(target_id)
                .ok_or_else(|| AppError::not_found("ssh_session_not_found", json!({})))?;
            Some(h.ssh_handle().clone())
        }
        #[cfg(not(target_os = "android"))]
        AiTarget::Local(target_id) => {
            if !locked(&state.pty_sessions)?.contains_key(target_id) {
                return Err(AppError::not_found("local_pty_not_found", json!({})));
            }
            None
        }
        #[cfg(target_os = "android")]
        AiTarget::Local(_) => {
            return Err(AppError::not_found("local_pty_not_found", json!({})))
        }
    };
    let target_id = target.id().to_string();

    // 锁里一次完成：拿 action_tx + 同步更新 stored target_id。
    // 不更新 stored 那一份的话，ai_list_sessions / AiSessionInfo::from 会一直
    // 报老 target_id；前端 resync 时反向把本地缓存覆盖回旧值。
    let tx = {
        let mut g = locked(&state.ai_sessions)?;
        let s = g
            .get_mut(&tab_id)
            .ok_or_else(|| AppError::not_found("ai_session_not_found", json!({})))?;
        s.target_id = target_id.clone();
        s.action_tx.clone()
    };
    tx.send(UserAction::RebindTarget {
        target_id,
        ssh_handle,
    })
    .map_err(|_| AppError::other("ai_session_stopped", json!({})))?;
    Ok(())
}

/// 取消当前正在进行的 LLM 流式响应。仅当 actor 阻塞在 chat() 时有效；
/// 否则 slot 为 None，这是 no-op（不算错——用户也可能恰好在响应完结那一刻按下）。
/// 会话本身（history / pending command / audit）全部保留。
#[tauri::command]
pub async fn ai_cancel_stream(
    state: State<'_, AppState>,
    tab_id: String,
) -> AppResult<()> {
    let slot = locked(&state.ai_sessions)?
        .get(&tab_id)
        .map(|s| s.cancel_slot.clone())
        .ok_or_else(|| AppError::not_found("ai_session_not_found", json!({})))?;
    // 先把 Notify clone 出来再释放锁——slot 用的是 std::sync::Mutex，
    // 持锁期间调 notify_one 阻塞 actor 端尝试清空 slot 的同一把锁。
    // 用代码块限定 guard 生命周期，notify_one 在 lock 释放后才执行。
    let notify = {
        let g = slot.lock().map_err(|_| AppError::Lock)?;
        g.as_ref().cloned()
    };
    if let Some(n) = notify {
        n.notify_one();
    }
    Ok(())
}

#[tauri::command]
pub async fn ai_audit_save(
    state: State<'_, AppState>,
    tab_id: String,
    file_path: String,
) -> AppResult<()> {
    let audit = locked(&state.ai_sessions)?
        .get(&tab_id)
        .map(|s| s.audit.clone())
        .ok_or_else(|| AppError::not_found("ai_session_not_found", json!({})))?;
    let g = audit.lock().map_err(|_| AppError::Lock)?;
    g.save_to_file(&PathBuf::from(file_path))?;
    Ok(())
}

/// 弹原生 Save 对话框选路径，再保存。返回保存的路径；用户取消返回 None。
/// Android 无 rfd 依赖，返回未实现错误。
#[tauri::command]
pub async fn ai_audit_save_pick(
    state: State<'_, AppState>,
    tab_id: String,
) -> AppResult<Option<String>> {
    #[cfg(target_os = "android")]
    {
        let _ = (state, tab_id);
        Err(AppError::other("android_no_dialog", json!({})))
    }
    #[cfg(not(target_os = "android"))]
    {
        let audit = locked(&state.ai_sessions)?
            .get(&tab_id)
            .map(|s| s.audit.clone())
            .ok_or_else(|| AppError::not_found("ai_session_not_found", json!({})))?;

        let default_dir = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        let default_name = format!(
            "rssh-diagnose-{}-{}.log",
            &tab_id[..tab_id.len().min(8)],
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );

        let pick = rfd::AsyncFileDialog::new()
            .set_directory(default_dir)
            .set_file_name(default_name)
            .add_filter("Log", &["log", "txt"])
            .save_file()
            .await;

        let Some(handle) = pick else { return Ok(None) };
        let path = handle.path().to_path_buf();
        let g = audit.lock().map_err(|_| AppError::Lock)?;
        g.save_to_file(&path)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    }
}

#[tauri::command]
pub async fn ai_audit_get(
    state: State<'_, AppState>,
    tab_id: String,
) -> AppResult<super::audit::AuditLog> {
    let audit = locked(&state.ai_sessions)?
        .get(&tab_id)
        .map(|s| s.audit.clone())
        .ok_or_else(|| AppError::not_found("ai_session_not_found", json!({})))?;
    let g = audit.lock().map_err(|_| AppError::Lock)?;
    Ok(g.clone())
}

#[tauri::command]
pub async fn ai_list_sessions(state: State<'_, AppState>) -> AppResult<Vec<AiSessionInfo>> {
    let g = locked(&state.ai_sessions)?;
    Ok(g.values().map(AiSessionInfo::from).collect())
}

// ─── 设置（BYOK） ──────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
pub struct AiSettings {
    pub provider: String,
    pub model: String,
    pub endpoint: Option<String>,
    pub has_api_key: bool,
    /// 危险模式总闸。off 时下面所有 auto_* 视同 false（per-tool 设置仍持久化，
    /// 但运行时不生效，方便用户切回 danger 时复原选择）。
    pub danger_mode: bool,
    /// per-tool 自动批准。命名直接映射到前端 CommandProposed.kind。
    pub auto_run_command: bool,
    pub auto_match_file: bool,
    pub auto_download_file: bool,
    pub auto_analyze_locally: bool,
    pub auto_patch_cp: bool,
    pub auto_patch_modify: bool,
    pub auto_patch_diff: bool,
    pub auto_patch_mv: bool,
    /// 远端 shell 自动探测：off 时远端假设 POSIX；on 时 AI panel 打开后跑探针。
    /// 详见 `key_auto_detect_remote_shell` 注释。
    pub auto_detect_remote_shell: bool,
}

/// per-tool auto-approve 字段的默认值 —— 在 ai_settings_get / ai_settings_set 间共享。
/// run_command + match_file 默认开（向后兼容旧 danger_mode 全开行为），其余默认关。
fn auto_default(name: &str) -> bool {
    matches!(name, "run_command" | "match_file")
}

fn read_auto(state: &AppState, name: &str) -> AppResult<bool> {
    Ok(crate::db::settings::get(&state.db, &key_auto(name))?
        .map(|v| v == "1")
        .unwrap_or_else(|| auto_default(name)))
}

/// `provider` 入参：传 `Some(p)` → 拉该 provider 的快照（不改 active）；
/// `None` → 拉当前 active provider 的快照。无任何兜底默认值，未存就是空。
#[tauri::command]
pub async fn ai_settings_get(
    state: State<'_, AppState>,
    provider: Option<String>,
) -> AppResult<AiSettings> {
    ai_settings_get_impl(&state, provider).await
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub async fn ai_settings_get_impl(
    state: &AppState,
    provider: Option<String>,
) -> AppResult<AiSettings> {
    let provider = match provider.filter(|s| !s.is_empty()) {
        Some(p) => p,
        None => crate::db::settings::get(&state.db, &key_provider())?
            .unwrap_or_else(|| "anthropic".into()),
    };
    let model = crate::db::settings::get(&state.db, &key_model(&provider))?.unwrap_or_default();
    let endpoint = crate::db::settings::get(&state.db, &key_endpoint(&provider))?;
    // API key 仍走 keychain
    let has_api_key = state
        .secret_store
        .get(&key_api_key(&provider))?
        .filter(|s| !s.is_empty())
        .is_some();
    let danger_mode = crate::db::settings::get(&state.db, &key_danger_mode())?
        .map(|v| v == "1")
        .unwrap_or(false);
    let auto_detect_remote_shell =
        crate::db::settings::get(&state.db, &key_auto_detect_remote_shell())?
            .map(|v| v == "1")
            .unwrap_or(false);
    Ok(AiSettings {
        provider,
        model,
        endpoint,
        has_api_key,
        danger_mode,
        auto_run_command: read_auto(state, "run_command")?,
        auto_match_file: read_auto(state, "match_file")?,
        auto_download_file: read_auto(state, "download_file")?,
        auto_analyze_locally: read_auto(state, "analyze_locally")?,
        auto_patch_cp: read_auto(state, "patch_cp")?,
        auto_patch_modify: read_auto(state, "patch_modify")?,
        auto_patch_diff: read_auto(state, "patch_diff")?,
        auto_patch_mv: read_auto(state, "patch_mv")?,
        auto_detect_remote_shell,
    })
}

/// 拉取指定 provider 的模型列表。
///
/// 优先用入参 `api_key` / `endpoint`（"试一下再保存"流程）；缺省时回落到
/// secret_store 里已保存的值。GLM 这种官方不开放 `/models` 的厂商，会返回
/// 硬编码白名单（见 `llm::glm`）。
#[tauri::command]
pub async fn ai_list_models(
    state: State<'_, AppState>,
    provider: String,
    api_key: Option<String>,
    endpoint: Option<String>,
) -> AppResult<Vec<llm::ModelInfo>> {
    ai_list_models_impl(&state, provider, api_key, endpoint).await
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub async fn ai_list_models_impl(
    state: &AppState,
    provider: String,
    api_key: Option<String>,
    endpoint: Option<String>,
) -> AppResult<Vec<llm::ModelInfo>> {
    // 入参先 trim：纯空白当作"未提供"，回落到 secret_store
    let api_key = match api_key.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(k) => k,
        None => state
            .secret_store
            .get(&key_api_key(&provider))?
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AppError::config("api_key_missing", json!({ "provider": provider })))?,
    };
    let endpoint = endpoint
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| crate::db::settings::get(&state.db, &key_endpoint(&provider)).ok().flatten());
    let client = llm::build_client(&provider, api_key, endpoint)?;
    client.list_models().await
}

/// A partial update of the AI settings: each field is "set this if `Some`, leave
/// untouched if `None`". Replaces the 14 positional `Option<_>` arguments that
/// `ai_settings_set` / `_impl` previously carried (and forwarded twice). Field
/// names match the frontend's camelCase wire shape.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsPatch {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub danger_mode: Option<bool>,
    pub auto_run_command: Option<bool>,
    pub auto_match_file: Option<bool>,
    pub auto_download_file: Option<bool>,
    pub auto_analyze_locally: Option<bool>,
    pub auto_patch_cp: Option<bool>,
    pub auto_patch_modify: Option<bool>,
    pub auto_patch_diff: Option<bool>,
    pub auto_patch_mv: Option<bool>,
    pub auto_detect_remote_shell: Option<bool>,
}

#[tauri::command]
pub async fn ai_settings_set(state: State<'_, AppState>, patch: AiSettingsPatch) -> AppResult<()> {
    ai_settings_set_impl(&state, patch).await
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub async fn ai_settings_set_impl(state: &AppState, patch: AiSettingsPatch) -> AppResult<()> {
    let AiSettingsPatch {
        provider,
        model,
        endpoint,
        api_key,
        danger_mode,
        auto_run_command,
        auto_match_file,
        auto_download_file,
        auto_analyze_locally,
        auto_patch_cp,
        auto_patch_modify,
        auto_patch_diff,
        auto_patch_mv,
        auto_detect_remote_shell,
    } = patch;
    if let Some(p) = provider.as_ref() {
        crate::db::settings::set(&state.db, &key_provider(), p)?;
    }
    let active_provider = provider
        .clone()
        .or_else(|| crate::db::settings::get(&state.db, &key_provider()).ok().flatten())
        .unwrap_or_else(|| "anthropic".into());
    if let Some(m) = model.as_ref() {
        crate::db::settings::set(&state.db, &key_model(&active_provider), m)?;
    }
    if let Some(e) = endpoint {
        crate::db::settings::set(&state.db, &key_endpoint(&active_provider), &e)?;
    }
    // API key 仍走 keychain；空串语义保留（用 delete 抹掉而不是存空）
    if let Some(k) = api_key {
        if k.is_empty() {
            state.secret_store.delete(&key_api_key(&active_provider))?;
        } else {
            state.secret_store.set(&key_api_key(&active_provider), &k)?;
        }
    }
    if let Some(on) = danger_mode {
        // 用 "1"/"0" 而不是 delete on false——显式记录用户的"我关了"，
        // 与"从未设置过"区分开，后续审计/排错更直接。
        crate::db::settings::set(&state.db, &key_danger_mode(), if on { "1" } else { "0" })?;
    }
    // per-tool 自动批准。同 danger_mode 的存储约定（"1"/"0"），None 不动。
    let auto_writes: &[(&str, Option<bool>)] = &[
        ("run_command", auto_run_command),
        ("match_file", auto_match_file),
        ("download_file", auto_download_file),
        ("analyze_locally", auto_analyze_locally),
        ("patch_cp", auto_patch_cp),
        ("patch_modify", auto_patch_modify),
        ("patch_diff", auto_patch_diff),
        ("patch_mv", auto_patch_mv),
    ];
    for (name, val) in auto_writes {
        if let Some(on) = val {
            crate::db::settings::set(&state.db, &key_auto(name), if *on { "1" } else { "0" })?;
        }
    }
    if let Some(on) = auto_detect_remote_shell {
        crate::db::settings::set(
            &state.db,
            &key_auto_detect_remote_shell(),
            if on { "1" } else { "0" },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AiSettingsPatch, AiTarget};
    use serde_json::json;

    #[test]
    fn patch_deserializes_camelcase_wire_shape() {
        // Frontend sends camelCase keys; the headless server deserializes the same
        // object. Pin that contract so a field rename can't silently break it.
        let p: AiSettingsPatch = serde_json::from_value(json!({
            "provider": "anthropic",
            "apiKey": "sk-x",
            "dangerMode": true,
            "autoRunCommand": false,
            "autoDetectRemoteShell": true,
        }))
        .unwrap();
        assert_eq!(p.provider.as_deref(), Some("anthropic"));
        assert_eq!(p.api_key.as_deref(), Some("sk-x"));
        assert_eq!(p.danger_mode, Some(true));
        assert_eq!(p.auto_run_command, Some(false));
        assert_eq!(p.auto_detect_remote_shell, Some(true));
    }

    #[test]
    fn patch_absent_fields_are_none() {
        // A partial update touches only the present keys; everything else stays None
        // so the impl leaves those settings untouched.
        let p: AiSettingsPatch = serde_json::from_value(json!({ "model": "claude" })).unwrap();
        assert_eq!(p.model.as_deref(), Some("claude"));
        assert!(p.provider.is_none());
        assert!(p.danger_mode.is_none());
        assert!(p.auto_patch_mv.is_none());
    }

    #[test]
    fn ai_target_deserializes_from_kind_id_wire() {
        // Frontend (and headless ws) send `{ kind, id }`; pin both variants.
        let ssh: AiTarget = serde_json::from_value(json!({ "kind": "ssh", "id": "s1" })).unwrap();
        assert!(matches!(ssh, AiTarget::Ssh(ref id) if id == "s1"));
        assert_eq!(ssh.id(), "s1");

        let local: AiTarget =
            serde_json::from_value(json!({ "kind": "local", "id": "p9" })).unwrap();
        assert!(matches!(local, AiTarget::Local(ref id) if id == "p9"));
        assert_eq!(local.id(), "p9");
    }

    #[test]
    fn ai_target_rejects_unknown_kind() {
        // The old `_ => unknown_target_kind` runtime branch is now a parse error —
        // an invalid kind can't even be constructed.
        let bad = serde_json::from_value::<AiTarget>(json!({ "kind": "wat", "id": "x" }));
        assert!(bad.is_err());
    }
}
