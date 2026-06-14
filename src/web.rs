use crate::config::AppConfig;
use crate::io::{export_analysis, import_source, import_xlsx_sheet, list_worksheets};
use crate::model::{AnalysisRun, ColumnMapping, LabelTermPolicy, RunSettings, SourceTable};
use crate::progress::ProgressUpdate;
use crate::schema::{suggest_mapping, validate_mapping};
use crate::worker::run_analysis_with_progress;
use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{stream, StreamExt};
use http::{Method, StatusCode};
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{self, HeaderValue};
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use url::form_urlencoded;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;

const INDEX_HTML: &str = include_str!("web_assets/index.html");
const APP_CSS: &str = include_str!("web_assets/app.css");
const APP_JS: &str = include_str!("web_assets/app.js");
const ANALYSIS_JS: &str = include_str!("web_assets/analysis.js");
const API_JS: &str = include_str!("web_assets/api.js");
const MAPPING_JS: &str = include_str!("web_assets/mapping.js");
const RESULTS_JS: &str = include_str!("web_assets/results.js");
const SOURCE_JS: &str = include_str!("web_assets/source.js");
const STATE_JS: &str = include_str!("web_assets/state.js");
const UI_JS: &str = include_str!("web_assets/ui.js");
const UTILS_JS: &str = include_str!("web_assets/utils.js");

#[derive(Clone, Default)]
struct WebState {
    inner: Arc<AppState>,
}

#[derive(Default)]
struct AppState {
    next_id: AtomicU64,
    sources: Mutex<HashMap<String, StoredSource>>,
    jobs: Mutex<HashMap<String, Arc<Mutex<AnalysisJob>>>>,
    config: AppConfig,
}

#[derive(Clone)]
struct StoredSource {
    file_name: String,
    path: PathBuf,
    worksheets: Vec<String>,
    source: SourceTable,
}

