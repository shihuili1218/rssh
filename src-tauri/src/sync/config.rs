//! Config sync: incremental merge (upsert by identity), never destructive.
//!
//! **merge_import** is the single import entry point for every path — file
//! `import` / `rssh config import` AND `github_pull` / `webdav_pull` /
//! `rssh config github pull` / `rssh config webdav pull`.
//! It does NOT clear local data: each entity is upserted by its identity key,
//! local-only entities survive, and a delete on one device is never propagated
//! to another (additive semantics — the deliberate trade-off chosen for sync).
//! Per-row failures are collected and reported together, never aborting the
//! rest of the import.
//!
//! Credential `secret` of None or empty is treated as "keep local". Telnet
//! login scripts carry an explicit upload-policy bit: empty + disabled means
//! preserve (scrubbed), while empty + enabled means propagate an intentional
//! script clear.
//!
//! Accepts a `serde_json::Value` of the shape:
//! ```json
//! { "version": 1, "profiles": [..], "credentials": [..],
//!   "forwards": [..], "serial_profiles": [..], "telnet_profiles": [..],
//!   "groups": [..], "skills": [..] }
//! ```
//! A missing top-level key means "that category was not synced" → the
//! corresponding local table is left untouched.

use std::path::Path;

use serde_json::{json, Value};

use crate::db::ai_command_blacklist::{self, BlacklistRow};
use crate::db::ai_redact_rule::{self, RedactRuleRow};
use crate::db::{
    credential, forward, group, highlight, profile, serial_profile, snippet, telnet_profile, Db,
};
use crate::error::{AppError, AppResult};
use crate::models::{
    Credential, DynamicDiscoverySource, Forward, Group, HighlightRule, Profile, SerialProfile,
    Snippet, TelnetProfile,
};
use crate::secret::{cred_secret_key, SecretStore};
use crate::telnet_profile::{self as telnet_profiles, LoginScriptIntent};

/// Structured record of a failed item. `aggregate_failure` serializes the whole
/// Vec into AppError params so the frontend can render every failure at once,
/// instead of the user retrying repeatedly to discover them one by one.
#[derive(Debug, Clone)]
pub struct ImportError {
    pub kind: &'static str,
    pub name: Option<String>,
    pub code: String,
}

fn aggregate_failure(errs: Vec<ImportError>) -> AppError {
    let count = errs.len();
    let details = errs
        .iter()
        .map(|e| {
            let name = e.name.as_deref().unwrap_or("?");
            format!("• {} '{}' ({})", e.kind, name, e.code)
        })
        .collect::<Vec<_>>()
        .join("\n");
    AppError::other(
        "import_partial_failed",
        json!({
            "count": count,
            "details": details,
        }),
    )
}

// ---------------------------------------------------------------------------
// merge_import — incremental, additive, never destructive
// ---------------------------------------------------------------------------

