use crate::model::{AnalysisRun, ColumnMapping};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const CURRENT_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingProfile {
    pub version: u16,
    pub mapping: ColumnMapping,
}

impl MappingProfile {
    pub fn new(mapping: ColumnMapping) -> Self {
        Self {
            version: CURRENT_VERSION,
            mapping,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisSession {
    pub version: u16,
    pub run: AnalysisRun,
}

impl AnalysisSession {
    pub fn new(run: AnalysisRun) -> Self {
        Self {
            version: CURRENT_VERSION,
            run,
        }
    }
}

pub fn save_mapping_profile(path: impl AsRef<Path>, mapping: &ColumnMapping) -> Result<()> {
    let profile = MappingProfile::new(mapping.clone());
    write_json(path, &profile)
}

pub fn load_mapping_profile(path: impl AsRef<Path>) -> Result<ColumnMapping> {
    let profile = read_json::<MappingProfile>(path)?;
    Ok(profile.mapping)
}

pub fn save_analysis_session(path: impl AsRef<Path>, run: &AnalysisRun) -> Result<()> {
    let session = AnalysisSession::new(run.clone());
    write_json(path, &session)
}

pub fn load_analysis_session(path: impl AsRef<Path>) -> Result<AnalysisRun> {
    let session = read_json::<AnalysisSession>(path)?;
    Ok(session.run)
}

fn write_json<T: Serialize>(path: impl AsRef<Path>, value: &T) -> Result<()> {
    let path = path.as_ref();
    let json = serde_json::to_string_pretty(value)?;
    fs::write(path, json).with_context(|| format!("failed to write {}", path.display()))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: impl AsRef<Path>) -> Result<T> {
    let path = path.as_ref();
    let json =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&json).with_context(|| format!("failed to parse {}", path.display()))
}
