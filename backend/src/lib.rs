//! Market watcher backend: candle storage, EMA computation and the JSON API.

pub mod aggregate;
pub mod api;
pub mod db;
pub mod indicators;
pub mod ingest;
pub mod models;
pub mod providers;

use std::sync::Arc;

use anyhow::Result;
use sqlx::SqlitePool;

use crate::ingest::Engine;
use crate::providers::Sources;

/// Build a database URL for a filesystem path, creating the parent directory.
pub fn sqlite_url(path: &str) -> Result<String> {
    if path.starts_with("sqlite:") {
        return Ok(path.to_string());
    }
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(format!("sqlite://{path}?mode=rwc"))
}

/// Open the database and build the refresh engine (providers included).
pub async fn bootstrap(database_path: &str) -> Result<(SqlitePool, Arc<Engine>)> {
    let pool = db::open(&sqlite_url(database_path)?).await?;
    let resolver: Arc<dyn crate::providers::SourceResolver> = Arc::new(Sources::new()?);
    let engine = Arc::new(Engine::new(pool.clone(), resolver));
    Ok((pool, engine))
}
