use anyhow::{bail, ensure, Context, Result};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Barrier, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
enum Engine {
    Turso(turso::Database),
    Sqlite(PathBuf),
}
enum Conn {
    Turso(turso::Connection),
    Sqlite(rusqlite::Connection),
}
impl Engine {
    async fn open(kind: &str, path: &Path) -> Result<Self> {
        Ok(if kind == "turso" {
            Self::Turso(
                turso::Builder::new_local(path.to_str().unwrap())
                    .build()
                    .await?,
            )
        } else {
            Self::Sqlite(path.into())
        })
    }
    async fn connect(&self) -> Result<Conn> {
        let c = match self {
            Self::Turso(db) => Conn::Turso(db.connect()?),
            Self::Sqlite(path) => {
                let c = rusqlite::Connection::open(path)?;
                c.busy_timeout(Duration::from_millis(0))?;
                Conn::Sqlite(c)
            }
        };
        c.exec("PRAGMA foreign_keys=ON").await?;
        c.exec("PRAGMA synchronous=FULL").await?;
        Ok(c)
    }
}
impl Conn {
    async fn exec(&self, sql: &str) -> Result<u64> {
        match self {
            Self::Turso(c) if sql.starts_with("PRAGMA") => {
                let mut rows = c.query(sql, ()).await.with_context(|| sql.to_string())?;
                while rows.next().await?.is_some() {}
                Ok(0)
            }
            Self::Turso(c) => Ok(c.execute(sql, ()).await.with_context(|| sql.to_string())?),
            Self::Sqlite(c) => {
                c.execute_batch(sql)?;
                Ok(c.changes())
            }
        }
    }
    async fn int(&self, sql: &str) -> Result<i64> {
        match self {
            Self::Turso(c) => {
                let mut rows = c.query(sql, ()).await?;
                Ok(rows
                    .next()
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("missing row"))?
                    .get::<i64>(0)?)
            }
            Self::Sqlite(c) => Ok(c.query_row(sql, [], |r| r.get(0))?),
        }
    }
    async fn text(&self, sql: &str) -> Result<String> {
        match self {
            Self::Turso(c) => {
                let mut rows = c.query(sql, ()).await?;
                Ok(rows
                    .next()
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("missing row"))?
                    .get::<String>(0)?)
            }
            Self::Sqlite(c) => Ok(c.query_row(sql, [], |r| r.get(0))?),
        }
    }
    fn begin(&self) -> &'static str {
        match self {
            Self::Turso(_) => "BEGIN CONCURRENT",
            Self::Sqlite(_) => "BEGIN IMMEDIATE",
        }
    }
}
fn emit(name: &str, detail: serde_json::Value) {
    println!("{}", json!({"probe":name,"detail":detail}));
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn retryable(e: &anyhow::Error) -> bool {
    let s = format!("{e:?}").to_lowercase();
    s.contains("busy") || s.contains("locked") || s.contains("conflict")
}
async fn connect_retry(e: &Engine) -> Result<Conn> {
    for n in 0..200 {
        match e.connect().await {
            Ok(c) => return Ok(c),
            Err(err) if retryable(&err) => {
                tokio::time::sleep(Duration::from_millis((n as u64).min(20) + 1)).await
            }
            Err(err) => return Err(err),
        }
    }
    bail!("connection initialization retry budget exhausted")
}
async fn setup(e: &Engine) -> Result<()> {
    let c = e.connect().await?;
    let mode = if matches!(e, Engine::Turso(_)) {
        "mvcc"
    } else {
        "wal"
    };
    c.exec(&format!("PRAGMA journal_mode='{mode}'")).await?;
    emit("journal_mode", json!(c.text("PRAGMA journal_mode").await?));
    emit("synchronous", json!(c.int("PRAGMA synchronous").await?));
    for sql in [
        "CREATE TABLE analyses(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, archived INTEGER NOT NULL DEFAULT 0)",
        "CREATE TABLE labels(id INTEGER PRIMARY KEY, analysis_id INTEGER NOT NULL REFERENCES analyses(id), value TEXT NOT NULL, version INTEGER NOT NULL)",
        "CREATE TABLE workflows(id INTEGER PRIMARY KEY, status TEXT NOT NULL, sme TEXT, owner TEXT, eta TEXT, version INTEGER NOT NULL)",
        "CREATE TABLE assignments(id INTEGER PRIMARY KEY, reviewer INTEGER, version INTEGER NOT NULL)",
        "CREATE TABLE comments(id TEXT PRIMARY KEY, analysis_id INTEGER REFERENCES analyses(id), author INTEGER NOT NULL, body TEXT NOT NULL, version INTEGER NOT NULL)",
        "CREATE TABLE history(id TEXT PRIMARY KEY, actor INTEGER NOT NULL, before_version INTEGER, after_version INTEGER)",
        "CREATE TABLE commands(id TEXT PRIMARY KEY, payload TEXT NOT NULL)",
        "CREATE TABLE notifications(id TEXT PRIMARY KEY, entity INTEGER NOT NULL)",
        "CREATE TABLE views(id INTEGER PRIMARY KEY, payload TEXT NOT NULL, version INTEGER NOT NULL)",
    ] { c.exec(sql).await?; }
    c.exec("BEGIN").await?;
    for i in 0..1000 {
        c.exec(&format!(
            "INSERT INTO analyses VALUES({i},'analysis-{i}',0)"
        ))
        .await?;
    }
    for i in 0..100 {
        c.exec(&format!(
            "INSERT INTO labels VALUES({i},{},'original',0)",
            i / 10
        ))
        .await?;
        c.exec(&format!(
            "INSERT INTO workflows VALUES({i},'Review',NULL,NULL,NULL,0)"
        ))
        .await?;
        c.exec(&format!("INSERT INTO assignments VALUES({i},NULL,0)"))
            .await?;
        c.exec(&format!("INSERT INTO views VALUES({i},'{{}}',0)"))
            .await?;
    }
    c.exec("COMMIT").await?;
    Ok(())
}
async fn schema(e: &Engine) -> Result<()> {
    let c = e.connect().await?;
    ensure!(
        c.exec("INSERT INTO analyses VALUES(1001,'analysis-0',0)")
            .await
            .is_err(),
        "unique constraint not enforced"
    );
    let fk = c
        .exec("INSERT INTO labels VALUES(1001,9999,'orphan',0)")
        .await;
    emit(
        "foreign_key",
        json!({"rejected":fk.is_err(),"error":fk.err().map(|e|e.to_string())}),
    );
    c.exec("BEGIN").await?;
    c.exec("UPDATE labels SET value='rollback' WHERE id=0")
        .await?;
    c.exec("ROLLBACK").await?;
    ensure!(c.text("SELECT value FROM labels WHERE id=0").await? == "original");
    c.exec("ALTER TABLE analyses ADD COLUMN created_at TEXT")
        .await?;
    c.exec("CREATE INDEX label_analysis ON labels(analysis_id)")
        .await?;
    ensure!(
        c.int("SELECT COUNT(*) FROM labels l JOIN analyses a ON a.id=l.analysis_id")
            .await?
            == 100
    );
    emit(
        "schema",
        json!({"unique":true,"rollback":true,"migration":true,"join":true}),
    );
    Ok(())
}

// The transaction models label + attributed history + retry record + notification atomically.
async fn command(c: &Conn, id: &str, row: usize, expected: i64) -> Result<&'static str> {
    c.exec(c.begin()).await?;
    let r=async {
        if c.int(&format!("SELECT COUNT(*) FROM commands WHERE id='{id}'")).await? > 0 { return Ok("replayed"); }
        if c.int(&format!("SELECT version FROM labels WHERE id={row}")).await? != expected { return Ok("stale"); }
        c.exec(&format!("UPDATE labels SET value='{id}',version=version+1 WHERE id={row} AND version={expected}")).await?;
        c.exec(&format!("INSERT INTO history VALUES('{id}',{row},{expected},{})",expected+1)).await?;
        c.exec(&format!("INSERT INTO commands VALUES('{id}','fixed-payload')")).await?;
        c.exec(&format!("INSERT INTO notifications VALUES('{id}',{row})")).await?;
        Ok::<_,anyhow::Error>("committed")
    }.await;
    match r {
        Ok(status) => match c.exec("COMMIT").await {
            Ok(_) => Ok(status),
            Err(e) => {
                let _ = c.exec("ROLLBACK").await;
                Err(e)
            }
        },
        Err(e) => {
            let _ = c.exec("ROLLBACK").await;
            Err(e)
        }
    }
}
async fn with_retry(c: &Conn, id: &str, row: usize, expected: i64) -> Result<(String, usize)> {
    for n in 0..200 {
        match command(c, id, row, expected).await {
            Ok(s) => return Ok((s.into(), n)),
            Err(e) if retryable(&e) => {
                tokio::time::sleep(Duration::from_millis((n as u64).min(20) + 1)).await
            }
            Err(e) => return Err(e),
        }
    }
    bail!("retry budget exhausted")
}
async fn load(e: &Engine, users: usize, rounds: usize, cap: usize) -> Result<()> {
    let base = e
        .connect()
        .await?
        .int("SELECT version FROM labels WHERE id=0")
        .await?;
    let gate = Arc::new(Barrier::new(users));
    let admission = Arc::new((Mutex::new(cap), Condvar::new()));
    let start = Instant::now();
    let handles: Vec<_> = (0..users)
        .map(|u| {
            let e = e.clone();
            let gate = gate.clone();
            let admission = admission.clone();
            std::thread::spawn(move || {
                runtime().block_on(async move {
                    gate.wait();
                    let c = connect_retry(&e).await?;
                    let mut samples = vec![];
                    let mut retries = 0;
                    for n in 0..rounds {
                        let t = Instant::now();
                        {
                            let mut available = admission.0.lock().unwrap();
                            while *available == 0 {
                                available = admission.1.wait(available).unwrap();
                            }
                            *available -= 1;
                        }
                        let outcome = with_retry(
                            &c,
                            &format!("load-{users}-{cap}-{u}-{n}"),
                            u,
                            base + n as i64,
                        )
                        .await;
                        {
                            *admission.0.lock().unwrap() += 1;
                            admission.1.notify_one();
                        }
                        let (s, r) = outcome?;
                        ensure!(s == "committed", "unexpected {s}");
                        retries += r;
                        samples.push(t.elapsed().as_secs_f64() * 1000.);
                    }
                    Ok::<_, anyhow::Error>((samples, retries))
                })
            })
        })
        .collect();
    let mut times = vec![];
    let mut retries = 0;
    for h in handles {
        let (s, r) = h.join().map_err(|_| anyhow::anyhow!("worker panicked"))??;
        times.extend(s);
        retries += r;
    }
    times.sort_by(f64::total_cmp);
    let c = e.connect().await?;
    ensure!(
        c.int("SELECT COUNT(*) FROM history").await?
            == c.int("SELECT COUNT(*) FROM commands").await?
    );
    ensure!(
        c.int("SELECT COUNT(*) FROM notifications").await?
            == c.int("SELECT COUNT(*) FROM commands").await?
    );
    for u in 0..users {
        ensure!(
            c.int(&format!("SELECT version FROM labels WHERE id={u}"))
                .await?
                == base + rounds as i64
        );
    }
    emit(
        "load",
        json!({"users":users,"writer_cap":cap,"transactions":times.len(),"elapsed_ms":start.elapsed().as_millis(),"p50_ms":times[times.len()/2],"p95_ms":times[times.len()*95/100],"p99_ms":times[times.len()*99/100],"max_ms":times[times.len()-1],"retries":retries}),
    );
    // Reset versions together for the next user-count workload; preserve history.
    c.exec("UPDATE labels SET version=0").await?;
    Ok(())
}
async fn races(e: &Engine) -> Result<()> {
    let c = e.connect().await?;
    let a = with_retry(&c, "once", 0, 0).await?;
    let b = with_retry(&c, "once", 0, 0).await?;
    let stale = with_retry(&c, "stale", 0, 0).await?;
    ensure!(a.0 == "committed" && b.0 == "replayed" && stale.0 == "stale");
    emit(
        "idempotency_and_stale",
        json!({"first":a.0,"retry":b.0,"later":stale.0}),
    );
    if !matches!(e, Engine::Turso(_)) {
        return Ok(());
    }
    let a = e.connect().await?;
    let b = e.connect().await?;
    a.exec("BEGIN CONCURRENT").await?;
    b.exec("BEGIN CONCURRENT").await?;
    a.exec("UPDATE assignments SET reviewer=1,version=1 WHERE id=0")
        .await?;
    let second = async {
        b.exec("UPDATE assignments SET reviewer=2,version=1 WHERE id=0")
            .await?;
        b.exec("COMMIT").await
    }
    .await;
    a.exec("COMMIT").await?;
    let _ = b.exec("ROLLBACK").await;
    ensure!(second.is_err(), "both overlapping claims committed");
    emit(
        "same_row_conflict",
        json!({"rejected":true,"error":second.unwrap_err().to_string()}),
    );
    a.exec("BEGIN CONCURRENT").await?;
    ensure!(a.int("SELECT archived FROM analyses WHERE id=0").await? == 0);
    b.exec("BEGIN CONCURRENT").await?;
    b.exec("UPDATE analyses SET archived=1 WHERE id=0").await?;
    b.exec("COMMIT").await?;
    let skew = async {
        a.exec("UPDATE labels SET value='after-archive' WHERE id=1")
            .await?;
        a.exec("COMMIT").await
    }
    .await;
    let _ = a.exec("ROLLBACK").await;
    emit(
        "unguarded_archive_race",
        json!({"edit_committed_after_archive":skew.is_ok(),"error":skew.err().map(|e|e.to_string()),"interpretation":"If true, application lifecycle coordination is required."}),
    );
    Ok(())
}
async fn backup_probe(e: &Engine, path: &Path) -> Result<()> {
    let c = e.connect().await?;
    let dest = path.with_extension("vacuum-backup.db");
    let r = c
        .exec(&format!(
            "VACUUM INTO '{}'",
            dest.to_str().unwrap().replace('\'', "''")
        ))
        .await;
    let supported = r.is_ok();
    emit(
        "vacuum_into",
        json!({"supported":supported,"error":r.err().map(|e|format!("{e:#}"))}),
    );
    if supported {
        let kind = if matches!(e, Engine::Turso(_)) {
            "turso"
        } else {
            "sqlite"
        };
        let snapshot = Engine::open(kind, &dest).await?;
        let restored = snapshot.connect().await?;
        for table in [
            "analyses",
            "labels",
            "workflows",
            "assignments",
            "comments",
            "history",
            "commands",
            "notifications",
            "views",
        ] {
            ensure!(
                restored
                    .int(&format!("SELECT COUNT(*) FROM {table}"))
                    .await?
                    == c.int(&format!("SELECT COUNT(*) FROM {table}")).await?,
                "snapshot count differs for {table}"
            );
        }
        ensure!(
            restored.text("SELECT value FROM labels WHERE id=0").await?
                == c.text("SELECT value FROM labels WHERE id=0").await?
        );
        emit(
            "vacuum_restore",
            json!({"table_counts_match":true,"label_matches":true,"concurrent_writes_during_backup":false}),
        );
    }
    Ok(())
}
async fn semantics(e: &Engine) -> Result<()> {
    let c = e.connect().await?;
    c.exec("BEGIN").await?;
    c.exec("UPDATE workflows SET status='InProgress',sme='sme@example.test',owner='owner@example.test',eta='2026-12-01',version=1 WHERE id=1").await?;
    c.exec("UPDATE assignments SET reviewer=7,version=version+1 WHERE id=1")
        .await?;
    c.exec("COMMIT").await?;
    ensure!(c.text("SELECT sme FROM workflows WHERE id=1").await? == "sme@example.test");
    c.exec("UPDATE workflows SET eta='2026-12-02',version=version+1 WHERE id=1")
        .await?;
    ensure!(c.int("SELECT reviewer FROM assignments WHERE id=1").await? == 7);
    c.exec("INSERT INTO comments VALUES('comment-owner',0,1,'original',0)")
        .await?;
    c.exec("UPDATE comments SET body='wrong',version=1 WHERE id='comment-owner' AND author=2 AND version=0").await?;
    ensure!(
        c.text("SELECT body FROM comments WHERE id='comment-owner'")
            .await?
            == "original"
    );
    c.exec("UPDATE comments SET body='edited',version=1 WHERE id='comment-owner' AND author=1 AND version=0").await?;
    c.exec("UPDATE comments SET body='stale',version=2 WHERE id='comment-owner' AND author=1 AND version=0").await?;
    ensure!(
        c.text("SELECT body FROM comments WHERE id='comment-owner'")
            .await?
            == "edited"
    );
    c.exec("DELETE FROM comments WHERE id='comment-owner' AND author=2")
        .await?;
    ensure!(
        c.int("SELECT COUNT(*) FROM comments WHERE id='comment-owner'")
            .await?
            == 1
    );
    c.exec("DELETE FROM comments WHERE id='comment-owner' AND author=1 AND version=1")
        .await?;
    ensure!(
        c.int("SELECT COUNT(*) FROM comments WHERE id='comment-owner'")
            .await?
            == 0
    );
    emit(
        "metadata_and_comments",
        json!({"reviewer_separate":true,"owner_checks":true,"stale_comment_preserved":true}),
    );
    if matches!(e, Engine::Turso(_)) {
        let a = e.connect().await?;
        let b = e.connect().await?;
        a.exec("BEGIN CONCURRENT").await?;
        let parent = a.int("SELECT version FROM workflows WHERE id=2").await?;
        b.exec("BEGIN CONCURRENT").await?;
        b.exec("UPDATE workflows SET status='Reviewed',version=version+1 WHERE id=2")
            .await?;
        b.exec("COMMIT").await?;
        let result = async {
            a.exec(&format!(
                "UPDATE workflows SET status='Reviewed',version={} WHERE id=3",
                parent + 1
            ))
            .await?;
            a.exec("COMMIT").await
        }
        .await;
        let _ = a.exec("ROLLBACK").await;
        emit(
            "unguarded_parent_theme_race",
            json!({"stale_parent_read_committed":result.is_ok(),"error":result.err().map(|e|e.to_string())}),
        );
        // Deliberately begin both independent writes before either commit.
        a.exec("BEGIN CONCURRENT").await?;
        b.exec("BEGIN CONCURRENT").await?;
        a.exec("UPDATE labels SET value='overlap-a' WHERE id=90")
            .await?;
        b.exec("UPDATE labels SET value='overlap-b' WHERE id=91")
            .await?;
        b.exec("COMMIT").await?;
        a.exec("COMMIT").await?;
        ensure!(c.text("SELECT value FROM labels WHERE id=90").await? == "overlap-a");
        ensure!(c.text("SELECT value FROM labels WHERE id=91").await? == "overlap-b");
        emit(
            "overlapping_independent_writers",
            json!({"both_committed":true}),
        );
    }
    Ok(())
}
async fn guarded(e: &Engine) -> Result<()> {
    if !matches!(e, Engine::Turso(_)) {
        return Ok(());
    }
    for parent in [false, true] {
        let lock = Arc::new(tokio::sync::RwLock::new(()));
        let c = e.connect().await?;
        c.exec("UPDATE analyses SET archived=0 WHERE id=0").await?;
        let read = lock.read().await;
        c.exec("BEGIN CONCURRENT").await?;
        let before = if parent {
            c.int("SELECT version FROM workflows WHERE id=4").await?
        } else {
            c.int("SELECT archived FROM analyses WHERE id=0").await?
        };
        let other = e.clone();
        let l = lock.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            runtime().block_on(async move {
                ensure!(
                    l.try_write().is_err(),
                    "exclusive change bypassed active edit"
                );
                tx.send(())?;
                let _write = l.write().await;
                let b = other.connect().await?;
                b.exec("BEGIN CONCURRENT").await?;
                b.exec(if parent {
                    "UPDATE workflows SET version=version+1 WHERE id=4"
                } else {
                    "UPDATE analyses SET archived=1 WHERE id=0"
                })
                .await?;
                b.exec("COMMIT").await?;
                Ok::<_, anyhow::Error>(())
            })
        });
        rx.recv_timeout(Duration::from_secs(10))?;
        c.exec(if parent {
            "UPDATE workflows SET version=version+1 WHERE id=5"
        } else {
            "UPDATE labels SET value='before-archive' WHERE id=2"
        })
        .await?;
        c.exec("COMMIT").await?;
        drop(read);
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("guard worker panicked"))??;
        let _read = lock.read().await;
        let after = if parent {
            c.int("SELECT version FROM workflows WHERE id=4").await?
        } else {
            c.int("SELECT archived FROM analyses WHERE id=0").await?
        };
        ensure!(after != before, "next edit did not see changed dependency");
        emit(
            if parent {
                "guarded_parent_theme"
            } else {
                "guarded_archive"
            },
            json!({"exclusive_change_waited_for_edit":true,"next_edit_sees_change":true}),
        );
    }
    // Expose why a preallocated sequence is unsafe, then test publishing committed rows.
    let a = e.connect().await?;
    let b = e.connect().await?;
    let reader = e.connect().await?;
    a.exec("CREATE TABLE delivery_probe(id INTEGER PRIMARY KEY,published INTEGER)")
        .await?;
    a.exec("BEGIN CONCURRENT").await?;
    b.exec("BEGIN CONCURRENT").await?;
    a.exec("INSERT INTO delivery_probe VALUES(100,NULL)")
        .await?;
    b.exec("INSERT INTO delivery_probe VALUES(101,NULL)")
        .await?;
    b.exec("COMMIT").await?;
    ensure!(reader.int("SELECT MAX(id) FROM delivery_probe").await? == 101);
    reader
        .exec("UPDATE delivery_probe SET published=1 WHERE id=101")
        .await?;
    a.exec("COMMIT").await?;
    ensure!(
        reader
            .int("SELECT COUNT(*) FROM delivery_probe WHERE id>101")
            .await?
            == 0
    );
    ensure!(
        reader
            .int("SELECT COUNT(*) FROM delivery_probe WHERE published IS NULL")
            .await?
            == 1
    );
    reader
        .exec("UPDATE delivery_probe SET published=2 WHERE id=100")
        .await?;
    ensure!(
        reader
            .int("SELECT id FROM delivery_probe WHERE published>1")
            .await?
            == 100
    );
    emit(
        "out_of_order_delivery",
        json!({"preallocated_cursor_misses_late_commit":true,"postcommit_publication_recovers_it":true,"full_sse_not_tested":true}),
    );
    Ok(())
}
async fn run(args: Vec<String>) -> Result<()> {
    let mode = &args[1];
    let kind = &args[2];
    let path = Path::new(&args[3]);
    let e = Engine::open(kind, path).await?;
    match mode.as_str() {
        "suite" => {
            setup(&e).await?;
            schema(&e).await?;
            load(&e, 10, 20, 10).await?;
            load(&e, 50, 20, 50).await?;
            load(&e, 50, 20, 8).await?;
            load(&e, 50, 20, 4).await?;
            races(&e).await?;
            semantics(&e).await?;
            guarded(&e).await?;
            backup_probe(&e, path).await?;
        }
        "crash" => {
            setup(&e).await?;
            let c = e.connect().await?;
            with_retry(&c, "durable", 0, 0).await?;
            println!("ACKNOWLEDGED");
            c.exec(c.begin()).await?;
            c.exec("UPDATE labels SET value='uncommitted',version=99 WHERE id=0")
                .await?;
            c.exec("INSERT INTO history VALUES('uncommitted',0,1,99)")
                .await?;
            println!("UNCOMMITTED");
            use std::io::Write;
            std::io::stdout().flush()?;
            std::thread::sleep(Duration::from_secs(120));
        }
        "verify" => {
            let c = e.connect().await?;
            ensure!(c.text("SELECT value FROM labels WHERE id=0").await? == "durable");
            ensure!(c.int("SELECT version FROM labels WHERE id=0").await? == 1);
            for table in ["history", "commands", "notifications"] {
                ensure!(c.int(&format!("SELECT COUNT(*) FROM {table}")).await? == 1);
            }
            ensure!(with_retry(&c, "durable", 0, 0).await?.0 == "replayed");
            emit(
                "recovery",
                json!({"acknowledged_survives":true,"uncommitted_absent":true,"retry_deduplicated":true}),
            );
        }
        _ => bail!("unknown mode"),
    }
    Ok(())
}
fn main() {
    if let Err(e) = runtime().block_on(run(std::env::args().collect())) {
        eprintln!("{e:?}");
        std::process::exit(1);
    }
}
