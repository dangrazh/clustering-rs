//! Catalog-wide views and lifecycle operations: permanent deletion, title regeneration, all
//! assigned work and the operator's view of every user's jobs.
use super::*;
use crate::workflow::Workflow;

/// Titles are analysis-wide, so only the owner or a SysAdmin may regenerate them. Series
/// identify their periods by fingerprint, which titles are part of.
async fn regenerate_allowed(c: &Connection, info: &AnalysisInfo, user: &User, admin: bool, expected: u64) -> Result<()> {
    if info.owner.id != user.id && !admin {
        return Err(AppError::Forbidden.into());
    }
    if info.archived {
        return Err(AppError::Archived.into());
    }
    expect(info.version, expected)?;
    if let Some(r) = c
        .query(
            "SELECT s.name FROM trend_membership m JOIN trend_series s ON s.id=m.series WHERE m.analysis=?",
            [info.id.as_str()],
        )
        .await?
        .next()
        .await?
    {
        return Err(AppError::Conflict(format!(
            "This analysis belongs to the series “{}”. Its titles cannot be regenerated while series track its clusters.",
            r.get::<String>(0)?
        ))
        .into());
    }
    Ok(())
}

impl Store {
    /// Permanently removes a saved analysis and everything recorded about it. Only the owner
    /// or a SysAdmin may delete; analyses that belong to a series must leave it first. The
    /// outcome names the artifact file when nothing else references it any more.
    pub async fn delete_analysis(
        &self,
        user: &User,
        admin: bool,
        aid: &str,
        command: &str,
        expected: u64,
    ) -> Result<Value> {
        uuid::Uuid::parse_str(command)?;
        let _m = self.maintenance.read().await;
        let catalog = self.lock("catalog");
        let _c = catalog.write().await;
        let analysis = self.lock(&format!("analysis:{aid}"));
        let _a = analysis.write().await;
        let series = self.lock("series-catalog");
        let _s = series.write().await;
        let _w = self.writers.acquire().await?;
        let c = self.connect().await?;
        execute(&c, "BEGIN CONCURRENT").await?;
        let hash = digest(format!("delete-analysis:{aid}:{expected}").as_bytes());
        let result = async {
            if let Some(r) = receipt(&c, &user.id, command, &hash).await? {
                return Ok(r);
            }
            let info = info(&c, aid).await?;
            if info.owner.id != user.id && !admin {
                return Err(AppError::Forbidden.into());
            }
            expect(info.version, expected)?;
            if let Some(r) = c
                .query(
                    "SELECT s.name FROM trend_membership m JOIN trend_series s ON s.id=m.series WHERE m.analysis=?",
                    [aid],
                )
                .await?
                .next()
                .await?
            {
                return Err(AppError::Conflict(format!(
                    "This analysis belongs to the series “{}”. Replace its period there before deleting it.",
                    r.get::<String>(0)?
                ))
                .into());
            }
            for table in [
                "entities",
                "reviewers",
                "comments",
                "history",
                "audit",
                "views",
                "dashboard_entities",
                "dashboard_ready",
                "column_metadata",
            ] {
                c.execute(&format!("DELETE FROM {table} WHERE analysis=?"), [aid])
                    .await?;
            }
            c.execute("DELETE FROM analyses WHERE id=?", [aid]).await?;
            let references = c
                .query(
                    "SELECT (SELECT COUNT(*) FROM analyses WHERE artifact=?)+(SELECT COUNT(*) FROM jobs WHERE artifact=?)",
                    params![info.artifact.as_str(), info.artifact.as_str()],
                )
                .await?
                .next()
                .await?
                .context("Missing reference count")?
                .get::<i64>(0)?;
            outbox(&c, aid, json!({"kind":"deleteAnalysis","name":info.name})).await?;
            let result = json!({"deleted":true,"artifact":(references==0).then_some(info.artifact)});
            save_receipt(&c, &user.id, command, &hash, &result).await?;
            Ok(result)
        }
        .await;
        finish(&c, result).await
    }

    /// Checks that `user` may regenerate an analysis's titles, before any work is done.
    /// The same checks run again when the new titles are stored.
    pub async fn check_regenerate_titles(&self, user: &User, admin: bool, aid: &str, expected: u64) -> Result<AnalysisInfo> {
        let c = self.connect().await?;
        let info = info(&c, aid).await?;
        regenerate_allowed(&c, &info, user, admin, expected).await?;
        Ok(info)
    }

