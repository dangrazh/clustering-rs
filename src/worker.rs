use crate::clustering::{cluster_incidents, unclustered_rows};
use crate::model::{AnalysisRun, ColumnMapping, RunSettings, SourceTable, TimingMetrics};
use crate::schema::build_records;
use anyhow::Result;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Instant;

#[derive(Debug)]
pub enum WorkerMessage {
    Started,
    Progress(ProgressUpdate),
    Finished(Result<Box<AnalysisRun>>),
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub step: usize,
    pub total_steps: usize,
    pub stage: String,
    pub detail: String,
}

impl ProgressUpdate {
    pub fn new(
        step: usize,
        total_steps: usize,
        stage: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            step,
            total_steps,
            stage: stage.into(),
            detail: detail.into(),
        }
    }

    pub fn fraction(&self) -> f32 {
        if self.total_steps == 0 {
            0.0
        } else {
            self.step as f32 / self.total_steps as f32
        }
    }
}

pub fn spawn_analysis(
    source: SourceTable,
    mapping: ColumnMapping,
    settings: RunSettings,
) -> Receiver<WorkerMessage> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(WorkerMessage::Started);
        let progress_sender = sender.clone();
        let result = run_analysis_with_progress(source, mapping, settings, |progress| {
            let _ = progress_sender.send(WorkerMessage::Progress(progress));
        })
        .map(Box::new);
        let _ = sender.send(WorkerMessage::Finished(result));
    });
    receiver
}

pub fn run_analysis(
    source: SourceTable,
    mapping: ColumnMapping,
    settings: RunSettings,
) -> Result<AnalysisRun> {
    run_analysis_with_progress(source, mapping, settings, |_| {})
}

pub fn run_analysis_with_progress(
    source: SourceTable,
    mapping: ColumnMapping,
    settings: RunSettings,
    mut progress: impl FnMut(ProgressUpdate),
) -> Result<AnalysisRun> {
    const TOTAL_STEPS: usize = 8;
    progress(ProgressUpdate::new(
        1,
        TOTAL_STEPS,
        "Validating column mapping",
        format!("{} source rows available", source.row_count()),
    ));

    let preprocessing_start = Instant::now();
    progress(ProgressUpdate::new(
        2,
        TOTAL_STEPS,
        "Building incident records",
        "Checking mandatory fields and collecting selected text fields",
    ));
    let (processed_incidents, ignored_rows) = build_records(&source, &mapping)?;
    let preprocessing_ms = preprocessing_start.elapsed().as_millis();

    let clustering_start = Instant::now();
    let mut clustering_step = 3;
    let clusters = cluster_incidents(&processed_incidents, &settings, |stage, detail| {
        progress(ProgressUpdate::new(
            clustering_step.min(7),
            TOTAL_STEPS,
            stage,
            detail,
        ));
        clustering_step += 1;
    });
    let clustering_ms = clustering_start.elapsed().as_millis();
    progress(ProgressUpdate::new(
        7,
        TOTAL_STEPS,
        "Assigning unclustered incidents",
        "Collecting processed rows that did not meet cluster thresholds",
    ));
    let unclustered_row_indices = unclustered_rows(&processed_incidents, &clusters);

    progress(ProgressUpdate::new(
        8,
        TOTAL_STEPS,
        "Finalizing analysis",
        format!(
            "{} clusters, {} unclustered incidents, {} ignored rows",
            clusters.len(),
            unclustered_row_indices.len(),
            ignored_rows.len()
        ),
    ));

    Ok(AnalysisRun {
        source,
        mapping,
        settings,
        processed_incidents,
        ignored_rows,
        clusters,
        unclustered_row_indices,
        timings: TimingMetrics {
            preprocessing_ms,
            clustering_ms,
            ..Default::default()
        },
    })
}
