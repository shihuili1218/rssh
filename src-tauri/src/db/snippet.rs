use std::path::Path;

use crate::error::{AppError, AppResult};
use crate::models::Snippet;

pub fn load(data_dir: &Path) -> AppResult<Vec<Snippet>> {
    let path = data_dir.join("snippets.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let data = std::fs::read_to_string(&path)?;
    // 文件存在但解析失败 = 用户数据可能损坏。早 fail 让用户察觉，比 silent 清空更好。
    serde_json::from_str(&data).map_err(|e| {
        AppError::other(
            "snippet_parse_failed",
            serde_json::json!({
                "path": path.to_string_lossy(),
                "err": e.to_string(),
            }),
        )
    })
}

pub fn save(data_dir: &Path, snippets: &[Snippet]) -> AppResult<()> {
    let path = data_dir.join("snippets.json");
    let data = serde_json::to_string_pretty(snippets).map_err(|e| {
        crate::error::AppError::other("serde_failed", serde_json::json!({ "err": e.to_string() }))
    })?;
    std::fs::write(path, data)?;
    Ok(())
}
