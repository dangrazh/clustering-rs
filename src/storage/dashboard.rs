use super::*;

pub(super) async fn migrate(c: &Connection) -> Result<()> {
    if scalar(c, "SELECT version FROM schema_version").await? == 2 {
        return Ok(());
    }
    execute(c, "BEGIN").await?;
    let result = async {
        c.execute_batch("CREATE TABLE dashboard_preferences(user_id TEXT PRIMARY KEY REFERENCES users(id), value TEXT NOT NULL); CREATE TABLE dashboard_entities(analysis TEXT NOT NULL REFERENCES analyses(id), entity TEXT NOT NULL, parent TEXT NOT NULL, label TEXT NOT NULL, incidents INTEGER NOT NULL, PRIMARY KEY(analysis,entity)); CREATE TABLE dashboard_ready(analysis TEXT PRIMARY KEY REFERENCES analyses(id)); ALTER TABLE reviewers ADD COLUMN user_id TEXT; CREATE INDEX reviewer_user ON reviewers(user_id,analysis); CREATE INDEX dashboard_parent ON dashboard_entities(analysis,parent); CREATE INDEX analysis_owner ON analyses(creator,created);").await?;
        let mut rows=c.query("SELECT analysis,entity,value FROM reviewers",()).await?;
        while let Some(row)=rows.next().await? {
            let reviewer:Reviewer=serde_json::from_str(&row.get::<String>(2)?)?;
            if !reviewer.unconfirmed { c.execute("UPDATE reviewers SET user_id=? WHERE analysis=? AND entity=?",params![reviewer.user_id,row.get::<String>(0)?,row.get::<String>(1)?]).await?; }
        }
        c.execute("UPDATE schema_version SET version=2",()).await?;
        Ok::<_,anyhow::Error>(())
    }.await;
    match result {
        Ok(()) => execute(c, "COMMIT").await,
        Err(e) => {
            let _ = execute(c, "ROLLBACK").await;
            Err(e)
        }
    }
}
pub(super) async fn insert_summaries(c: &Connection, aid: &str, values: &[Value]) -> Result<()> {
    if values.is_empty() {
        return Ok(());
    }
    for value in values {
        c.execute(
            "INSERT OR REPLACE INTO dashboard_entities VALUES(?,?,?,?,?)",
            params![
                aid,
                value["entity"].as_str().context("Entity missing")?,
                value["parent"].as_str().context("Parent missing")?,
                value["label"].as_str().unwrap_or(""),
                value["incidents"].as_i64().unwrap_or(0)
            ],
        )
        .await?;
    }
    c.execute("INSERT OR IGNORE INTO dashboard_ready VALUES(?)", [aid])
        .await?;
    Ok(())
}
impl Store {
    pub fn entity_summaries(run: &crate::model::AnalysisRun) -> Vec<Value> {
        let mut values = vec![];
        for cluster in &run.clusters {
            let parent = cluster.id.0.to_string();
            values.push(json!({"entity":parent,"parent":parent,"label":cluster.label,"incidents":cluster.incident_row_indices.len()}));
            for theme in &cluster.subgroups {
                values.push(json!({"entity":format!("{}:{}",parent,theme.id),"parent":parent,"label":theme.label,"incidents":theme.incident_row_indices.len()}));
            }
        }
        values
    }
    pub async fn backfill_dashboard(&self, artifacts: &crate::artifacts::Artifacts) -> Result<()> {
        let c = self.connect().await?;
        let mut rows=c.query("SELECT id,artifact FROM analyses WHERE id NOT IN (SELECT analysis FROM dashboard_ready)",()).await?;
        let mut missing = vec![];
        while let Some(row) = rows.next().await? {
            missing.push((row.get::<String>(0)?, row.get::<String>(1)?));
        }
        drop(rows);
        for (aid, name) in missing {
            tracing::info!(analysis=%aid,"Indexing dashboard metadata");
            let artifact = artifacts
                .load(&name)
                .await
                .with_context(|| format!("Cannot index analysis {aid}"))?;
            let values = Self::entity_summaries(&artifact.run);
            execute(&c, "BEGIN").await?;
            let result = async {
                insert_summaries(&c, &aid, &values).await?;
                c.execute(
                    "INSERT OR IGNORE INTO dashboard_ready VALUES(?)",
                    [aid.as_str()],
                )
                .await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(e) = result {
                let _ = execute(&c, "ROLLBACK").await;
                return Err(e);
            }
            execute(&c, "COMMIT").await?;
        }
        Ok(())
    }
    pub async fn dashboard_preferences(&self, user: &User) -> Result<Value> {
        let c = self.connect().await?;
        Ok(
            match c
                .query(
                    "SELECT value FROM dashboard_preferences WHERE user_id=?",
                    [user.id.as_str()],
                )
                .await?
                .next()
                .await?
            {
                Some(r) => serde_json::from_str(&r.get::<String>(0)?)?,
                None => json!({}),
            },
        )
    }
    pub async fn save_dashboard_preferences(&self, user: &User, value: &Value) -> Result<()> {
        ensure!(
            value.is_object() && value.to_string().len() <= 65536,
            "Invalid dashboard preferences"
        );
        for key in ["mine", "all"] {
            if let Some(section) = value.get(key) {
                ensure!(section.is_object(), "Invalid dashboard section");
                ensure!(
                    section
                        .get("search")
                        .is_none_or(|v| v.as_str().is_some_and(|s| s.len() <= 800)),
                    "Invalid dashboard search"
                );
                ensure!(
                    section.get("archived").is_none_or(Value::is_boolean),
                    "Invalid archive filter"
                );
            }
        }
        if let Some(section) = value.get("assigned") {
            ensure!(section.is_object(), "Invalid assigned settings");
            ensure!(
                section.get("archived").is_none_or(Value::is_boolean),
                "Invalid archive filter"
            );
            if let Some(status) = section.get("status") {
                ensure!(
                    status == ""
                        || serde_json::from_value::<crate::workflow::ReviewStatus>(status.clone())
                            .is_ok(),
                    "Invalid workflow status"
                );
            }
        }
        if let Some(expanded) = value.get("expanded") {
            ensure!(
                expanded.as_array().is_some_and(|items| items.len() <= 1000
                    && items
                        .iter()
                        .all(|v| v.as_str().is_some_and(|s| s.len() <= 150))),
                "Invalid expanded groups"
            );
        }
        let _m = self.maintenance.read().await;
        let _p = self.writers.acquire().await?;
        self.connect().await?.execute("INSERT INTO dashboard_preferences VALUES(?,?) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value",params![user.id.clone(),value.to_string()]).await?;
        Ok(())
    }
    #[cfg(test)]
    pub async fn dashboard_data(&self, user: &User) -> Result<Value> {
        self.dashboard_page(user, 0).await
    }
    pub async fn dashboard_page(&self, user: &User, offset: i64) -> Result<Value> {
        ensure!(
            (0..=10_000_000).contains(&offset),
            "Invalid dashboard offset"
        );
        let c = self.connect().await?;
        let mut rows=c.query("SELECT a.id,a.name,a.created,a.archived,u.id,u.name FROM analyses a JOIN users u ON u.id=a.creator ORDER BY a.created DESC,a.id LIMIT 501 OFFSET ?",[offset]).await?;
        let mut analyses = vec![];
        while let Some(r) = rows.next().await? {
            analyses.push(json!({"id":r.get::<String>(0)?,"name":r.get::<String>(1)?,"created":r.get::<i64>(2)?,"archived":r.get::<i64>(3)?!=0,"owner":{"id":r.get::<String>(4)?,"name":r.get::<String>(5)?}}));
        }
        let mut rows=c.query("SELECT d.analysis,d.entity,d.parent,COALESCE(e.label,d.label),d.incidents,e.workflow,p.workflow FROM dashboard_entities d JOIN reviewers r ON r.analysis=d.analysis AND r.entity=d.parent JOIN entities e ON e.analysis=d.analysis AND e.entity=d.entity JOIN entities p ON p.analysis=d.analysis AND p.entity=d.parent WHERE r.user_id=? ORDER BY d.analysis,CAST(d.parent AS INTEGER),d.entity LIMIT 501 OFFSET ?",params![user.id.clone(),offset]).await?;
        let mut assigned = vec![];
        while let Some(r) = rows.next().await? {
            let own: Option<Workflow> = serde_json::from_str(&r.get::<String>(5)?)?;
            let parent: Option<Workflow> = serde_json::from_str(&r.get::<String>(6)?)?;
            let workflow = own.or(parent).unwrap_or_default();
            assigned.push(json!({"analysisId":r.get::<String>(0)?,"entity":r.get::<String>(1)?,"parent":r.get::<String>(2)?,"label":r.get::<String>(3)?,"incidents":r.get::<i64>(4)?,"status":workflow.status}));
        }
        let more = analyses.len() > 500 || assigned.len() > 500;
        analyses.truncate(500);
        assigned.truncate(500);
        Ok(
            json!({"analyses":analyses,"assigned":assigned,"nextOffset":if more {Some(offset+500)} else {None}}),
        )
    }
}
