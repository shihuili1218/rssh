use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{stream, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::models::{
    validate_name, ConnectorSpec, DynamicDiscoveredTarget, DynamicDiscoveryConfig,
    DynamicDiscoveryContext, DynamicDiscoveryError, DynamicDiscoveryPlatform,
    DynamicDiscoverySnapshot, DynamicDiscoverySource, DynamicDiscoveryToolStatus,
};
use crate::state::AppState;

pub(crate) const SOURCES_SETTING_KEY: &str = "dynamic_discovery_sources";

struct CapturedOutput {
    stdout: String,
    stderr: String,
}

fn platform_program(platform: DynamicDiscoveryPlatform) -> &'static str {
    match platform {
        DynamicDiscoveryPlatform::Docker => "docker",
        DynamicDiscoveryPlatform::K8s => "kubectl",
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(not(windows))]
fn executable_names(program: &str) -> Vec<String> {
    vec![program.to_string()]
}

pub(crate) struct ResolvedDiscoveryProgram {
    pub executable: PathBuf,
    pub search_path: OsString,
}

#[cfg(windows)]
fn executable_names(program: &str) -> Vec<String> {
    if Path::new(program).extension().is_some() {
        vec![program.to_string()]
    } else {
        vec![format!("{program}.exe"), program.to_string()]
    }
}

/// Resolve an allowlisted discovery CLI without assuming that a desktop app
/// inherited the user's login-shell PATH. Finder/LaunchServices normally does
/// not include Homebrew or `/usr/local/bin`, while `tauri dev` does.
fn resolve_executable_in(
    program: &str,
    inherited_path: Option<&OsStr>,
    fallback_dirs: &[PathBuf],
) -> Option<ResolvedDiscoveryProgram> {
    let mut dirs = inherited_path
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    dirs.extend(fallback_dirs.iter().cloned());
    let search_path = std::env::join_paths(&dirs).ok()?;

    let names = executable_names(program);
    for dir in dirs {
        for name in &names {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(ResolvedDiscoveryProgram {
                    executable: candidate,
                    search_path,
                });
            }
        }
    }
    None
}

fn discovery_cli_fallback_dirs() -> Vec<PathBuf> {
    let mut fallback_dirs = Vec::new();

    #[cfg(unix)]
    {
        if let Some(home) = dirs::home_dir() {
            fallback_dirs.push(home.join(".docker").join("bin"));
            fallback_dirs.push(home.join(".local").join("bin"));
            #[cfg(target_os = "macos")]
            fallback_dirs.push(
                home.join("Applications")
                    .join("Docker.app")
                    .join("Contents")
                    .join("Resources")
                    .join("bin"),
            );
        }
        fallback_dirs.push(PathBuf::from("/opt/homebrew/bin"));
        fallback_dirs.push(PathBuf::from("/usr/local/bin"));
        fallback_dirs.push(PathBuf::from("/usr/bin"));
        fallback_dirs.push(PathBuf::from("/bin"));
        #[cfg(target_os = "linux")]
        fallback_dirs.push(PathBuf::from("/snap/bin"));
        #[cfg(target_os = "macos")]
        fallback_dirs.push(PathBuf::from(
            "/Applications/Docker.app/Contents/Resources/bin",
        ));
    }

    #[cfg(windows)]
    {
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            fallback_dirs.push(
                PathBuf::from(program_files)
                    .join("Docker")
                    .join("Docker")
                    .join("resources")
                    .join("bin"),
            );
        }
        if let Some(program_data) = std::env::var_os("ProgramData") {
            fallback_dirs.push(PathBuf::from(program_data).join("chocolatey").join("bin"));
        }
        if let Some(home) = dirs::home_dir() {
            fallback_dirs.push(home.join("scoop").join("shims"));
        }
    }

    fallback_dirs
}

pub(crate) fn resolve_dynamic_discovery_program(
    platform: DynamicDiscoveryPlatform,
) -> AppResult<ResolvedDiscoveryProgram> {
    let program = platform_program(platform);
    let inherited_path = std::env::var_os("PATH");
    resolve_executable_in(
        program,
        inherited_path.as_deref(),
        &discovery_cli_fallback_dirs(),
    )
    .ok_or_else(|| {
        AppError::other(
            "dynamic_discovery_cli_unavailable",
            json!({ "program": program }),
        )
    })
}

