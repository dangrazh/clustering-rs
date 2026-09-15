use super::*;
use crate::session::{self, ReviewData, ViewState};
use crate::workflow::Mutation;
use http_body_util::Limited;

#[cfg(test)]
use crate::fixtures;

pub(super) fn cluster_workbook(
    run: &AnalysisRun,
    review: &ReviewData,
    request: ClusterViewExportRequest,
) -> Result<Vec<u8>> {
    cluster_workbook_collaborative(run, review, request, None)
}
pub(super) fn cluster_workbook_collaborative(
    run: &AnalysisRun,
    review: &ReviewData,
    request: ClusterViewExportRequest,
    metadata: Option<&crate::storage::PortableMeta>,
) -> Result<Vec<u8>> {
    let selected = request
        .drilldown_row_indices
        .map(|r| r.into_iter().collect::<HashSet<_>>());
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name("Cluster View")?;
    let headers = [
        "Level",
        "Cluster ID",
        "Cluster Label",
        "Theme ID",
        "Theme Label",
        "Count",
        "State",
        "Inherited from",
        "SME email",
        "Implementation owner",
        "ETA",
        "Context only",
        "Cluster reviewer",
    ];
    for (column, label) in headers.iter().enumerate() {
        sheet.write_string(0, column as u16, *label)?;
    }
    let mut keys = Vec::new();
    let mut row = 1;
    for cluster in &run.clusters {
        for theme in std::iter::once(None).chain(cluster.subgroups.iter().map(Some)) {
            let key = theme.map_or_else(
                || cluster.id.0.to_string(),
                |t| format!("{}:{}", cluster.id.0, t.id),
            );
            let rows = theme.map_or(&cluster.incident_row_indices, |t| &t.incident_row_indices);
            let count = filtered_export_count(rows, selected.as_ref());
            if selected.is_some() && count == 0 {
                continue;
            }
            let workflow = review.annotations.effective(&key)?;
            let inherited = review.annotations.entries[&key].workflow.is_none();
            let context = !request.workflow_states.is_empty()
                && !request.workflow_states.contains(&workflow.status);
            if theme.is_some() && context {
                continue;
            }
            let values = vec![
                if theme.is_some() {
                    "Theme".into()
                } else {
                    "Cluster".into()
                },
                cluster.id.to_string(),
                review
                    .annotations
                    .label(&cluster.id.0.to_string(), &cluster.label)
                    .to_owned(),
                theme.map(|t| t.id.to_string()).unwrap_or_default(),
                theme
                    .map(|t| {
                        review
                            .annotations
                            .label(&format!("{}:{}", cluster.id.0, t.id), &t.label)
                            .to_owned()
                    })
                    .unwrap_or_default(),
                count.to_string(),
                workflow.status.label().into(),
                if inherited {
                    cluster.id.to_string()
                } else {
                    String::new()
                },
                workflow.assignments.sme_email.clone().unwrap_or_default(),
                workflow.assignments.owner_email.clone().unwrap_or_default(),
                workflow
                    .assignments
                    .eta
                    .map(|d| d.to_string())
                    .unwrap_or_default(),
                if context { "Yes".into() } else { "No".into() },
                metadata
                    .and_then(|m| m.reviewers.get(&cluster.id.0.to_string()))
                    .and_then(|r| {
                        r.recorded.as_ref().map(|a| {
                            format!(
                                "{} <{}>{}",
                                a.name,
                                a.email,
                                if r.unconfirmed { " (unconfirmed)" } else { "" }
                            )
                        })
                    })
                    .unwrap_or_default(),
            ];
            for (column, value) in values.iter().enumerate() {
                if column == 5 {
                    sheet.write_number(row, column as u16, count as f64)?;
                } else {
                    sheet.write_string(row, column as u16, truncate_for_excel(value))?;
                }
            }
            keys.push(key);
            row += 1;
        }
    }
    let sheet = workbook.add_worksheet();
    sheet.set_name("Comments")?;
    for (column, value) in [
        "Entity",
        "Comment ID",
        "Author",
        "Email",
        "Created (UTC)",
        "Updated (UTC)",
        "Part",
        "Text",
    ]
    .iter()
    .enumerate()
    {
        sheet.write_string(0, column as u16, *value)?;
    }
    let mut row = 1;
    for key in &keys {
        for comment in &review.annotations.entries[key].comments {
            for (part, text) in excel_chunks(&comment.text).into_iter().enumerate() {
                let values = [
                    key.as_str(),
                    &comment.id,
                    &comment.author.name,
                    &comment.author.email,
                    &comment.created_at,
                    comment.updated_at.as_deref().unwrap_or(""),
                    &(part + 1).to_string(),
                    &text,
                ];
                for (column, value) in values.iter().enumerate() {
                    sheet.write_string(row, column as u16, *value)?;
                }
                row += 1;
            }
        }
    }
    let sheet = workbook.add_worksheet();
    sheet.set_name("Workflow History")?;
    for (column, value) in [
        "Entity",
        "Source entity",
        "Event ID",
        "Author",
        "Email",
        "Timestamp (UTC)",
        "Action",
        "Part",
        "Before",
        "After",
    ]
    .iter()
    .enumerate()
    {
        sheet.write_string(0, column as u16, *value)?;
    }
    let mut row = 1;
    for key in &keys {
        let entry = &review.annotations.entries[key];
        let sources = std::iter::once(key.as_str()).chain(if entry.workflow.is_none() {
            key.split_once(':').map(|(p, _)| p)
        } else {
            None
        });
        for source in sources {
            for event in &review.annotations.entries[source].history {
                let before = excel_chunks(&serde_json::to_string(&event.before)?);
                let after = excel_chunks(&serde_json::to_string(&event.after)?);
                for part in 0..before.len().max(after.len()) {
                    let values = [
                        key.as_str(),
                        source,
                        &event.id,
                        &event.actor.name,
                        &event.actor.email,
                        &event.timestamp,
                        &event.action,
                        &(part + 1).to_string(),
                        before.get(part).map(String::as_str).unwrap_or(""),
                        after.get(part).map(String::as_str).unwrap_or(""),
                    ];
                    for (column, value) in values.iter().enumerate() {
                        sheet.write_string(row, column as u16, *value)?;
                    }
                    row += 1;
                }
            }
        }
    }
    if let Some(metadata) = metadata {
        let sheet = workbook.add_worksheet();
        sheet.set_name("Label and Reviewer History")?;
        for (i, h) in [
            "Entity",
            "Actor",
            "Email",
            "Timestamp",
            "Action",
            "Before",
            "After",
            "Part",
        ]
        .iter()
        .enumerate()
        {
            sheet.write_string(0, i as u16, *h)?;
        }
        let mut row = 1;
        for event in &metadata.audit {
            let before = excel_chunks(&event["before"].to_string());
            let after = excel_chunks(&event["after"].to_string());
            for part in 0..before.len().max(after.len()) {
                let values = [
                    event["entity"].as_str().unwrap_or(""),
                    event["actor"]["name"].as_str().unwrap_or(""),
                    event["actor"]["email"].as_str().unwrap_or(""),
                    event["timestamp"].as_str().unwrap_or(""),
                    event["action"].as_str().unwrap_or(""),
                    before.get(part).map(String::as_str).unwrap_or(""),
                    after.get(part).map(String::as_str).unwrap_or(""),
                ];
                for (col, value) in values.iter().enumerate() {
                    sheet.write_string(row, col as u16, *value)?;
                }
                sheet.write_number(row, 7, (part + 1) as f64)?;
                row += 1;
            }
        }
    }
    Ok(workbook.save_to_buffer()?)
}
fn excel_chunks(value: &str) -> Vec<String> {
    let mut result = vec![String::new()];
    let mut units = 0;
    for ch in value.chars() {
        if units + ch.len_utf16() > 30_000 {
            result.push(String::new());
            units = 0;
        }
        result.last_mut().unwrap().push(ch);
        units += ch.len_utf16();
    }
    result
}

