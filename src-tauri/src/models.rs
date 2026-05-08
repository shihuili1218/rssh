use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub credential_id: Option<String>,
    pub bastion_profile_id: Option<String>,
    pub init_command: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
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
    pub color: String,
    pub enabled: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastEvent(pub f64, pub String, pub String);

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
}
