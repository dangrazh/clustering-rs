use super::*;
use crate::{
    storage::{
        self,
        trends::{public_snapshot, Task},
    },
    trends::{self, Action, Command},
};
use serde_json::json;

pub(super) async fn route(
    request: Request<Incoming>,
    state: WebState,
) -> Result<Response<BoxBody>> {
    let uri = request.uri().clone();
    let method = request.method().clone();
    let path = uri.path();
    let query = parse_query(uri.query());
    let user = state.user.as_ref().context("Sign in required")?;
    let store = &state.inner.store;
    let offset = query
        .get("offset")
        .map(|s| s.parse::<i64>())
        .transpose()?
        .unwrap_or(0);
    let value = if path == "/api/series/memberships" && method == Method::GET {
        store.trend_memberships().await?
    } else if path == "/api/series" {
        if method == Method::GET {
            store
                .series_catalog(
                    query.get("search").map(String::as_str).unwrap_or(""),
                    query.get("archived").is_some_and(|v| v == "true"),
                    offset,
                )
                .await?
        } else {
            anyhow::ensure!(method == Method::POST, "Unsupported method");
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Create {
                command_id: String,
                name: String,
            }
            let v: Create = shared::body(request).await?;
            store.create_series(user, &v.command_id, &v.name).await?
        }
    } else {
        let parts: Vec<_> = path.trim_start_matches("/api/series/").split('/').collect();
        let sid = parts[0];
        match (method, parts.get(1).copied().unwrap_or("")) {
            (Method::POST, "delete") => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Delete {
                    command_id: String,
                    expected: u64,
                }
                let v: Delete = shared::body(request).await?;
                store
                    .delete_series(user, sid, &v.command_id, v.expected)
                    .await?
            }
            (Method::GET, "") => public_snapshot(&store.series(sid).await?),
            (Method::GET, "trends") => {
                anyhow::ensure!((0..=1_000_000).contains(&offset), "Invalid offset");
                let s = store.series(sid).await?;
                let rows: Vec<_> = s
                    .problems
                    .iter()
                    .skip(offset as usize)
                    .take(100)
                    .map(|p| json!({"problem":p,"cells":trends::cells(&s,&p.id)}))
                    .collect();
                json!({"revision":s.revision,"rows":rows,"nextOffset":if offset as usize+100<s.problems.len(){Some(offset+100)}else{None}})
            }
            (Method::GET, "history") => store.trend_history(sid, offset).await?,
            (Method::GET, "tasks") if parts.len() == 2 => store.trend_tasks(sid).await?,
            (Method::GET, "tasks" | "previews") => {
                let t = store
                    .trend_task(parts.get(2).context("Missing task ID")?)
                    .await?;
                anyhow::ensure!(t.series == sid, "Task does not belong to series");
                json!({"id":t.id,"state":t.state,"progress":t.progress,"error":t.error,"expected":t.command.expected,"expires":t.expires,"action":t.command.action,"proposed":t.proposed.as_ref().map(public_snapshot)})
            }
            (Method::POST, "commands") => {
                let c: Command = shared::body(request).await?;
                store.series_command(user, sid, &c).await?
            }
            (Method::POST, "tasks") if parts.get(3) == Some(&"retry") => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Retry {
                    command_id: String,
                    expected: u64,
                }
                let payload: Retry = shared::body(request).await?;
                let task = store.trend_task(parts[2]).await?;
                anyhow::ensure!(
                    task.series == sid && matches!(task.state.as_str(), "failed" | "preview"),
                    "This task cannot be retried"
                );
                store
                    .series_command(
                        user,
                        sid,
                        &Command {
                            command_id: payload.command_id,
                            expected: payload.expected,
                            action: task.command.action,
                        },
                    )
                    .await?
            }
            (Method::POST, "previews") if parts.get(3) == Some(&"apply") => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Apply {
                    command_id: String,
                }
                let v: Apply = shared::body(request).await?;
                store
                    .apply_trend_preview(user, sid, parts[2], &v.command_id)
                    .await?
            }
            _ => anyhow::bail!("Unknown series operation"),
        }
    };
    json_response(StatusCode::OK, &value)
}

