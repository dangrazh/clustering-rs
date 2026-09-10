//! Frozen version-1 wire DTOs, serialized with Postcard and compressed with Zstandard.
use crate::model::*;
use crate::workflow::{Annotations, ReviewStatus};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    fs,
    io::{Read, Write},
    path::Path,
};

const MAGIC: &[u8; 8] = b"ICASESS\0";
const HEADER: usize = 52;
pub const MAX_FILE_BYTES: usize = 512 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 1024 * 1024 * 1024;
pub const COMPRESSION_LEVEL: i32 = 6;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewState {
    pub version: u16,
    pub selection: Option<Selection>,
    pub expanded_clusters: Vec<String>,
    pub detail_column_filters: Vec<ColumnFilter>,
    pub detail_sort: Option<Sort>,
    pub pivot_rows: Vec<u64>,
    pub pivot_columns: Vec<u64>,
    pub detail_drilldown_row_indices: Option<Vec<u64>>,
    pub detail_drilldown_label: String,
    pub workflow_states: Vec<ReviewStatus>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub r#type: String,
    pub cluster: Option<u64>,
    pub theme: Option<u64>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ColumnFilter {
    pub selected: Option<Vec<String>>,
    pub query: String,
    pub search_deselected: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sort {
    pub column: u64,
    pub direction: String,
}
impl ViewState {
    pub fn sanitize(&mut self, run: &AnalysisRun) {
        self.version = 1;
        let columns = run.source.headers.len() as u64;
        self.pivot_rows.retain(|c| *c < columns);
        self.pivot_columns
            .retain(|c| *c < columns && !self.pivot_rows.contains(c));
        self.detail_column_filters.truncate(columns as usize);
        if self.detail_sort.as_ref().is_some_and(|s| {
            s.column >= columns || !["asc", "desc", "none"].contains(&s.direction.as_str())
        }) {
            self.detail_sort = None;
        }
        self.expanded_clusters
            .retain(|id| run.clusters.iter().any(|c| c.id.0.to_string() == *id));
        if let Some(rows) = &mut self.detail_drilldown_row_indices {
            rows.retain(|r| *r < run.source.rows.len() as u64);
            rows.sort_unstable();
            rows.dedup();
        }
        if self
            .selection
            .as_ref()
            .is_some_and(|s| match s.r#type.as_str() {
                "all" => false,
                "cluster" => !run
                    .clusters
                    .iter()
                    .any(|c| Some(c.id.0 as u64) == s.cluster),
                "theme" => !run.clusters.iter().any(|c| {
                    Some(c.id.0 as u64) == s.cluster
                        && c.subgroups.iter().any(|t| Some(t.id as u64) == s.theme)
                }),
                _ => true,
            })
        {
            self.selection = None;
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewData {
    pub analysis_id: String,
    pub fingerprint: String,
    pub annotations: Annotations,
}
impl ReviewData {
    pub fn new(run: &AnalysisRun) -> Result<Self> {
        Ok(Self {
            analysis_id: uuid::Uuid::new_v4().to_string(),
            fingerprint: fingerprint(run)?,
            annotations: Annotations::new(run),
        })
    }
    pub fn validate(&self, run: &AnalysisRun) -> Result<()> {
        uuid::Uuid::parse_str(&self.analysis_id).context("Invalid analysis identity.")?;
        ensure!(
            self.fingerprint == fingerprint(run)?,
            "Analysis fingerprint mismatch."
        );
        self.annotations.validate(run)
    }
    pub fn matches(&self, other: &Self) -> Result<()> {
        ensure!(
            self.analysis_id == other.analysis_id && self.fingerprint == other.fingerprint,
            "This review-state file belongs to a different analysis."
        );
        Ok(())
    }
}
pub struct LoadedSession {
    pub run: AnalysisRun,
    pub review: ReviewData,
    pub view: ViewState,
}
#[derive(Serialize, Deserialize)]
struct SessionWire<'a> {
    run: RunWire<'a>,
    review: Cow<'a, ReviewData>,
    view: Cow<'a, ViewState>,
}
// Preserve binary v1 files created before editable labels were introduced.
#[derive(Serialize, Deserialize)]
struct ReviewV1 {
    analysis_id: String,
    fingerprint: String,
    annotations: AnnotationsV1,
}
#[derive(Serialize, Deserialize)]
struct AnnotationsV1 {
    revision: u64,
    entries: std::collections::BTreeMap<String, crate::workflow::Annotation>,
}
#[derive(Serialize, Deserialize)]
struct SessionV1<'a> {
    run: RunWire<'a>,
    review: ReviewV1,
    view: ViewState,
}
impl From<ReviewV1> for ReviewData {
    fn from(old: ReviewV1) -> Self {
        Self {
            analysis_id: old.analysis_id,
            fingerprint: old.fingerprint,
            annotations: Annotations {
                revision: old.annotations.revision,
                entries: old.annotations.entries,
                labels: Default::default(),
            },
        }
    }
}

// Source strings are borrowed when saving. Indices are explicit u64 varints with checked conversion.
// These field orders and the referenced non-index model types are frozen for schema version 1.
#[derive(Serialize, Deserialize)]
struct RunWire<'a> {
    source: Cow<'a, SourceTable>,
    mapping: MappingWire,
    settings: Cow<'a, RunSettings>,
    records: Vec<RecordWire<'a>>,
    ignored: Vec<(u64, bool, bool)>,
    clusters: Vec<ClusterWire<'a>>,
    unclustered: Vec<u64>,
    timings: Cow<'a, TimingMetrics>,
}
#[derive(Serialize, Deserialize)]
struct MappingWire {
    incident: Option<u64>,
    description: Option<u64>,
    additional: Vec<u64>,
    group: Option<u64>,
    service: Option<u64>,
    category: Option<u64>,
    ci: Option<u64>,
    date: Option<u64>,
}
#[derive(Serialize, Deserialize)]
struct RecordWire<'a> {
    row: u64,
    number: Cow<'a, str>,
    text: Cow<'a, str>,
    filters: Cow<'a, FilterValues>,
    date: Option<chrono::NaiveDate>,
}
#[derive(Serialize, Deserialize)]
struct ClusterWire<'a> {
    id: u64,
    label: Cow<'a, str>,
    rows: Vec<u64>,
    themes: Vec<ThemeWire<'a>>,
}
#[derive(Serialize, Deserialize)]
struct ThemeWire<'a> {
    id: u64,
    label: Cow<'a, str>,
    rows: Vec<u64>,
}
fn wide(v: &[usize]) -> Vec<u64> {
    v.iter().map(|v| *v as u64).collect()
}
fn narrow(v: u64) -> Result<usize> {
    usize::try_from(v).context("Index exceeds platform limits.")
}
fn indices(v: Vec<u64>) -> Result<Vec<usize>> {
    v.into_iter().map(narrow).collect()
}
impl<'a> RunWire<'a> {
    fn from_run(run: &'a AnalysisRun) -> Self {
        let m = &run.mapping;
        Self {
            source: Cow::Borrowed(&run.source),
            settings: Cow::Borrowed(&run.settings),
            timings: Cow::Borrowed(&run.timings),
            mapping: MappingWire {
                incident: m.incident_number.map(|v| v as u64),
                description: m.short_description.map(|v| v as u64),
                additional: wide(&m.additional_text),
                group: m.assignment_group.map(|v| v as u64),
                service: m.service.map(|v| v as u64),
                category: m.category.map(|v| v as u64),
                ci: m.configuration_item.map(|v| v as u64),
                date: m.date.map(|v| v as u64),
            },
            records: run
                .processed_incidents
                .iter()
                .map(|r| RecordWire {
                    row: r.source_row_index as u64,
                    number: Cow::Borrowed(&r.incident_number),
                    text: Cow::Borrowed(&r.analysis_text),
                    filters: Cow::Borrowed(&r.filter_values),
                    date: r.parsed_date,
                })
                .collect(),
            ignored: run
                .ignored_rows
                .iter()
                .map(|r| {
                    (
                        r.source_row_index as u64,
                        r.missing_incident_number,
                        r.missing_short_description,
                    )
                })
                .collect(),
            clusters: run
                .clusters
                .iter()
                .map(|c| ClusterWire {
                    id: c.id.0 as u64,
                    label: Cow::Borrowed(&c.label),
                    rows: wide(&c.incident_row_indices),
                    themes: c
                        .subgroups
                        .iter()
                        .map(|t| ThemeWire {
                            id: t.id as u64,
                            label: Cow::Borrowed(&t.label),
                            rows: wide(&t.incident_row_indices),
                        })
                        .collect(),
                })
                .collect(),
            unclustered: wide(&run.unclustered_row_indices),
        }
    }
    fn into_run(self) -> Result<AnalysisRun> {
        let m = self.mapping;
        Ok(AnalysisRun {
            source: self.source.into_owned(),
            settings: self.settings.into_owned(),
            timings: self.timings.into_owned(),
            mapping: ColumnMapping {
                incident_number: m.incident.map(narrow).transpose()?,
                short_description: m.description.map(narrow).transpose()?,
                additional_text: indices(m.additional)?,
                assignment_group: m.group.map(narrow).transpose()?,
                service: m.service.map(narrow).transpose()?,
                category: m.category.map(narrow).transpose()?,
                configuration_item: m.ci.map(narrow).transpose()?,
                date: m.date.map(narrow).transpose()?,
            },
            processed_incidents: self
                .records
                .into_iter()
                .map(|r| {
                    Ok(IncidentRecord {
                        source_row_index: narrow(r.row)?,
                        incident_number: r.number.into_owned(),
                        analysis_text: r.text.into_owned(),
                        filter_values: r.filters.into_owned(),
                        parsed_date: r.date,
                    })
                })
                .collect::<Result<_>>()?,
            ignored_rows: self
                .ignored
                .into_iter()
                .map(|(row, number, description)| {
                    Ok(IgnoredRow {
                        source_row_index: narrow(row)?,
                        missing_incident_number: number,
                        missing_short_description: description,
                    })
                })
                .collect::<Result<_>>()?,
            clusters: self
                .clusters
                .into_iter()
                .map(|c| {
                    Ok(Cluster {
                        id: ClusterId(narrow(c.id)?),
                        label: c.label.into_owned(),
                        incident_row_indices: indices(c.rows)?,
                        subgroups: c
                            .themes
                            .into_iter()
                            .map(|t| {
                                Ok(Subgroup {
                                    id: narrow(t.id)?,
                                    label: t.label.into_owned(),
                                    incident_row_indices: indices(t.rows)?,
                                })
                            })
                            .collect::<Result<_>>()?,
                    })
                })
                .collect::<Result<_>>()?,
            unclustered_row_indices: indices(self.unclustered)?,
        })
    }
}
struct HashWriter<W> {
    inner: W,
    hash: Sha256,
    count: u64,
}
impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if b.len() as u64 > MAX_DECODED_BYTES.saturating_sub(self.count) {
            return Err(std::io::Error::other(
                "Decoded session exceeds the 1 GiB limit.",
            ));
        }
        let n = self.inner.write(b)?;
        self.hash.update(&b[..n]);
        self.count += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
