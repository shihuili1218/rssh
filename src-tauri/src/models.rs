use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{AppError, AppResult};

/// 用户可见 name 字段的字符校验。拒绝所有 C0 控制符（含 ESC `\x1b` 和 BEL `\x07`）
/// 以及 DEL `\x7f`。这些字符会破坏 OSC 7337 协议（CLI → GUI 的 `rssh open <name>`
/// 转义序列），让恶意 profile/forward 名能注入额外终端转义。
/// 普通可打印 ASCII、空格、UTF-8 多字节字符均允许。
pub fn validate_name(name: &str) -> AppResult<()> {
    if name.is_empty() {
        return Err(AppError::config("name_empty", serde_json::json!({})));
    }
    for ch in name.chars() {
        let c = ch as u32;
        // C0 controls: 0x00-0x1F；DEL: 0x7F。任一都能终止 / 注入终端转义。
        if c < 0x20 || c == 0x7f {
            return Err(AppError::config(
                "name_has_control_char",
                serde_json::json!({ "codepoint": c }),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    /// DB 列声明 `TEXT NOT NULL DEFAULT ''`，model 类型对齐成 String。
    /// **应用层不变量**：Profile.credential_id 永远是一个真实 Credential 的 id。
    /// - 写入入口（`add.rs`/`edit.rs`/`ProfileEditor.svelte`/`do_import_ssh_entries`）
    ///   强制必填，从源头保证不变量。
    /// - 读取端（`open.rs`/`ssh_builder.rs`/`forward.rs`/`session.rs`）直接
    ///   `credential::get(&id)`，引用错就 fail-fast 报 `*_cred_not_found`，
    ///   不再 `is_empty()` 降级到"无凭证"。
    pub credential_id: String,
    pub bastion_profile_id: Option<String>,
    pub init_command: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_ssh_algorithms")]
    pub algorithms: SshAlgorithms,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshAlgorithms {
    #[serde(default = "default_kex_algorithms")]
    pub kex: Vec<String>,
    #[serde(default = "default_key_algorithms")]
    pub key: Vec<String>,
    #[serde(default = "default_cipher_algorithms")]
    pub cipher: Vec<String>,
    #[serde(default = "default_mac_algorithms")]
    pub mac: Vec<String>,
    #[serde(default = "default_compression_algorithms")]
    pub compression: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshAlgorithmCatalog {
    pub defaults: SshAlgorithms,
    pub supported: SshAlgorithms,
}

impl Default for SshAlgorithms {
    fn default() -> Self {
        Self {
            kex: default_kex_algorithms(),
            key: default_key_algorithms(),
            cipher: default_cipher_algorithms(),
            mac: default_mac_algorithms(),
            compression: default_compression_algorithms(),
        }
    }
}

fn deserialize_ssh_algorithms<'de, D>(deserializer: D) -> Result<SshAlgorithms, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(SshAlgorithms::default());
    }
    match serde_json::from_value(value) {
        Ok(algorithms) => Ok(algorithms),
        Err(e) => {
            log::warn!("failed to deserialize profile.algorithms, using defaults: {e}");
            Ok(SshAlgorithms::default())
        }
    }
}

pub fn default_ssh_algorithms() -> SshAlgorithms {
    SshAlgorithms::default()
}

fn default_kex_algorithms() -> Vec<String> {
    // ext-info-* and kex-strict-* are protocol markers, not selectable KEX.
    // ssh::algorithms adds the client-side markers to russh at runtime.
    [
        "mlkem768x25519-sha256",
        "curve25519-sha256",
        "curve25519-sha256@libssh.org",
        "diffie-hellman-group-exchange-sha256",
        "diffie-hellman-group18-sha512",
        "diffie-hellman-group17-sha512",
        "diffie-hellman-group16-sha512",
        "diffie-hellman-group15-sha512",
        "diffie-hellman-group14-sha256",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_key_algorithms() -> Vec<String> {
    [
        "ssh-ed25519",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
        "rsa-sha2-512",
        "rsa-sha2-256",
        "ssh-rsa",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_cipher_algorithms() -> Vec<String> {
    [
        "chacha20-poly1305@openssh.com",
        "aes256-gcm@openssh.com",
        "aes256-ctr",
        "aes192-ctr",
        "aes128-ctr",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_mac_algorithms() -> Vec<String> {
    [
        "hmac-sha2-512-etm@openssh.com",
        "hmac-sha2-256-etm@openssh.com",
        "hmac-sha2-512",
        "hmac-sha2-256",
        "hmac-sha1-etm@openssh.com",
        "hmac-sha1",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_compression_algorithms() -> Vec<String> {
    ["none", "zlib", "zlib@openssh.com"]
        .into_iter()
        .map(String::from)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialType {
    Password,
    Key,
    Interactive,
    Agent,
    None,
}

impl CredentialType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Key => "key",
            Self::Interactive => "interactive",
            Self::Agent => "agent",
            Self::None => "none",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "password" => Self::Password,
            "key" => Self::Key,
            "interactive" => Self::Interactive,
            "agent" => Self::Agent,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub id: String,
    pub name: String,
    pub username: String,
    #[serde(rename = "type")]
    pub credential_type: CredentialType,
    pub secret: Option<String>,
    #[serde(default)]
    pub save_to_remote: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForwardType {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Forward {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub forward_type: ForwardType,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub profile_id: String,
    /// Optional group membership — shares the same `groups` table as profiles.
    /// `#[serde(default)]` keeps older exported payloads (no group_id) importable.
    #[serde(default)]
    pub group_id: Option<String>,
}

/// Saved serial console — a peer of `Profile`/`Forward`. No secret, no FK: just
/// a named port + line framing. snake_case fields match the profile/forward
/// convention and feed the runtime `serial_open` config 1:1 (no remapping).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialProfile {
    pub id: String,
    pub name: String,
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub parity: String,
    pub stop_bits: u8,
    pub flow_control: String,
    // ── Tabby-style extras. All frontend-applied (TerminalPane transforms)
    //    EXCEPT `xany`, which is a termios wire flag set in `serial::open`.
    //    serde defaults keep older exported payloads (lacking these keys) importable.
    #[serde(default)]
    pub xany: bool,
    #[serde(default = "default_input_newline")]
    pub input_newline: String, // cr | lf | crlf — what Enter sends
    #[serde(default = "default_output_newline")]
    pub output_newline: String, // raw | cr | lf | crlf — incoming → CRLF normalization
    #[serde(default)]
    pub local_echo: bool,
    #[serde(default = "default_backspace")]
    pub backspace: String, // del | bs | csi3 — what Backspace/Delete sends
    #[serde(default)]
    pub slow_send: bool, // send one byte at a time (slow devices / bootloaders)
    #[serde(default = "default_input_mode")]
    pub input_mode: String, // normal | hex | line
    #[serde(default = "default_output_mode")]
    pub output_mode: String, // text | hex
    #[serde(default)]
    pub login_script: String, // expect/send lines, run on connect
    /// Optional group membership — same `groups` table as profiles/forwards.
    #[serde(default)]
    pub group_id: Option<String>,
}

/// How local echo is selected for a telnet session.
///
/// `Auto` follows RFC 854 ECHO negotiation, while `On` / `Off` are explicit
/// user overrides for broken peers. The old `local_echo` boolean remains in
/// `TelnetProfile` as a wire-compatibility field for pre-v0.2.12 clients.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelnetEchoMode {
    #[default]
    Auto,
    On,
    Off,
}

impl TelnetEchoMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

/// Saved telnet endpoint — a peer of `SerialProfile`, with its optional login
/// script stored separately in `SecretStore`. Only line-discipline knobs that
/// make sense for a telnet NVT live here — no baud/parity/hex/slow_send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelnetProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default = "default_telnet_port")]
    pub port: u16,
    /// What Enter sends. Default crlf — RFC 854's NVT end-of-line.
    #[serde(default = "default_telnet_input_newline")]
    pub input_newline: String, // cr | lf | crlf
    #[serde(default = "default_output_newline")]
    pub output_newline: String, // raw | cr | lf | crlf — incoming → CRLF normalization
    #[serde(default)]
    pub local_echo: bool,
    /// Explicit three-state echo policy. `None` means a legacy payload: derive
    /// it from `local_echo` (`true` -> On, `false` -> Off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo_mode: Option<TelnetEchoMode>,
    #[serde(default = "default_backspace")]
    pub backspace: String, // del | bs | csi3 — what Backspace/Delete sends
    #[serde(default)]
    pub login_script: String, // expect/send lines, run on connect
    /// Upload the script inside the already-encrypted remote sync payload.
    /// False scrubs it while preserving any receiving device's local script.
    #[serde(default)]
    pub save_script_to_remote: bool,
    /// Optional group membership — same `groups` table as profiles/forwards.
    #[serde(default)]
    pub group_id: Option<String>,
}

impl TelnetProfile {
    pub fn resolved_echo_mode(&self) -> TelnetEchoMode {
        self.echo_mode.unwrap_or(if self.local_echo {
            TelnetEchoMode::On
        } else {
            // Preserve the pre-echo_mode checkbox semantics for legacy data.
            TelnetEchoMode::Off
        })
    }
}

// --- Dynamic discovery ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicDiscoveryPlatform {
    Docker,
    #[serde(rename = "k8s")]
    K8s,
}

fn default_container_shell() -> String {
    "sh".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "platform", rename_all = "snake_case")]
pub enum DynamicDiscoveryConfig {
    Docker {
        context: String,
        #[serde(default = "default_container_shell")]
        shell: String,
    },
    #[serde(rename = "k8s")]
    K8s {
        context: String,
        #[serde(default)]
        namespace: Option<String>,
        #[serde(default = "default_container_shell")]
        shell: String,
    },
}

impl DynamicDiscoveryConfig {
    pub fn platform(&self) -> DynamicDiscoveryPlatform {
        match self {
            Self::Docker { .. } => DynamicDiscoveryPlatform::Docker,
            Self::K8s { .. } => DynamicDiscoveryPlatform::K8s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicDiscoverySource {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(flatten)]
    pub config: DynamicDiscoveryConfig,
}

impl DynamicDiscoverySource {
    pub fn platform(&self) -> DynamicDiscoveryPlatform {
        self.config.platform()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicDiscoveryContext {
    pub platform: DynamicDiscoveryPlatform,
    pub name: String,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicDiscoveryToolStatus {
    pub platform: DynamicDiscoveryPlatform,
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectorSpec {
    DockerExec {
        context: String,
        container_id: String,
        container_name: String,
        shell: String,
    },
    KubectlExec {
        context: String,
        namespace: String,
        pod: String,
        container: Option<String>,
        shell: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicDiscoveredTarget {
    pub id: String,
    pub source_id: String,
    pub source_name: String,
    pub platform: DynamicDiscoveryPlatform,
    pub name: String,
    pub sub: String,
    pub connector_spec: ConnectorSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicDiscoveryError {
    pub source_id: String,
    pub source_name: String,
    pub platform: DynamicDiscoveryPlatform,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicDiscoverySnapshot {
    pub targets: Vec<DynamicDiscoveredTarget>,
    pub errors: Vec<DynamicDiscoveryError>,
}

fn default_telnet_port() -> u16 {
    23
}
fn default_telnet_input_newline() -> String {
    "crlf".into()
}

fn default_input_newline() -> String {
    "cr".into()
}
fn default_output_newline() -> String {
    "raw".into()
}
fn default_backspace() -> String {
    "del".into()
}
fn default_input_mode() -> String {
    "normal".into()
}
fn default_output_mode() -> String {
    "text".into()
}

// --- Group ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub color: String,
    pub sort_order: i32,
}

// --- Highlight ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightRule {
    pub keyword: String,
    #[serde(default)]
    pub name: String,
    pub color: String,
    pub enabled: bool,
    #[serde(default)]
    pub is_regex: bool,
    #[serde(default)]
    pub is_case_sensitive: bool,
}

// --- Snippet ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub name: String,
    pub command: String,
}

// --- Session Recording ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastHeader {
    pub version: u8,
    pub width: u32,
    pub height: u32,
    pub timestamp: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DB credentials.type 列存的就是这套小写字符串——CLI 直接读 DB（AGENT.md P5）。
    /// 改 as_str 字面值 = 数据库里存量 credential 全部解不出。
    #[test]
    fn credential_type_as_str_stable_literals() {
        assert_eq!(CredentialType::Password.as_str(), "password");
        assert_eq!(CredentialType::Key.as_str(), "key");
        assert_eq!(CredentialType::Interactive.as_str(), "interactive");
        assert_eq!(CredentialType::Agent.as_str(), "agent");
        assert_eq!(CredentialType::None.as_str(), "none");
    }

    #[test]
    fn credential_type_roundtrip_through_string() {
        for t in [
            CredentialType::Password,
            CredentialType::Key,
            CredentialType::Interactive,
            CredentialType::Agent,
            CredentialType::None,
        ] {
            assert_eq!(CredentialType::from_str(t.as_str()), t);
        }
    }

    #[test]
    fn credential_type_unknown_falls_back_to_none() {
        // 防御 schema 漂移：DB 里出现未知 type 时不该 panic，
        // 也不该错认成某个有效类型——退到 None 让上层显式处理。
        assert_eq!(CredentialType::from_str(""), CredentialType::None);
        assert_eq!(CredentialType::from_str("bogus"), CredentialType::None);
        assert_eq!(CredentialType::from_str("Password"), CredentialType::None);
        // 大小写敏感
        assert_eq!(CredentialType::from_str("PASSWORD"), CredentialType::None);
    }

    #[test]
    fn validate_name_accepts_normal() {
        assert!(validate_name("prod-web").is_ok());
        assert!(validate_name("生产 1 号").is_ok());
        assert!(validate_name("a:b@c.example").is_ok());
        assert!(validate_name("with spaces").is_ok());
        assert!(validate_name(";semicolons;").is_ok()); // ; 不是 OSC 终止符
    }

    #[test]
    fn validate_name_rejects_empty() {
        assert_eq!(validate_name("").unwrap_err().code(), "name_empty");
    }

    #[test]
    fn validate_name_rejects_esc_and_bel() {
        // ESC \x1b 和 BEL \x07 是 OSC 7337 的关键终止符 —— 注入主战场
        assert_eq!(
            validate_name("evil\x1b]52;c;...\x07").unwrap_err().code(),
            "name_has_control_char"
        );
        assert_eq!(
            validate_name("end\x07start").unwrap_err().code(),
            "name_has_control_char"
        );
    }

    #[test]
    fn validate_name_rejects_other_c0_and_del() {
        // 任何 C0 控制符都拦：NUL / TAB / LF / CR / DEL
        for c in ['\x00', '\t', '\n', '\r', '\x7f'] {
            let s = format!("a{c}b");
            assert_eq!(
                validate_name(&s).unwrap_err().code(),
                "name_has_control_char",
                "char {:?} should be rejected",
                c
            );
        }
    }

    #[test]
    fn credential_type_serde_matches_as_str() {
        // serde 的 rename_all = "lowercase" 必须和 as_str 完全一致——
        // 不一致会让 JSON 序列化和 DB 字符串对不上。
        for t in [
            CredentialType::Password,
            CredentialType::Key,
            CredentialType::Interactive,
            CredentialType::Agent,
            CredentialType::None,
        ] {
            let json = serde_json::to_string(&t).unwrap();
            assert_eq!(json, format!("\"{}\"", t.as_str()));
        }
    }

    #[test]
    fn profile_malformed_algorithms_deserializes_to_default() {
        let p: Profile = serde_json::from_value(serde_json::json!({
            "id": "p1",
            "name": "P1",
            "host": "h.example",
            "port": 22,
            "credential_id": "c1",
            "bastion_profile_id": null,
            "init_command": null,
            "group_id": null,
            "algorithms": { "kex": "not-a-list" }
        }))
        .unwrap();

        assert_eq!(p.algorithms, SshAlgorithms::default());
    }
}
