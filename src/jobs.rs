//! Durable FIFO queue. Only the server writes the database; children receive file paths.
use crate::{
    model::{ColumnMapping, RunSettings, SourceTable},
    session::{self, ReviewData, ViewState},
    storage::{id, now, Store, User},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufWriter, Write},
    path::Path,
    sync::Arc,
};
use turso::params;
#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;

/// Reclaim only while starting, before accepting requests or launching workers.
pub async fn startup_cleanup(store: &Store) -> Result<()> {
    let c = store.connect().await?;
    c.execute(
        "DELETE FROM jobs WHERE expires<=? AND state IN ('finished','failed','cancelled')",
        [now()],
    )
    .await?;
    c.execute("DELETE FROM sessions WHERE expires<=?", [now()])
        .await?;
    let mut refs = std::collections::HashSet::new();
    for sql in [
        "SELECT artifact FROM analyses",
        "SELECT artifact FROM jobs WHERE artifact IS NOT NULL",
        "SELECT input FROM jobs WHERE input IS NOT NULL",
    ] {
        let mut rows = c.query(sql, ()).await?;
        while let Some(r) = rows.next().await? {
            refs.insert(r.get::<String>(0)?);
        }
    }
    for sub in ["artifacts", "inputs", "staging"] {
        for file in std::fs::read_dir(store.root.join(sub))? {
            let file = file?;
            if !file.file_type()?.is_file() {
                continue;
            }
            let name = file.file_name().to_string_lossy().into_owned();
            if refs.contains(&name) {
                continue;
            }
            if file.metadata()?.modified()?.elapsed().unwrap_or_default()
                > std::time::Duration::from_secs(86400)
            {
                std::fs::remove_file(file.path())?;
            }
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct Input {
    pub source: SourceTable,
    pub mapping: ColumnMapping,
    pub settings: RunSettings,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub owner: String,
    pub state: String,
    pub input: Option<String>,
    pub artifact: Option<String>,
    pub metadata: serde_json::Value,
    pub created: i64,
    pub expires: Option<i64>,
    pub error: Option<String>,
}
pub async fn enqueue(store: &Arc<Store>, user: &User, input: Input) -> Result<String> {
    let metadata = serde_json::json!({"summary": source_summary(&input.source, None)});
    let job_id = format!("job-{}", id());
    let name = format!("{job_id}.json");
    let path = store.root.join("inputs").join(&name);
    tokio::task::spawn_blocking(move || {
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, &input)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        crate::artifacts::sync_directory(path.parent().unwrap())?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    let _m = store.maintenance.read().await;
    let lock = store.lock("job-queue");
    let _g = lock.write().await;
    let _p = store.writers.acquire().await?;
    let c = store.connect().await?;
    ensure!(
        crate::storage::scalar(
            &c,
            "SELECT COUNT(*) FROM jobs WHERE state IN ('queued','running')"
        )
        .await?
            < (crate::config::number("APP_MAX_QUEUED_JOBS", 10, 1, 100)? as i64),
        "The clustering queue is full. Retry after a job completes."
    );
    // Preserve acceptance order even when multiple submissions share a millisecond.
    let created = chrono::Utc::now()
        .timestamp_millis()
        .max(crate::storage::scalar(&c, "SELECT COALESCE(MAX(created),0)+1 FROM jobs").await?);
    c.execute(
        "INSERT INTO jobs(id,owner,state,input,metadata,created) VALUES(?,?,'queued',?,?,?)",
        params![
            job_id.clone(),
            user.id.clone(),
            name,
            metadata.to_string(),
            created
        ],
    )
    .await?;
    Ok(job_id)
}
pub fn source_summary(source: &SourceTable, processed_rows: Option<usize>) -> serde_json::Value {
    let filename = source
        .source_path
        .as_ref()
        .and_then(|p| p.to_str())
        .and_then(|p| p.rsplit(['/', '\\']).next())
        .filter(|name| !name.is_empty() && !name.starts_with("upload-source-"));
    serde_json::json!({"sourceFileName":filename,"sourceRows":source.rows.len(),"processedRows":processed_rows,"columnCount":source.headers.len()})
}
pub async fn save_summary(
    store: &Store,
    jid: &str,
    summary: serde_json::Value,
) -> Result<serde_json::Value> {
    let _m = store.maintenance.read().await;
    // Initial central saving also updates job metadata under the catalog guard.
    let catalog = store.lock("catalog");
    let _catalog = catalog.read().await;
    let lock = store.lock(&format!("job:{jid}"));
    let _g = lock.write().await;
    let _p = store.writers.acquire().await?;
    let job = get(store, jid).await?;
    let mut metadata = job.metadata;
    if metadata["summary"]["processedRows"].is_number() && summary["processedRows"].is_null() {
        return Ok(metadata);
    }
    metadata["summary"] = summary;
    let c = store.connect().await?;
    c.execute(
        "UPDATE jobs SET metadata=? WHERE id=?",
        params![metadata.to_string(), jid],
    )
    .await?;
    Ok(metadata)
}
pub async fn list(store: &Store, owner: &str) -> Result<Vec<Job>> {
    let c = store.connect().await?;
    let mut rows=c.query("SELECT id,owner,state,input,artifact,metadata,created,expires,error FROM jobs WHERE owner=? AND (expires IS NULL OR expires>?) ORDER BY created DESC",params![owner,now()]).await?;
    let mut jobs = vec![];
    while let Some(r) = rows.next().await? {
        jobs.push(row(r)?);
    }
    Ok(jobs)
}
fn optional(row: &turso::Row, col: usize) -> Result<Option<String>> {
    Ok(match row.get_value(col)? {
        turso::Value::Text(s) => Some(s),
        _ => None,
    })
}
fn row(r: turso::Row) -> Result<Job> {
    Ok(Job {
        id: r.get(0)?,
        owner: r.get(1)?,
        state: r.get(2)?,
        input: optional(&r, 3)?,
        artifact: optional(&r, 4)?,
        metadata: serde_json::from_str(&r.get::<String>(5)?)?,
        created: r.get(6)?,
        expires: match r.get_value(7)? {
            turso::Value::Integer(v) => Some(v),
            _ => None,
        },
        error: optional(&r, 8)?,
    })
}
pub async fn get(store: &Store, jid: &str) -> Result<Job> {
    let c = store.connect().await?;
    let r=c.query("SELECT id,owner,state,input,artifact,metadata,created,expires,error FROM jobs WHERE id=?",[jid]).await?.next().await?.context("Job not found")?;
    row(r)
}
pub async fn next(store: &Store) -> Result<Option<Job>> {
    let c = store.connect().await?;
    let r=c.query("SELECT id,owner,state,input,artifact,metadata,created,expires,error FROM jobs WHERE state='queued' ORDER BY created,id LIMIT 1",()).await?.next().await?;
    r.map(row).transpose()
}
pub async fn change(
    store: &Store,
    jid: &str,
    from: &str,
    to: &str,
    artifact: Option<&str>,
    error: Option<&str>,
) -> Result<bool> {
    let _m = store.maintenance.read().await;
    let lock = store.lock(&format!("job:{jid}"));
    let _g = lock.write().await;
    let _p = store.writers.acquire().await?;
    let c = store.connect().await?;
    let expires = if matches!(to, "finished" | "failed" | "cancelled") {
        Some(now() + retention_seconds()?)
    } else {
        None
    };
    Ok(c.execute("UPDATE jobs SET state=?,artifact=COALESCE(?,artifact),error=?,expires=? WHERE id=? AND state=?",params![to,artifact,error,expires,jid,from]).await?!=0)
}
pub async fn cancel(store: &Store, user: &User, jid: &str) -> Result<()> {
    let job = get(store, jid).await?;
    ensure!(job.owner == user.id, "Only the owner can cancel a job");
    ensure!(
        matches!(job.state.as_str(), "queued" | "running"),
        "Job is no longer cancellable"
    );
    let _m = store.maintenance.read().await;
    let lock = store.lock(&format!("job:{jid}"));
    let _g = lock.write().await;
    let _p = store.writers.acquire().await?;
    let c = store.connect().await?;
    ensure!(c.execute("UPDATE jobs SET state='cancelled',error='Cancelled by owner',expires=? WHERE id=? AND owner=? AND state IN ('queued','running')",params![now()+retention_seconds()?,jid,user.id.clone()]).await?!=0,"Job finished before cancellation");
    Ok(())
}
pub async fn interrupted(store: &Store) -> Result<()> {
    let _p = store.writers.acquire().await?;
    let c = store.connect().await?;
    c.execute("UPDATE jobs SET state='failed',error='Interrupted by application restart. Resubmit explicitly.',expires=? WHERE state='running'",[now()+retention_seconds()?]).await?;
    Ok(())
}
pub async fn retain_import(store: &Store, user: &User, jid: &str, artifact: &str) -> Result<()> {
    let _m = store.maintenance.read().await;
    let _p = store.writers.acquire().await?;
    let c = store.connect().await?;
    c.execute("INSERT INTO jobs(id,owner,state,artifact,metadata,created,expires) VALUES(?,?,'finished',?,'{\"imported\":true}',?,?)",params![jid,user.id.clone(),artifact,chrono::Utc::now().timestamp_millis(),now()+retention_seconds()?]).await?;
    Ok(())
}
pub fn worker(input: &Path, output: &Path) -> Result<()> {
    ensure!(
        std::fs::metadata(input)?.len() <= 2 * 1024 * 1024 * 1024,
        "Worker input too large"
    );
    let input: Input =
        serde_json::from_reader(std::io::BufReader::new(std::fs::File::open(input)?))?;
    let run = crate::worker::run_analysis_with_progress(
        input.source,
        input.mapping,
        input.settings,
        |progress| {
            if let Ok(line) = serde_json::to_string(&progress) {
                println!("{line}");
            }
        },
    )?;
    let review = ReviewData::new(&run)?;
    let bytes =
        session::encode_portable(&run, &review, &ViewState::default(), &Default::default())?;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    crate::artifacts::sync_directory(output.parent().context("Output has no parent")?)?;
    Ok(())
}

fn retention_seconds() -> Result<i64> {
    Ok(crate::config::number("APP_JOB_RETENTION_DAYS", 7, 1, 365)? as i64 * 86400)
}
