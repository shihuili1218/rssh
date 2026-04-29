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