pub fn fingerprint(run: &AnalysisRun) -> Result<String> {
    let writer = postcard::to_io(
        &RunWire::from_run(run),
        HashWriter {
            inner: std::io::sink(),
            hash: Sha256::new(),
            count: 0,
        },
    )?;
    Ok(format!("{:x}", writer.hash.finalize()))
}
fn encode<T: Serialize>(kind: u8, value: &T, level: i32) -> Result<Vec<u8>> {
    let mut bytes = vec![0; HEADER];
    let mut encoder = zstd::stream::write::Encoder::new(
        LimitedWriter {
            inner: &mut bytes,
            remaining: MAX_FILE_BYTES - HEADER,
        },
        level,
    )?;
    encoder.include_checksum(true)?;
    let writer = postcard::to_io(
        value,
        HashWriter {
            inner: encoder,
            hash: Sha256::new(),
            count: 0,
        },
    )?;
    let count = writer.count;
    let digest = writer.hash.finalize();
    writer.inner.finish()?;
    ensure!(
        count <= MAX_DECODED_BYTES && bytes.len() <= MAX_FILE_BYTES,
        "Session exceeds supported file size."
    );
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&2u16.to_le_bytes());
    bytes[10] = kind;
    bytes[11] = 1;
    bytes[12..20].copy_from_slice(&count.to_le_bytes());
    bytes[20..52].copy_from_slice(&digest);
    Ok(bytes)
}
struct LimitedWriter<W> {
    inner: W,
    remaining: usize,
}
impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if b.len() > self.remaining {
            return Err(std::io::Error::other(
                "Compressed session exceeds the 512 MiB limit.",
            ));
        }
        let count = self.inner.write(b)?;
        self.remaining -= count;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