pub(super) fn handles(path: &str) -> bool {
    path.starts_with("/api/jobs/")
        && [
            "/review",
            "/review/mutate",
            "/session/save",
            "/incidents/export",
        ]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}
async fn body(request: Request<Incoming>, limit: usize) -> Result<Bytes> {
    Ok(Limited::new(request.into_body(), limit)
        .collect()
        .await
        .map_err(|e| anyhow::anyhow!("Cannot read upload: {e}"))?
        .to_bytes())
}
pub(super) fn find_job(state: &WebState, id: &str) -> Result<Arc<Mutex<AnalysisJob>>> {
    state
        .inner
        .jobs
        .lock()
        .expect("job state poisoned")
        .get(id)
        .cloned()
        .context("Job not found.")
}
pub(super) fn snapshot(
    job: &Arc<Mutex<AnalysisJob>>,
) -> Result<(Arc<AnalysisRun>, ReviewData, ViewState)> {
    let job = job.lock().expect("job state poisoned");
    Ok((
        job.result.clone().context("Analysis is not ready.")?,
        job.review.clone().context("Review data is not ready.")?,
        job.view.clone(),
    ))
}
fn review_response(
    review: &ReviewData,
    view: &ViewState,
    read_only: &HashSet<String>,
) -> Result<Response<BoxBody>> {
    let mut actions = BTreeMap::new();
    for key in review.annotations.entries.keys() {
        let workflow = review.annotations.effective(key)?;
        actions.insert(
            key,
            serde_json::json!({"next":workflow.status.next(),"canUndo":workflow.path.len()>1}),
        );
    }
    let comment_access: serde_json::Map<String, serde_json::Value> = review
        .annotations
        .entries
        .values()
        .flat_map(|e| {
            e.comments.iter().map(|c| {
                (
                    c.id.clone(),
                    serde_json::json!({"canEdit":!read_only.contains(&c.id)}),
                )
            })
        })
        .collect();
    json_response(
        StatusCode::OK,
        &serde_json::json!({"review":review,"view":view,"allowedActions":actions,"commentAccess":comment_access}),
    )
}
fn binary(bytes: Vec<u8>, name: &str) -> Result<Response<BoxBody>> {
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(full_body(bytes))?)
}
pub(super) async fn restore(
    request: Request<Incoming>,
    state: WebState,
) -> Result<Response<BoxBody>> {
    let bytes = body(request, session::MAX_FILE_BYTES).await?;
    let mut loaded = tokio::task::spawn_blocking(move || session::decode_session(&bytes)).await??;
    for reviewer in loaded.metadata.reviewers.values_mut() {
        reviewer.user_id = None;
        reviewer.unconfirmed = reviewer.recorded.is_some();
    }
    let job_id = state.next_id("job");
    let artifact = state
        .inner
        .artifacts
        .publish(
            Arc::new(loaded.run.clone()),
            loaded.review.clone(),
            loaded.view.clone(),
            loaded.metadata.clone(),
        )
        .await?;
    crate::jobs::retain_import(
        &state.inner.store,
        state.user.as_ref().context("Sign in required")?,
        &job_id,
        &artifact,
    )
    .await?;
    crate::jobs::save_summary(
        &state.inner.store,
        &job_id,
        crate::jobs::source_summary(
            &loaded.run.source,
            Some(loaded.run.processed_incidents.len()),
        ),
    )
    .await?;
    let (sender, _) = broadcast::channel(1);
    let job = AnalysisJob {
        owner: state.user.as_ref().context("Sign in required")?.id.clone(),
        central: false,
        publishing: false,
        imported_comments: loaded
            .review
            .annotations
            .entries
            .values()
            .flat_map(|e| e.comments.iter().map(|c| c.id.clone()))
            .collect(),
        metadata: loaded.metadata,
        status: JobStatus::Finished,
        message: "Binary session loaded.".into(),
        started_at: Instant::now(),
        finished_at: Some(Instant::now()),
        progress_log: vec![],
        result: Some(Arc::new(loaded.run)),
        review: Some(loaded.review),
        view: loaded.view,
        error: None,
        events: sender,
    };
    state
        .inner
        .jobs
        .lock()
        .expect("job state poisoned")
        .insert(job_id.clone(), Arc::new(Mutex::new(job)));
    json_response(StatusCode::OK, &StartAnalysisResponse { job_id })
}
pub(super) async fn route(
    path: &str,
    request: Request<Incoming>,
    state: WebState,
) -> Result<Response<BoxBody>> {
    let tail = path.trim_start_matches("/api/jobs/");
    let (id, operation) = tail.split_once('/').context("Invalid job path.")?;
    let job = find_job(&state, id)?;
    if request.method() == Method::GET && operation == "review" {
        let (_, review, view) = snapshot(&job)?;
        return review_response(&review, &view, &job.lock().unwrap().imported_comments);
    }
    anyhow::ensure!(
        request.method() == Method::POST,
        "Use POST for this operation."
    );
    match operation {
        "review/mutate" => {
            let mut mutation: Mutation =
                serde_json::from_slice(&body(request, 2 * 1024 * 1024).await?)?;
            let mut locked = job.lock().expect("job state poisoned");
            anyhow::ensure!(
                !locked.central && !locked.publishing,
                "Analysis is being centrally saved. Retry shortly."
            );
            mutation.actor = state.user.as_ref().context("Sign in required")?.actor();
            if let crate::workflow::Action::EditComment { id, .. }
            | crate::workflow::Action::DeleteComment { id } = &mutation.action
            {
                anyhow::ensure!(
                    !locked.imported_comments.contains(id),
                    "Imported comments are read-only."
                );
            }
            let review = locked.review.as_mut().context("Review data not ready.")?;
            if review.annotations.revision != mutation.revision {
                return Ok(json_error(
                    StatusCode::CONFLICT,
                    "Review data changed. Reload review data and retry.",
                ));
            }
            review.annotations.mutate(mutation)?;
            review_response(
                locked.review.as_ref().unwrap(),
                &locked.view,
                &locked.imported_comments,
            )
        }
        "session/save" => {
            let mut view: ViewState =
                serde_json::from_slice(&body(request, 16 * 1024 * 1024).await?)?;
            let (run, review, _) = snapshot(&job)?;
            view.sanitize(&run);
            let metadata = job.lock().unwrap().metadata.clone();
            let bytes = tokio::task::spawn_blocking(move || {
                session::encode_portable(&run, &review, &view, &metadata)
            })
            .await??;
            binary(bytes, "incident_analysis.icas")
        }
        "incidents/export" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Export {
                row_indices: Vec<usize>,
            }
            let payload: Export = serde_json::from_slice(&body(request, 16 * 1024 * 1024).await?)?;
            let (run, review, _) = snapshot(&job)?;
            let bytes = tokio::task::spawn_blocking(move || {
                crate::io::export_analysis_bytes_with_labels(
                    &run,
                    Some(&payload.row_indices),
                    Some(&review.annotations),
                )
            })
            .await??;
            excel_response(bytes, "clustered_incidents.xlsx")
        }
        _ => Ok(json_error(
            StatusCode::NOT_FOUND,
            "Unknown review operation.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::*;
    use calamine::{Reader, Xlsx};
    #[test]
    fn workbook_contains_context_inheritance_comments_and_history() {
        let run = fixtures::run(10);
        let mut review = ReviewData::new(&run).unwrap();
        let actor = Actor {
            name: "Reviewer".into(),
            email: "reviewer@example.org".into(),
        };
        for action in [
            Action::Transition {
                status: ReviewStatus::Reviewed,
                assignments: Assignments::default(),
            },
            Action::AddComment {
                text: "🛠".repeat(40_000),
            },
        ] {
            review
                .annotations
                .mutate(Mutation {
                    revision: review.annotations.revision,
                    target: "1:1".into(),
                    actor: actor.clone(),
                    action,
                })
                .unwrap();
        }
        review
            .annotations
            .labels
            .insert("1".into(), "Edited cluster".into());
        review
            .annotations
            .labels
            .insert("1:1".into(), "Edited theme".into());
        let bytes = cluster_workbook(
            &run,
            &review,
            ClusterViewExportRequest {
                drilldown_row_indices: Some(vec![0, 2]),
                workflow_states: vec![ReviewStatus::Reviewed],
            },
        )
        .unwrap();
        let mut book: Xlsx<_> =
            calamine::open_workbook_from_rs(std::io::Cursor::new(bytes)).unwrap();
        let tree = book.worksheet_range("Cluster View").unwrap();
        assert_eq!(tree.height(), 3);
        assert_eq!(
            tree.get_value((1, 2)).unwrap().to_string(),
            "Edited cluster"
        );
        assert_eq!(tree.get_value((2, 4)).unwrap().to_string(), "Edited theme");
        assert_eq!(tree.get_value((1, 11)).unwrap().to_string(), "Yes");
        assert_eq!(tree.get_value((2, 6)).unwrap().to_string(), "Reviewed");
        let comments = book.worksheet_range("Comments").unwrap();
        let text: String = comments.rows().skip(1).map(|r| r[7].to_string()).collect();
        assert_eq!(text, "🛠".repeat(40_000));
        let history = book.worksheet_range("Workflow History").unwrap();
        assert_eq!(history.height(), 2);
        assert_eq!(history.get_value((1, 6)).unwrap().to_string(), "transition");
        let incidents = crate::io::export_analysis_bytes_with_labels(
            &run,
            Some(&[0, 2]),
            Some(&review.annotations),
        )
        .unwrap();
        let mut book: Xlsx<_> =
            calamine::open_workbook_from_rs(std::io::Cursor::new(incidents)).unwrap();
        let sheet = book.worksheet_range_at(0).unwrap().unwrap();
        assert_eq!(sheet.height(), 3);
        assert_eq!(
            sheet.get_value((1, 4)).unwrap().to_string(),
            "Edited cluster"
        );
        assert_eq!(sheet.get_value((1, 7)).unwrap().to_string(), "Edited theme");
        assert_eq!(sheet.width(), run.source.headers.len() + 6);
    }
}
