use super::*;
use crate::jobs;
use tokio::io::{AsyncBufReadExt, BufReader};

pub(super) fn empty(owner: String, message: &str) -> AnalysisJob {
    let (events, _) = broadcast::channel(128);
    AnalysisJob {
        owner,
        central: false,
        publishing: false,
        metadata: Default::default(),
        imported_comments: Default::default(),
        status: JobStatus::Running,
        message: message.into(),
        started_at: Instant::now(),
        finished_at: None,
        progress_log: vec![],
        result: None,
        review: None,
        view: Default::default(),
        error: None,
        events,
    }
}
pub(super) async fn prepare(state: &WebState, jid: &str) -> Result<Arc<Mutex<AnalysisJob>>> {
    trim_results(state, jid);
    let user = state.user.as_ref().context("Sign in required")?;
    let record = jobs::get(&state.inner.store, jid).await?;
    if record.owner != user.id || record.expires.is_some_and(|e| e <= crate::storage::now()) {
        return Err(crate::storage::AppError::Forbidden.into());
    }
    if let Some(existing) = state.inner.jobs.lock().unwrap().get(jid) {
        let pin = existing.clone();
        let mut existing = existing.lock().unwrap();
        anyhow::ensure!(existing.owner == user.id, "Job owner mismatch");
        if matches!(record.state.as_str(), "failed" | "cancelled")
            && existing.status != JobStatus::Failed
        {
            existing.status = JobStatus::Failed;
            existing.message = record.error.clone().unwrap_or(record.state.clone());
            existing.error = record.error.clone();
            existing.finished_at = Some(Instant::now());
        }
        if record.state != "finished" || existing.result.is_some() {
            return Ok(pin);
        }
    }
    let mut job = empty(record.owner.clone(), &record.state);
    job.central = record.metadata["analysisId"].is_string();
    if record.state == "finished" {
        let artifact = state
            .inner
            .artifacts
            .load(record.artifact.as_deref().context("Job artifact missing")?)
            .await?;
        job.status = JobStatus::Finished;
        job.finished_at = Some(Instant::now());
        job.result = Some(artifact.run.clone());
        job.review = Some(artifact.review.clone());
        job.view = artifact.view.clone();
        job.metadata = artifact.metadata.clone();
        if record.metadata["imported"] == true {
            job.imported_comments = artifact
                .review
                .annotations
                .entries
                .values()
                .flat_map(|e| e.comments.iter().map(|c| c.id.clone()))
                .collect();
            for reviewer in job.metadata.reviewers.values_mut() {
                reviewer.user_id = None;
                reviewer.unconfirmed = reviewer.recorded.is_some();
            }
        }
    } else if matches!(record.state.as_str(), "failed" | "cancelled") {
        job.status = JobStatus::Failed;
        job.finished_at = Some(Instant::now());
        job.error = record.error;
    }
    let mut cache = state.inner.jobs.lock().unwrap();
    if let Some(existing) = cache.get(jid) {
        existing.lock().unwrap().result = job.result;
    } else {
        cache.insert(jid.into(), Arc::new(Mutex::new(job)));
    }
    Ok(cache.get(jid).unwrap().clone())
}
fn trim_results(state: &WebState, keep: &str) {
    let cache = state.inner.jobs.lock().unwrap();
    while cache
        .values()
        .filter(|j| j.lock().unwrap().result.is_some())
        .count()
        >= 5
    {
        let victim = cache
            .iter()
            .filter(|(id, j)| {
                id.as_str() != keep && Arc::strong_count(j) == 1 && {
                    let j = j.lock().unwrap();
                    j.result.is_some() && j.finished_at.is_some() && !j.publishing
                }
            })
            .min_by_key(|(_, j)| j.lock().unwrap().finished_at);
        let Some((_, job)) = victim else {
            break;
        };
        // Keep temporary review drafts in memory; only immutable data is reloadable.
        job.lock().unwrap().result = None;
    }
}
pub(super) async fn supervise(state: WebState) {
    loop {
        let result = async {
            if let Some(job) = jobs::next(&state.inner.store).await? {
                if jobs::change(&state.inner.store, &job.id, "queued", "running", None, None)
                    .await?
                {
                    if let Err(error) = run_one(&state, &job).await {
                        jobs::change(
                            &state.inner.store,
                            &job.id,
                            "running",
                            "failed",
                            None,
                            Some(&error.to_string()),
                        )
                        .await?;
                        if let Some(live) = state.inner.jobs.lock().unwrap().get(&job.id).cloned() {
                            record_finished(&live, Err(error));
                        }
                    }
                }
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            tracing::error!(%error,"Clustering supervisor will retry");
        }
        trim_results(&state, "");
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
async fn run_one(state: &WebState, job: &jobs::Job) -> Result<()> {
    run_child(state, job, std::env::current_exe()?).await
}
pub(super) async fn run_child(
    state: &WebState,
    job: &jobs::Job,
    executable: PathBuf,
) -> Result<()> {
    let input = job.input.as_deref().context("Queued job input missing")?;
    anyhow::ensure!(
        input == format!("{}.json", job.id),
        "Invalid job input reference"
    );
    let output = state
        .inner
        .store
        .root
        .join("staging")
        .join(format!("{}.icas", crate::storage::id()));
    let mut child = tokio::process::Command::new(executable)
        .env_clear()
        .envs(std::env::vars_os().filter(|(name, _)| {
            // In particular, no Entra credentials or server session configuration.
            matches!(
                name.to_string_lossy().to_ascii_uppercase().as_str(),
                "PATH"
                    | "SYSTEMROOT"
                    | "WINDIR"
                    | "TEMP"
                    | "TMP"
                    | "TMPDIR"
                    | "APP_WORKER_THREADS"
                    | "RUST_LOG"
            )
        }))
        .arg("worker")
        .arg(state.inner.store.root.join("inputs").join(input))
        .arg(&output)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    let mut lines =
        BufReader::new(child.stdout.take().context("Worker output unavailable")?).lines();
    let mut poll = tokio::time::interval(std::time::Duration::from_millis(500));
    let mut output_open = true;
    let started = Instant::now();
    let timeout = std::time::Duration::from_secs(crate::config::number(
        "APP_JOB_TIMEOUT_SECONDS",
        3600,
        60,
        86400,
    )? as u64);
    tracing::info!(job_id = %job.id, child_pid = ?child.id(), "Clustering child started");
    loop {
        tokio::select! {
            _=tokio::time::sleep_until(tokio::time::Instant::from_std(started + timeout))=>{
                child.kill().await?;let _=child.wait().await;
                anyhow::bail!("Clustering exceeded its configured time limit; resubmit explicitly");
            },
            status=child.wait()=>{anyhow::ensure!(status?.success(),"Clustering child failed; inspect server logs");break;},
            line=lines.next_line(),if output_open=>match line?{
                Some(line)=>if let Ok(progress)=serde_json::from_str::<ProgressUpdate>(&line){if let Some(live)=state.inner.jobs.lock().unwrap().get(&job.id).cloned(){record_progress(&live,progress);}},
                None=>output_open=false,
            },
            _=poll.tick()=>if jobs::get(&state.inner.store,&job.id).await?.state=="cancelled"{
                child.kill().await?;let _=child.wait().await;
                if let Some(live)=state.inner.jobs.lock().unwrap().get(&job.id).cloned(){record_finished(&live,Err(anyhow::anyhow!("Cancelled by owner")));}return Ok(());
            },
        }
    }
    let output_copy = output.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        crate::session::decode_session(&std::fs::read(output_copy)?)
    })
    .await??;
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
    jobs::save_summary(
        &state.inner.store,
        &job.id,
        jobs::source_summary(
            &loaded.run.source,
            Some(loaded.run.processed_incidents.len()),
        ),
    )
    .await?;
    if jobs::change(
        &state.inner.store,
        &job.id,
        "running",
        "finished",
        Some(&artifact),
        None,
    )
    .await?
    {
        if let Some(live) = state.inner.jobs.lock().unwrap().get(&job.id).cloned() {
            record_finished(&live, Ok(loaded));
        }
    }
    std::fs::remove_file(output)?;
    tracing::info!(job_id = %job.id, elapsed_ms = started.elapsed().as_millis(), "Clustering child completed");
    Ok(())
}