fn trim_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

async fn run_success(
    platform: DynamicDiscoveryPlatform,
    args: Vec<String>,
    timeout_secs: u64,
) -> AppResult<CapturedOutput> {
    let program = platform_program(platform);
    let resolved = resolve_dynamic_discovery_program(platform)?;
    let mut cmd = tokio::process::Command::new(resolved.executable);
    // kubectl exec-auth plugins and Docker SSH/credential helpers inherit this
    // environment, so resolving only the top-level binary is insufficient.
    cmd.env("PATH", resolved.search_path);
    cmd.args(&args).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output())
        .await
        .map_err(|_| {
            AppError::other(
                "dynamic_discovery_timeout",
                json!({ "program": program, "secs": timeout_secs }),
            )
        })?
        .map_err(|e| {
            let code = if e.kind() == std::io::ErrorKind::NotFound {
                "dynamic_discovery_cli_unavailable"
            } else {
                "dynamic_discovery_command_failed"
            };
            AppError::other(code, json!({ "program": program, "err": e.to_string() }))
        })?;

    let stdout = trim_output(&output.stdout);
    let stderr = trim_output(&output.stderr);
    if !output.status.success() {
        return Err(AppError::other(
            "dynamic_discovery_command_failed",
            json!({
                "program": program,
                "status": output.status.code(),
                "err": if stderr.is_empty() { stdout.clone() } else { stderr.clone() },
            }),
        ));
    }

    Ok(CapturedOutput { stdout, stderr })
}

pub(crate) fn read_dynamic_discovery_sources_from_db(
    db: &crate::db::Db,
) -> AppResult<Vec<DynamicDiscoverySource>> {
    let Some(raw) = crate::db::settings::get(db, SOURCES_SETTING_KEY)? else {
        return Ok(vec![]);
    };
    if raw.trim().is_empty() {
        return Ok(vec![]);
    }
    serde_json::from_str(&raw).map_err(|e| {
        AppError::config(
            "dynamic_discovery_settings_invalid",
            json!({ "err": e.to_string() }),
        )
    })
}

fn read_sources(state: &AppState) -> AppResult<Vec<DynamicDiscoverySource>> {
    read_dynamic_discovery_sources_from_db(&state.db)
}

fn has_control(s: &str) -> bool {
    s.chars().any(|ch| {
        let c = ch as u32;
        c < 0x20 || c == 0x7f
    })
}

fn validate_arg(label: &'static str, value: &str) -> AppResult<()> {
    if value.trim().is_empty() {
        return Err(AppError::config(
            "dynamic_discovery_field_required",
            json!({ "field": label }),
        ));
    }
    if has_control(value) {
        return Err(AppError::config(
            "dynamic_discovery_field_has_control_char",
            json!({ "field": label }),
        ));
    }
    Ok(())
}

