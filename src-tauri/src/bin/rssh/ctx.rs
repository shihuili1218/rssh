//! CliCtx — DB + 懒加载 SecretStore。Deref<Target=Db> 让所有 `db::*::*(ctx, ...)`
//! 调用零改动透传。`secret_store` 只在首次访问时探测系统 keychain，避免
//! `rssh ls` 等只读命令付出 keychain 探测延迟。

use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use rssh_lib::db::Db;
use rssh_lib::secret::{self, SecretStore};

pub(crate) struct CliCtx {
    pub db: Arc<Db>,
    pub secret_store: OnceLock<Arc<dyn SecretStore>>,
}

impl CliCtx {
    pub fn secret_store(&self) -> &Arc<dyn SecretStore> {
        self.secret_store
            .get_or_init(|| secret::open(self.db.clone()))
    }
}

impl Deref for CliCtx {
    type Target = Db;
    fn deref(&self) -> &Db {
        &self.db
    }
}