    /// Points an analysis at a new artifact whose generated titles were rewritten. Clusters,
    /// reviews, comments and renames are unchanged; renames keep taking precedence. Returns the
    /// replaced artifact's name when nothing references it any more.
    #[allow(clippy::too_many_arguments)]
    pub async fn regenerate_titles(
        &self,
        user: &User,
        admin: bool,
        aid: &str,
        command: &str,
        expected: u64,
        artifact: &str,
        fingerprint: &str,
        summaries: &[Value],
    ) -> Result<Value> {
        uuid::Uuid::parse_str(command)?;
        ensure!(!artifact.contains('/') && !artifact.contains('\\'), "Invalid artifact name");
        let _m = self.maintenance.read().await;
        let analysis = self.lock(&format!("analysis:{aid}"));
        let _a = analysis.write().await;
        let series = self.lock("series-catalog");
        let _s = series.write().await;
        let _w = self.writers.acquire().await?;
        let c = self.connect().await?;
        execute(&c, "BEGIN CONCURRENT").await?;
        let hash = digest(format!("regenerate-titles:{aid}:{expected}:{artifact}").as_bytes());
        let result = async {
            if let Some(r) = receipt(&c, &user.id, command, &hash).await? {
                return Ok(r);
            }
            let info = info(&c, aid).await?;
            regenerate_allowed(&c, &info, user, admin, expected).await?;
            c.execute(
                "UPDATE analyses SET artifact=?,fingerprint=?,version=version+1 WHERE id=?",
                params![artifact, fingerprint, aid],
            )
            .await?;
            c.execute("DELETE FROM dashboard_entities WHERE analysis=?", [aid]).await?;
            dashboard::insert_summaries(&c, aid, summaries).await?;
            audit(&c, aid, "", user, "regenerateTitles", json!(info.artifact), json!(artifact)).await?;
            let references = c
                .query(
                    "SELECT (SELECT COUNT(*) FROM analyses WHERE artifact=?)+(SELECT COUNT(*) FROM jobs WHERE artifact=?)",
                    params![info.artifact.as_str(), info.artifact.as_str()],
                )
                .await?
                .next()
                .await?
                .context("Missing reference count")?
                .get::<i64>(0)?;
            outbox(&c, aid, json!({"kind":"regenerateTitles","by":user.name,"byId":user.id})).await?;
            let result = json!({
                "version": info.version + 1,
                "replaced": (references == 0 && info.artifact != artifact).then_some(info.artifact),
            });
            save_receipt(&c, &user.id, command, &hash, &result).await?;
            Ok(result)
        }
        .await;
        finish(&c, result).await
    }

    /// Every confirmed assignment, ordered by assignee. A theme without its own reviewer
    /// inherits its cluster's reviewer, as in "My assigned work".
    pub async fn all_assigned(&self, offset: i64) -> Result<Value> {
        ensure!((0..=10_000_000).contains(&offset), "Invalid offset");
        let c = self.connect().await?;
        let mut rows = c
            .query(
                "SELECT d.analysis,d.entity,d.parent,COALESCE(e.label,d.label),d.incidents,e.workflow,p.workflow,COALESCE(p.label,pd.label),pd.incidents,u.id,u.name \
                 FROM dashboard_entities d \
                 JOIN dashboard_entities pd ON pd.analysis=d.analysis AND pd.entity=d.parent \
                 LEFT JOIN reviewers own ON own.analysis=d.analysis AND own.entity=d.entity \
                 LEFT JOIN reviewers r ON r.analysis=d.analysis AND r.entity=d.parent \
                 JOIN entities e ON e.analysis=d.analysis AND e.entity=d.entity \
                 JOIN entities p ON p.analysis=d.analysis AND p.entity=d.parent \
                 JOIN users u ON u.id=CASE WHEN own.value IS NOT NULL AND own.value<>'null' THEN own.user_id ELSE r.user_id END \
                 ORDER BY u.name,u.id,d.analysis,CAST(d.parent AS INTEGER),d.entity LIMIT 1001 OFFSET ?",
                [offset],
            )
            .await?;
        let mut assigned = vec![];
        while let Some(r) = rows.next().await? {
            let own: Option<Workflow> = serde_json::from_str(&r.get::<String>(5)?)?;
            let parent: Option<Workflow> = serde_json::from_str(&r.get::<String>(6)?)?;
            let workflow = own.or(parent).unwrap_or_default();
            assigned.push(json!({"analysisId":r.get::<String>(0)?,"entity":r.get::<String>(1)?,"parent":r.get::<String>(2)?,"label":r.get::<String>(3)?,"incidents":r.get::<i64>(4)?,"status":workflow.status,"parentLabel":r.get::<String>(7)?,"parentIncidents":r.get::<i64>(8)?,"assignee":{"id":r.get::<String>(9)?,"name":r.get::<String>(10)?}}));
        }
        let more = assigned.len() > 1000;
        assigned.truncate(1000);
        Ok(json!({"assigned":assigned,"nextOffset":more.then_some(offset+1000)}))
    }

    /// Display names for job owners, for the operator's view of all jobs.
    pub async fn user_names(&self) -> Result<HashMap<String, String>> {
        let c = self.connect().await?;
        let mut rows = c.query("SELECT id,name FROM users", ()).await?;
        let mut names = HashMap::new();
        while let Some(r) = rows.next().await? {
            names.insert(r.get::<String>(0)?, r.get::<String>(1)?);
        }
        Ok(names)
    }
}
