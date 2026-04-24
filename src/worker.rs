use crate::clustering::{cluster_incidents, unclustered_rows};
use crate::model::{AnalysisRun, ColumnMapping, RunSettings, SourceTable, TimingMetrics};
use crate::progress::{ProgressReporter, ProgressUpdate};
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

pub fn spawn_analysis(
    source: SourceTable,
    mapping: ColumnMapping,
    settings: RunSettings,
) -> Receiver<WorkerMessage> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(WorkerMessage::Started);
        let progress_sender = sender.clone();
        let result = run_analysis_with_progress(source, mapping, settings, move |progress| {
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
    progress: impl Fn(ProgressUpdate) + Send + Sync + 'static,
) -> Result<AnalysisRun> {
    const TOTAL_STEPS: usize = 8;
    let reporter = ProgressReporter::new(progress);

    reporter.substep(
        1,
        TOTAL_STEPS,
        "Validating column mapping",
        1,
        2,
        "Checking required fields",
        format!("{} source rows available", source.row_count()),
    );
    reporter.substep(
        1,
        TOTAL_STEPS,
        "Validating column mapping",
        2,
        2,
        "Verifying mapped column indices",
        format!("{} mapped columns selected", mapped_column_count(&mapping)),
    );

    let preprocessing_start = Instant::now();
    let (processed_incidents, ignored_rows) = build_records(&source, &mapping, Some(&reporter))?;
    let preprocessing_ms = preprocessing_start.elapsed().as_millis();

    let clustering_start = Instant::now();
    let clusters = cluster_incidents(&processed_incidents, &settings, Some(&reporter));
    let clustering_ms = clustering_start.elapsed().as_millis();
    let unclustered_row_indices = unclustered_rows(&processed_incidents, &clusters, Some(&reporter));

    reporter.substep(
        8,
        TOTAL_STEPS,
        "Finalizing analysis",
        1,
        2,
        "Assembling analysis package",
        format!(
            "Preparing run with {} processed incidents and {} clusters",
            processed_incidents.len(),
            clusters.len()
        ),
    );
    reporter.substep(
        8,
        TOTAL_STEPS,
        "Finalizing analysis",
        2,
        2,
        "Publishing results",
        format!(
            "{} clusters, {} unclustered incidents, {} ignored rows",
            clusters.len(),
            unclustered_row_indices.len(),
            ignored_rows.len()
        ),
    );

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

fn mapped_column_count(mapping: &ColumnMapping) -> usize {
    mapping.additional_text.len()
        + [
            mapping.incident_number,
            mapping.short_description,
            mapping.assignment_group,
            mapping.service,
            mapping.category,
            mapping.configuration_item,
            mapping.date,
        ]
        .into_iter()
        .flatten()
        .count()
}
