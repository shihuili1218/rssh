use std::sync::Arc;

use super::SecretStore;
use crate::db::{self, Db};
use crate::error::AppResult;

pub struct DbStore {
    db: Arc<Db>,
}

impl DbStore {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }
}

impl SecretStore for DbStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        db::secret::get(&self.db, key)
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        db::secret::set(&self.db, key, value)
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        db::secret::delete(&self.db, key)
    }

    fn backend_name(&self) -> &'static str {
        "db"
    }
}
