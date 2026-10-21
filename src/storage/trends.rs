//! Atomic series snapshots and durable, explicitly confirmed recalculation proposals.
use super::*;
use crate::trends::{Action, Command as TrendCommand, Profile, Series};

pub(super) async fn migrate(c: &Connection) -> Result<()> {
    if scalar(c, "SELECT version FROM schema_version").await? >= 3 {
        return Ok(());
    }
    execute(c, "BEGIN").await?;
    let result=async {
        c.execute_batch("CREATE TABLE trend_series(id TEXT PRIMARY KEY, name TEXT NOT NULL, normalized_name TEXT NOT NULL UNIQUE, archived INTEGER NOT NULL, revision INTEGER NOT NULL, value TEXT NOT NULL);
        CREATE TABLE trend_membership(analysis TEXT PRIMARY KEY REFERENCES analyses(id), series TEXT NOT NULL REFERENCES trend_series(id), period TEXT NOT NULL);
        CREATE INDEX trend_membership_series ON trend_membership(series);
        CREATE TABLE trend_profiles(cache_key TEXT PRIMARY KEY, value BLOB NOT NULL);
        CREATE TABLE trend_tasks(id TEXT PRIMARY KEY, series TEXT NOT NULL REFERENCES trend_series(id), state TEXT NOT NULL, created INTEGER NOT NULL, value TEXT NOT NULL);
        CREATE INDEX trend_task_queue ON trend_tasks(state,created);
        CREATE TABLE trend_history(id TEXT PRIMARY KEY, series TEXT NOT NULL REFERENCES trend_series(id), created INTEGER NOT NULL, value TEXT NOT NULL);
        CREATE INDEX trend_history_series ON trend_history(series,created);
        CREATE TABLE outbox_v3(id TEXT PRIMARY KEY, analysis TEXT, value TEXT NOT NULL, published INTEGER NOT NULL DEFAULT 0, resource_kind TEXT NOT NULL, resource_id TEXT NOT NULL);
        INSERT INTO outbox_v3 SELECT id,analysis,value,published,'analysis',analysis FROM outbox;
        DROP TABLE outbox;
        ALTER TABLE outbox_v3 RENAME TO outbox;
        CREATE INDEX outbox_analysis ON outbox(analysis);
        CREATE TABLE deliveries_v3(sequence INTEGER PRIMARY KEY, outbox_id TEXT NOT NULL UNIQUE, analysis TEXT, value TEXT NOT NULL, resource_kind TEXT NOT NULL, resource_id TEXT NOT NULL);
        INSERT INTO deliveries_v3 SELECT sequence,outbox_id,analysis,value,'analysis',analysis FROM deliveries;
        DROP TABLE deliveries;
        ALTER TABLE deliveries_v3 RENAME TO deliveries;
        CREATE INDEX delivery_analysis ON deliveries(analysis,sequence);
        UPDATE schema_version SET version=3;").await?;
        Ok(())
    }.await;
    finish(c, result).await
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub series: String,
    pub user: User,
    pub command: TrendCommand,
    pub state: String,
    pub progress: String,
    pub error: Option<String>,
    pub proposed: Option<Series>,
    pub expires: i64,
}
pub async fn read(c: &Connection, sid: &str) -> Result<Series> {
    let row = c
        .query("SELECT value FROM trend_series WHERE id=?", [sid])
        .await?
        .next()
        .await?
        .ok_or_else(|| AppError::Missing("Series not found".into()))?;
    Ok(serde_json::from_str(&row.get::<String>(0)?)?)
}
async fn check_membership(c: &Connection, aid: &str, sid: &str) -> Result<()> {
    if let Some(row) = c.query(
        "SELECT s.name,s.archived FROM trend_membership m JOIN trend_series s ON s.id=m.series WHERE m.analysis=? AND m.series<>?",
        params![aid, sid],
    ).await?.next().await? {
        return Err(AppError::Conflict(format!(
            "This analysis already belongs to series “{}”{}. Open that series to continue, or choose a different analysis. Each analysis can belong to only one series.",
            row.get::<String>(0)?,
            if row.get::<i64>(1)? != 0 { " (archived; restore it to edit)" } else { "" },
        )).into());
    }
    Ok(())
}
pub async fn write(c: &Connection, s: &Series) -> Result<()> {
    s.validate()?;
    for p in &s.periods {
        check_membership(c, &p.analysis, &s.id).await?;
    }
    let (name, normalized) = normalize_name(&s.name)?;
    c.execute("INSERT INTO trend_series VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name, normalized_name=excluded.normalized_name, archived=excluded.archived, revision=excluded.revision, value=excluded.value",params![s.id.clone(),name,normalized,s.archived as i64,s.revision as i64,serde_json::to_string(s)?]).await?;
    c.execute(
        "DELETE FROM trend_membership WHERE series=?",
        [s.id.clone()],
    )
    .await?;
    for p in &s.periods {
        // Global primary key enforces membership even across distinct series commands.
        c.execute(
            "INSERT INTO trend_membership VALUES(?,?,?)",
            params![p.analysis.clone(), s.id.clone(), p.id.clone()],
        )
        .await?;
    }
    c.execute(
        "INSERT INTO outbox(id,resource_id,value,resource_kind) VALUES(?,?,?,'series')",
        params![
            id(),
            s.id.clone(),
            json!({"kind":"series","revision":s.revision}).to_string()
        ],
    )
    .await?;
    Ok(())
}
async fn record(
    c: &Connection,
    user: &User,
    command: &TrendCommand,
    before: &Series,
    after: &Series,
) -> Result<()> {
    // Retain decisions, not large text profiles, in audit records.
    let audit = json!({"actor":user,"time":now(),"command":command,"before":public_snapshot(before),"after":public_snapshot(after)});
    c.execute(
        "INSERT INTO trend_history VALUES(?,?,?,?)",
        params![id(), after.id.clone(), now(), audit.to_string()],
    )
    .await?;
    Ok(())
}
pub fn public_snapshot(s: &Series) -> Value {
    let mut result = serde_json::to_value(s).expect("Serializable series");
    if let Some(m) = result.get_mut("model").and_then(Value::as_object_mut) {
        m.remove("weights");
    }
    if let Some(periods) = result["periods"].as_array_mut() {
        for p in periods {
            if let Some(occurrences) = p["occurrences"].as_array_mut() {
                for o in occurrences {
                    let n = o["rows"].as_array().map(Vec::len).unwrap_or(0);
                    o.as_object_mut().unwrap().remove("rows");
                    o["count"] = json!(n);
                }
            }
        }
    }
    result
}
impl Store {
    pub async fn delete_series(
        &self,
        user: &User,
        sid: &str,
        command: &str,
        expected: u64,
    ) -> Result<Value> {
        ensure!(
            !command.is_empty() && command.len() <= 100,
            "Invalid command ID"
        );
        let _m = self.maintenance.read().await;
        let lock = self.lock("series-catalog");
        let _l = lock.write().await;
        let _w = self.writers.acquire().await?;
        let c = self.connect().await?;
        execute(&c, "BEGIN CONCURRENT").await?;
        let hash = digest(format!("delete-series:{sid}:{expected}").as_bytes());
        let result = async {
            if let Some(r) = receipt(&c, &user.id, command, &hash).await? {
                return Ok(r);
            }
            let before = read(&c, sid).await?;
            if before.revision != expected {
                return Err(AppError::Conflict(
                    "Series changed. Refresh and review it before deleting.".into(),
                )
                .into());
            }
            c.execute("DELETE FROM trend_membership WHERE series=?", [sid])
                .await?;
            c.execute("DELETE FROM trend_tasks WHERE series=?", [sid])
                .await?;
            c.execute("DELETE FROM trend_history WHERE series=?", [sid])
                .await?;
            c.execute("DELETE FROM trend_series WHERE id=?", [sid])
                .await?;
            c.execute(
                "INSERT INTO outbox(id,resource_id,value,resource_kind) VALUES(?,?,?,'series')",
                params![
                    id(),
                    sid,
                    json!({"kind":"series","deleted":true,"revision":expected+1}).to_string()
                ],
            )
            .await?;
            let result = json!({"deleted":true});
            save_receipt(&c, &user.id, command, &hash, &result).await?;
            Ok(result)
        }
        .await;
        finish(&c, result).await
    }
    pub async fn trend_memberships(&self) -> Result<Value> {
        let c = self.connect().await?;
        let mut rows = c.query("SELECT m.analysis,m.series,s.name,s.archived FROM trend_membership m JOIN trend_series s ON s.id=m.series", ()).await?;
        let mut items = vec![];
        while let Some(row) = rows.next().await? {
            items.push(json!({"analysis":row.get::<String>(0)?,"series":row.get::<String>(1)?,"name":row.get::<String>(2)?,"archived":row.get::<i64>(3)?!=0}));
        }
        Ok(json!(items))
    }
    pub async fn trend_source_labels(&self, aid: &str) -> Result<BTreeMap<usize, String>> {
        let c = self.connect().await?;
        let mut rows = c
            .query(
                "SELECT entity,label FROM entities WHERE analysis=? AND label IS NOT NULL",
                [aid],
            )
            .await?;
        let mut labels = BTreeMap::new();
        while let Some(row) = rows.next().await? {
            if let Ok(cluster) = row.get::<String>(0)?.parse::<usize>() {
                labels.insert(cluster, row.get::<String>(1)?);
            }
        }
        Ok(labels)
    }
    pub async fn series(&self, sid: &str) -> Result<Series> {
        read(&self.connect().await?, sid).await
    }
    pub async fn series_catalog(&self, search: &str, archived: bool, offset: i64) -> Result<Value> {
        ensure!((0..=1_000_000).contains(&offset), "Invalid offset");
        let c = self.connect().await?;
        let mut rows=c.query("SELECT id,name,archived,revision FROM trend_series WHERE (?=1 OR archived=0) AND instr(lower(name),lower(?))>0 ORDER BY normalized_name,id LIMIT 101 OFFSET ?",params![archived as i64,search,offset]).await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            out.push(json!({"id":r.get::<String>(0)?,"name":r.get::<String>(1)?,"archived":r.get::<i64>(2)?!=0,"revision":r.get::<i64>(3)?}));
        }
        let next = if out.len() > 100 {
            out.pop();
            Some(offset + 100)
        } else {
            None
        };
        Ok(json!({"items":out,"nextOffset":next}))
    }
    pub async fn create_series(&self, user: &User, command: &str, name: &str) -> Result<Value> {
        let _m = self.maintenance.read().await;
        let lock = self.lock("series-catalog");
        let _l = lock.write().await;
        let _w = self.writers.acquire().await?;
        let c = self.connect().await?;
        execute(&c, "BEGIN CONCURRENT").await?;
        let hash = digest(serde_json::to_string(&json!({"createSeries":name}))?.as_bytes());
        let result = async {
            if let Some(r) = receipt(&c, &user.id, command, &hash).await? {
                return Ok(r);
            }
            let s = Series::new(normalize_name(name)?.0);
            write(&c, &s).await?;
            let result = public_snapshot(&s);
            save_receipt(&c, &user.id, command, &hash, &result).await?;
            Ok(result)
        }
        .await;
        finish(&c, result).await
    }
    pub async fn series_command(
        &self,
        user: &User,
        sid: &str,
        command: &TrendCommand,
    ) -> Result<Value> {
        ensure!(
            !command.command_id.is_empty() && command.command_id.len() <= 100,
            "Invalid command ID"
        );
        let _m = self.maintenance.read().await;
        let lock = self.lock("series-catalog");
        let _l = lock.write().await;
        let _w = self.writers.acquire().await?;
        let c = self.connect().await?;
        execute(&c, "BEGIN CONCURRENT").await?;
        let hash = digest(serde_json::to_string(&(sid, command))?.as_bytes());
        let result = async {
            if let Some(r) = receipt(&c, &user.id, &command.command_id, &hash).await? {
                return Ok(r);
            }
            let before = read(&c, sid).await?;
            if before.revision != command.expected {
                return Err(AppError::Conflict(
                    "Series changed. Refresh and review the new state.".into(),
                )
                .into());
            }
            ensure!(
                !before.archived || matches!(command.action, Action::Archive { archived: false }),
                "Series is archived"
            );
            if let Action::AddPeriod { analysis, .. } = &command.action {
                check_membership(&c, analysis, sid).await?;
            }
            let direct = matches!(
                command.action,
                Action::Rename { .. } | Action::Archive { .. } | Action::RenameProblem { .. }
            );
            let result = if direct {
                let mut after = before.clone();
                after.apply(&command.action, user)?;
                after.revision += 1;
                write(&c, &after).await?;
                record(&c, user, command, &before, &after).await?;
                json!({"saved":true,"revision":after.revision})
            } else {
                let task = Task {
                    id: id(),
                    series: sid.into(),
                    user: user.clone(),
                    command: command.clone(),
                    state: "queued".into(),
                    progress: "Waiting for matching worker".into(),
                    error: None,
                    proposed: None,
                    expires: now() + 86400,
                };
                c.execute(
                    "INSERT INTO trend_tasks VALUES(?,?,?,?,?)",
                    params![
                        task.id.clone(),
                        sid,
                        "queued",
                        now(),
                        serde_json::to_string(&task)?
                    ],
                )
                .await?;
                json!({"task":task.id})
            };
            save_receipt(&c, &user.id, &command.command_id, &hash, &result).await?;
            Ok(result)
        }
        .await;
        finish(&c, result).await
    }
    pub async fn trend_task(&self, task: &str) -> Result<Task> {
        let c = self.connect().await?;
        let row = c
            .query("SELECT value FROM trend_tasks WHERE id=?", [task])
            .await?
            .next()
            .await?
            .context("Task not found")?;
        Ok(serde_json::from_str(&row.get::<String>(0)?)?)
    }
    pub async fn trend_tasks(&self, sid: &str) -> Result<Value> {
        let c = self.connect().await?;
        let mut rows=c.query("SELECT value FROM trend_tasks WHERE series=? ORDER BY created DESC,id DESC LIMIT 50",[sid]).await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            let t: Task = serde_json::from_str(&r.get::<String>(0)?)?;
            out.push(json!({"id":t.id,"state":t.state,"progress":t.progress,"error":t.error,"expected":t.command.expected,"action":t.command.action}));
        }
        Ok(json!(out))
    }
    pub async fn next_trend_task(&self) -> Result<Option<Task>> {
        let c = self.connect().await?;
        let row = c
            .query(
                "SELECT value FROM trend_tasks WHERE state='queued' ORDER BY created,id LIMIT 1",
                (),
            )
            .await?
            .next()
            .await?;
        row.map(|r| Ok(serde_json::from_str(&r.get::<String>(0)?)?))
            .transpose()
    }
    pub async fn save_trend_task(&self, task: &Task) -> Result<()> {
        let _m = self.maintenance.read().await;
        let _w = self.writers.acquire().await?;
        self.connect()
            .await?
            .execute(
                "UPDATE trend_tasks SET state=?,value=? WHERE id=?",
                params![
                    task.state.clone(),
                    serde_json::to_string(task)?,
                    task.id.clone()
                ],
            )
            .await?;
        Ok(())
    }
    pub async fn recover_trend_tasks(&self) -> Result<()> {
        let c = self.connect().await?;
        let mut rows = c
            .query("SELECT value FROM trend_tasks WHERE state='running'", ())
            .await?;
        let mut tasks = vec![];
        while let Some(r) = rows.next().await? {
            tasks.push(serde_json::from_str::<Task>(&r.get::<String>(0)?)?);
        }
        drop(rows);
        for mut t in tasks {
            t.state = "queued".into();
            t.progress = "Resuming after restart".into();
            self.save_trend_task(&t).await?;
        }
        Ok(())
    }
    pub async fn trend_profile(&self, key: &str) -> Result<Option<Vec<Profile>>> {
        let c = self.connect().await?;
        let row = c
            .query("SELECT value FROM trend_profiles WHERE cache_key=?", [key])
            .await?
            .next()
            .await?;
        row.map(|r| {
            let bytes = r.get::<Vec<u8>>(0)?;
            Ok(serde_json::from_slice(&zstd::stream::decode_all(
                bytes.as_slice(),
            )?)?)
        })
        .transpose()
    }
    pub async fn save_trend_profile(&self, key: &str, value: &[Profile]) -> Result<()> {
        let bytes = zstd::stream::encode_all(serde_json::to_vec(value)?.as_slice(), 3)?;
        let _m = self.maintenance.read().await;
        let _w = self.writers.acquire().await?;
        self.connect()
            .await?
            .execute(
                "INSERT INTO trend_profiles VALUES(?,?) ON CONFLICT(cache_key) DO NOTHING",
                params![key, bytes],
            )
            .await?;
        Ok(())
    }
    pub async fn trend_history(&self, sid: &str, offset: i64) -> Result<Value> {
        ensure!((0..=1_000_000).contains(&offset), "Invalid offset");
        let c = self.connect().await?;
        let mut rows=c.query("SELECT value FROM trend_history WHERE series=? ORDER BY created DESC,id DESC LIMIT 50 OFFSET ?",params![sid,offset]).await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            out.push(serde_json::from_str::<Value>(&r.get::<String>(0)?)?);
        }
        Ok(json!(out))
    }
    pub async fn apply_trend_preview(
        &self,
        user: &User,
        sid: &str,
        tid: &str,
        command: &str,
    ) -> Result<Value> {
        let _m = self.maintenance.read().await;
        let lock = self.lock("series-catalog");
        let _l = lock.write().await;
        let _w = self.writers.acquire().await?;
        let c = self.connect().await?;
        execute(&c, "BEGIN CONCURRENT").await?;
        let hash = digest(format!("apply:{sid}:{tid}").as_bytes());
        let result = async {
            if let Some(r) = receipt(&c, &user.id, command, &hash).await? {
                return Ok(r);
            }
            let row = c
                .query(
                    "SELECT value FROM trend_tasks WHERE id=? AND series=?",
                    params![tid, sid],
                )
                .await?
                .next()
                .await?
                .context("Preview not found")?;
            let mut task: Task = serde_json::from_str(&row.get::<String>(0)?)?;
            ensure!(
                task.state == "preview" && task.expires > now(),
                "Preview is expired or no longer available"
            );
            let before = read(&c, sid).await?;
            if before.revision != task.command.expected || before.archived {
                return Err(AppError::Conflict(
                    "Preview is stale. Generate and review a new proposal.".into(),
                )
                .into());
            }
            let mut after = task.proposed.take().context("Preview has no proposal")?;
            for zero in &mut after.zeros {
                if !before
                    .zeros
                    .iter()
                    .any(|old| old.period == zero.period && old.problem == zero.problem)
                {
                    zero.actor = user.clone();
                    zero.time = now();
                }
            }
            for p in &after.periods {
                let source = info(&c, &p.analysis).await?;
                ensure!(
                    source.fingerprint == p.fingerprint,
                    "Source changed since preview"
                );
            }
            after.revision = before.revision + 1;
            write(&c, &after).await?;
            record(&c, user, &task.command, &before, &after).await?;
            task.state = "applied".into();
            task.progress = "Saved".into();
            c.execute(
                "UPDATE trend_tasks SET state='applied',value=? WHERE id=?",
                params![serde_json::to_string(&task)?, tid],
            )
            .await?;
            let result = json!({"saved":true,"revision":after.revision});
            save_receipt(&c, &user.id, command, &hash, &result).await?;
            Ok(result)
        }
        .await;
        finish(&c, result).await
    }
}