struct AnalysisJob {
    status: JobStatus,
    message: String,
    started_at: Instant,
    finished_at: Option<Instant>,
    progress_log: Vec<ProgressEvent>,
    result: Option<AnalysisRun>,
    error: Option<String>,
    events: broadcast::Sender<ProgressEvent>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum JobStatus {
    Running,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    kind: String,
    elapsed_ms: u128,
    message: String,
    progress: Option<ProgressUpdate>,
    result_summary: Option<AnalysisSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceResponse {
    source_id: String,
    file_name: String,
    worksheets: Vec<String>,
    selected_worksheet: Option<String>,
    headers: Vec<String>,
    row_count: usize,
    preview_rows: Vec<Vec<String>>,
    suggested_mapping: ColumnMapping,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisSummary {
    clusters: usize,
    processed_incidents: usize,
    ignored_rows: usize,
    unclustered_incidents: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartAnalysisRequest {
    source_id: String,
    mapping: ColumnMapping,
    settings: RunSettings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreSessionRequest {
    run: AnalysisRun,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartAnalysisResponse {
    job_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobSnapshot {
    status: JobStatus,
    message: String,
    elapsed_ms: u128,
    progress_log: Vec<ProgressEvent>,
    result_summary: Option<AnalysisSummary>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PivotRequest {
    row_indices: Vec<usize>,
    row_columns: Vec<usize>,
    column_columns: Vec<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PivotResponse {
    record_count: usize,
    headers: Vec<String>,
    rows: Vec<PivotResponseRow>,
    numeric_columns: Vec<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PivotResponseRow {
    cells: Vec<String>,
    total: bool,
    row_indices: Vec<usize>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

pub async fn serve(address: SocketAddr, config: AppConfig) -> Result<()> {
    let listener = TcpListener::bind(address).await?;
    let state = WebState {
        inner: Arc::new(AppState {
            next_id: AtomicU64::default(),
            sources: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
            config,
        }),
    };
    tracing::info!("web UI listening on http://{address}");

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service_fn(move |request| route(request, state.clone())))
                .await
            {
                tracing::warn!(%err, "HTTP connection failed");
            }
        });
    }
}

async fn route(
    request: Request<Incoming>,
    state: WebState,
) -> Result<Response<BoxBody>, Infallible> {
    Ok(match handle(request, state).await {
        Ok(response) => response,
        Err(err) => json_error(StatusCode::BAD_REQUEST, err.to_string()),
    })
}

async fn handle(request: Request<Incoming>, state: WebState) -> Result<Response<BoxBody>> {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path();

    match (method, path) {
        (Method::GET, "/") => Ok(text_response(
            StatusCode::OK,
            "text/html; charset=utf-8",
            INDEX_HTML,
        )),
        (Method::GET, "/app.css") => Ok(text_response(
            StatusCode::OK,
            "text/css; charset=utf-8",
            APP_CSS,
        )),
        (Method::GET, "/app.js") => Ok(text_response(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            APP_JS,
        )),
        (Method::GET, "/analysis.js") => Ok(text_response(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            ANALYSIS_JS,
        )),
        (Method::GET, "/api.js") => Ok(text_response(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            API_JS,
        )),
        (Method::GET, "/mapping.js") => Ok(text_response(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            MAPPING_JS,
        )),
        (Method::GET, "/results.js") => Ok(text_response(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            RESULTS_JS,
        )),
        (Method::GET, "/source.js") => Ok(text_response(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            SOURCE_JS,
        )),
        (Method::GET, "/state.js") => Ok(text_response(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            STATE_JS,
        )),
        (Method::GET, "/ui.js") => Ok(text_response(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            UI_JS,
        )),
        (Method::GET, "/utils.js") => Ok(text_response(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            UTILS_JS,
        )),
        (Method::POST, "/api/import") => import_endpoint(request, state).await,
        (Method::POST, "/api/sessions") => restore_session_endpoint(request, state).await,
        (Method::POST, "/api/analyze") => start_analysis_endpoint(request, state).await,
        (Method::GET, path)
            if path.starts_with("/api/sources/") && path.ends_with("/worksheet") =>
        {
            worksheet_endpoint(&uri, state).await
        }
        (Method::GET, path) if path.starts_with("/api/jobs/") && path.ends_with("/events") => {
            events_endpoint(path, state)
        }
        (Method::GET, path) if path.starts_with("/api/jobs/") && path.ends_with("/result") => {
            result_endpoint(path, state)
        }
        (Method::POST, path) if path.starts_with("/api/jobs/") && path.ends_with("/pivot") => {
            pivot_endpoint(path, request, state).await
        }
        (Method::GET, path) if path.starts_with("/api/jobs/") && path.ends_with("/export") => {
            export_endpoint(path, state).await
        }
        (Method::GET, path) if path.starts_with("/api/jobs/") => job_endpoint(path, state),
        _ => Ok(json_error(StatusCode::NOT_FOUND, "not found")),
    }
}

async fn import_endpoint(request: Request<Incoming>, state: WebState) -> Result<Response<BoxBody>> {
    let query = parse_query(request.uri().query());
    let file_name = query
        .get("filename")
        .cloned()
        .filter(|name| !name.trim().is_empty())
        .context("filename query parameter is required")?;
    let body = request.into_body().collect().await?.to_bytes();
    anyhow::ensure!(!body.is_empty(), "uploaded file is empty");

    let source_id = state.next_id("source");
    let path = upload_path(&source_id, &file_name);
    std::fs::write(&path, &body).with_context(|| format!("failed to write {}", path.display()))?;

    let worksheets = if is_excel(&path) {
        list_worksheets(&path)?
    } else {
        Vec::new()
    };
    let source = if let Some(sheet) = worksheets.first() {
        import_xlsx_sheet(&path, sheet)?
    } else {
        import_source(&path)?
    };

    let stored = StoredSource {
        file_name,
        path,
        worksheets,
        source,
    };
    let response = source_response(&source_id, &stored);
    state
        .inner
        .sources
        .lock()
        .expect("source state poisoned")
        .insert(source_id, stored);

    Ok(json_response(StatusCode::OK, &response)?)
}

async fn worksheet_endpoint(uri: &http::Uri, state: WebState) -> Result<Response<BoxBody>> {
    let path = uri.path();
    let source_id = path
        .trim_start_matches("/api/sources/")
        .trim_end_matches("/worksheet")
        .trim_end_matches('/');
    let query = parse_query(uri.query());
    let sheet = query
        .get("sheet")
        .context("sheet query parameter is required")?;

    let mut sources = state.inner.sources.lock().expect("source state poisoned");
    let stored = sources.get_mut(source_id).context("source not found")?;
    anyhow::ensure!(
        stored.worksheets.iter().any(|candidate| candidate == sheet),
        "worksheet was not found in the uploaded workbook"
    );
    stored.source = import_xlsx_sheet(&stored.path, sheet)?;
    let response = source_response(source_id, stored);
    Ok(json_response(StatusCode::OK, &response)?)
}

async fn start_analysis_endpoint(
    request: Request<Incoming>,
    state: WebState,
) -> Result<Response<BoxBody>> {
    let body = request.into_body().collect().await?.to_bytes();
    let mut payload: StartAnalysisRequest = serde_json::from_slice(&body)?;
    let source = {
        let sources = state.inner.sources.lock().expect("source state poisoned");
        sources
            .get(&payload.source_id)
            .map(|stored| stored.source.clone())
            .context("source not found")?
    };
    validate_mapping(&payload.mapping, &source)?;
    payload.settings.label_terms =
        merge_label_term_policy(&state.inner.config.label_terms, &payload.settings.label_terms);

    let job_id = state.next_id("job");
    let (sender, _) = broadcast::channel(128);
    let job = Arc::new(Mutex::new(AnalysisJob {
        status: JobStatus::Running,
        message: "Analysis worker started.".to_owned(),
        started_at: Instant::now(),
        finished_at: None,
        progress_log: Vec::new(),
        result: None,
        error: None,
        events: sender.clone(),
    }));
    state
        .inner
        .jobs
        .lock()
        .expect("job state poisoned")
        .insert(job_id.clone(), job.clone());

    std::thread::spawn(move || {
        let progress_job = job.clone();
        let result = run_analysis_with_progress(
            source,
            payload.mapping,
            payload.settings,
            move |progress| record_progress(&progress_job, progress),
        );
        record_finished(&job, result);
    });

    Ok(json_response(
        StatusCode::ACCEPTED,
        &StartAnalysisResponse { job_id },
    )?)
}

fn merge_label_term_policy(system: &LabelTermPolicy, run: &LabelTermPolicy) -> LabelTermPolicy {
    LabelTermPolicy {
        boosted: merge_terms(&system.boosted, &run.boosted),
        suppressed: merge_terms(&system.suppressed, &run.suppressed),
        excluded: merge_terms(&system.excluded, &run.excluded),
    }
}

fn merge_terms(left: &[String], right: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::<String>::new();
    left.iter()
        .chain(right)
        .filter_map(|term| {
            let term = term.trim();
            if term.is_empty() || !seen.insert(term.to_owned()) {
                None
            } else {
                Some(term.to_owned())
            }
        })
        .collect()
}

async fn restore_session_endpoint(
    request: Request<Incoming>,
    state: WebState,
) -> Result<Response<BoxBody>> {
    let body = request.into_body().collect().await?.to_bytes();
    let payload: RestoreSessionRequest = serde_json::from_slice(&body)?;
    anyhow::ensure!(
        !payload.run.source.headers.is_empty(),
        "session source headers are missing"
    );

    let job_id = state.next_id("job");
    let (sender, _) = broadcast::channel(1);
    let summary = analysis_summary(&payload.run);
    let job = Arc::new(Mutex::new(AnalysisJob {
        status: JobStatus::Finished,
        message: format!(
            "Session loaded: {} clusters, {} ignored rows.",
            summary.clusters, summary.ignored_rows
        ),
        started_at: Instant::now(),
        finished_at: Some(Instant::now()),
        progress_log: Vec::new(),
        result: Some(payload.run),
        error: None,
        events: sender,
    }));
    state
        .inner
        .jobs
        .lock()
        .expect("job state poisoned")
        .insert(job_id.clone(), job);

    Ok(json_response(
        StatusCode::OK,
        &StartAnalysisResponse { job_id },
    )?)
}

fn events_endpoint(path: &str, state: WebState) -> Result<Response<BoxBody>> {
    let job_id = path
        .trim_start_matches("/api/jobs/")
        .trim_end_matches("/events")
        .trim_end_matches('/');
    let (snapshot, receiver) = {
        let jobs = state.inner.jobs.lock().expect("job state poisoned");
        let job = jobs.get(job_id).context("job not found")?;
        let job = job.lock().expect("job state poisoned");
        (snapshot_from_job(&job), job.events.subscribe())
    };

    let status_kind = snapshot.status_kind().to_owned();
    let elapsed_ms = snapshot.elapsed_ms;
    let message = snapshot.message.clone();
    let initial_events = snapshot.progress_log.into_iter().chain(
        snapshot
            .result_summary
            .clone()
            .map(|summary| ProgressEvent {
                kind: status_kind,
                elapsed_ms,
                message,
                progress: None,
                result_summary: Some(summary),
            })
            .into_iter(),
    );

    let initial = stream::iter(initial_events.map(sse_frame));
    let live = stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok(event) => Some((sse_frame(event), receiver)),
            Err(broadcast::error::RecvError::Lagged(_)) => Some((
                sse_frame(ProgressEvent {
                    kind: "status".to_owned(),
                    elapsed_ms: 0,
                    message: "Progress stream lagged; latest job state is still available."
                        .to_owned(),
                    progress: None,
                    result_summary: None,
                }),
                receiver,
            )),
            Err(broadcast::error::RecvError::Closed) => None,
        }
    });

    let body = BodyExt::boxed(StreamBody::new(initial.chain(live)));
    let mut response = Response::new(body);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(response)
}

fn job_endpoint(path: &str, state: WebState) -> Result<Response<BoxBody>> {
    let job_id = path.trim_start_matches("/api/jobs/").trim_end_matches('/');
    let jobs = state.inner.jobs.lock().expect("job state poisoned");
    let job = jobs.get(job_id).context("job not found")?;
    let job = job.lock().expect("job state poisoned");
    Ok(json_response(StatusCode::OK, &snapshot_from_job(&job))?)
}

fn result_endpoint(path: &str, state: WebState) -> Result<Response<BoxBody>> {
    let job_id = path
        .trim_start_matches("/api/jobs/")
        .trim_end_matches("/result")
        .trim_end_matches('/');
    let jobs = state.inner.jobs.lock().expect("job state poisoned");
    let job = jobs.get(job_id).context("job not found")?;
    let job = job.lock().expect("job state poisoned");
    let result = job
        .result
        .as_ref()
        .context("analysis result is not ready")?;
    Ok(json_response(StatusCode::OK, result)?)
}

async fn pivot_endpoint(
    path: &str,
    request: Request<Incoming>,
    state: WebState,
) -> Result<Response<BoxBody>> {
    let job_id = path
        .trim_start_matches("/api/jobs/")
        .trim_end_matches("/pivot")
        .trim_end_matches('/');
    let body = request.into_body().collect().await?.to_bytes();
    let payload: PivotRequest = serde_json::from_slice(&body)?;
    let response = {
        let jobs = state.inner.jobs.lock().expect("job state poisoned");
        let job = jobs.get(job_id).context("job not found")?;
        let job = job.lock().expect("job state poisoned");
        let analysis = job
            .result
            .as_ref()
            .context("analysis result is not ready for pivot")?;
        build_pivot_response(analysis, payload)?
    };
    Ok(json_response(StatusCode::OK, &response)?)
}

async fn export_endpoint(path: &str, state: WebState) -> Result<Response<BoxBody>> {
    let job_id = path
        .trim_start_matches("/api/jobs/")
        .trim_end_matches("/export")
        .trim_end_matches('/');
    let analysis = {
        let jobs = state.inner.jobs.lock().expect("job state poisoned");
        let job = jobs.get(job_id).context("job not found")?;
        let job = job.lock().expect("job state poisoned");
        job.result
            .clone()
            .context("analysis result is not ready for export")?
    };
    let path = std::env::temp_dir().join(format!("incident-clustering-{job_id}.xlsx"));
    export_analysis(&analysis, &path)?;
    let bytes = std::fs::read(&path)?;
    let _ = std::fs::remove_file(&path);

    let mut response = Response::new(full_body(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"clustered_incidents.xlsx\""),
    );
    Ok(response)
}

fn build_pivot_response(analysis: &AnalysisRun, request: PivotRequest) -> Result<PivotResponse> {
    let column_count = analysis.source.headers.len();
    validate_pivot_columns(&request.row_columns, column_count)?;
    validate_pivot_columns(&request.column_columns, column_count)?;
    let row_columns = request.row_columns;
    let column_columns = request
        .column_columns
        .into_iter()
        .filter(|column| !row_columns.contains(column))
        .collect::<Vec<_>>();
    let mut pivot = PivotAccumulator::default();

    for row_index in request.row_indices.iter().copied() {
        let Some(row) = analysis.source.rows.get(row_index) else {
            continue;
        };
        let row_key = pivot_key(row, &row_columns, "Count");
        let column_key = pivot_key(row, &column_columns, "Count");
        pivot.add(row_index, row_key, column_key);
    }

    Ok(pivot.into_response(
        request.row_indices.len(),
        &analysis.source.headers,
        &row_columns,
        &column_columns,
    ))
}

fn validate_pivot_columns(columns: &[usize], column_count: usize) -> Result<()> {
    for column in columns {
        anyhow::ensure!(
            *column < column_count,
            "pivot column index {column} is out of range"
        );
    }
    Ok(())
}

#[derive(Default)]
struct PivotAccumulator {
    row_keys: BTreeSet<Vec<String>>,
    column_keys: BTreeSet<Vec<String>>,
    counts: BTreeMap<(Vec<String>, Vec<String>), usize>,
    row_totals: BTreeMap<Vec<String>, usize>,
    row_prefix_totals: BTreeMap<Vec<String>, usize>,
    column_totals: BTreeMap<Vec<String>, usize>,
    row_members: BTreeMap<Vec<String>, Vec<usize>>,
    all_members: Vec<usize>,
    grand_total: usize,
}

impl PivotAccumulator {
    fn add(&mut self, row_index: usize, row_key: Vec<String>, column_key: Vec<String>) {
        self.row_keys.insert(row_key.clone());
        self.column_keys.insert(column_key.clone());
        *self.counts.entry((row_key.clone(), column_key.clone())).or_default() += 1;
        *self.row_totals.entry(row_key.clone()).or_default() += 1;
        for prefix_len in 1..=row_key.len() {
            *self
                .row_prefix_totals
                .entry(row_key[..prefix_len].to_vec())
                .or_default() += 1;
        }
        *self.column_totals.entry(column_key).or_default() += 1;
        self.row_members.entry(row_key).or_default().push(row_index);
        self.all_members.push(row_index);
        self.grand_total += 1;
    }

    fn into_response(
        mut self,
        _record_count: usize,
        headers: &[String],
        row_columns: &[usize],
        column_columns: &[usize],
    ) -> PivotResponse {
        if self.row_keys.is_empty() {
            self.row_keys.insert(vec!["Count".to_owned()]);
        }
        if self.column_keys.is_empty() {
            self.column_keys.insert(vec!["Count".to_owned()]);
        }

        let mut row_keys = self.row_keys.into_iter().collect::<Vec<_>>();
        row_keys.sort_by(|left, right| {
            compare_pivot_row_order(
                left,
                right,
                &self.row_prefix_totals,
                &self.row_totals,
            )
        });
        let mut column_keys = self.column_keys.into_iter().collect::<Vec<_>>();
        column_keys.sort_by(|left, right| {
            self.column_totals
                .get(right)
                .copied()
                .unwrap_or_default()
                .cmp(&self.column_totals.get(left).copied().unwrap_or_default())
                .then_with(|| left.cmp(right))
        });
        let mut response_headers = pivot_headers(headers, row_columns, column_columns, &column_keys);
        let numeric_start = if row_columns.is_empty() { 1 } else { row_columns.len() };
        let mut numeric_columns = (numeric_start..response_headers.len()).collect::<Vec<_>>();
        if column_columns.is_empty() && !response_headers.is_empty() {
            numeric_columns = vec![response_headers.len() - 1];
        }

        let mut rows = Vec::with_capacity(row_keys.len() + usize::from(!row_columns.is_empty()));
        let mut previous_row_key: Option<&[String]> = None;
        for row_key in &row_keys {
            let mut cells = display_row_key(row_key, previous_row_key, row_columns.is_empty());
            for column_key in &column_keys {
                cells.push(
                    self.counts
                        .get(&(row_key.clone(), column_key.clone()))
                        .copied()
                        .unwrap_or_default()
                        .to_string(),
                );
            }
            if !column_columns.is_empty() {
                cells.push(self.row_totals.get(row_key).copied().unwrap_or_default().to_string());
            }
            rows.push(PivotResponseRow {
                cells,
                total: false,
                row_indices: self
                    .row_members
                    .get(row_key)
                    .cloned()
                    .unwrap_or_default(),
            });
            previous_row_key = Some(row_key);
        }

        if !row_columns.is_empty() {
            let mut total_cells = vec![String::new(); row_columns.len().saturating_sub(1)];
            total_cells.insert(0, "Total".to_owned());
            if column_columns.is_empty() {
                total_cells.push(self.grand_total.to_string());
            } else {
                for column_key in &column_keys {
                    total_cells.push(
                        self.column_totals
                            .get(column_key)
                            .copied()
                            .unwrap_or_default()
                            .to_string(),
                    );
                }
                total_cells.push(self.grand_total.to_string());
            }
            rows.push(PivotResponseRow {
                cells: total_cells,
                total: true,
                row_indices: self.all_members.clone(),
            });
        }

        if response_headers.is_empty() {
            response_headers.push("Count".to_owned());
            numeric_columns.push(0);
        }

        PivotResponse {
            record_count: self.grand_total,
            headers: response_headers,
            rows,
            numeric_columns,
        }
    }
}

fn pivot_headers(
    headers: &[String],
    row_columns: &[usize],
    column_columns: &[usize],
    column_keys: &[Vec<String>],
) -> Vec<String> {
    let mut response_headers = if row_columns.is_empty() {
        vec!["Summary".to_owned()]
    } else {
        row_columns
            .iter()
            .map(|column| headers[*column].clone())
            .collect::<Vec<_>>()
    };
    if column_columns.is_empty() {
        response_headers.push("Count".to_owned());
    } else {
        response_headers.extend(column_keys.iter().map(|key| key.join(" / ")));
        response_headers.push("Total".to_owned());
    }
    response_headers
}

fn pivot_key(row: &[String], columns: &[usize], fallback: &str) -> Vec<String> {
    if columns.is_empty() {
        return vec![fallback.to_owned()];
    }
    columns
        .iter()
        .map(|column| {
            row.get(*column)
                .map(|value| {
                    let value = value.trim();
                    if value.is_empty() {
                        "(blank)".to_owned()
                    } else {
                        value.to_owned()
                    }
                })
                .unwrap_or_else(|| "(blank)".to_owned())
        })
        .collect()
}

fn compare_pivot_row_order(
    left: &[String],
    right: &[String],
    prefix_totals: &BTreeMap<Vec<String>, usize>,
    row_totals: &BTreeMap<Vec<String>, usize>,
) -> std::cmp::Ordering {
    let common_len = left.len().min(right.len());
    for index in 0..common_len {
        let left_prefix = &left[..=index];
        let right_prefix = &right[..=index];
        if left_prefix == right_prefix {
            continue;
        }
        let left_total = prefix_totals
            .get(left_prefix)
            .copied()
            .unwrap_or_default();
        let right_total = prefix_totals
            .get(right_prefix)
            .copied()
            .unwrap_or_default();
        return right_total
            .cmp(&left_total)
            .then_with(|| left_prefix.cmp(right_prefix));
    }

    row_totals
        .get(right)
        .copied()
        .unwrap_or_default()
        .cmp(&row_totals.get(left).copied().unwrap_or_default())
        .then_with(|| left.cmp(right))
}

fn display_row_key(
    row_key: &[String],
    previous_row_key: Option<&[String]>,
    summary_row: bool,
) -> Vec<String> {
    if summary_row {
        return vec![row_key.join(" / ")];
    }
    row_key
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let repeated_parent_path = previous_row_key.is_some_and(|previous| {
                previous.len() > index
                    && previous[..=index]
                        .iter()
                        .zip(&row_key[..=index])
                        .all(|(left, right)| left == right)
            });
            if repeated_parent_path
            {
                String::new()
            } else {
                value.clone()
            }
        })
        .collect()
}

fn record_progress(job: &Arc<Mutex<AnalysisJob>>, progress: ProgressUpdate) {
    let mut job = job.lock().expect("job state poisoned");
    let event = ProgressEvent {
        kind: "progress".to_owned(),
        elapsed_ms: job.started_at.elapsed().as_millis(),
        message: format!("{}: {}", progress.stage, progress.detail),
        progress: Some(progress),
        result_summary: None,
    };
    job.message = event.message.clone();
    job.progress_log.push(event.clone());
    if job.progress_log.len() > 80 {
        job.progress_log.remove(0);
    }
    let _ = job.events.send(event);
}

fn record_finished(job: &Arc<Mutex<AnalysisJob>>, result: Result<AnalysisRun>) {
    let mut job = job.lock().expect("job state poisoned");
    job.finished_at = Some(Instant::now());
    match result {
        Ok(run) => {
            let summary = analysis_summary(&run);
            job.status = JobStatus::Finished;
            job.message = format!(
                "Analysis complete: {} clusters, {} ignored rows.",
                summary.clusters, summary.ignored_rows
            );
            job.result = Some(run);
            let _ = job.events.send(ProgressEvent {
                kind: "finished".to_owned(),
                elapsed_ms: elapsed_ms(&job),
                message: job.message.clone(),
                progress: None,
                result_summary: Some(summary),
            });
        }
        Err(err) => {
            job.status = JobStatus::Failed;
            job.message = format!("Analysis failed: {err}");
            job.error = Some(err.to_string());
            let _ = job.events.send(ProgressEvent {
                kind: "failed".to_owned(),
                elapsed_ms: elapsed_ms(&job),
                message: job.message.clone(),
                progress: None,
                result_summary: None,
            });
        }
    }
}

fn snapshot_from_job(job: &AnalysisJob) -> JobSnapshot {
    JobSnapshot {
        status: job.status,
        message: job.message.clone(),
        elapsed_ms: elapsed_ms(job),
        progress_log: job.progress_log.clone(),
        result_summary: job.result.as_ref().map(analysis_summary),
        error: job.error.clone(),
    }
}

impl JobSnapshot {
    fn status_kind(&self) -> &'static str {
        match self.status {
            JobStatus::Running => "status",
            JobStatus::Finished => "finished",
            JobStatus::Failed => "failed",
        }
    }
}

fn source_response(source_id: &str, stored: &StoredSource) -> SourceResponse {
    SourceResponse {
        source_id: source_id.to_owned(),
        file_name: stored.file_name.clone(),
        worksheets: stored.worksheets.clone(),
        selected_worksheet: stored.source.worksheet_name.clone(),
        headers: stored.source.headers.clone(),
        row_count: stored.source.row_count(),
        preview_rows: stored.source.rows.iter().take(50).cloned().collect(),
        suggested_mapping: suggest_mapping(&stored.source.headers),
    }
}

fn analysis_summary(run: &AnalysisRun) -> AnalysisSummary {
    AnalysisSummary {
        clusters: run.clusters.len(),
        processed_incidents: run.processed_incidents.len(),
        ignored_rows: run.ignored_rows.len(),
        unclustered_incidents: run.unclustered_row_indices.len(),
    }
}

fn elapsed_ms(job: &AnalysisJob) -> u128 {
    job.finished_at
        .map(|finished_at| finished_at.duration_since(job.started_at))
        .unwrap_or_else(|| job.started_at.elapsed())
        .as_millis()
}

impl WebState {
    fn next_id(&self, prefix: &str) -> String {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{prefix}-{id}")
    }
}

fn parse_query(query: Option<&str>) -> HashMap<String, String> {
    form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .into_owned()
        .collect()
}

fn upload_path(source_id: &str, file_name: &str) -> PathBuf {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("dat");
    std::env::temp_dir().join(format!("incident-clustering-{source_id}.{extension}"))
}

fn is_excel(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("xlsx" | "xlsm" | "xls")
    )
}