pub(super) async fn supervise(state: WebState) {
    if let Err(error) = state.inner.store.recover_trend_tasks().await {
        tracing::error!(%error,"Trend recovery failed");
        return;
    }
    loop {
        let result = async {
            // A previous iteration may have lost its final write during temporary storage failure.
            state.inner.store.recover_trend_tasks().await?;
            if let Some(mut task) = state.inner.store.next_trend_task().await? {
                task.state = "running".into();
                task.progress = "Preparing source profiles".into();
                state.inner.store.save_trend_task(&task).await?;
                match compute(&state, &mut task).await {
                    Ok(()) => {}
                    Err(error) => {
                        task.state = "failed".into();
                        task.error = Some(error.to_string());
                        task.progress = "No changes saved. Refresh and retry the operation.".into();
                    }
                }
                state.inner.store.save_trend_task(&task).await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            tracing::error!(%error,"Trend worker will retry");
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
pub(super) async fn compute(state: &WebState, task: &mut Task) -> Result<()> {
    let store = &state.inner.store;
    let mut s = store.series(&task.series).await?;
    anyhow::ensure!(
        s.revision == task.command.expected && !s.archived,
        "Series changed. Refresh and retry the operation."
    );
    if let Some(model) = s.model.as_mut() {
        model.automatic = true;
    }
    if let Action::AddPeriod {
        analysis,
        start,
        end,
        replace,
    } = &task.command.action
    {
        trends::validate_dates(start, end)?;
        let info = store.analysis(analysis).await?;
        let gate = store.lock("heavy-computation");
        let _permit = gate.write().await;
        let source = state.inner.artifacts.load(&info.artifact).await?;
        trends::validate_source(&source.run)?;
        if s.model.is_none() {
            let run = source.run.clone();
            let fingerprint = info.fingerprint.clone();
            // Strong automatic matching approved in ER-07; thresholds and safeguards remain.
            let automatic = true;
            s.model = Some(
                tokio::task::spawn_blocking(move || {
                    let mut m = trends::fit(&run, fingerprint, automatic);
                    trends::configure_model(&mut m)?;
                    Ok::<_, anyhow::Error>(m)
                })
                .await??,
            );
        }
        let run = source.run.clone();
        let model = s.model.clone().context("Missing model")?;
        let ps = tokio::task::spawn_blocking(move || trends::profiles(&run, &model)).await?;
        let key = profile_key(&info.fingerprint, s.model.as_ref().unwrap())?;
        store.save_trend_profile(&key, &ps).await?;
        let mut period = trends::period(
            info.id,
            info.name,
            info.fingerprint,
            start.clone(),
            end.clone(),
            source.run.processed_incidents.len(),
            &ps,
        );
        for (cluster, label) in store.trend_source_labels(&period.analysis).await? {
            if let Some(o) = period.occurrences.iter_mut().find(|o| o.cluster == cluster) {
                o.label = label;
            }
        }
        if let Some(pid) = replace {
            let old = s
                .periods
                .iter()
                .find(|p| &p.id == pid)
                .context("Replacement period not found")?;
            anyhow::ensure!(
                old.start == *start && old.end == *end,
                "Replacement must retain the period dates"
            );
            anyhow::ensure!(
                old.analysis != period.analysis,
                "Choose a different replacement analysis"
            );
            period.id = old.id.clone();
            for o in &mut period.occurrences {
                o.replacement = true;
            }
            s.periods.retain(|p| &p.id != pid);
        }
        s.periods.push(period);
        s.periods.sort_by(|a, b| a.start.cmp(&b.start));
        s.validate()?;
    } else {
        s.apply(&task.command.action, &task.user)?;
    }
    if s.periods.is_empty() {
        task.proposed = Some(s);
        task.state = "preview".into();
        task.progress = "Review and confirm changes".into();
        return Ok(());
    }
    if matches!(task.command.action, Action::RebuildModel) {
        let earliest = s
            .periods
            .first()
            .context("Add a period before rebuilding the model")?;
        let gate = store.lock("heavy-computation");
        let _permit = gate.write().await;
        let info = store.analysis(&earliest.analysis).await?;
        let run = state
            .inner
            .artifacts
            .load(&info.artifact)
            .await?
            .run
            .clone();
        let fingerprint = info.fingerprint;
        s.model = Some(
            tokio::task::spawn_blocking(move || {
                let mut m = trends::fit(&run, fingerprint, true);
                trends::configure_model(&mut m)?;
                Ok::<_, anyhow::Error>(m)
            })
            .await??,
        );
    }
    let model = s.model.clone().context("Missing model")?;
    let mut all = BTreeMap::new();
    for (i, p) in s.periods.iter().enumerate() {
        task.progress = format!("Preparing period {} of {}", i + 1, s.periods.len());
        store.save_trend_task(task).await?;
        let key = profile_key(&p.fingerprint, &model)?;
        let profiles = if let Some(profiles) = store.trend_profile(&key).await? {
            profiles
        } else {
            let gate = store.lock("heavy-computation");
            let _permit = gate.write().await;
            let info = store.analysis(&p.analysis).await?;
            anyhow::ensure!(
                info.fingerprint == p.fingerprint,
                "Source fingerprint changed"
            );
            let source = state.inner.artifacts.load(&info.artifact).await?;
            trends::validate_source(&source.run)?;
            let run = source.run.clone();
            let model = model.clone();
            let profiles =
                tokio::task::spawn_blocking(move || trends::profiles(&run, &model)).await?;
            store.save_trend_profile(&key, &profiles).await?;
            profiles
        };
        all.insert(p.fingerprint.clone(), profiles);
    }
    task.progress = "Comparing periods and preparing impact preview".into();
    store.save_trend_task(task).await?;
    let gate = store.lock("heavy-computation");
    let _permit = gate.write().await;
    let proposed = tokio::task::spawn_blocking(move || {
        trends::recalculate(&mut s, &all)?;
        Ok::<_, anyhow::Error>(s)
    })
    .await??;
    anyhow::ensure!(
        store.series(&task.series).await?.revision == task.command.expected,
        "Series changed during matching. Refresh and retry."
    );
    task.proposed = Some(proposed);
    task.state = "preview".into();
    task.progress = "Review and confirm changes".into();
    task.expires = storage::now() + 86400;
    Ok(())
}
fn profile_key(fingerprint: &str, model: &trends::Model) -> Result<String> {
    Ok(storage::digest(&serde_json::to_vec(&(fingerprint, model))?))
}
