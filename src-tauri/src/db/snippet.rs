use std::path::Path;

use crate::error::AppResult;
use crate::models::Snippet;

pub fn load(data_dir: &Path) -> AppResult<Vec<Snippet>> {
    let path = data_dir.join("snippets.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let data = std::fs::read_to_string(path)?;
    let snippets: Vec<Snippet> = serde_json::from_str(&data).unwrap_or_default();
    Ok(snippets)
}

pub fn save(data_dir: &Path, snippets: &[Snippet]) -> AppResult<()> {
    let path = data_dir.join("snippets.json");
    let data = serde_json::to_string_pretty(snippets)
        .map_err(|e| crate::error::AppError::Other(e.to_string()))?;
    std::fs::write(path, data)?;
    Ok(())
}
