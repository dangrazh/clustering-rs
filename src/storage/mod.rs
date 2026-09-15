//! Durable application records. Only the server process owns this database.
use crate::{
    session::{ReviewData, ViewState},
    workflow::{Action, Actor, Annotation, Annotations, Mutation, Workflow},
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::sync::{RwLock, Semaphore};
use turso::{params, Connection};
use unicode_normalization::UnicodeNormalization;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Sign in to continue.")]
    Unauthorized,
    #[error("This operation is not permitted.")]
    Forbidden,
    #[error("{0}")]
    Conflict(String),
    #[error("Analysis is archived. Restore it before editing.")]
    Archived,
    #[error("{0}")]
    Missing(String),
    #[error("The server is busy. Retry shortly.")]
    Busy,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub email: String,
}
impl User {
    pub fn actor(&self) -> Actor {
        Actor {
            name: self.name.clone(),
            email: self.email.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisInfo {
    pub id: String,
    pub name: String,
    pub artifact: String,
    pub fingerprint: String,
    pub archived: bool,
    pub version: u64,
    pub created: i64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reviewer {
    pub user_id: Option<String>,
    pub recorded: Option<Actor>,
    pub unconfirmed: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableMeta {
    pub reviewers: BTreeMap<String, Reviewer>,
    pub audit: Vec<Value>,
    #[serde(default)]
    pub read_only_comments: std::collections::BTreeSet<String>,
    #[serde(skip)]
    pub source_job: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub command_id: String,
    pub target: String,
    pub action: Value,
    #[serde(default)]
    pub expected: u64,
    #[serde(default)]
    pub parent_version: Option<u64>,
}
pub struct InitialReview<'a> {
    pub review: &'a ReviewData,
    pub view: &'a ViewState,
    pub metadata: &'a PortableMeta,
}
pub struct Store {
    db: turso::Database,
    pub root: PathBuf,
    pub maintenance: RwLock<()>,
    pub writers: Semaphore,
    locks: Mutex<HashMap<String, Weak<RwLock<()>>>>,
    publisher: tokio::sync::Mutex<()>,
    _instance: std::fs::File,
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn normalize_name(name: &str) -> Result<(String, String)> {
    let name = name.trim();
    ensure!(
        !name.is_empty() && name.chars().count() <= 200 && !name.chars().any(char::is_control),
        "Enter an analysis name of 1–200 characters."
    );
    // Locale-independent Unicode lowercase after canonical normalization.
    Ok((
        name.into(),
        name.nfc().flat_map(char::to_lowercase).collect(),
    ))
}
pub async fn execute(c: &Connection, sql: &str) -> Result<()> {
    c.execute(sql, ()).await?;
    Ok(())
}
async fn pragma(c: &Connection, sql: &str) -> Result<()> {
    let mut r = c.query(sql, ()).await?;
    while r.next().await?.is_some() {}
    Ok(())
}
pub async fn scalar(c: &Connection, sql: &str) -> Result<i64> {
    Ok(c.query(sql, ())
        .await?
        .next()
        .await?
        .context("Missing scalar")?
        .get(0)?)
}
pub fn retryable(e: &anyhow::Error) -> bool {
    // Engine errors are inspected, never arbitrary client strings.
    e.chain()
        .filter_map(|e| e.downcast_ref::<turso::Error>())
        .any(|e| {
            matches!(e, turso::Error::Busy(_) | turso::Error::BusySnapshot(_))
                || format!("{e}").to_lowercase().contains("conflict")
        })
}
impl Store {
    pub async fn open(root: impl AsRef<Path>, writers: usize) -> Result<Arc<Self>> {
        ensure!((1..=16).contains(&writers), "Writer limit must be 1–16.");
        std::fs::create_dir_all(root.as_ref())?;
        let root = root.as_ref().canonicalize()?;
        let instance = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("instance.lock"))?;
        fs2::FileExt::try_lock_exclusive(&instance)
            .context("Another application or backup owns the data directory")?;
        for sub in ["artifacts", "staging", "inputs"] {
            std::fs::create_dir_all(root.join(sub))?;
        }
        let db = turso::Builder::new_local(
            root.join("analysis.db")
                .to_str()
                .context("Invalid data path")?,
        )
        .build()
        .await?;
        let s = Arc::new(Self {
            db,
            root,
            maintenance: RwLock::new(()),
            writers: Semaphore::new(writers),
            locks: Mutex::new(HashMap::new()),
            publisher: tokio::sync::Mutex::new(()),
            _instance: instance,
        });
        let c = s.connect().await?;
        pragma(&c, "PRAGMA journal_mode='mvcc'").await?;
        let exists = scalar(
            &c,
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_version'",
        )
        .await?;
        if exists == 0 {
            execute(&c, "BEGIN").await?;
            if let Err(e) = c.execute_batch(include_str!("schema.sql")).await {
                let _ = execute(&c, "ROLLBACK").await;
                return Err(e.into());
            }
            execute(&c, "COMMIT").await?;
        }
        ensure!(
            scalar(&c, "SELECT version FROM schema_version").await? == 1,
            "Unsupported database schema version"
        );
        Ok(s)
    }
    pub async fn connect(&self) -> Result<Connection> {
        let c = self.db.connect()?;
        pragma(&c, "PRAGMA foreign_keys=ON").await?;
        pragma(&c, "PRAGMA synchronous=FULL").await?;
        Ok(c)
    }
    pub fn lock(&self, key: &str) -> Arc<RwLock<()>> {
        let mut locks = self.locks.lock().unwrap();
        if let Some(l) = locks.get(key).and_then(Weak::upgrade) {
            return l;
        }
        locks.retain(|_, v| v.strong_count() > 0);
        let l = Arc::new(RwLock::new(()));
        locks.insert(key.into(), Arc::downgrade(&l));
        l
    }
    pub async fn user(&self, external: &str, name: &str, email: &str) -> Result<User> {
        ensure!(!name.trim().is_empty(), "Missing user name");
        let _m = self.maintenance.read().await;
        let l = self.lock("users");
        let _g = l.write().await;
        let _p = self.writers.acquire().await?;
        let c = self.connect().await?;
        let existing = c
            .query("SELECT id FROM users WHERE external_id=?", [external])
            .await?
            .next()
            .await?;
        let user_id = match existing {
            Some(r) => r.get::<String>(0)?,
            None => id(),
        };
        c.execute("INSERT INTO users(id,external_id,name,email) VALUES(?,?,?,?) ON CONFLICT(external_id) DO UPDATE SET name=excluded.name,email=excluded.email",params![user_id.clone(),external,name,email]).await?;
        Ok(User {
            id: user_id,
            name: name.into(),
            email: email.into(),
        })
    }
    pub async fn users(&self) -> Result<Vec<User>> {
        let c = self.connect().await?;
        let mut rows = c
            .query(
                "SELECT id,name,email FROM users WHERE active=1 ORDER BY name",
                (),
            )
            .await?;
        let mut users = vec![];
        while let Some(r) = rows.next().await? {
            users.push(User {
                id: r.get(0)?,
                name: r.get(1)?,
                email: r.get(2)?,
            });
        }
        Ok(users)
    }
    pub async fn analysis(&self, aid: &str) -> Result<AnalysisInfo> {
        let c = self.connect().await?;
        info(&c, aid).await
    }
    pub async fn list(&self, search: &str, archived: bool) -> Result<Vec<AnalysisInfo>> {
        let c = self.connect().await?;
        let mut rows=c.query("SELECT id FROM analyses WHERE archived=? AND instr(normalized_name,?)>0 ORDER BY created DESC LIMIT 1000",params![i64::from(archived),search.trim().nfc().flat_map(char::to_lowercase).collect::<String>()]).await?;
        let mut ids = vec![];
        while let Some(r) = rows.next().await? {
            ids.push(r.get::<String>(0)?);
        }
        drop(rows);
        let mut out = vec![];
        for aid in ids {
            out.push(info(&c, &aid).await?);
        }
        Ok(out)
    }
    pub async fn create(
        &self,
        user: &User,
        command: &str,
        name: &str,
        artifact: &str,
        initial: InitialReview<'_>,
    ) -> Result<String> {
        let InitialReview {
            review,
            view,
            metadata: meta,
        } = initial;
        let (name, normalized) = normalize_name(name)?;
        uuid::Uuid::parse_str(command)?;
        let hash = digest(&serde_json::to_vec(&(
            name.clone(),
            artifact,
            review.analysis_id.clone(),
            view,
            meta,
        ))?);
        let _m = self.maintenance.read().await;
        let l = self.lock("catalog");
        let _g = l.write().await;
        let _p = self.writers.acquire().await?;
        let c = self.connect().await?;
        execute(&c, "BEGIN CONCURRENT").await?;
        let result=async{
            if let Some(v)=receipt(&c,&user.id,command,&hash).await?{return Ok(v["id"].as_str().context("Invalid receipt")?.to_owned());}
            ensure!(!artifact.contains('/')&&!artifact.contains('\\'),"Invalid artifact name");
            if c.query("SELECT id FROM analyses WHERE normalized_name=?",[normalized.as_str()]).await?.next().await?.is_some(){return Err(AppError::Conflict("An analysis with this name already exists.".into()).into());}
            let aid=id();
            if let Some(jid)=&meta.source_job {
                let row=c.query("SELECT metadata FROM jobs WHERE id=? AND owner=?",params![jid.clone(),user.id.clone()]).await?.next().await?.context("Owned source job missing")?;
                let mut metadata:Value=serde_json::from_str(&row.get::<String>(0)?)?;
                ensure!(metadata["analysisId"].is_null(),"This result was already saved centrally");
                metadata["analysisId"]=json!(aid);
                c.execute("UPDATE jobs SET metadata=?,expires=NULL WHERE id=?",params![metadata.to_string(),jid.clone()]).await?;
            }
            c.execute("INSERT INTO analyses(id,name,normalized_name,artifact,fingerprint,creator,created) VALUES(?,?,?,?,?,?,?)",params![aid.clone(),name,normalized,artifact,review.fingerprint.clone(),user.id.clone(),now()]).await?;
            for (key,entry) in &review.annotations.entries {
                c.execute("INSERT INTO entities(analysis,entity,workflow,label) VALUES(?,?,?,?)",params![aid.clone(),key.clone(),serde_json::to_string(&entry.workflow)?,review.annotations.labels.get(key).cloned()]).await?;
                if !key.contains(':'){
                    let reviewer=meta.reviewers.get(key).cloned().unwrap_or_default();
                    c.execute("INSERT INTO reviewers(analysis,entity,value) VALUES(?,?,?)",params![aid.clone(),key.clone(),serde_json::to_string(&reviewer)?]).await?;
                }
                for comment in &entry.comments{let read_only=meta.read_only_comments.contains(&comment.id);c.execute("INSERT INTO comments(analysis,entity,id,owner,imported,value) VALUES(?,?,?,?,?,?)",params![aid.clone(),key.clone(),comment.id.clone(),if read_only{None}else{Some(user.id.clone())},i64::from(read_only),serde_json::to_string(comment)?]).await?;}
                for event in &entry.history{c.execute("INSERT INTO history VALUES(?,?,?,?)",params![aid.clone(),key.clone(),event.id.clone(),serde_json::to_string(event)?]).await?;}
            }
            for event in &meta.audit{c.execute("INSERT INTO audit VALUES(?,?,?,?)",params![aid.clone(),id(),event["entity"].as_str().unwrap_or(""),event.to_string()]).await?;}
            c.execute("INSERT INTO views VALUES(?,?,?)",params![aid.clone(),user.id.clone(),serde_json::to_string(view)?]).await?;
            save_receipt(&c,&user.id,command,&hash,&json!({"id":aid})).await?;outbox(&c,&aid,json!({"kind":"catalog"})).await?;Ok::<_,anyhow::Error>(aid)
        }.await;
        finish(&c, result).await
    }
    pub async fn review(&self, aid: &str, user: &User) -> Result<Value> {
        let c = self.connect().await?;
        execute(&c, "BEGIN").await?;
        let result = self.review_on(&c, aid, user).await;
        finish(&c, result).await
    }
    async fn review_on(&self, c: &Connection, aid: &str, user: &User) -> Result<Value> {
        let analysis = info(c, aid).await?;
        let mut annotations = Annotations::default();
        let mut versions = serde_json::Map::new();
        let mut reviewers = BTreeMap::<String, Value>::new();
        let mut comment_access = serde_json::Map::new();
        let mut rows=c.query("SELECT entity,workflow,workflow_version,label,label_version FROM entities WHERE analysis=?",[aid]).await?;
        while let Some(r) = rows.next().await? {
            let key = r.get::<String>(0)?;
            let wf: Option<Workflow> = serde_json::from_str(&r.get::<String>(1)?)?;
            let w = r.get::<i64>(2)?;
            let lv = r.get::<i64>(4)?;
            annotations.entries.insert(
                key.clone(),
                Annotation {
                    workflow: wf,
                    ..Default::default()
                },
            );
            if let turso::Value::Text(label) = r.get_value(3)? {
                annotations.labels.insert(key.clone(), label);
            }
            versions.insert(key, json!({"workflow":w,"label":lv}));
        }
        let mut rows=c.query("SELECT entity,id,owner,imported,value,version FROM comments WHERE analysis=? ORDER BY id",[aid]).await?;
        while let Some(r) = rows.next().await? {
            let key = r.get::<String>(0)?;
            let cid = r.get::<String>(1)?;
            let owner = match r.get_value(2)? {
                turso::Value::Text(v) => Some(v),
                _ => None,
            };
            let imported = r.get::<i64>(3)? != 0;
            annotations
                .entries
                .get_mut(&key)
                .context("Missing comment entity")?
                .comments
                .push(serde_json::from_str(&r.get::<String>(4)?)?);
            comment_access.insert(cid,json!({"canEdit":!analysis.archived&&!imported&&owner.as_deref()==Some(&user.id),"version":r.get::<i64>(5)?}));
        }
        let mut rows = c
            .query(
                "SELECT entity,value FROM history WHERE analysis=? ORDER BY id",
                [aid],
            )
            .await?;
        while let Some(r) = rows.next().await? {
            annotations
                .entries
                .get_mut(&r.get::<String>(0)?)
                .context("Missing history entity")?
                .history
                .push(serde_json::from_str(&r.get::<String>(1)?)?);
        }
        for entry in annotations.entries.values_mut() {
            entry
                .comments
                .sort_by(|a, b| a.created_at.cmp(&b.created_at));
            entry.history.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        }
        let mut rows = c
            .query(
                "SELECT entity,value,version FROM reviewers WHERE analysis=?",
                [aid],
            )
            .await?;
        while let Some(r) = rows.next().await? {
            reviewers.insert(r.get(0)?,json!({"value":serde_json::from_str::<Value>(&r.get::<String>(1)?)?,"version":r.get::<i64>(2)?}));
        }
        let mut audit = vec![];
        let mut rows = c
            .query(
                "SELECT value FROM audit WHERE analysis=? ORDER BY id",
                [aid],
            )
            .await?;
        while let Some(r) = rows.next().await? {
            audit.push(serde_json::from_str::<Value>(&r.get::<String>(0)?)?);
        }
        let view = c
            .query(
                "SELECT value FROM views WHERE analysis=? AND user_id=?",
                params![aid, user.id.clone()],
            )
            .await?
            .next()
            .await?
            .map(|r| r.get::<String>(0))
            .transpose()?
            .map(|v| serde_json::from_str::<ViewState>(&v))
            .transpose()?
            .unwrap_or_default();
        let mut allowed = serde_json::Map::new();
        for key in annotations.entries.keys() {
            let wf = annotations.effective(key)?;
            allowed.insert(key.clone(),json!({"next":if analysis.archived{vec![]}else{wf.status.next().to_vec()},"canUndo":!analysis.archived&&wf.path.len()>1}));
        }
        let shared_revision = c
            .query("SELECT COUNT(*) FROM outbox WHERE analysis=?", [aid])
            .await?
            .next()
            .await?
            .context("Missing shared revision")?
            .get::<i64>(0)?;
        Ok(
            json!({"sharedRevision":shared_revision,"review":ReviewData{analysis_id:aid.into(),fingerprint:analysis.fingerprint.clone(),annotations},"view":view,"allowedActions":allowed,"versions":versions,"reviewers":reviewers,"commentAccess":comment_access,"audit":audit,"analysis":analysis}),
        )
    }
    pub async fn command(self: &Arc<Self>, aid: String, user: User, cmd: Command) -> Result<Value> {
        // A disconnected HTTP task cannot drop a transaction after acquiring guards.
        let s = self.clone();
        tokio::spawn(async move { s.command_inner(&aid, &user, &cmd).await }).await?
    }
    async fn command_inner(&self, aid: &str, user: &User, cmd: &Command) -> Result<Value> {
        let started = std::time::Instant::now();
        uuid::Uuid::parse_str(&cmd.command_id)?;
        let hash = digest(&serde_json::to_vec(&(aid, cmd))?);
        let kind = cmd.action["type"].as_str().context("Missing action type")?;
        let lifecycle = matches!(kind, "renameAnalysis" | "archive" | "restore");
        let _m = self.maintenance.read().await;
        let catalog = self.lock("catalog");
        let _cat = if kind == "renameAnalysis" {
            Some(catalog.write().await)
        } else {
            None
        };
        let lock = self.lock(&format!("analysis:{aid}"));
        let mut read = None;
        let mut write = None;
        if lifecycle {
            write = Some(lock.write().await)
        } else {
            read = Some(lock.read().await)
        };
        let parent = cmd.target.split(':').next().unwrap_or("");
        let pl = self.lock(&format!("workflow:{aid}:{parent}"));
        let workflow = matches!(kind, "transition" | "assign" | "undo" | "inherit");
        let mut pr = None;
        let mut pw = None;
        if workflow {
            if cmd.target.contains(':') {
                pr = Some(pl.read().await)
            } else {
                pw = Some(pl.write().await)
            }
        }
        let _guards = (read, write, pr, pw);
        let _permit = self.writers.acquire().await?;
        let admission_ms = started.elapsed().as_millis();
        for attempt in 0..20 {
            let c = self.connect().await?;
            let result = async {
                execute(&c, "BEGIN CONCURRENT").await?;
                let r = self.apply(&c, aid, user, cmd, &hash).await;
                finish(&c, r).await
            }
            .await;
            match result {
                Err(e) if retryable(&e) && attempt < 19 => {
                    tokio::time::sleep(Duration::from_millis(5 + attempt * 5)).await
                }
                other => {
                    tracing::debug!(
                        analysis_id = aid,
                        kind,
                        admission_ms,
                        elapsed_ms = started.elapsed().as_millis(),
                        retries = attempt,
                        succeeded = other.is_ok(),
                        "Shared command outcome"
                    );
                    return other;
                }
            }
        }
        Err(AppError::Busy.into())
    }
    async fn apply(
        &self,
        c: &Connection,
        aid: &str,
        user: &User,
        cmd: &Command,
        hash: &str,
    ) -> Result<Value> {
        if let Some(v) = receipt(c, &user.id, &cmd.command_id, hash).await? {
            return Ok(v);
        }
        let analysis = info(c, aid).await?;
        let kind = cmd.action["type"].as_str().unwrap();
        if analysis.archived && kind != "restore" {
            return Err(AppError::Archived.into());
        }
        let mut result = json!({"saved":true});
        if matches!(kind, "renameAnalysis" | "archive" | "restore") {
            expect(analysis.version, cmd.expected)?;
            if kind == "renameAnalysis" {
                let (name, norm) =
                    normalize_name(cmd.action["name"].as_str().context("Missing name")?)?;
                if c.query(
                    "SELECT id FROM analyses WHERE normalized_name=? AND id<>?",
                    params![norm.clone(), aid],
                )
                .await?
                .next()
                .await?
                .is_some()
                {
                    return Err(AppError::Conflict(
                        "An analysis with this name already exists.".into(),
                    )
                    .into());
                }
                c.execute(
                    "UPDATE analyses SET name=?,normalized_name=?,version=version+1 WHERE id=?",
                    params![name, norm, aid],
                )
                .await?;
            } else {
                c.execute(
                    "UPDATE analyses SET archived=?,version=version+1 WHERE id=?",
                    params![i64::from(kind == "archive"), aid],
                )
                .await?;
            }
            result["version"] = json!(analysis.version + 1);
        } else {
            let row=c.query("SELECT workflow,workflow_version,label,label_version FROM entities WHERE analysis=? AND entity=?",params![aid,cmd.target.clone()]).await?.next().await?.ok_or_else(||AppError::Missing("Cluster or theme not found.".into()))?;
            let wf: Option<Workflow> = serde_json::from_str(&row.get::<String>(0)?)?;
            let wv = row.get::<i64>(1)? as u64;
            let lv = row.get::<i64>(3)? as u64;
            if kind == "reviewer" {
                ensure!(
                    !cmd.target.contains(':'),
                    "Assign reviewers at cluster level."
                );
                let old = c
                    .query(
                        "SELECT value,version FROM reviewers WHERE analysis=? AND entity=?",
                        params![aid, cmd.target.clone()],
                    )
                    .await?
                    .next()
                    .await?
                    .context("Reviewer record missing")?;
                expect(old.get::<i64>(1)? as u64, cmd.expected)?;
                let assigned = if let Some(uid) = cmd.action["userId"].as_str() {
                    let r = c
                        .query(
                            "SELECT name,email FROM users WHERE id=? AND active=1",
                            [uid],
                        )
                        .await?
                        .next()
                        .await?
                        .context("Reviewer must sign in first.")?;
                    Reviewer {
                        user_id: Some(uid.into()),
                        recorded: Some(Actor {
                            name: r.get(0)?,
                            email: r.get(1)?,
                        }),
                        unconfirmed: false,
                    }
                } else {
                    Reviewer::default()
                };
                c.execute(
                    "UPDATE reviewers SET value=?,version=version+1 WHERE analysis=? AND entity=?",
                    params![serde_json::to_string(&assigned)?, aid, cmd.target.clone()],
                )
                .await?;
                audit(
                    c,
                    aid,
                    &cmd.target,
                    user,
                    kind,
                    serde_json::from_str(&old.get::<String>(0)?)?,
                    serde_json::to_value(assigned)?,
                )
                .await?;
            } else {
                let mut data = Annotations::default();
                data.entries.insert(
                    cmd.target.clone(),
                    Annotation {
                        workflow: wf.clone(),
                        ..Default::default()
                    },
                );
                let workflow = matches!(kind, "transition" | "assign" | "undo" | "inherit");
                if cmd.target.contains(':') {
                    let parent = cmd.target.split(':').next().unwrap();
                    let r=c.query("SELECT workflow,workflow_version FROM entities WHERE analysis=? AND entity=?",params![aid,parent]).await?.next().await?.context("Parent missing")?;
                    if workflow && (wf.is_none() || kind == "inherit") {
                        expect(
                            r.get::<i64>(1)? as u64,
                            cmd.parent_version.context("Parent version required")?,
                        )?;
                    }
                    data.entries.insert(
                        parent.into(),
                        Annotation {
                            workflow: serde_json::from_str(&r.get::<String>(0)?)?,
                            ..Default::default()
                        },
                    );
                }
                if kind == "rename" {
                    expect(lv, cmd.expected)?;
                } else if workflow {
                    expect(wv, cmd.expected)?;
                }
                if matches!(kind, "editComment" | "deleteComment") {
                    let cid = cmd.action["id"].as_str().context("Comment id required")?;
                    let r=c.query("SELECT owner,imported,value,version FROM comments WHERE analysis=? AND entity=? AND id=?",params![aid,cmd.target.clone(),cid]).await?.next().await?.ok_or_else(||AppError::Conflict("Comment was deleted.".into()))?;
                    if r.get::<i64>(1)? != 0
                        || r.get_value(0)? != turso::Value::Text(user.id.clone())
                    {
                        return Err(AppError::Forbidden.into());
                    }
                    expect(r.get::<i64>(3)? as u64, cmd.expected)?;
                    data.entries
                        .get_mut(&cmd.target)
                        .unwrap()
                        .comments
                        .push(serde_json::from_str(&r.get::<String>(2)?)?);
                }
                let action: Action = serde_json::from_value(cmd.action.clone())?;
                data.mutate(Mutation {
                    revision: 0,
                    target: cmd.target.clone(),
                    actor: user.actor(),
                    action,
                })?;
                let entry = &data.entries[&cmd.target];
                if kind == "rename" {
                    let new = &data.labels[&cmd.target];
                    let old = match row.get_value(2)? {
                        turso::Value::Text(v) => json!(v),
                        _ => Value::Null,
                    };
                    c.execute("UPDATE entities SET label=?,label_version=label_version+1 WHERE analysis=? AND entity=?",params![new.clone(),aid,cmd.target.clone()]).await?;
                    audit(c, aid, &cmd.target, user, kind, old, json!(new)).await?;
                } else if workflow {
                    c.execute("UPDATE entities SET workflow=?,workflow_version=workflow_version+1 WHERE analysis=? AND entity=?",params![serde_json::to_string(&entry.workflow)?,aid,cmd.target.clone()]).await?;
                    for event in &entry.history {
                        c.execute(
                            "INSERT INTO history VALUES(?,?,?,?)",
                            params![
                                aid,
                                cmd.target.clone(),
                                event.id.clone(),
                                serde_json::to_string(event)?
                            ],
                        )
                        .await?;
                    }
                } else if kind == "addComment" {
                    let comment = entry.comments.last().context("Comment missing")?;
                    c.execute(
                        "INSERT INTO comments(analysis,entity,id,owner,value) VALUES(?,?,?,?,?)",
                        params![
                            aid,
                            cmd.target.clone(),
                            comment.id.clone(),
                            user.id.clone(),
                            serde_json::to_string(comment)?
                        ],
                    )
                    .await?;
                    result["commentId"] = json!(comment.id);
                } else if kind == "editComment" {
                    let comment = &entry.comments[0];
                    c.execute(
                        "UPDATE comments SET value=?,version=version+1 WHERE analysis=? AND id=?",
                        params![serde_json::to_string(comment)?, aid, comment.id.clone()],
                    )
                    .await?;
                } else if kind == "deleteComment" {
                    c.execute(
                        "DELETE FROM comments WHERE analysis=? AND id=?",
                        params![aid, cmd.action["id"].as_str().unwrap()],
                    )
                    .await?;
                } else {
                    bail!("Unknown action");
                }
            }
        }
        save_receipt(c, &user.id, &cmd.command_id, hash, &result).await?;
        outbox(c, aid, json!({"kind":kind,"entity":cmd.target})).await?;
        Ok(result)
    }
    pub async fn save_view(&self, aid: &str, user: &User, view: &ViewState) -> Result<()> {
        let _m = self.maintenance.read().await;
        let l = self.lock(&format!("view:{aid}:{}", user.id));
        let _g = l.write().await;
        let _p = self.writers.acquire().await?;
        let c = self.connect().await?;
        c.execute("INSERT INTO views VALUES(?,?,?) ON CONFLICT(analysis,user_id) DO UPDATE SET value=excluded.value",params![aid,user.id.clone(),serde_json::to_string(view)?]).await?;
        Ok(())
    }
    pub async fn publish(&self) -> Result<()> {
        let _m = self.maintenance.read().await;
        let _g = self.publisher.lock().await;
        let _p = self.writers.acquire().await?;
        let c = self.connect().await?;
        execute(&c, "BEGIN CONCURRENT").await?;
        let r = async {
            let mut rows = c
                .query(
                    "SELECT id,analysis,value FROM outbox WHERE published=0 ORDER BY id LIMIT 100",
                    (),
                )
                .await?;
            let mut batch = vec![];
            while let Some(r) = rows.next().await? {
                batch.push((
                    r.get::<String>(0)?,
                    r.get::<String>(1)?,
                    r.get::<String>(2)?,
                ));
            }
            drop(rows);
            let mut sequence =
                scalar(&c, "SELECT COALESCE(MAX(sequence),0) FROM deliveries").await?;
            for (oid, aid, value) in batch {
                sequence += 1;
                c.execute(
                    "INSERT INTO deliveries VALUES(?,?,?,?)",
                    params![sequence, oid.clone(), aid, value],
                )
                .await?;
                c.execute("UPDATE outbox SET published=1 WHERE id=?", [oid])
                    .await?;
            }
            Ok(())
        }
        .await;
        finish(&c, r).await
    }
    pub async fn delivery_cursor(&self) -> Result<i64> {
        let c = self.connect().await?;
        scalar(&c, "SELECT COALESCE(MAX(sequence),0) FROM deliveries").await
    }
    pub async fn events(&self, after: i64) -> Result<Vec<Value>> {
        let c = self.connect().await?;
        let mut rows=c.query("SELECT sequence,analysis,value FROM deliveries WHERE sequence>? ORDER BY sequence LIMIT 100",[after]).await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            out.push(json!({"sequence":r.get::<i64>(0)?,"analysisId":r.get::<String>(1)?,"change":serde_json::from_str::<Value>(&r.get::<String>(2)?)?}));
        }
        Ok(out)
    }
}
async fn info(c: &Connection, aid: &str) -> Result<AnalysisInfo> {
    let r = c
        .query(
            "SELECT id,name,artifact,fingerprint,archived,version,created FROM analyses WHERE id=?",
            [aid],
        )
        .await?
        .next()
        .await?
        .ok_or_else(|| AppError::Missing("Analysis not found.".into()))?;
    Ok(AnalysisInfo {
        id: r.get(0)?,
        name: r.get(1)?,
        artifact: r.get(2)?,
        fingerprint: r.get(3)?,
        archived: r.get::<i64>(4)? != 0,
        version: r.get::<i64>(5)? as u64,
        created: r.get(6)?,
    })
}
fn expect(actual: u64, expected: u64) -> Result<()> {
    if actual != expected {
        return Err(AppError::Conflict(
            "This value changed. Review the current value before submitting again.".into(),
        )
        .into());
    }
    Ok(())
}
async fn finish<T>(c: &Connection, result: Result<T>) -> Result<T> {
    match result {
        Ok(v) => match execute(c, "COMMIT").await {
            Ok(()) => Ok(v),
            Err(e) => {
                let _ = execute(c, "ROLLBACK").await;
                Err(e)
            }
        },
        Err(e) => {
            let _ = execute(c, "ROLLBACK").await;
            Err(e)
        }
    }
}
async fn receipt(c: &Connection, user: &str, command: &str, hash: &str) -> Result<Option<Value>> {
    let r = c
        .query(
            "SELECT digest,outcome FROM receipts WHERE user_id=? AND command=?",
            params![user, command],
        )
        .await?
        .next()
        .await?;
    if let Some(r) = r {
        ensure!(
            r.get::<String>(0)? == hash,
            "Command identifier reused with different content"
        );
        return Ok(Some(serde_json::from_str(&r.get::<String>(1)?)?));
    }
    Ok(None)
}
async fn save_receipt(
    c: &Connection,
    user: &str,
    command: &str,
    hash: &str,
    outcome: &Value,
) -> Result<()> {
    c.execute(
        "INSERT INTO receipts VALUES(?,?,?,?)",
        params![user, command, hash, outcome.to_string()],
    )
    .await?;
    Ok(())
}
async fn outbox(c: &Connection, aid: &str, value: Value) -> Result<()> {
    c.execute(
        "INSERT INTO outbox(id,analysis,value) VALUES(?,?,?)",
        params![id(), aid, value.to_string()],
    )
    .await?;
    Ok(())
}
async fn audit(
    c: &Connection,
    aid: &str,
    entity: &str,
    user: &User,
    kind: &str,
    before: Value,
    after: Value,
) -> Result<()> {
    let event = json!({"entity":entity,"actor":user.actor(),"action":kind,"timestamp":chrono::Utc::now().to_rfc3339(),"before":before,"after":after});
    c.execute(
        "INSERT INTO audit VALUES(?,?,?,?)",
        params![aid, id(), entity, event.to_string()],
    )
    .await?;
    Ok(())
}
