use super::*;
use crate::{
    auth::{cookie, Session},
    session::{ReviewData, ViewState},
    storage::{AppError, Command, PortableMeta},
};
use serde_json::json;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Presence {
    user: crate::storage::User,
    analysis_id: String,
    view_id: String,
    cluster: Option<String>,
    pub(super) seen: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Heartbeat {
    view_id: String,
    cluster: Option<String>,
}

pub(super) fn error_response(error: &anyhow::Error) -> Response<BoxBody> {
    if crate::storage::retryable(error) {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "The database is busy. Your pending change can be retried.",
        );
    }
    let status = match error.downcast_ref::<AppError>() {
        Some(AppError::Unauthorized) => StatusCode::UNAUTHORIZED,
        Some(AppError::Forbidden) => StatusCode::FORBIDDEN,
        Some(AppError::Conflict(_) | AppError::Archived) => StatusCode::CONFLICT,
        Some(AppError::Missing(_)) => StatusCode::NOT_FOUND,
        Some(AppError::Busy) => StatusCode::SERVICE_UNAVAILABLE,
        None => StatusCode::BAD_REQUEST,
    };
    json_error(status, error.to_string())
}
pub(super) async fn authorize(request: &Request<Incoming>, state: &mut WebState) -> Result<()> {
    let cookies = request
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let session = state.inner.auth.session(cookies).await?;
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        check_csrf(request, &session)?;
    }
    state.user = Some(session.user);
    Ok(())
}
fn check_csrf(request: &Request<Incoming>, session: &Session) -> Result<()> {
    if request
        .headers()
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        != Some(session.csrf.as_str())
    {
        return Err(AppError::Forbidden.into());
    }
    Ok(())
}
pub(super) async fn auth_route(
    request: Request<Incoming>,
    state: WebState,
) -> Result<Response<BoxBody>> {
    if matches!(request.uri().path(), "/auth/login" | "/login")
        && state.inner.auth.allowlist.is_some()
    {
        if request.method() == Method::GET {
            return Ok(Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store")
                .body(full_body(
                    include_str!("../web_assets/email-login.html")
                        .as_bytes()
                        .to_vec(),
                ))?);
        }
        if request.method() == Method::POST {
            // Custom headers require a same-origin request or an approved CORS preflight.
            // This server does not grant cross-origin login requests.
            if request
                .headers()
                .get("x-requested-with")
                .and_then(|h| h.to_str().ok())
                != Some("same-origin")
            {
                return Err(AppError::Forbidden.into());
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct EmailLogin {
                email: String,
            }
            let bytes = http_body_util::Limited::new(request.into_body(), 4096)
                .collect()
                .await
                .map_err(|_| anyhow::anyhow!("Invalid sign-in request"))?
                .to_bytes();
            let login: EmailLogin = serde_json::from_slice(&bytes)?;
            let cookie = state.inner.auth.email_login(&login.email).await?;
            return Ok(Response::builder()
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CACHE_CONTROL, "no-store")
                .header(header::SET_COOKIE, cookie)
                .body(full_body(b"{\"signedIn\":true}".to_vec()))?);
        }
    }
    let cookies = request
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let (location, cookie_value) = match (request.method(), request.uri().path()) {
        (&Method::GET, "/auth/login" | "/login") => state.inner.auth.login()?,
        (&Method::GET, "/auth/callback") => {
            let query = parse_query(request.uri().query());
            let token = state
                .inner
                .auth
                .callback(
                    query.get("state").context("Missing login state")?,
                    query.get("code").context("Login was not completed")?,
                    cookie(cookies, "login_state").unwrap_or(""),
                )
                .await?;
            ("/".into(), token)
        }
        (&Method::POST, "/auth/logout") => {
            let session = state.inner.auth.session(cookies).await?;
            check_csrf(&request, &session)?;
            state.inner.auth.logout(cookies).await?;
            ("/".into(), state.inner.auth.session_cookie("", 0))
        }
        _ => return Ok(json_error(StatusCode::NOT_FOUND, "Not found")),
    };
    Ok(Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::SET_COOKIE, cookie_value)
        .header(header::CACHE_CONTROL, "no-store")
        .body(full_body(Vec::new()))?)
}
pub(super) fn handles(path: &str) -> bool {
    matches!(
        path,
        "/api/me" | "/api/users" | "/api/analyses" | "/api/changes" | "/api/jobs"
    ) || path.starts_with("/api/analyses/")
        || path.starts_with("/api/jobs/")
            && (path.ends_with("/save-central")
                || path.ends_with("/cancel")
                || path.ends_with("/resubmit")
                || path.ends_with("/review/save")
                || path.ends_with("/review/prepare")
                || path.ends_with("/review/commit"))
}
pub(super) async fn prepare(request: &Request<Incoming>, state: &mut WebState) -> Result<()> {
    let path = request.uri().path();
    let user = state.user.as_ref().context("Sign in required")?;
    if let Some(tail) = path.strip_prefix("/api/sources/") {
        let sid = tail.split('/').next().unwrap();
        let sources = state.inner.sources.lock().unwrap();
        if sources.get(sid).is_none_or(|s| s.owner != user.id) {
            return Err(AppError::Forbidden.into());
        }
    }
    if let Some(tail) = path.strip_prefix("/api/jobs/") {
        let jid = tail.split('/').next().unwrap();
        state.request_job = Some(jobs_runtime::prepare(state, jid).await?);
    }
    Ok(())
}
async fn body<T: serde::de::DeserializeOwned>(request: Request<Incoming>) -> Result<T> {
    let bytes = http_body_util::Limited::new(request.into_body(), 16 * 1024 * 1024)
        .collect()
        .await
        .map_err(|e| anyhow::anyhow!("Request too large or incomplete: {e}"))?
        .to_bytes();
    Ok(serde_json::from_slice(&bytes)?)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitialSave {
    command_id: String,
    name: String,
    view: ViewState,
}
pub(super) async fn route(
    request: Request<Incoming>,
    state: WebState,
) -> Result<Response<BoxBody>> {
    let uri = request.uri().clone();
    let path = uri.path();
    let method = request.method().clone();
    let user = state.user.as_ref().context("Sign in required")?.clone();
    let store = &state.inner.store;
    match (method.clone(), path) {
        (Method::GET, "/api/me") => {
            let session = state
                .inner
                .auth
                .session(
                    request
                        .headers()
                        .get(header::COOKIE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or(""),
                )
                .await?;
            return json_response(StatusCode::OK, &session);
        }
        (Method::GET, "/api/users") => return json_response(StatusCode::OK, &store.users().await?),
        (Method::GET, "/api/jobs") => {
            let mut jobs = crate::jobs::list(store, &user.id).await?;
            for job in &mut jobs {
                if job.metadata["summary"].is_object() {
                    continue;
                }
                // Older retained jobs acquire their summary once, without changing artifacts.
                let summary = if let Some(name) = &job.artifact {
                    state.inner.artifacts.load(name).await.map(|a| {
                        crate::jobs::source_summary(
                            &a.run.source,
                            Some(a.run.processed_incidents.len()),
                        )
                    })
                } else if let Some(name) = &job.input {
                    if name != &format!("{}.json", job.id) {
                        continue;
                    }
                    let path = store.root.join("inputs").join(name);
                    tokio::task::spawn_blocking(move || {
                        anyhow::ensure!(
                            std::fs::metadata(&path)?.len() <= 2 * 1024 * 1024 * 1024,
                            "Job input too large"
                        );
                        let input: crate::jobs::Input = serde_json::from_reader(
                            std::io::BufReader::new(std::fs::File::open(path)?),
                        )?;
                        Ok::<_, anyhow::Error>(crate::jobs::source_summary(&input.source, None))
                    })
                    .await?
                } else {
                    continue;
                };
                if let Ok(summary) = summary {
                    job.metadata = crate::jobs::save_summary(store, &job.id, summary).await?;
                }
            }
            return json_response(StatusCode::OK, &jobs);
        }
        (Method::GET, "/api/analyses") => {
            let query = parse_query(uri.query());
            return json_response(
                StatusCode::OK,
                &store
                    .list(
                        query.get("search").map(String::as_str).unwrap_or(""),
                        query.get("archived").is_some_and(|s| s == "true"),
                    )
                    .await?,
            );
        }
        (Method::GET, "/api/changes") => return changes(request, state).await,
        _ => {}
    }
    if let Some(tail) = path.strip_prefix("/api/jobs/") {
        let (jid, operation) = tail.split_once('/').context("Invalid job route")?;
        if operation == "cancel" && method == Method::POST {
            crate::jobs::cancel(store, &user, jid).await?;
            return json_response(StatusCode::OK, &json!({"cancelled":true}));
        }
        if operation == "resubmit" && method == Method::POST {
            let old = crate::jobs::get(store, jid).await?;
            anyhow::ensure!(
                matches!(old.state.as_str(), "failed" | "cancelled"),
                "Only failed or cancelled jobs can be resubmitted"
            );
            let input = old.input.context("This job has no clustering input")?;
            anyhow::ensure!(input == format!("{jid}.json"), "Invalid input reference");
            let path = store.root.join("inputs").join(input);
            let input = tokio::task::spawn_blocking(move || {
                serde_json::from_reader::<_, crate::jobs::Input>(std::io::BufReader::new(
                    std::fs::File::open(path)?,
                ))
                .map_err(anyhow::Error::from)
            })
            .await??;
            let job_id = crate::jobs::enqueue(store, &user, input).await?;
            return json_response(StatusCode::ACCEPTED, &StartAnalysisResponse { job_id });
        }
        anyhow::ensure!(
            operation == "save-central" && method == Method::POST,
            "Review replacement is no longer supported"
        );
        let payload: InitialSave = body(request).await?;
        let job = review_api::find_job(&state, jid)?;
        let (run, review, meta) = {
            let mut locked = job.lock().unwrap();
            anyhow::ensure!(!locked.publishing, "Initial save already in progress");
            let run = locked
                .result
                .clone()
                .context("Clustering is not complete")?;
            let review = locked.review.clone().context("Review is not ready")?;
            locked.publishing = true;
            let mut meta = locked.metadata.clone();
            meta.read_only_comments = locked.imported_comments.iter().cloned().collect();
            meta.source_job = Some(jid.into());
            (run, review, meta)
        };
        // Keep publication alive when the browser disconnects. The user can retry the same command.
        let state = state.clone();
        let jid = jid.to_owned();
        let aid = tokio::spawn(async move {
            let result = async {
                let mut view = payload.view;
                view.sanitize(&run);
                let artifact = state
                    .inner
                    .artifacts
                    .publish(run, review.clone(), view.clone(), meta.clone())
                    .await?;
                state
                    .inner
                    .store
                    .create(
                        &user,
                        &payload.command_id,
                        &payload.name,
                        &artifact,
                        crate::storage::InitialReview {
                            review: &review,
                            view: &view,
                            metadata: &meta,
                        },
                    )
                    .await
            }
            .await;
            if let Ok(job) = review_api::find_job(&state, &jid) {
                let mut locked = job.lock().unwrap();
                locked.publishing = false;
                if result.is_ok() {
                    locked.central = true;
                }
            }
            result
        })
        .await??;
        return json_response(StatusCode::CREATED, &json!({"id":aid}));
    }
    let tail = path
        .strip_prefix("/api/analyses/")
        .context("Unknown route")?;
    let (aid, operation) = tail.split_once('/').unwrap_or((tail, "review"));
    match (method, operation) {
        (Method::POST, "presence") => {
            let heartbeat: Heartbeat = body(request).await?;
            uuid::Uuid::parse_str(&heartbeat.view_id)?;
            let info = store.analysis(aid).await?;
            if let Some(cluster) = &heartbeat.cluster {
                let loaded = state.inner.artifacts.load(&info.artifact).await?;
                anyhow::ensure!(
                    loaded
                        .run
                        .clusters
                        .iter()
                        .any(|c| c.id.0.to_string() == *cluster),
                    "Cluster not found"
                );
            }
            let mut presence = state.inner.presence.lock().unwrap();
            let now = crate::storage::now();
            presence.retain(|_, p| now - p.seen < 60);
            presence.insert(
                format!("{}:{}", user.id, heartbeat.view_id),
                Presence {
                    user,
                    analysis_id: aid.into(),
                    view_id: heartbeat.view_id,
                    cluster: heartbeat.cluster,
                    seen: now,
                },
            );
            json_response(
                StatusCode::OK,
                &presence
                    .values()
                    .filter(|p| p.analysis_id == aid)
                    .collect::<Vec<_>>(),
            )
        }
        (Method::GET, "review") => json_response(StatusCode::OK, &store.review(aid, &user).await?),
        (Method::GET, "result") => {
            let info = store.analysis(aid).await?;
            let artifact = state.inner.artifacts.load(&info.artifact).await?;
            let bytes =
                tokio::task::spawn_blocking(move || serde_json::to_vec(&artifact.run)).await??;
            Ok(Response::builder()
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CACHE_CONTROL, "no-store")
                .body(full_body(bytes))?)
        }
        (Method::POST, "commands") => {
            let cmd: Command = body(request).await?;
            match store.command(aid.into(), user.clone(), cmd).await {
                Ok(outcome) => json_response(
                    StatusCode::OK,
                    &json!({"outcome":outcome,"snapshot":store.review(aid,&user).await?}),
                ),
                Err(e)
                    if matches!(
                        e.downcast_ref::<AppError>(),
                        Some(AppError::Conflict(_) | AppError::Archived)
                    ) =>
                {
                    json_response(
                        StatusCode::CONFLICT,
                        &json!({"error":e.to_string(),"snapshot":store.review(aid,&user).await?}),
                    )
                }
                Err(e) => Err(e),
            }
        }
        (Method::POST, "view") => {
            let mut view: ViewState = body(request).await?;
            let info = store.analysis(aid).await?;
            let artifact = state.inner.artifacts.load(&info.artifact).await?;
            view.sanitize(&artifact.run);
            store.save_view(aid, &user, &view).await?;
            json_response(StatusCode::OK, &json!({"saved":true}))
        }
        (Method::POST, "incidents/export") => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Export {
                row_indices: Vec<usize>,
            }
            let payload: Export = body(request).await?;
            let snapshot = store.review(aid, &user).await?;
            let info = store.analysis(aid).await?;
            let artifact = state.inner.artifacts.load(&info.artifact).await?;
            let review: ReviewData = serde_json::from_value(snapshot["review"].clone())?;
            let bytes = tokio::task::spawn_blocking(move || {
                crate::io::export_analysis_bytes_with_labels(
                    &artifact.run,
                    Some(&payload.row_indices),
                    Some(&review.annotations),
                )
            })
            .await??;
            excel_response(bytes, "clustered_incidents.xlsx")
        }
        (Method::POST, "cluster-view/export") => {
            let payload: ClusterViewExportRequest = body(request).await?;
            let snapshot = store.review(aid, &user).await?;
            let info = store.analysis(aid).await?;
            let artifact = state.inner.artifacts.load(&info.artifact).await?;
            let review: ReviewData = serde_json::from_value(snapshot["review"].clone())?;
            let metadata = portable(&snapshot)?;
            let bytes = tokio::task::spawn_blocking(move || {
                review_api::cluster_workbook_collaborative(
                    &artifact.run,
                    &review,
                    payload,
                    Some(&metadata),
                )
            })
            .await??;
            excel_response(bytes, "cluster_view.xlsx")
        }
        (Method::POST, "pivot" | "pivot/export") => {
            let payload: PivotRequest = body(request).await?;
            let info = store.analysis(aid).await?;
            let artifact = state.inner.artifacts.load(&info.artifact).await?;
            let pivot =
                tokio::task::spawn_blocking(move || build_pivot_response(&artifact.run, payload))
                    .await??;
            if operation == "pivot" {
                json_response(StatusCode::OK, &pivot)
            } else {
                let bytes =
                    tokio::task::spawn_blocking(move || build_pivot_workbook(&pivot)).await??;
                excel_response(bytes, "pivot.xlsx")
            }
        }
        (Method::POST, "session/save") => {
            let mut view: ViewState = body(request).await?;
            let snapshot = store.review(aid, &user).await?;
            let info = store.analysis(aid).await?;
            let artifact = state.inner.artifacts.load(&info.artifact).await?;
            view.sanitize(&artifact.run);
            let review: ReviewData = serde_json::from_value(snapshot["review"].clone())?;
            let metadata = portable(&snapshot)?;
            let bytes = tokio::task::spawn_blocking(move || {
                crate::session::encode_portable(&artifact.run, &review, &view, &metadata)
            })
            .await??;
            Ok(Response::builder()
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"incident_analysis.icas\"",
                )
                .body(full_body(bytes))?)
        }
        _ => Ok(json_error(StatusCode::NOT_FOUND, "Not found")),
    }
}
fn portable(snapshot: &serde_json::Value) -> Result<PortableMeta> {
    let mut meta = PortableMeta::default();
    if let Some(reviewers) = snapshot["reviewers"].as_object() {
        for (key, value) in reviewers {
            meta.reviewers
                .insert(key.clone(), serde_json::from_value(value["value"].clone())?);
        }
    }
    meta.audit = serde_json::from_value(snapshot["audit"].clone())?;
    Ok(meta)
}
async fn changes(request: Request<Incoming>, state: WebState) -> Result<Response<BoxBody>> {
    let cookies = request
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let previous = request
        .headers()
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v >= 0);
    let current = state.inner.store.delivery_cursor().await?;
    let after = previous.filter(|v| *v <= current).unwrap_or(current);
    let ready = stream::once(async move {
        Ok::<_, Infallible>(Frame::data(Bytes::from(format!(
            "id: {after}\nevent: ready\ndata: {{}}\n\n"
        ))))
    });
    let stream = stream::unfold(
        (state, cookies, after),
        |(state, cookies, mut after)| async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if state.inner.auth.session(&cookies).await.is_err() {
                return None;
            }
            let result = state.inner.store.events(after).await;
            let mut frame = String::new();
            match result {
                Ok(events) => {
                    for event in events {
                        after = event["sequence"].as_i64().unwrap();
                        frame.push_str(&format!("id: {after}\ndata: {event}\n\n"));
                    }
                }
                Err(error) => tracing::warn!(%error,"Change delivery will retry"),
            }
            if frame.is_empty() {
                frame = ": heartbeat\n\n".into();
            }
            Some((
                Ok::<_, Infallible>(Frame::data(Bytes::from(frame))),
                (state, cookies, after),
            ))
        },
    );
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache, no-transform")
        .header("x-accel-buffering", "no")
        .body(BodyExt::boxed(StreamBody::new(ready.chain(stream))))?)
}
pub(super) fn start_publisher(store: Arc<crate::storage::Store>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(error) = store.publish().await {
                tracing::warn!(%error,"Durable publication will retry");
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
}