fn sse_frame(event: ProgressEvent) -> Result<Frame<Bytes>, Infallible> {
    let json = serde_json::to_string(&event).unwrap_or_else(|err| {
        format!(
            "{{\"kind\":\"failed\",\"elapsedMs\":0,\"message\":\"failed to serialize event: {err}\",\"progress\":null,\"resultSummary\":null}}"
        )
    });
    Ok(Frame::data(Bytes::from(format!("data: {json}\n\n"))))
}

fn json_response<T: Serialize>(status: StatusCode, value: &T) -> Result<Response<BoxBody>> {
    let body = serde_json::to_vec(value)?;
    let mut response = Response::builder()
        .status(status)
        .body(full_body(body))
        .expect("response builder should be valid");
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    Ok(response)
}

fn json_error(status: StatusCode, error: impl Into<String>) -> Response<BoxBody> {
    json_response(
        status,
        &ErrorResponse {
            error: error.into(),
        },
    )
    .expect("error response should serialize")
}

fn text_response(
    status: StatusCode,
    content_type: &'static str,
    body: &'static str,
) -> Response<BoxBody> {
    let mut response = Response::builder()
        .status(status)
        .body(full_body(body))
        .expect("response builder should be valid");
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn full_body(body: impl Into<Bytes>) -> BoxBody {
    Full::new(body.into()).boxed()
}