/// Upsert every entity by identity. Does not clear local data; a single item's
/// failure does not abort the others. `data_dir` is the app data directory,
/// used by file-backed categories (snippets) that live outside the DB.
pub fn merge_import(db: &Db, ss: &dyn SecretStore, data_dir: &Path, data: &Value) -> AppResult<()> {
    let mut errors: Vec<ImportError> = Vec::new();

    if let Some(arr) = data["credentials"].as_array() {
        for item in arr {
            match serde_json::from_value::<Credential>(item.clone()) {
                Ok(c) => {
                    if let Err(e) = credential::insert(db, &c) {
                        errors.push(ImportError {
                            kind: "credential",
                            name: Some(c.name.clone()),
                            code: e.code().to_string(),
                        });
                        continue;
                    }
                    // merge semantics: only write the secret when import carries
                    // a non-empty one; otherwise keep the local secret (avoid a
                    // scrubbed-on-push secret overwriting it back to None).
                    if let Some(s) = c.secret.as_deref().filter(|s| !s.is_empty()) {
                        if let Err(e) = ss.set(&cred_secret_key(&c.id), s) {
                            errors.push(ImportError {
                                kind: "credential_secret",
                                name: Some(c.name),
                                code: e.code().to_string(),
                            });
                        }
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "credential",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    if let Some(arr) = data["profiles"].as_array() {
        for item in arr {
            match serde_json::from_value::<Profile>(item.clone()) {
                Ok(p) => {
                    if let Err(e) = profile::insert(db, &p) {
                        errors.push(ImportError {
                            kind: "profile",
                            name: Some(p.name),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "profile",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    if let Some(arr) = data["forwards"].as_array() {
        for item in arr {
            match serde_json::from_value::<Forward>(item.clone()) {
                Ok(f) => {
                    if let Err(e) = forward::insert(db, &f) {
                        errors.push(ImportError {
                            kind: "forward",
                            name: Some(f.name),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "forward",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    if let Some(arr) = data["serial_profiles"].as_array() {
        for item in arr {
            match serde_json::from_value::<SerialProfile>(item.clone()) {
                Ok(s) => {
                    if let Err(e) = serial_profile::insert(db, &s) {
                        errors.push(ImportError {
                            kind: "serial_profile",
                            name: Some(s.name),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "serial_profile",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    if let Some(arr) = data["telnet_profiles"].as_array() {
        for item in arr {
            match serde_json::from_value::<TelnetProfile>(item.clone()) {
                Ok(t) => {
                    let update = if !t.login_script.is_empty() {
                        LoginScriptIntent::Set(t.login_script.clone())
                    } else if t.save_script_to_remote {
                        // When upload is explicitly enabled, an empty script is
                        // an intentional clear. With upload disabled it is only
                        // a scrubbed placeholder and must preserve local state.
                        LoginScriptIntent::Delete
                    } else {
                        LoginScriptIntent::Preserve
                    };
                    if let Err(e) = telnet_profiles::upsert(db, ss, &t, update) {
                        errors.push(ImportError {
                            kind: "telnet_profile",
                            name: Some(t.name.clone()),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "telnet_profile",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    if let Some(arr) = data["groups"].as_array() {
        for item in arr {
            match serde_json::from_value::<Group>(item.clone()) {
                Ok(g) => {
                    if let Err(e) = group::insert(db, &g) {
                        errors.push(ImportError {
                            kind: "group",
                            name: Some(g.name),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "group",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    // skills: upsert by id; merge never clears local (even if payload has skills:[]).
    if let Some(arr) = data
        .get("skills")
        .filter(|v| !v.is_null())
        .and_then(Value::as_array)
    {
        for item in arr {
            match parse_skill(item) {
                Ok(Some(s)) => {
                    if let Err(e) = crate::db::ai_skill::upsert(db, &s) {
                        errors.push(ImportError {
                            kind: "skill",
                            name: Some(s.id),
                            code: e.code().to_string(),
                        });
                    }
                }
                Ok(None) => {} // builtin skip
                Err(_) => errors.push(ImportError {
                    kind: "skill",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    // highlights — identity = keyword (the local autoincrement id is not synced)
    if let Some(arr) = data["highlights"].as_array() {
        for item in arr {
            match serde_json::from_value::<HighlightRule>(item.clone()) {
                Ok(h) => {
                    if let Err(e) = highlight::upsert_by_keyword(db, &h) {
                        errors.push(ImportError {
                            kind: "highlight",
                            name: Some(h.keyword),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "highlight",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    // ai_redact_rules — identity = id, upsert
    if let Some(arr) = data["ai_redact_rules"].as_array() {
        for item in arr {
            match serde_json::from_value::<RedactRuleRow>(item.clone()) {
                Ok(r) => {
                    // Validating save (compilable + non-zero-width), never the raw
                    // upsert: a bad synced regex would otherwise persist and brick
                    // AI sessions on this device (compiled() is fail-closed). An
                    // invalid rule is rejected into `errors`, not written.
                    let rec = crate::ai::redact_rules::RedactRuleRecord {
                        id: r.id.clone(),
                        pattern: r.pattern,
                        replacement: r.replacement,
                    };
                    if let Err(e) = crate::ai::redact_rules::save(db, &rec) {
                        errors.push(ImportError {
                            kind: "ai_redact_rule",
                            name: Some(rec.id),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "ai_redact_rule",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    // ai_command_blacklist — identity = name; additive upsert (never deletes)
    if let Some(arr) = data["ai_command_blacklist"].as_array() {
        for item in arr {
            match serde_json::from_value::<BlacklistRow>(item.clone()) {
                Ok(b) => {
                    if let Err(e) = ai_command_blacklist::upsert(db, &b) {
                        errors.push(ImportError {
                            kind: "ai_command_blacklist",
                            name: Some(b.name),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "ai_command_blacklist",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    // snippets — identity = name; file-backed, merged outside the DB
    if let Some(arr) = data["snippets"].as_array() {
        let parsed: Result<Vec<Snippet>, _> = arr
            .iter()
            .map(|i| serde_json::from_value::<Snippet>(i.clone()))
            .collect();
        match parsed {
            Ok(snips) => {
                if let Err(e) = snippet::merge_by_name(data_dir, &snips) {
                    errors.push(ImportError {
                        kind: "snippet",
                        name: None,
                        code: e.code().to_string(),
                    });
                }
            }
            Err(_) => errors.push(ImportError {
                kind: "snippet",
                name: None,
                code: "parse_failed".into(),
            }),
        }
    }
    // dynamic_discovery_sources — identity = id; settings-backed, merged
    // additively so a remote delete does not wipe local source config.
    if let Some(v) = data
        .get("dynamic_discovery_sources")
        .filter(|v| !v.is_null())
    {
        match serde_json::from_value::<Vec<DynamicDiscoverySource>>(v.clone()) {
            Ok(incoming) => {
                if let Err(e) = merge_and_save_dynamic_discovery_sources(db, incoming) {
                    errors.push(ImportError {
                        kind: "dynamic_discovery_source",
                        name: None,
                        code: e.code().to_string(),
                    });
                }
            }
            Err(_) => errors.push(ImportError {
                kind: "dynamic_discovery_source",
                name: None,
                code: "parse_failed".into(),
            }),
        }
    }
    // ai — provider settings (an object, not a list of rows)
    if let Some(ai) = data.get("ai").filter(|v| !v.is_null()) {
        if let Err(e) = crate::ai::commands::import_ai_settings(db, ss, ai) {
            errors.push(ImportError {
                kind: "ai_settings",
                name: None,
                code: e.code().to_string(),
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(aggregate_failure(errors))
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn parse_skill(item: &Value) -> AppResult<Option<crate::db::ai_skill::UserSkill>> {
    use crate::ai::skills::SkillRecord;
    let s: SkillRecord = serde_json::from_value(item.clone()).map_err(|e| {
        AppError::config(
            "import_parse_failed",
            json!({ "field": "skills", "err": e.to_string() }),
        )
    })?;
    if s.builtin {
        return Ok(None);
    }
    Ok(Some(crate::db::ai_skill::UserSkill {
        id: s.id,
        name: s.name,
        description: s.description,
        content: s.content,
    }))
}

fn merge_dynamic_discovery_sources(
    mut local: Vec<DynamicDiscoverySource>,
    incoming: Vec<DynamicDiscoverySource>,
) -> Vec<DynamicDiscoverySource> {
    for source in incoming {
        if let Some(existing) = local.iter_mut().find(|it| it.id == source.id) {
            *existing = source;
        } else {
            local.push(source);
        }
    }
    local
}

fn merge_and_save_dynamic_discovery_sources(
    db: &Db,
    incoming: Vec<DynamicDiscoverySource>,
) -> AppResult<()> {
    let local = crate::commands::discovery::read_dynamic_discovery_sources_from_db(db)?;
    let merged = merge_dynamic_discovery_sources(local, incoming);
    crate::commands::discovery::save_dynamic_discovery_sources_to_db(db, merged)
}

// ---------------------------------------------------------------------------
// Export — the counterpart to merge_import. One builder for every transport
// (GUI commands + headless server + `rssh` CLI) so the on-disk/on-GitHub shape
// can never drift between them.
// ---------------------------------------------------------------------------

/// Per-category sync toggles + the profile group filter. All booleans default
/// to ON (absent setting = included) so turning on sync keeps today's
/// "sync everything" behavior; the user opts OUT per category.
#[derive(Debug)]
pub struct SyncPrefs {
    skills: bool,
    highlights: bool,
    snippets: bool,
    ai_redact: bool,
    ai_blacklist: bool,
    ai: bool,
    /// Group filter shared by profiles / forwards / serial_profiles.
    /// `None` = everything (the "all groups" sentinel). `Some(ids)` = only rows
    /// whose group is in the set; the "" id matches ungrouped rows.
    profile_group_ids: Option<Vec<String>>,
}

/// What flavor of payload to build.
pub enum ExportMode {
    /// Full local backup: every category, every secret, no toggles.
    LocalBackup,
    /// Remote push (GitHub / WebDAV): apply per-category toggles + group filter;
    /// scrub the secret of credentials flagged local-only.
    RemotePush(SyncPrefs),
}

/// Read the per-category toggles + profile group filter from settings. The GUI
/// and `rssh config github push` / `rssh config webdav push` both feed the
/// result into `build_payload` so the same opt-outs apply no matter which
/// remote transport pushes to the shared repo / WebDAV endpoint.
pub fn read_sync_prefs(db: &Db) -> AppResult<SyncPrefs> {
    // Absent or any value other than "0" → on. Only an explicit "0" disables.
    let flag = |key: &str| -> AppResult<bool> {
        Ok(crate::db::settings::get(db, key)?.is_none_or(|v| v != "0"))
    };
    // Empty string / absent → None → all profiles (incl. ungrouped); this is
    // the "all groups selected" default. A JSON array → that exact set
    // (an empty array means sync no profiles). Malformed → error, never None:
    // silently falling back to None would widen a deliberately-narrowed export
    // back to every profile (a privacy leak), which is fail-OPEN, not safe.
    let profile_group_ids = match crate::db::settings::get(db, "sync_profile_group_ids")? {
        Some(s) if !s.trim().is_empty() => {
            Some(serde_json::from_str::<Vec<String>>(&s).map_err(|e| {
                AppError::config(
                    "sync_profile_group_ids_invalid",
                    json!({ "err": e.to_string() }),
                )
            })?)
        }
        _ => None,
    };
    Ok(SyncPrefs {
        skills: flag("sync_include_skills")?,
        highlights: flag("sync_include_highlights")?,
        ai_redact: flag("sync_include_ai_redact")?,
        ai_blacklist: flag("sync_include_ai_blacklist")?,
        snippets: flag("sync_include_snippets")?,
        ai: flag("sync_include_ai")?,
        profile_group_ids,
    })
}

fn to_val<T: serde::Serialize>(v: T) -> AppResult<Value> {
    serde_json::to_value(v)
        .map_err(|e| AppError::other("serde_failed", json!({ "err": e.to_string() })))
}

/// Apply the push-time group filter to any grouped category. `None` prefs (local
/// backup) or a `None` group filter (the "all groups" sentinel) keep everything;
/// `Some(ids)` retains only rows whose group is in the set — ungrouped rows
/// (group_id = None) drop out, exactly like profiles. Shared by profiles /
/// forwards / serial_profiles so the three filters can't drift apart.
fn retain_by_groups<T>(
    items: &mut Vec<T>,
    prefs: Option<&SyncPrefs>,
    group_of: impl Fn(&T) -> Option<&str>,
) {
    let Some(gids) = prefs.and_then(|p| p.profile_group_ids.as_ref()) else {
        return;
    };
    let set: std::collections::HashSet<&str> = gids.iter().map(String::as_str).collect();
    // An ungrouped row (group_id = None) keys to "" — selecting the "" sentinel
    // (the UI's "Ungrouped" chip) includes it. A real group id is never empty.
    items.retain(|it| set.contains(group_of(it).unwrap_or("")));
}

fn collect_credentials_with_secrets(
    db: &Db,
    ss: &dyn SecretStore,
) -> AppResult<Vec<crate::models::Credential>> {
    let mut creds = credential::list(db)?;
    for c in creds.iter_mut() {
        c.secret = ss.get(&cred_secret_key(&c.id))?;
    }
    Ok(creds)
}

/// Build the export payload as a JSON value — the single source of truth for
/// the sync shape, shared by local export AND remote push (GUI *and* CLI) so the
/// JSON can't drift between them. On push, a disabled category's key is simply
/// omitted (absence = "not synced"); merge_import then leaves that local table
/// alone. `data_dir` feeds the file-backed `snippets` category.
pub fn build_payload(
    db: &Db,
    ss: &dyn SecretStore,
    data_dir: &Path,
    mode: &ExportMode,
) -> AppResult<Value> {
    let prefs = match mode {
        ExportMode::RemotePush(p) => Some(p),
        ExportMode::LocalBackup => None,
    };
    let on = |pick: fn(&SyncPrefs) -> bool| prefs.is_none_or(pick);

    let mut out = serde_json::Map::new();
    out.insert("version".into(), json!(1));
    out.insert("exported_at".into(), json!(chrono::Utc::now().to_rfc3339()));

    // profiles / forwards / serial_profiles — always present, filtered to the
    // selected groups on push. The forwards/serial per-category opt-out toggles
    // were removed: "sync everything" is now expressed as "select all groups".
    let mut profiles = profile::list(db)?;
    retain_by_groups(&mut profiles, prefs, |p| p.group_id.as_deref());
    out.insert("profiles".into(), to_val(profiles)?);

    // credentials + groups are the referential closure of the always-exported
    // profiles/forwards/serial (profile→credential_id, *→group_id). Gating them
    // behind their own toggle would leave dangling refs on the other device, so
    // both always ride along. Secret upload stays gated per-credential by
    // save_to_remote — orthogonal to (and unaffected by) the removed toggle.
    let mut credentials = collect_credentials_with_secrets(db, ss)?;
    if prefs.is_some() {
        for c in credentials.iter_mut() {
            if !c.save_to_remote {
                c.secret = None;
            }
        }
    }
    out.insert("credentials".into(), to_val(credentials)?);
    let mut forwards = forward::list(db)?;
    retain_by_groups(&mut forwards, prefs, |f| f.group_id.as_deref());
    out.insert("forwards".into(), to_val(forwards)?);
    out.insert("groups".into(), to_val(group::list(db)?)?);
    let mut serials = serial_profile::list(db)?;
    retain_by_groups(&mut serials, prefs, |s| s.group_id.as_deref());
    out.insert("serial_profiles".into(), to_val(serials)?);
    let mut telnets = telnet_profile::list(db)?;
    retain_by_groups(&mut telnets, prefs, |t| t.group_id.as_deref());
    for telnet in &mut telnets {
        if prefs.is_some() && !telnet.save_script_to_remote {
            telnet_profiles::reconcile_legacy_plaintext(db, ss, &telnet.id)?;
            telnet.login_script.clear();
        } else {
            telnet_profiles::hydrate(db, ss, telnet)?;
        }
    }
    out.insert("telnet_profiles".into(), to_val(telnets)?);
    out.insert(
        "dynamic_discovery_sources".into(),
        to_val(crate::commands::discovery::read_dynamic_discovery_sources_from_db(db)?)?,
    );
    if on(|p| p.skills) {
        out.insert("skills".into(), to_val(crate::ai::skills::list_user(db)?)?);
    }
    if on(|p| p.highlights) {
        out.insert("highlights".into(), to_val(highlight::list(db)?)?);
    }
    if on(|p| p.snippets) {
        out.insert("snippets".into(), to_val(snippet::load(data_dir)?)?);
    }
    if on(|p| p.ai_redact) {
        out.insert("ai_redact_rules".into(), to_val(ai_redact_rule::list(db)?)?);
    }
    if on(|p| p.ai_blacklist) {
        out.insert(
            "ai_command_blacklist".into(),
            to_val(ai_command_blacklist::list(db)?)?,
        );
    }
    // "AI 配置" — one toggle for the whole AI section: active provider + each
    // provider's model/endpoint AND api_key, all-or-nothing. The key rides in the
    // payload as plaintext, then the whole payload is encrypted with the sync
    // password before upload, so the remote never sees it in clear.
    if on(|p| p.ai) {
        out.insert(
            "ai".into(),
            crate::ai::commands::export_ai_settings(db, ss, true)?,
        );
    }

    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CredentialType;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-process SecretStore for tests. No keychain, no DB table.
    #[derive(Default)]
    struct MemStore {
        inner: Mutex<HashMap<String, String>>,
    }
    impl SecretStore for MemStore {
        fn get(&self, key: &str) -> AppResult<Option<String>> {
            Ok(self.inner.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> AppResult<()> {
            self.inner
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, key: &str) -> AppResult<()> {
            self.inner.lock().unwrap().remove(key);
            Ok(())
        }
        fn backend_name(&self) -> &'static str {
            "mem"
        }
    }

    fn fixture() -> (Db, MemStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory().unwrap();
        (db, MemStore::default(), dir)
    }

    fn cred(id: &str, name: &str, secret: Option<&str>) -> Credential {
        Credential {
            id: id.into(),
            name: name.into(),
            username: "u".into(),
            credential_type: CredentialType::Password,
            secret: secret.map(String::from),
            save_to_remote: true,
        }
    }

    fn prof(id: &str, name: &str, cred_id: &str) -> Profile {
        Profile {
            id: id.into(),
            name: name.into(),
            host: "h.example".into(),
            port: 22,
            credential_id: cred_id.into(),
            bastion_profile_id: None,
            init_command: None,
            group_id: None,
            algorithms: Default::default(),
        }
    }

    fn serial(id: &str, name: &str) -> SerialProfile {
        SerialProfile {
            id: id.into(),
            name: name.into(),
            port: "/dev/ttyUSB0".into(),
            baud_rate: 115200,
            data_bits: 8,
            parity: "none".into(),
            stop_bits: 1,
            flow_control: "none".into(),
            xany: false,
            input_newline: "cr".into(),
            output_newline: "raw".into(),
            local_echo: false,
            backspace: "del".into(),
            slow_send: false,
            input_mode: "normal".into(),
            output_mode: "text".into(),
            login_script: String::new(),
            group_id: None,
        }
    }

    fn payload(v: Value) -> Value {
        v
    }

    #[test]
    fn merge_keeps_local_only_rows() {
        let (db, ss, dir) = fixture();
        // local-only credential + profile
        credential::insert(&db, &cred("local", "Local", Some("p"))).unwrap();
        profile::insert(&db, &prof("plocal", "PLocal", "local")).unwrap();

        // payload brings a different credential/profile
        let data = payload(json!({
            "version": 1,
            "credentials": [serde_json::to_value(cred("remote", "Remote", Some("q"))).unwrap()],
            "profiles": [serde_json::to_value(prof("premote", "PRemote", "remote")).unwrap()],
        }));
        merge_import(&db, &ss, dir.path(), &data).unwrap();

        let creds = credential::list(&db).unwrap();
        assert!(creds.iter().any(|c| c.id == "local"), "local survives");
        assert!(creds.iter().any(|c| c.id == "remote"), "remote added");
        let profs = profile::list(&db).unwrap();
        assert_eq!(profs.len(), 2);
    }

    #[test]
    fn merge_defaults_null_profile_algorithms() {
        let (db, ss, dir) = fixture();
        let data = payload(json!({
            "version": 1,
            "profiles": [{
                "id": "p1",
                "name": "P1",
                "host": "h.example",
                "port": 22,
                "credential_id": "c1",
                "bastion_profile_id": null,
                "init_command": null,
                "group_id": null,
                "algorithms": null
            }]
        }));

        merge_import(&db, &ss, dir.path(), &data).unwrap();

        let got = profile::get(&db, "p1").unwrap();
        assert_eq!(got.algorithms, crate::models::SshAlgorithms::default());
    }

    #[test]
    fn export_includes_profile_algorithms() {
        let (db, ss, dir) = fixture();
        let mut p = prof("p1", "P1", "c1");
        p.algorithms.kex.push("diffie-hellman-group1-sha1".into());
        profile::insert(&db, &p).unwrap();

        let out = build_payload(&db, &ss, dir.path(), &ExportMode::LocalBackup).unwrap();
        let algorithms = &out["profiles"][0]["algorithms"];
        assert!(algorithms.is_object());
        assert!(algorithms["kex"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "diffie-hellman-group1-sha1"));
    }

    #[test]
    fn merge_overwrites_same_id() {
        let (db, ss, dir) = fixture();
        credential::insert(&db, &cred("c1", "Old", Some("old"))).unwrap();

        let data = payload(json!({
            "version": 1,
            "credentials": [serde_json::to_value(cred("c1", "New", Some("new"))).unwrap()],
        }));
        merge_import(&db, &ss, dir.path(), &data).unwrap();

        let creds = credential::list(&db).unwrap();
        let c1 = creds.iter().find(|c| c.id == "c1").unwrap();
        assert_eq!(c1.name, "New", "same id overwritten");
        assert_eq!(
            ss.get(&cred_secret_key("c1")).unwrap().as_deref(),
            Some("new"),
            "secret overwritten"
        );
    }

    #[test]
    fn merge_does_not_propagate_delete() {
        // device A has p1+p2; remote (device B) deleted p2 → payload only has p1.
        // After merge, p2 must STILL exist locally (additive, no delete).
        let (db, ss, dir) = fixture();
        credential::insert(&db, &cred("c1", "C1", Some("s"))).unwrap();
        profile::insert(&db, &prof("p1", "P1", "c1")).unwrap();
        profile::insert(&db, &prof("p2", "P2", "c1")).unwrap();

        let data = payload(json!({
            "version": 1,
            "profiles": [serde_json::to_value(prof("p1", "P1", "c1")).unwrap()],
        }));
        merge_import(&db, &ss, dir.path(), &data).unwrap();

        let profs = profile::list(&db).unwrap();
        assert!(profs.iter().any(|p| p.id == "p2"), "delete not propagated");
    }

    #[test]
    fn merge_empty_secret_keeps_local() {
        // push scrubbed the secret to None; pulling back must keep local secret.
        let (db, ss, dir) = fixture();
        credential::insert(&db, &cred("c1", "C1", Some("local-secret"))).unwrap();
        ss.set(&cred_secret_key("c1"), "local-secret").unwrap();

        let data = payload(json!({
            "version": 1,
            "credentials": [serde_json::to_value(cred("c1", "C1", None)).unwrap()],
        }));
        merge_import(&db, &ss, dir.path(), &data).unwrap();

        assert_eq!(
            ss.get(&cred_secret_key("c1")).unwrap().as_deref(),
            Some("local-secret"),
            "scrubbed secret did not overwrite local"
        );
    }

    #[test]
    fn merge_missing_serial_key_keeps_local() {
        // payload without a serial_profiles key must not touch local serial rows.
        let (db, ss, dir) = fixture();
        serial_profile::insert(&db, &serial("s1", "Board")).unwrap();

        let data = payload(json!({ "version": 1, "profiles": [] }));
        merge_import(&db, &ss, dir.path(), &data).unwrap();

        let serials = serial_profile::list(&db).unwrap();
        assert!(serials.iter().any(|s| s.id == "s1"), "local serial kept");
    }

    fn telnet(id: &str, name: &str) -> TelnetProfile {
        TelnetProfile {
            id: id.into(),
            name: name.into(),
            host: "10.0.0.1".into(),
            port: 23,
            input_newline: "crlf".into(),
            output_newline: "raw".into(),
            local_echo: false,
            echo_mode: Some(crate::models::TelnetEchoMode::Auto),
            backspace: "del".into(),
            login_script: String::new(),
            save_script_to_remote: false,
            group_id: None,
        }
    }

    fn insert_telnet_with_script(
        db: &Db,
        ss: &dyn SecretStore,
        mut profile: TelnetProfile,
        script: &str,
    ) {
        profile.login_script = script.into();
        telnet_profiles::upsert(db, ss, &profile, LoginScriptIntent::Set(script.into())).unwrap();
    }

    fn stored_telnet_script(db: &Db, ss: &dyn SecretStore, id: &str) -> String {
        let mut profile = telnet_profile::get(db, id).unwrap();
        telnet_profiles::hydrate(db, ss, &mut profile).unwrap();
        profile.login_script
    }

    fn docker_source(id: &str, name: &str, context: &str) -> DynamicDiscoverySource {
        DynamicDiscoverySource {
            id: id.into(),
            name: name.into(),
            enabled: true,
            config: crate::models::DynamicDiscoveryConfig::Docker {
                context: context.into(),
                shell: "sh".into(),
            },
        }
    }

    #[test]
    fn merge_missing_telnet_key_keeps_local() {
        // payload without a telnet_profiles key must not touch local telnet rows.
        let (db, ss, dir) = fixture();
        telnet_profile::insert(&db, &telnet("t1", "Switch")).unwrap();

        let data = payload(json!({ "version": 1, "profiles": [] }));
        merge_import(&db, &ss, dir.path(), &data).unwrap();

        let telnets = telnet_profile::list(&db).unwrap();
        assert!(telnets.iter().any(|t| t.id == "t1"), "local telnet kept");
    }

    #[test]
    fn merge_imports_telnet_profiles() {
        let (db, ss, dir) = fixture();
        let data = payload(json!({
            "version": 1,
            "telnet_profiles": [serde_json::to_value(telnet("t1", "Switch")).unwrap()],
        }));
        merge_import(&db, &ss, dir.path(), &data).unwrap();
        assert_eq!(telnet_profile::get(&db, "t1").unwrap().name, "Switch");
    }

    #[test]
    fn export_includes_telnet_profiles() {
        let (db, ss, dir) = fixture();
        telnet_profile::insert(&db, &telnet("t1", "Switch")).unwrap();
        let out = build_payload(&db, &ss, dir.path(), &ExportMode::LocalBackup).unwrap();
        let arr = out["telnet_profiles"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "Switch");
    }

    #[test]
    fn telnet_login_script_uses_secret_store_across_import_and_export() {
        let (db, ss, dir) = fixture();
        let mut remote = telnet("t1", "Switch");
        remote.login_script = "expect Password:\nsend hunter2".into();
        let data = json!({
            "version": 1,
            "telnet_profiles": [serde_json::to_value(remote).unwrap()],
        });

        merge_import(&db, &ss, dir.path(), &data).unwrap();

        assert!(telnet_profile::get(&db, "t1")
            .unwrap()
            .login_script
            .is_empty());
        assert_eq!(
            stored_telnet_script(&db, &ss, "t1"),
            "expect Password:\nsend hunter2"
        );
        let out = build_payload(&db, &ss, dir.path(), &ExportMode::LocalBackup).unwrap();
        assert_eq!(
            out["telnet_profiles"][0]["login_script"],
            "expect Password:\nsend hunter2"
        );
    }

    #[test]
    fn scrubbed_telnet_script_does_not_erase_local_secret() {
        let (db, ss, dir) = fixture();
        insert_telnet_with_script(&db, &ss, telnet("t1", "Local"), "local script");
        let data = json!({
            "version": 1,
            "telnet_profiles": [serde_json::to_value(telnet("t1", "Remote")).unwrap()],
        });

        merge_import(&db, &ss, dir.path(), &data).unwrap();

        assert_eq!(stored_telnet_script(&db, &ss, "t1"), "local script");
    }

    #[test]
    fn explicit_remote_telnet_script_clear_deletes_local_secret() {
        let (db, ss, dir) = fixture();
        insert_telnet_with_script(&db, &ss, telnet("t1", "Local"), "local script");
        let mut remote = telnet("t1", "Remote");
        remote.save_script_to_remote = true;
        let data = json!({
            "version": 1,
            "telnet_profiles": [serde_json::to_value(remote).unwrap()],
        });

        merge_import(&db, &ss, dir.path(), &data).unwrap();

        assert!(stored_telnet_script(&db, &ss, "t1").is_empty());
    }

    #[test]
    fn remote_push_scrubs_telnet_script_unless_enabled() {
        let (db, ss, dir) = fixture();
        let mut local_only = telnet("local", "Local only");
        local_only.save_script_to_remote = false;
        insert_telnet_with_script(&db, &ss, local_only, "local secret");
        let mut shared = telnet("shared", "Shared");
        shared.save_script_to_remote = true;
        insert_telnet_with_script(&db, &ss, shared, "shared secret");

        let prefs = read_sync_prefs(&db).unwrap();
        let out = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        let rows = out["telnet_profiles"].as_array().unwrap();
        let by_id = |id: &str| rows.iter().find(|row| row["id"] == id).unwrap();

        assert_eq!(by_id("local")["login_script"], "");
        assert_eq!(by_id("shared")["login_script"], "shared secret");
    }

    #[test]
    fn remote_push_reconciles_legacy_plaintext_before_scrubbing_payload() {
        let (db, ss, dir) = fixture();
        db.with_transaction(|tx| {
            tx.execute(
                "INSERT INTO telnet_profiles (id, name, host, login_script) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["legacy", "Legacy", "10.0.0.2", "send old-secret"],
            )?;
            Ok(())
        })
        .unwrap();

        let prefs = read_sync_prefs(&db).unwrap();
        let out = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();

        assert_eq!(out["telnet_profiles"][0]["login_script"], "");
        let state = telnet_profile::login_script_state(&db, "legacy").unwrap();
        assert!(state.legacy_script.is_empty());
        let version = state.version.unwrap();
        assert_eq!(
            ss.get(&crate::secret::telnet_login_script_key("legacy", &version))
                .unwrap()
                .as_deref(),
            Some("send old-secret")
        );
        assert_eq!(
            crate::db::settings::get(&db, telnet_profile::PURGED_EPOCH_SETTING).unwrap(),
            crate::db::settings::get(&db, telnet_profile::PURGE_EPOCH_SETTING).unwrap(),
        );
    }

    #[test]
    fn export_keeps_legacy_script_when_secret_migration_has_not_finished() {
        let (db, ss, dir) = fixture();
        db.with_transaction(|tx| {
            tx.execute(
                "INSERT INTO telnet_profiles (id, name, host, login_script) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["legacy", "Legacy", "10.0.0.2", "send old-secret"],
            )?;
            Ok(())
        })
        .unwrap();

        let out = build_payload(&db, &ss, dir.path(), &ExportMode::LocalBackup).unwrap();

        assert_eq!(out["telnet_profiles"][0]["login_script"], "send old-secret");
    }

    #[test]
    fn export_includes_dynamic_discovery_sources() {
        let (db, ss, dir) = fixture();
        crate::commands::discovery::save_dynamic_discovery_sources_to_db(
            &db,
            vec![docker_source("dyn1", "Docker Dev", "desktop-linux")],
        )
        .unwrap();

        let out = build_payload(&db, &ss, dir.path(), &ExportMode::LocalBackup).unwrap();
        let arr = out["dynamic_discovery_sources"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "dyn1");
        assert_eq!(arr[0]["name"], "Docker Dev");
    }

    #[test]
    fn merge_dynamic_discovery_sources_by_id() {
        let (db, ss, dir) = fixture();
        crate::commands::discovery::save_dynamic_discovery_sources_to_db(
            &db,
            vec![
                docker_source("keep", "Local Only", "local"),
                docker_source("same", "Old", "old-context"),
            ],
        )
        .unwrap();

        let data = json!({
            "version": 1,
            "dynamic_discovery_sources": [
                serde_json::to_value(docker_source("same", "New", "new-context")).unwrap(),
                serde_json::to_value(docker_source("new", "Remote", "remote-context")).unwrap(),
            ],
        });
        merge_import(&db, &ss, dir.path(), &data).unwrap();

        let sources =
            crate::commands::discovery::read_dynamic_discovery_sources_from_db(&db).unwrap();
        assert!(
            sources.iter().any(|s| s.id == "keep"),
            "local-only source survives"
        );
        let same = sources.iter().find(|s| s.id == "same").unwrap();
        assert_eq!(same.name, "New", "same id overwritten");
        assert!(sources.iter().any(|s| s.id == "new"), "remote source added");
    }

    #[test]
    fn merge_keeps_invalid_local_discovery_settings_untouched() {
        let (db, ss, dir) = fixture();
        crate::db::settings::set(
            &db,
            crate::commands::discovery::SOURCES_SETTING_KEY,
            "not-json",
        )
        .unwrap();
        let data = json!({
            "version": 1,
            "dynamic_discovery_sources": [
                serde_json::to_value(docker_source("remote", "Remote", "remote-context"))
                    .unwrap(),
            ],
        });

        let err = merge_import(&db, &ss, dir.path(), &data).unwrap_err();

        assert_eq!(err.code(), "import_partial_failed");
        assert_eq!(
            crate::db::settings::get(&db, crate::commands::discovery::SOURCES_SETTING_KEY,)
                .unwrap()
                .as_deref(),
            Some("not-json"),
        );
    }

    // ── Phase 2: the five new categories ──────────────────────────────

    #[test]
    fn merge_highlights_upsert_by_keyword() {
        let (db, ss, dir) = fixture();
        // Use non-default keywords to be independent of any seeded defaults.
        highlight::insert(
            &db,
            &HighlightRule {
                keyword: "MYKEY".into(),
                name: "My Key".into(),
                color: "#000".into(),
                enabled: true,
                is_regex: true,
                is_case_sensitive: false,
            },
        )
        .unwrap();
        let data = json!({
            "version": 1,
            "highlights": [
                {"keyword": "MYKEY", "name": "Renamed", "color": "#f00", "enabled": false, "is_regex": true, "is_case_sensitive": true},
                {"keyword": "OTHER", "color": "#0f0", "enabled": true},
            ],
        });
        merge_import(&db, &ss, dir.path(), &data).unwrap();
        let hs = highlight::list(&db).unwrap();
        let mk = hs.iter().find(|h| h.keyword == "MYKEY").unwrap();
        assert_eq!(mk.color, "#f00", "color overwritten");
        assert_eq!(mk.name, "Renamed", "name overwritten");
        assert!(mk.is_regex, "is_regex overwritten");
        assert!(mk.is_case_sensitive, "is_case_sensitive overwritten");
        assert!(!mk.enabled);
        let other = hs.iter().find(|h| h.keyword == "OTHER").unwrap();
        assert_eq!(other.color, "#0f0", "new keyword added");
        assert_eq!(
            hs.iter().filter(|h| h.keyword == "MYKEY").count(),
            1,
            "no duplicate row"
        );
    }

    #[test]
    fn merge_highlights_old_3_field_payload() {
        let (db, ss, dir) = fixture();
        // Pre-sync clients only send keyword/color/enabled (no is_regex/name/
        // is_case_sensitive). Missing fields default via #[serde(default)]; then
        // upsert normalizes the legacy plain-text rule to its escaped-regex form,
        // so the "every rule is a regex" invariant holds even across versions.
        // "a.b" proves the escape ran: the dot is neutralized to a literal.
        let data = json!({
            "version": 1,
            "highlights": [
                {"keyword": "a.b", "color": "#00f", "enabled": true},
            ],
        });
        merge_import(&db, &ss, dir.path(), &data).unwrap();
        let hs = highlight::list(&db).unwrap();
        let h = hs.iter().find(|h| h.color == "#00f").unwrap();
        assert_eq!(
            h.keyword, r"a\.b",
            "legacy plain text escaped to an equivalent regex"
        );
        assert_eq!(h.name, "a.b", "empty name seeded from the original keyword");
        assert!(h.is_regex, "legacy text rule normalized to regex on import");
        assert!(
            !h.is_case_sensitive,
            "missing is_case_sensitive defaults to false"
        );
        assert!(h.enabled);
    }

    #[test]
    fn merge_ai_redact_rules_by_id() {
        let (db, ss, dir) = fixture();
        let data = json!({
            "version": 1,
            "ai_redact_rules": [{"id": "u1", "pattern": "secret", "replacement": "<X>"}],
        });
        merge_import(&db, &ss, dir.path(), &data).unwrap();
        let rules = crate::db::ai_redact_rule::list(&db).unwrap();
        assert!(rules.iter().any(|r| r.id == "u1" && r.replacement == "<X>"));
    }

    #[test]
    fn merge_redact_rule_rejects_invalid_regex() {
        let (db, ss, dir) = fixture();
        // A bad regex must NOT reach the DB — compiled() is fail-closed, so a
        // synced invalid rule would otherwise brick AI sessions on this device.
        let data = json!({
            "version": 1,
            "ai_redact_rules": [{"id": "bad", "pattern": "(", "replacement": "<X>"}],
        });
        // Surfaced as an aggregate import failure (not silently swallowed)…
        merge_import(&db, &ss, dir.path(), &data).unwrap_err();
        // …and the bad rule never reached the DB.
        let rules = crate::db::ai_redact_rule::list(&db).unwrap();
        assert!(
            !rules.iter().any(|r| r.id == "bad"),
            "invalid regex not persisted"
        );
    }

    #[test]
    fn merge_ai_blacklist_is_additive() {
        let (db, ss, dir) = fixture();
        // table is seeded with defaults; merge must add, not wipe.
        let before = ai_command_blacklist::list(&db).unwrap().len();
        let data = json!({
            "version": 1,
            "ai_command_blacklist": [{"name": "frobnicate", "category": "destructive"}],
        });
        merge_import(&db, &ss, dir.path(), &data).unwrap();
        let rows = ai_command_blacklist::list(&db).unwrap();
        assert!(rows
            .iter()
            .any(|r| r.name == "frobnicate" && r.category == "destructive"));
        assert!(rows.len() > before, "additive, defaults kept");
    }

    #[test]
    fn merge_snippets_by_name() {
        let (db, ss, dir) = fixture();
        snippet::save(
            dir.path(),
            &[Snippet {
                name: "a".into(),
                command: "old".into(),
            }],
        )
        .unwrap();
        let data = json!({
            "version": 1,
            "snippets": [
                {"name": "a", "command": "new"},
                {"name": "b", "command": "bcmd"},
            ],
        });
        merge_import(&db, &ss, dir.path(), &data).unwrap();
        let snips = snippet::load(dir.path()).unwrap();
        assert_eq!(
            snips.iter().find(|s| s.name == "a").unwrap().command,
            "new",
            "same name overwritten"
        );
        assert!(snips.iter().any(|s| s.name == "b"), "new name added");
        assert_eq!(snips.iter().filter(|s| s.name == "a").count(), 1);
    }

    #[test]
    fn merge_ai_providers_and_active() {
        let (db, ss, dir) = fixture();
        let data = json!({
            "version": 1,
            "ai": {
                "active_provider": "openai",
                "providers": [
                    {"provider": "anthropic", "model": "claude-x",
                     "endpoint": "https://e", "api_key": "sk-123"},
                    {"provider": "bogus", "model": "x", "api_key": "y"},
                ],
            },
        });
        merge_import(&db, &ss, dir.path(), &data).unwrap();
        assert_eq!(
            crate::db::settings::get(&db, "ai_anthropic_model")
                .unwrap()
                .as_deref(),
            Some("claude-x")
        );
        assert_eq!(
            crate::db::settings::get(&db, "ai_provider")
                .unwrap()
                .as_deref(),
            Some("openai"),
            "active provider applied"
        );
        assert_eq!(
            ss.get(&crate::secret::setting_key("ai_anthropic_key"))
                .unwrap()
                .as_deref(),
            Some("sk-123"),
            "api key written to secret store"
        );
        assert!(
            crate::db::settings::get(&db, "ai_bogus_model")
                .unwrap()
                .is_none(),
            "unknown provider ignored"
        );
    }

    #[test]
    fn merge_rejects_unsupported_active_provider() {
        let (db, ss, dir) = fixture();
        crate::db::settings::set(&db, "ai_provider", "anthropic").unwrap();
        // active_provider must clear the same allowlist as provider rows —
        // otherwise ai_provider points at a backend with no config row.
        let data = json!({
            "version": 1,
            "ai": { "active_provider": "bogus", "providers": [] },
        });
        merge_import(&db, &ss, dir.path(), &data).unwrap();
        assert_eq!(
            crate::db::settings::get(&db, "ai_provider")
                .unwrap()
                .as_deref(),
            Some("anthropic"),
            "unsupported active_provider rejected, prior value kept"
        );
    }

    #[test]
    fn merge_ai_empty_model_endpoint_keeps_local() {
        let (db, ss, dir) = fixture();
        crate::db::settings::set(&db, "ai_anthropic_model", "claude-x").unwrap();
        crate::db::settings::set(&db, "ai_anthropic_endpoint", "https://local").unwrap();
        // A blank model/endpoint in the payload (old/hand-edited) must be a
        // no-op, not a destructive clear — additive merge.
        let data = json!({
            "version": 1,
            "ai": { "providers": [{"provider": "anthropic", "model": "", "endpoint": ""}] },
        });
        merge_import(&db, &ss, dir.path(), &data).unwrap();
        assert_eq!(
            crate::db::settings::get(&db, "ai_anthropic_model")
                .unwrap()
                .as_deref(),
            Some("claude-x"),
            "empty model did not overwrite local"
        );
        assert_eq!(
            crate::db::settings::get(&db, "ai_anthropic_endpoint")
                .unwrap()
                .as_deref(),
            Some("https://local"),
            "empty endpoint did not overwrite local"
        );
    }

    #[test]
    fn export_ai_settings_omits_local_only_prefs() {
        let (db, ss, _dir) = fixture();
        crate::db::settings::set(&db, "ai_anthropic_model", "claude-x").unwrap();
        crate::db::settings::set(&db, "ai_danger_mode", "1").unwrap();
        let ai = crate::ai::commands::export_ai_settings(&db, &ss, true).unwrap();
        let s = serde_json::to_string(&ai).unwrap();
        assert!(s.contains("anthropic"));
        assert!(!s.contains("danger_mode"), "danger_mode not synced");
        assert!(!s.contains("auto_run"), "auto_run not synced");
    }

    #[test]
    fn export_ai_omits_empty_model_and_endpoint() {
        let (db, ss, _dir) = fixture();
        // Only an API key configured — model/endpoint unset. Export must NOT
        // emit empty "model"/"endpoint": importing "" would wipe a populated
        // value on another device (a destructive clear; additive-merge forbids).
        ss.set(&crate::secret::setting_key("ai_anthropic_key"), "sk-zzz")
            .unwrap();
        let ai = crate::ai::commands::export_ai_settings(&db, &ss, true).unwrap();
        let prov = ai["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["provider"] == "anthropic")
            .expect("key-only provider still exported");
        assert!(prov.get("model").is_none(), "empty model not emitted");
        assert!(prov.get("endpoint").is_none(), "empty endpoint not emitted");
        assert_eq!(prov["api_key"], "sk-zzz");
    }

    #[test]
    fn export_ai_treats_whitespace_as_unset_and_trims() {
        let (db, ss, _dir) = fixture();
        // Whitespace-only model/endpoint are "effectively unset" — not exported
        // (they'd hide the official-endpoint placeholder + risk a destructive
        // merge). A real key is trimmed before export.
        crate::db::settings::set(&db, "ai_anthropic_model", "   ").unwrap();
        crate::db::settings::set(&db, "ai_anthropic_endpoint", "\t").unwrap();
        ss.set(
            &crate::secret::setting_key("ai_anthropic_key"),
            "  sk-zzz  ",
        )
        .unwrap();
        let ai = crate::ai::commands::export_ai_settings(&db, &ss, true).unwrap();
        let prov = ai["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["provider"] == "anthropic")
            .expect("provider present via key");
        assert!(prov.get("model").is_none(), "whitespace model not emitted");
        assert!(
            prov.get("endpoint").is_none(),
            "whitespace endpoint not emitted"
        );
        assert_eq!(prov["api_key"], "sk-zzz", "key trimmed on export");
    }

    #[test]
    fn merge_old_payload_without_new_keys_is_noop() {
        let (db, ss, dir) = fixture();
        highlight::insert(
            &db,
            &HighlightRule {
                keyword: "KEEP".into(),
                name: "KEEP".into(),
                color: "#1".into(),
                enabled: true,
                is_regex: true,
                is_case_sensitive: false,
            },
        )
        .unwrap();
        let data = json!({ "version": 1, "profiles": [] });
        merge_import(&db, &ss, dir.path(), &data).unwrap();
        assert!(highlight::list(&db)
            .unwrap()
            .iter()
            .any(|h| h.keyword == "KEEP"));
    }
}