fn normalize_optional_namespace(namespace: Option<String>) -> Option<String> {
    namespace.and_then(|ns| {
        let trimmed = ns.trim().to_string();
        if trimmed.is_empty() || trimmed == "*" {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_sources(sources: &mut [DynamicDiscoverySource]) {
    for source in sources {
        if let DynamicDiscoveryConfig::K8s { namespace, .. } = &mut source.config {
            *namespace = normalize_optional_namespace(namespace.take());
        }
    }
}

fn validate_sources(sources: &[DynamicDiscoverySource]) -> AppResult<()> {
    let mut ids = std::collections::HashSet::new();
    for source in sources {
        validate_arg("id", &source.id)?;
        if !ids.insert(source.id.as_str()) {
            return Err(AppError::config(
                "dynamic_discovery_duplicate_id",
                json!({ "id": source.id }),
            ));
        }
        validate_name(&source.name)?;
        match &source.config {
            DynamicDiscoveryConfig::Docker { context, shell } => {
                validate_arg("context", context)?;
                validate_arg("shell", shell)?;
            }
            DynamicDiscoveryConfig::K8s {
                context,
                namespace,
                shell,
            } => {
                validate_arg("context", context)?;
                if let Some(ns) = namespace {
                    if has_control(ns) {
                        return Err(AppError::config(
                            "dynamic_discovery_field_has_control_char",
                            json!({ "field": "namespace" }),
                        ));
                    }
                }
                validate_arg("shell", shell)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn save_dynamic_discovery_sources_to_db(
    db: &crate::db::Db,
    mut sources: Vec<DynamicDiscoverySource>,
) -> AppResult<()> {
    normalize_sources(&mut sources);
    validate_sources(&sources)?;
    let raw = serde_json::to_string(&sources)
        .map_err(|e| AppError::other("serde_failed", json!({ "err": e.to_string() })))?;
    crate::db::settings::set(db, SOURCES_SETTING_KEY, &raw)
}

#[tauri::command]
pub fn list_dynamic_discovery_sources(
    state: State<'_, AppState>,
) -> AppResult<Vec<DynamicDiscoverySource>> {
    read_sources(&state)
}

#[tauri::command]
pub fn save_dynamic_discovery_sources(
    state: State<'_, AppState>,
    sources: Vec<DynamicDiscoverySource>,
) -> AppResult<()> {
    save_dynamic_discovery_sources_to_db(&state.db, sources)
}

#[tauri::command]
pub async fn dynamic_discovery_tool_status(
    platform: DynamicDiscoveryPlatform,
) -> AppResult<DynamicDiscoveryToolStatus> {
    let args = match platform {
        DynamicDiscoveryPlatform::Docker => vec!["--version".to_string()],
        DynamicDiscoveryPlatform::K8s => {
            vec!["version".to_string(), "--client=true".to_string()]
        }
    };
    match run_success(platform, args, 3).await {
        Ok(out) => Ok(DynamicDiscoveryToolStatus {
            platform,
            available: true,
            version: Some(if out.stdout.is_empty() {
                out.stderr
            } else {
                out.stdout
            }),
            error: None,
        }),
        Err(e) => Ok(DynamicDiscoveryToolStatus {
            platform,
            available: false,
            version: None,
            error: Some(e.to_string()),
        }),
    }
}

#[tauri::command]
pub async fn list_dynamic_discovery_contexts(
    platform: DynamicDiscoveryPlatform,
) -> AppResult<Vec<DynamicDiscoveryContext>> {
    match platform {
        DynamicDiscoveryPlatform::Docker => list_docker_contexts().await,
        DynamicDiscoveryPlatform::K8s => list_k8s_contexts().await,
    }
}

async fn list_docker_contexts() -> AppResult<Vec<DynamicDiscoveryContext>> {
    let out = run_success(
        DynamicDiscoveryPlatform::Docker,
        vec![
            "context".into(),
            "ls".into(),
            "--format".into(),
            "{{json .}}".into(),
        ],
        5,
    )
    .await?;
    parse_docker_contexts(&out.stdout)
}

async fn list_k8s_contexts() -> AppResult<Vec<DynamicDiscoveryContext>> {
    let out = run_success(
        DynamicDiscoveryPlatform::K8s,
        vec![
            "config".into(),
            "get-contexts".into(),
            "-o".into(),
            "name".into(),
        ],
        5,
    )
    .await?;
    let current = run_success(
        DynamicDiscoveryPlatform::K8s,
        vec!["config".into(), "current-context".into()],
        3,
    )
    .await
    .ok()
    .map(|o| o.stdout);
    Ok(parse_k8s_contexts(&out.stdout, current.as_deref()))
}

#[tauri::command]
pub async fn discover_dynamic_targets(
    state: State<'_, AppState>,
) -> AppResult<DynamicDiscoverySnapshot> {
    let sources = read_sources(&state)?;
    discover_sources(sources).await
}

async fn discover_sources(
    sources: Vec<DynamicDiscoverySource>,
) -> AppResult<DynamicDiscoverySnapshot> {
    Ok(discover_sources_with(
        sources,
        |source| async move { discover_source(&source).await },
    )
    .await)
}

async fn discover_sources_with<F, Fut>(
    sources: Vec<DynamicDiscoverySource>,
    discover: F,
) -> DynamicDiscoverySnapshot
where
    F: Fn(DynamicDiscoverySource) -> Fut,
    Fut: std::future::Future<Output = AppResult<Vec<DynamicDiscoveredTarget>>>,
{
    let mut targets = Vec::new();
    let mut errors = Vec::new();

    const MAX_CONCURRENT_SOURCES: usize = 4;
    let pending = sources
        .into_iter()
        .filter(|s| s.enabled)
        .enumerate()
        .map(|(index, source)| {
            let result = discover(source.clone());
            async move { (index, source, result.await) }
        });
    let mut results = stream::iter(pending)
        .buffer_unordered(MAX_CONCURRENT_SOURCES)
        .collect::<Vec<_>>()
        .await;
    // Network/process completion order is nondeterministic; preserve configured
    // source order for the error list while still discovering concurrently.
    results.sort_by_key(|(index, _, _)| *index);
    for (_, source, result) in results {
        match result {
            Ok(mut found) => targets.append(&mut found),
            Err(e) => errors.push(DynamicDiscoveryError {
                source_id: source.id.clone(),
                source_name: source.name.clone(),
                platform: source.platform(),
                message: e.to_string(),
            }),
        }
    }

    targets.sort_by(|a, b| {
        a.source_name
            .cmp(&b.source_name)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    DynamicDiscoverySnapshot { targets, errors }
}

async fn discover_source(
    source: &DynamicDiscoverySource,
) -> AppResult<Vec<DynamicDiscoveredTarget>> {
    match &source.config {
        DynamicDiscoveryConfig::Docker { context, shell } => {
            discover_docker(source, context, shell).await
        }
        DynamicDiscoveryConfig::K8s {
            context,
            namespace,
            shell,
        } => discover_k8s(source, context, namespace.as_deref(), shell).await,
    }
}

#[derive(Deserialize)]
struct DockerContextRow {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Current")]
    current: Option<serde_json::Value>,
}

fn parse_docker_json_lines<T>(stdout: &str, command: &'static str) -> AppResult<Vec<T>>
where
    T: serde::de::DeserializeOwned,
{
    stdout
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<T>(line).map_err(|e| {
                AppError::other(
                    "dynamic_discovery_parse_failed",
                    json!({
                        "platform": "docker",
                        "command": command,
                        "line": index + 1,
                        "err": e.to_string(),
                    }),
                )
            })
        })
        .collect()
}

fn parse_docker_contexts(stdout: &str) -> AppResult<Vec<DynamicDiscoveryContext>> {
    let rows = parse_docker_json_lines::<DockerContextRow>(stdout, "context ls")?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name = row.name.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let current = match row.current {
                Some(serde_json::Value::Bool(v)) => v,
                Some(serde_json::Value::String(s)) => s == "*" || s.eq_ignore_ascii_case("true"),
                _ => false,
            };
            Some(DynamicDiscoveryContext {
                platform: DynamicDiscoveryPlatform::Docker,
                name,
                current,
            })
        })
        .collect())
}

fn parse_k8s_contexts(stdout: &str, current: Option<&str>) -> Vec<DynamicDiscoveryContext> {
    let current = current.unwrap_or("").trim();
    stdout
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| DynamicDiscoveryContext {
            platform: DynamicDiscoveryPlatform::K8s,
            name: name.to_string(),
            current: name == current,
        })
        .collect()
}

#[derive(Deserialize)]
struct DockerPsRow {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "Names")]
    names: String,
    #[serde(rename = "Status")]
    status: String,
}

async fn discover_docker(
    source: &DynamicDiscoverySource,
    context: &str,
    shell: &str,
) -> AppResult<Vec<DynamicDiscoveredTarget>> {
    let out = run_success(
        DynamicDiscoveryPlatform::Docker,
        vec![
            "--context".into(),
            context.to_string(),
            "ps".into(),
            "--format".into(),
            "{{json .}}".into(),
        ],
        8,
    )
    .await?;
    parse_docker_ps(source, context, shell, &out.stdout)
}

fn parse_docker_ps(
    source: &DynamicDiscoverySource,
    context: &str,
    shell: &str,
    stdout: &str,
) -> AppResult<Vec<DynamicDiscoveredTarget>> {
    let rows = parse_docker_json_lines::<DockerPsRow>(stdout, "ps")?;

    Ok(rows
        .into_iter()
        .filter(|row| !row.id.trim().is_empty())
        .map(|row| {
            let container_name = row
                .names
                .split(',')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(row.id.as_str())
                .to_string();
            DynamicDiscoveredTarget {
                id: format!("docker_exec:{context}:{}", row.id),
                source_id: source.id.clone(),
                source_name: source.name.clone(),
                platform: DynamicDiscoveryPlatform::Docker,
                name: container_name.clone(),
                sub: format!("{} · {} · {}", row.image, row.status, context),
                connector_spec: ConnectorSpec::DockerExec {
                    context: context.to_string(),
                    container_id: row.id,
                    container_name,
                    shell: shell.to_string(),
                },
            }
        })
        .collect())
}

#[derive(Deserialize)]
struct K8sPodList {
    items: Vec<K8sPod>,
}

#[derive(Deserialize)]
struct K8sPod {
    metadata: K8sMetadata,
    spec: K8sPodSpec,
    status: Option<K8sPodStatus>,
}

#[derive(Deserialize)]
struct K8sMetadata {
    name: String,
    namespace: Option<String>,
}

#[derive(Deserialize)]
struct K8sPodSpec {
    #[serde(default)]
    containers: Vec<K8sContainer>,
}

#[derive(Deserialize)]
struct K8sContainer {
    name: String,
}

#[derive(Deserialize)]
struct K8sPodStatus {
    phase: Option<String>,
}

async fn discover_k8s(
    source: &DynamicDiscoverySource,
    context: &str,
    namespace: Option<&str>,
    shell: &str,
) -> AppResult<Vec<DynamicDiscoveredTarget>> {
    let mut args = vec![
        "--context".to_string(),
        context.to_string(),
        "get".into(),
        "pods".into(),
        "--field-selector=status.phase=Running".into(),
        "-o".into(),
        "json".into(),
    ];
    if let Some(ns) = namespace {
        args.push("-n".into());
        args.push(ns.to_string());
    } else {
        args.push("-A".into());
    }

    let out = run_success(DynamicDiscoveryPlatform::K8s, args, 10).await?;
    parse_k8s_pods(source, context, namespace, shell, &out.stdout)
}

fn parse_k8s_pods(
    source: &DynamicDiscoverySource,
    context: &str,
    namespace_filter: Option<&str>,
    shell: &str,
    stdout: &str,
) -> AppResult<Vec<DynamicDiscoveredTarget>> {
    let list: K8sPodList = serde_json::from_str(stdout).map_err(|e| {
        AppError::other(
            "dynamic_discovery_parse_failed",
            json!({
                "platform": "k8s",
                "command": "get pods",
                "line": e.line(),
                "err": e.to_string(),
            }),
        )
    })?;
    let mut targets = Vec::new();
    for pod in list.items {
        let phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            .unwrap_or("");
        if phase != "Running" {
            continue;
        }
        let namespace = pod
            .metadata
            .namespace
            .as_deref()
            .or(namespace_filter)
            .unwrap_or("default")
            .to_string();
        if pod.spec.containers.is_empty() {
            continue;
        }
        let multi = pod.spec.containers.len() > 1;
        for c in pod.spec.containers {
            let name = if multi {
                format!("{}/{}", pod.metadata.name, c.name)
            } else {
                pod.metadata.name.clone()
            };
            let container = Some(c.name.clone());
            targets.push(DynamicDiscoveredTarget {
                id: format!(
                    "kubectl_exec:{context}:{namespace}:{}:{}",
                    pod.metadata.name, c.name
                ),
                source_id: source.id.clone(),
                source_name: source.name.clone(),
                platform: DynamicDiscoveryPlatform::K8s,
                name,
                sub: format!("{namespace} · Running · {context}"),
                connector_spec: ConnectorSpec::KubectlExec {
                    context: context.to_string(),
                    namespace: namespace.clone(),
                    pod: pod.metadata.name.clone(),
                    container,
                    shell: shell.to_string(),
                },
            });
        }
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DynamicDiscoveryConfig;

    #[cfg(unix)]
    fn make_executable(path: &std::path::Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, script).unwrap();
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cli_resolver_uses_fallback_when_gui_path_omits_the_program() {
        let inherited = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();
        let docker = fallback.path().join("docker");
        make_executable(&docker, "#!/bin/sh\nexit 0\n");
        let path = std::env::join_paths([inherited.path()]).unwrap();

        let resolved = resolve_executable_in(
            "docker",
            Some(path.as_os_str()),
            &[fallback.path().to_owned()],
        );

        let resolved = resolved.unwrap();
        assert_eq!(resolved.executable, docker);
        assert!(std::env::split_paths(&resolved.search_path).any(|dir| dir == fallback.path()));
    }

    #[cfg(unix)]
    #[test]
    fn cli_resolver_prefers_inherited_path_over_fallbacks() {
        let inherited = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();
        let inherited_docker = inherited.path().join("docker");
        let fallback_docker = fallback.path().join("docker");
        make_executable(&inherited_docker, "#!/bin/sh\nexit 0\n");
        make_executable(&fallback_docker, "#!/bin/sh\nexit 0\n");
        let path = std::env::join_paths([inherited.path()]).unwrap();

        let resolved = resolve_executable_in(
            "docker",
            Some(path.as_os_str()),
            &[fallback.path().to_owned()],
        );

        let resolved = resolved.unwrap();
        assert_eq!(resolved.executable, inherited_docker);
        assert!(std::env::split_paths(&resolved.search_path).any(|dir| dir == fallback.path()));
    }

    #[cfg(unix)]
    #[test]
    fn resolved_search_path_is_inherited_by_cli_helpers() {
        let inherited = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();
        let docker = fallback.path().join("docker");
        let helper = fallback.path().join("docker-helper");
        make_executable(&docker, "#!/bin/sh\nexec docker-helper\n");
        make_executable(&helper, "#!/bin/sh\nexit 0\n");
        let path = std::env::join_paths([inherited.path()]).unwrap();
        let resolved = resolve_executable_in(
            "docker",
            Some(path.as_os_str()),
            &[fallback.path().to_owned()],
        )
        .unwrap();

        let status = std::process::Command::new(resolved.executable)
            .env("PATH", resolved.search_path)
            .status()
            .unwrap();

        assert!(status.success());
    }

    #[cfg(unix)]
    #[test]
    fn cli_resolver_rejects_non_executable_files_and_directories() {
        let non_executable = tempfile::tempdir().unwrap();
        std::fs::write(non_executable.path().join("docker"), "not executable").unwrap();
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("docker")).unwrap();

        let resolved = resolve_executable_in(
            "docker",
            None,
            &[
                non_executable.path().to_owned(),
                directory.path().to_owned(),
            ],
        );

        assert!(resolved.is_none());
    }

    fn docker_source() -> DynamicDiscoverySource {
        DynamicDiscoverySource {
            id: "src1".into(),
            name: "Docker".into(),
            enabled: true,
            config: DynamicDiscoveryConfig::Docker {
                context: "prod".into(),
                shell: "sh".into(),
            },
        }
    }

    fn k8s_source() -> DynamicDiscoverySource {
        DynamicDiscoverySource {
            id: "src2".into(),
            name: "K8S".into(),
            enabled: true,
            config: DynamicDiscoveryConfig::K8s {
                context: "stage".into(),
                namespace: None,
                shell: "sh".into(),
            },
        }
    }

    #[test]
    fn namespace_named_all_is_preserved_when_sources_are_saved() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let mut source = k8s_source();
        let DynamicDiscoveryConfig::K8s { namespace, .. } = &mut source.config else {
            unreachable!();
        };
        *namespace = Some(" all ".into());

        save_dynamic_discovery_sources_to_db(&db, vec![source]).unwrap();

        let saved = read_dynamic_discovery_sources_from_db(&db).unwrap();
        let DynamicDiscoveryConfig::K8s { namespace, .. } = &saved[0].config else {
            unreachable!();
        };
        assert_eq!(namespace.as_deref(), Some("all"));
    }

    #[test]
    fn empty_and_star_namespaces_still_mean_all_namespaces() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let mut empty = k8s_source();
        let DynamicDiscoveryConfig::K8s { namespace, .. } = &mut empty.config else {
            unreachable!();
        };
        *namespace = Some("  ".into());
        let mut star = k8s_source();
        star.id = "src3".into();
        let DynamicDiscoveryConfig::K8s { namespace, .. } = &mut star.config else {
            unreachable!();
        };
        *namespace = Some(" * ".into());

        save_dynamic_discovery_sources_to_db(&db, vec![empty, star]).unwrap();

        let saved = read_dynamic_discovery_sources_from_db(&db).unwrap();
        assert!(saved.iter().all(|source| matches!(
            &source.config,
            DynamicDiscoveryConfig::K8s {
                namespace: None,
                ..
            }
        )));
    }

    #[test]
    fn docker_context_json_lines_parse_current() {
        let out = r#"{"Name":"default","Current":true}
{"Name":"prod","Current":false}"#;
        let contexts = parse_docker_contexts(out).unwrap();
        assert_eq!(contexts.len(), 2);
        assert_eq!(contexts[0].name, "default");
        assert!(contexts[0].current);
        assert_eq!(contexts[1].name, "prod");
    }

    #[test]
    fn malformed_docker_context_json_reports_its_line() {
        let out = concat!("{\"Name\":\"default\",\"Current\":true}\n", "{broken}\n",);

        let err = parse_docker_contexts(out).unwrap_err();

        assert_eq!(err.code(), "dynamic_discovery_parse_failed");
        assert!(err.to_string().contains("\"line\":2"));
        assert!(err.to_string().contains("\"command\":\"context ls\""));
    }

    #[test]
    fn k8s_context_names_mark_current() {
        let contexts = parse_k8s_contexts("dev\nprod\n", Some("prod"));
        assert_eq!(contexts.len(), 2);
        assert!(!contexts[0].current);
        assert!(contexts[1].current);
    }

    #[test]
    fn docker_ps_becomes_docker_exec_targets() {
        let out = r#"{"ID":"abc123","Image":"nginx:latest","Names":"web","Status":"Up 2 minutes"}"#;
        let targets = parse_docker_ps(&docker_source(), "prod", "sh", out).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "docker_exec:prod:abc123");
        assert!(matches!(
            targets[0].connector_spec,
            ConnectorSpec::DockerExec { .. }
        ));
    }

    #[test]
    fn malformed_docker_ps_json_reports_its_line() {
        let out = concat!(
            "{\"ID\":\"abc123\",\"Image\":\"nginx\",\"Names\":\"web\",\"Status\":\"Up\"}\n",
            "not-json\n",
        );

        let err = parse_docker_ps(&docker_source(), "prod", "sh", out).unwrap_err();

        assert_eq!(err.code(), "dynamic_discovery_parse_failed");
        assert!(err.to_string().contains("\"line\":2"));
        assert!(err.to_string().contains("\"command\":\"ps\""));
    }

    #[tokio::test]
    async fn enabled_sources_are_discovered_concurrently() {
        let mut second = docker_source();
        second.id = "src2".into();
        second.name = "Docker 2".into();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));

        tokio::time::timeout(
            Duration::from_secs(1),
            discover_sources_with(vec![docker_source(), second], move |_| {
                let barrier = barrier.clone();
                async move {
                    barrier.wait().await;
                    Ok(Vec::new())
                }
            }),
        )
        .await
        .expect("one slow source must not block the next source from starting");
    }

    #[test]
    fn running_k8s_pod_becomes_one_target_per_container() {
        let json = r#"{
          "items": [{
            "metadata": {"name": "api-123", "namespace": "default"},
            "status": {"phase": "Running"},
            "spec": {"containers": [{"name": "api"}, {"name": "sidecar"}]}
          }, {
            "metadata": {"name": "pending", "namespace": "default"},
            "status": {"phase": "Pending"},
            "spec": {"containers": [{"name": "api"}]}
          }]
        }"#;
        let targets = parse_k8s_pods(&k8s_source(), "stage", None, "sh", json).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].name, "api-123/api");
        assert_eq!(targets[1].name, "api-123/sidecar");
        assert!(matches!(
            targets[0].connector_spec,
            ConnectorSpec::KubectlExec { .. }
        ));
    }
}
