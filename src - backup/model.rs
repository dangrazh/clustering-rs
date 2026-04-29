use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

pub type RowIndex = usize;
pub type ColumnIndex = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClusterId(pub usize);

impl ClusterId {
    pub const UNCLUSTERED: Self = Self(0);
}

impl std::fmt::Display for ClusterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if *self == Self::UNCLUSTERED {
            f.write_str("Unclustered")
        } else {
            write!(f, "C{:04}", self.0)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColumnRole {
    IncidentNumber,
    ShortDescription,
    AdditionalText,
    AssignmentGroup,
    Service,
    Category,
    ConfigurationItem,
    Date,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnMapping {
    pub incident_number: Option<ColumnIndex>,
    pub short_description: Option<ColumnIndex>,
    pub additional_text: Vec<ColumnIndex>,
    pub assignment_group: Option<ColumnIndex>,
    pub service: Option<ColumnIndex>,
    pub category: Option<ColumnIndex>,
    pub configuration_item: Option<ColumnIndex>,
    pub date: Option<ColumnIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceTable {
    pub source_path: Option<PathBuf>,
    pub worksheet_name: Option<String>,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl SourceTable {
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentRecord {
    pub source_row_index: RowIndex,
    pub incident_number: String,
    pub analysis_text: String,
    pub filter_values: FilterValues,
    pub parsed_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterValues {
    pub assignment_group: Option<String>,
    pub service: Option<String>,
    pub category: Option<String>,
    pub configuration_item: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredRow {
    pub source_row_index: RowIndex,
    pub missing_incident_number: bool,
    pub missing_short_description: bool,
}

pub type TermId = u32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextFeatures {
    pub row_index: RowIndex,
    pub terms: BTreeMap<TermId, f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subgroup {
    pub id: usize,
    pub label: String,
    pub incident_row_indices: Vec<RowIndex>,
}

impl Subgroup {
    pub fn size(&self) -> usize {
        self.incident_row_indices.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cluster {
    pub id: ClusterId,
    pub label: String,
    pub incident_row_indices: Vec<RowIndex>,
    pub subgroups: Vec<Subgroup>,
}

impl Cluster {
    pub fn size(&self) -> usize {
        self.incident_row_indices.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSettings {
    pub minimum_cluster_size: usize,
    pub similarity_threshold_percent: u8,
    pub subgroup_similarity_threshold_percent: u8,
}

impl Default for RunSettings {
    fn default() -> Self {
        Self {
            minimum_cluster_size: 50,
            similarity_threshold_percent: 42,
            subgroup_similarity_threshold_percent: 58,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingMetrics {
    pub import_ms: u128,
    pub preprocessing_ms: u128,
    pub clustering_ms: u128,
    pub labeling_ms: u128,
    pub export_ms: u128,
}

impl TimingMetrics {
    pub fn set_clustering_duration(&mut self, duration: Duration) {
        self.clustering_ms = duration.as_millis();
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisRun {
    pub source: SourceTable,
    pub mapping: ColumnMapping,
    pub settings: RunSettings,
    pub processed_incidents: Vec<IncidentRecord>,
    pub ignored_rows: Vec<IgnoredRow>,
    pub clusters: Vec<Cluster>,
    pub unclustered_row_indices: Vec<RowIndex>,
    pub timings: TimingMetrics,
}
