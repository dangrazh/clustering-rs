use crate::model::LabelTermPolicy;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub struct RuntimeConfig {
    pub data_dir: PathBuf,
    pub database_writers: usize,
    pub cache_bytes: usize,
}
pub fn number(name: &str, default: usize, min: usize, max: usize) -> Result<usize> {
    let value = std::env::var(name)
        .ok()
        .map(|v| v.parse::<usize>())
        .transpose()
        .with_context(|| format!("Invalid {name}"))?
        .unwrap_or(default);
    anyhow::ensure!((min..=max).contains(&value), "{name} must be {min}–{max}");
    Ok(value)
}
impl RuntimeConfig {
    pub fn from_env() -> Result<Self> {
        number("APP_JOB_RETENTION_DAYS", 7, 1, 365)?;
        number("APP_MAX_QUEUED_JOBS", 10, 1, 100)?;
        number("APP_JOB_TIMEOUT_SECONDS", 3600, 60, 86400)?;
        number(
            "APP_WORKER_THREADS",
            std::thread::available_parallelism()
                .map(|n| n.get().saturating_sub(2).max(1))
                .unwrap_or(1),
            1,
            64,
        )?;
        Ok(Self {
            data_dir: std::env::var("APP_DATA_DIR")
                .unwrap_or_else(|_| "data".into())
                .into(),
            database_writers: number("APP_DATABASE_WRITERS", 4, 1, 16)?,
            cache_bytes: number("APP_CACHE_MIB", 4096, 64, 32768)? * 1024 * 1024,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub label_terms: LabelTermPolicy,
}

impl AppConfig {
    pub fn load_from_env() -> Result<Self> {
        let path = std::env::var("CLUSTERING_WEB_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("incident-clustering-config.json"));
        Self::load_optional(path)
    }

    fn load_optional(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            tracing::info!(
                path = %path.display(),
                "configuration file not found; using built-in defaults"
            );
            return Ok(Self::default());
        }
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read configuration {}", path.display()))?;
        let config = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse configuration {}", path.display()))?;
        tracing::info!(path = %path.display(), "loaded configuration");
        Ok(config)
    }
}