struct HashReader<R> {
    inner: R,
    hash: Sha256,
    count: u64,
}
impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(b)?;
        self.hash.update(&b[..n]);
        self.count += n as u64;
        Ok(n)
    }
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8], kind: u8) -> Result<T> {
    ensure!(
        bytes.len() >= HEADER && &bytes[..8] == MAGIC,
        "Unsupported format. Select a compressed binary file; legacy JSON is not supported."
    );
    ensure!(bytes.len() <= MAX_FILE_BYTES, "File is too large.");
    ensure!(
        matches!(u16::from_le_bytes([bytes[8], bytes[9]]), 1 | 2) && bytes[11] == 1,
        "Unsupported session version or codec."
    );
    ensure!(
        bytes[10] == kind,
        "Wrong file type: select a session or review-state file as appropriate."
    );
    let size = u64::from_le_bytes(bytes[12..20].try_into()?);
    ensure!(size <= MAX_DECODED_BYTES, "Decoded file is too large.");
    let mut decoder = zstd::stream::read::Decoder::new(&bytes[HEADER..])?;
    decoder.window_log_max(27)?;
    let reader = HashReader {
        inner: decoder.take(size + 1),
        hash: Sha256::new(),
        count: 0,
    };
    // Bound individual text fields; large collections stream without a decompressed byte buffer.
    let mut scratch = vec![0; 1024 * 1024];
    let (value, (mut reader, _)) = postcard::from_io::<T, _>((reader, &mut scratch)).context(
        "Invalid or truncated binary data (individual text fields must be under 1 MiB).",
    )?;
    ensure!(
        reader.read(&mut [0])? == 0,
        "Unexpected trailing session data."
    );
    ensure!(
        reader.count == size && reader.hash.finalize().as_slice() == &bytes[20..52],
        "Session integrity check failed."
    );
    Ok(value)
}
pub fn encode_session(run: &AnalysisRun, review: &ReviewData, view: &ViewState) -> Result<Vec<u8>> {
    encode_session_level(run, review, view, COMPRESSION_LEVEL)
}
pub fn encode_session_level(
    run: &AnalysisRun,
    review: &ReviewData,
    view: &ViewState,
    level: i32,
) -> Result<Vec<u8>> {
    // Reject a save that could not be read back using the bounded text scratch buffer.
    for text in run
        .source
        .headers
        .iter()
        .chain(run.source.rows.iter().flatten())
        .map(String::as_str)
        .chain(
            run.processed_incidents
                .iter()
                .flat_map(|r| [r.incident_number.as_str(), r.analysis_text.as_str()]),
        )
    {
        ensure!(
            text.len() < 1024 * 1024,
            "An incident text field exceeds the 1 MiB session limit."
        );
    }
    encode(
        1,
        &SessionWire {
            run: RunWire::from_run(run),
            review: Cow::Borrowed(review),
            view: Cow::Borrowed(view),
        },
        level,
    )
}
pub fn decode_session(bytes: &[u8]) -> Result<LoadedSession> {
    let (run, review, mut view) = if bytes.get(8..10) == Some(&[1, 0]) {
        let wire: SessionV1<'_> = decode(bytes, 1)?;
        (
            wire.run.into_run()?,
            ReviewData::from(wire.review),
            wire.view,
        )
    } else {
        let wire: SessionWire<'_> = decode(bytes, 1)?;
        (
            wire.run.into_run()?,
            wire.review.into_owned(),
            wire.view.into_owned(),
        )
    };
    validate_run(&run)?;
    review.validate(&run)?;
    view.sanitize(&run);
    Ok(LoadedSession { run, review, view })
}
pub fn encode_review(review: &ReviewData) -> Result<Vec<u8>> {
    encode(2, review, COMPRESSION_LEVEL)
}
pub fn decode_review(bytes: &[u8]) -> Result<ReviewData> {
    if bytes.get(8..10) == Some(&[1, 0]) {
        Ok(ReviewData::from(decode::<ReviewV1>(bytes, 2)?))
    } else {
        decode(bytes, 2)
    }
}
pub fn validate_run(run: &AnalysisRun) -> Result<()> {
    use std::collections::HashSet;
    ensure!(!run.source.headers.is_empty(), "Source headers missing.");
    crate::schema::validate_mapping(&run.mapping, &run.source)?;
    let mut rows = HashSet::new();
    for r in &run.processed_incidents {
        ensure!(
            r.source_row_index < run.source.rows.len() && rows.insert(r.source_row_index),
            "Invalid or duplicate incident row."
        );
    }
    let mut ignored = HashSet::new();
    for r in &run.ignored_rows {
        ensure!(
            r.source_row_index < run.source.rows.len()
                && !rows.contains(&r.source_row_index)
                && ignored.insert(r.source_row_index),
            "Invalid ignored row."
        );
    }
    let mut ids = HashSet::new();
    let mut assigned = HashSet::new();
    for c in &run.clusters {
        ensure!(c.id.0 != 0 && ids.insert(c.id), "Invalid cluster ID.");
        let mut members = HashSet::new();
        for r in &c.incident_row_indices {
            ensure!(
                rows.contains(r) && members.insert(*r) && assigned.insert(*r),
                "Invalid cluster membership."
            );
        }
        let mut themes = HashSet::new();
        let mut themed = HashSet::new();
        for t in &c.subgroups {
            ensure!(themes.insert(t.id), "Duplicate theme ID.");
            for r in &t.incident_row_indices {
                ensure!(
                    members.contains(r) && themed.insert(*r),
                    "Invalid theme membership."
                );
            }
        }
    }
    for r in &run.unclustered_row_indices {
        ensure!(
            rows.contains(r) && assigned.insert(*r),
            "Invalid unclustered row."
        );
    }
    ensure!(assigned == rows, "Incomplete incident memberships.");
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingProfile {
    pub version: u16,
    pub mapping: ColumnMapping,
}
impl MappingProfile {
    pub fn new(mapping: ColumnMapping) -> Self {
        Self {
            version: 1,
            mapping,
        }
    }
}
pub fn save_mapping_profile(path: impl AsRef<Path>, mapping: &ColumnMapping) -> Result<()> {
    Ok(fs::write(
        path,
        serde_json::to_vec_pretty(&MappingProfile::new(mapping.clone()))?,
    )?)
}
pub fn load_mapping_profile(path: impl AsRef<Path>) -> Result<ColumnMapping> {
    Ok(serde_json::from_slice::<MappingProfile>(&fs::read(path)?)?.mapping)
}
pub fn save_analysis_session(path: impl AsRef<Path>, run: &AnalysisRun) -> Result<()> {
    Ok(fs::write(
        path,
        encode_session(run, &ReviewData::new(run)?, &ViewState::default())?,
    )?)
}
pub fn load_analysis_session(path: impl AsRef<Path>) -> Result<AnalysisRun> {
    Ok(decode_session(&fs::read(path)?)?.run)
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    #[test]
    fn binary_v1_session_and_review_still_load() {
        let run = AnalysisRun {
            source: SourceTable {
                source_path: None,
                worksheet_name: None,
                headers: vec!["Number".into(), "Description".into()],
                rows: vec![],
            },
            mapping: ColumnMapping {
                incident_number: Some(0),
                short_description: Some(1),
                ..Default::default()
            },
            settings: Default::default(),
            processed_incidents: vec![],
            ignored_rows: vec![],
            clusters: vec![],
            unclustered_row_indices: vec![],
            timings: Default::default(),
        };
        let current = ReviewData::new(&run).unwrap();
        let legacy = || ReviewV1 {
            analysis_id: current.analysis_id.clone(),
            fingerprint: current.fingerprint.clone(),
            annotations: AnnotationsV1 {
                revision: 0,
                entries: Default::default(),
            },
        };
        let mut session = encode(
            1,
            &SessionV1 {
                run: RunWire::from_run(&run),
                review: legacy(),
                view: Default::default(),
            },
            COMPRESSION_LEVEL,
        )
        .unwrap();
        session[8..10].copy_from_slice(&1u16.to_le_bytes());
        let loaded = decode_session(&session).unwrap();
        assert_eq!(loaded.run, run);
        assert!(loaded.review.annotations.labels.is_empty());
        let mut review = encode(2, &legacy(), COMPRESSION_LEVEL).unwrap();
        review[8..10].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            decode_review(&review).unwrap().analysis_id,
            current.analysis_id
        );
    }
}
