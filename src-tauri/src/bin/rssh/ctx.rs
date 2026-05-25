//! CliCtx — DB + 懒加载 SecretStore。Deref<Target=Db> 让所有 `db::*::*(ctx, ...)`
//! 调用零改动透传。`secret_store` 只在首次访问时探测系统 keychain，避免
//! `rssh ls` 等只读命令付出 keychain 探测延迟。
//!
//! 首次构造 SecretStore 同时跑一次启动迁移（idempotent），跟 GUI 入口对齐。
//! Marker 在 DB 共享，两入口任一跑过另一入口启动时直接跳过。

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use rssh_lib::db::Db;
use rssh_lib::migration;
use rssh_lib::secret::{self, SecretStore};

pub(crate) struct CliCtx {
    pub db: Arc<Db>,
    pub data_dir: PathBuf,
    pub secret_store: OnceLock<Arc<dyn SecretStore>>,
}

impl CliCtx {
    /// 失败场景：sticky backend 标记是 keyring 但当前 keychain 拿不到 → 写 stderr
    /// 退出 1，跟 `Db::open` 失败处理一致。CLI 不能 silently fallback file（会让
    /// 旧密文全废）。
    /// 签名仍返 `&Arc<dyn SecretStore>`，让 12 处调用方零改动。
    pub fn secret_store(&self) -> &Arc<dyn SecretStore> {
        self.secret_store.get_or_init(|| {
            let sys = match secret::open(self.db.clone(), &self.data_dir) {
                Ok(s) => s,
                Err(e) => {
                    // CLI 不接 i18n catalog（见 helpers/tui.rs），跟 main.rs 的
                    // `Failed to open database` 风格对齐。技术细节同时入 log。
                    log::error!("secret::open failed: {e}");
                    eprintln!("Failed to open secret storage: {e}");
                    std::process::exit(1);
                }
            };
            // 启动一次性迁移。CLI 不阻塞执行（log warn，下次启动重试）。
            if let Err(e) = migration::run_migrations(
                &self.db,
                sys.raw_keyring.as_deref(),
                sys.store.as_ref(),
            ) {
                log::warn!("migration failed (will retry on next startup): {e}");
            }
            sys.store
        })
    }
}

impl Deref for CliCtx {
    type Target = Db;
    fn deref(&self) -> &Db {
        &self.db
    }
}
