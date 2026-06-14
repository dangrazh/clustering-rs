use crate::model::LabelTermPolicy;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
