//! CLI 内部 helper 集合。
//!
//! - `tui`：交互式终端 IO（prompt、read_password、menu_select 等）
//! - `ssh_builder`：把 Profile + bastion 链 + 凭证编进 `Command::new("ssh")`
//! - `cred`：Credential 写盘（DB 元数据 + SecretStore secret）+ 通用 find_id_by_name

pub mod cred;
pub mod ssh_builder;
pub mod tui;

pub use cred::{find_id_by_name, load_cred_secrets, update_cred_with_secrets, upsert_cred_with_secrets};
pub use ssh_builder::build_ssh_command;
pub use tui::{
    confirm, die, hex_to_rgb, menu_select, prompt, prompt_default, prompt_optional,
    prompt_secret_default, read_multiline, read_password,
};
