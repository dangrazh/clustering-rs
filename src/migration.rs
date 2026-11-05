//! Portable migration with an offline snapshot boundary and recoverable activation.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};
use tokio::sync::RwLock;

pub struct Coordinator {
    pub paused: AtomicBool,
    pub gate: RwLock<()>,
    pub operation: tokio::sync::Mutex<()>,
}
pub fn coordinator() -> &'static Coordinator {
    static INSTANCE: OnceLock<Coordinator> = OnceLock::new();
    INSTANCE.get_or_init(|| Coordinator {
        paused: AtomicBool::new(false),
        gate: RwLock::new(()),
        operation: tokio::sync::Mutex::new(()),
    })
}
pub fn paused() -> bool {
    coordinator().paused.load(Ordering::SeqCst)
}
pub fn pause() {
    coordinator().paused.store(true, Ordering::SeqCst);
}
pub fn resume() {
    coordinator().paused.store(false, Ordering::SeqCst);
}
pub fn service_lock(root: &Path) -> Result<fs::File> {
    let file=fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(base(root)?.join("service.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file).context("Stop the application before offline backup; another process owns this installation")?;
    Ok(file)
}
pub fn directory(root: &Path) -> Result<PathBuf> {
    Ok(base(root)?.join(".migration"))
}
fn base(root: &Path) -> Result<PathBuf> {
    let root = std::path::absolute(root)?;
    if root
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|p| p == ".stores")
    {
        Ok(root
            .parent()
            .unwrap()
            .parent()
            .context("Invalid store root")?
            .to_owned())
    } else {
        Ok(root)
    }
}
pub fn resolve(root: &Path) -> Result<PathBuf> {
    let base = base(root)?;
    let marker = base.join("active-store.json");
    if !marker.exists() {
        return Ok(base);
    }
    let value: Value = serde_json::from_slice(&fs::read(marker)?)?;
    if value["id"].is_null() {
        return Ok(base);
    }
    let id = value["id"]
        .as_str()
        .context("Invalid active-store marker")?;
    uuid::Uuid::parse_str(id)?;
    let active = base.join(".stores").join(id);
    ensure!(
        active.join("analysis.db").is_file(),
        "Selected active database is missing; do not initialize an empty replacement"
    );
    Ok(active)
}
pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::create_dir_all(path.parent().context("Missing parent")?)?;
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    crate::artifacts::sync_directory(path.parent().unwrap())
}
pub fn status(root: &Path) -> Result<Value> {
    let path = directory(root)?.join("status.json");
    if path.exists() {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    } else {
        Ok(json!({"state":"idle"}))
    }
}
pub fn limit() -> Result<u64> {
    Ok(crate::config::number("APP_MIGRATION_MAX_GIB", 64, 1, 1024)? as u64 * 1024 * 1024 * 1024)
}

/// Remove only generated transfer files, never active/previous datasets or recovery markers.
pub fn discard_packages(root: &Path) -> Result<usize> {
    let dir = directory(root)?;
    if !dir.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some((id, extension)) = name.to_str().and_then(|n| n.split_once('.')) else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_err()
            || !matches!(extension, "icam" | "upload" | "restore" | "partial")
        {
            continue;
        }
        ensure!(
            !entry.file_type()?.is_symlink(),
            "Unexpected symlink in migration storage"
        );
        if entry.file_type()?.is_dir() {
            ensure!(extension == "restore", "Unexpected migration directory");
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
        count += 1;
    }
    write_json(&dir.join("status.json"), &json!({"state":"idle"}))?;
    Ok(count)
}
pub fn file_hash(path: &Path) -> Result<(u64, String)> {
    let mut input = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buf = [0; 65536];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        bytes += n as u64;
    }
    Ok((bytes, format!("{:x}", hash.finalize())))
}
fn disk_usage(root: &Path) -> Result<u64> {
    let mut pending = vec![root.to_owned()];
    let mut total = 0u64;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                total = total
                    .checked_add(entry.metadata()?.len())
                    .context("Storage size overflow")?;
            }
            // External symlink targets are not part of this installation's storage footprint.
        }
    }
    Ok(total)
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub engine: String,
    pub application: String,
    pub schema: i64,
    pub created: i64,
    pub files: Vec<Entry>,
}
fn safe_path(path: &str) -> bool {
    if matches!(
        path,
        "analysis.db" | "analysis.db-wal" | "analysis.db-log" | "analysis.db-shm"
    ) {
        return true;
    }
    let Some((dir, name)) = path.split_once('/') else {
        return false;
    };
    matches!(dir, "artifacts" | "inputs")
        && !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}
fn dataset_files(root: &Path) -> Result<Vec<String>> {
    let mut files = vec![];
    for name in [
        "analysis.db",
        "analysis.db-wal",
        "analysis.db-log",
        "analysis.db-shm",
    ] {
        if root.join(name).exists() {
            ensure!(
                !root.join(name).symlink_metadata()?.file_type().is_symlink(),
                "Symlink in storage"
            );
            files.push(name.into());
        }
    }
    for dir in ["artifacts", "inputs"] {
        let directory = root.join(dir);
        if !directory.exists() {
            continue;
        }
        ensure!(
            !directory.symlink_metadata()?.file_type().is_symlink(),
            "Symlink in storage"
        );
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            ensure!(entry.file_type()?.is_file(), "Unexpected non-file in {dir}");
            let path = format!(
                "{dir}/{}",
                entry.file_name().to_str().context("Invalid file name")?
            );
            ensure!(safe_path(&path), "Unsafe data file name");
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}
fn exclusive(root: &Path) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("instance.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file)
        .context("Database is still in use; migration requires a fully stopped store")?;
    Ok(file)
}
pub fn pack(root: &Path, destination: &Path) -> Result<Manifest> {
    let _lock = exclusive(root)?;
    let mut files = vec![];
    for path in dataset_files(root)? {
        let (bytes, sha256) = file_hash(&root.join(&path))?;
        files.push(Entry {
            path,
            bytes,
            sha256,
        });
    }
    ensure!(
        files.iter().any(|f| f.path == "analysis.db"),
        "Database missing"
    );
    let total: u64 = files.iter().map(|f| f.bytes).sum();
    ensure!(total <= limit()?, "Migration exceeds configured size limit");
    ensure!(
        fs2::available_space(destination.parent().unwrap())? > total + 16 * 1024 * 1024,
        "Insufficient disk space for export"
    );
    let manifest = Manifest {
        format: 1,
        engine: "turso-0.7.2".into(),
        application: env!("CARGO_PKG_VERSION").into(),
        schema: 5,
        created: crate::storage::now(),
        files,
    };
    let header = serde_json::to_vec(&manifest)?;
    ensure!(
        header.len() <= 16 * 1024 * 1024,
        "Migration manifest too large"
    );
    let temporary = destination.with_extension("partial");
    let mut output = fs::File::create(&temporary)?;
    output.write_all(b"ICAMIG01")?;
    output.write_all(&(header.len() as u64).to_le_bytes())?;
    output.write_all(&header)?;
    for entry in &manifest.files {
        ensure!(
            std::io::copy(&mut fs::File::open(root.join(&entry.path))?, &mut output)?
                == entry.bytes,
            "File changed during snapshot"
        );
    }
    output.sync_all()?;
    drop(output);
    fs::rename(temporary, destination)?;
    crate::artifacts::sync_directory(destination.parent().unwrap())?;
    Ok(manifest)
}
pub fn unpack(package: &Path, destination: &Path) -> Result<Manifest> {
    ensure!(!destination.exists(), "Staging destination already exists");
    let mut input = fs::File::open(package)?;
    let mut magic = [0; 8];
    input.read_exact(&mut magic)?;
    ensure!(&magic == b"ICAMIG01", "Not a supported migration package");
    input.read_exact(&mut magic)?;
    let header_len = u64::from_le_bytes(magic);
    ensure!(header_len <= 16 * 1024 * 1024, "Manifest too large");
    let mut header = vec![0; header_len as usize];
    input.read_exact(&mut header)?;
    let manifest: Manifest = serde_json::from_slice(&header)?;
    ensure!(manifest.format == 1 && manifest.engine == "turso-0.7.2" && manifest.schema == 5 && manifest.application == env!("CARGO_PKG_VERSION"), "Incompatible migration package: use the same application build and supported database version");
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    for entry in &manifest.files {
        ensure!(
            safe_path(&entry.path) && seen.insert(entry.path.to_ascii_lowercase()),
            "Unsafe or duplicate package path"
        );
        total = total
            .checked_add(entry.bytes)
            .context("Package size overflow")?;
    }
    ensure!(seen.contains("analysis.db"), "Package has no database");
    ensure!(
        total <= limit()?
            && input.metadata()?.len()
                == total
                    .checked_add(header_len + 16)
                    .context("Size overflow")?,
        "Package is oversized, truncated or has trailing data"
    );
    ensure!(
        fs2::available_space(destination.parent().unwrap())? > total + 16 * 1024 * 1024,
        "Insufficient space for restore"
    );
    fs::create_dir(destination)?;
    for entry in &manifest.files {
        let path = destination.join(&entry.path);
        fs::create_dir_all(path.parent().unwrap())?;
        let mut out = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        ensure!(
            std::io::copy(&mut (&mut input).take(entry.bytes), &mut out)? == entry.bytes,
            "Truncated package"
        );
        out.sync_all()?;
        drop(out);
        ensure!(
            file_hash(&path)? == (entry.bytes, entry.sha256.clone()),
            "Checksum mismatch: {}",
            entry.path
        );
        crate::artifacts::sync_directory(path.parent().unwrap())?;
    }
    crate::artifacts::sync_directory(destination)?;
    Ok(manifest)
}

pub async fn inspect(store: &crate::storage::Store, integrity: bool) -> Result<Value> {
    let c = store.connect().await?;
    let mut counts = serde_json::Map::new();
    for table in [
        "analyses",
        "users",
        "trend_series",
        "jobs",
        "comments",
        "history",
        "reviewers",
        "views",
        "trend_tasks",
        "trend_history",
    ] {
        counts.insert(
            table.into(),
            json!(crate::storage::scalar(&c, &format!("SELECT COUNT(*) FROM {table}")).await?),
        );
    }
    counts.insert(
        "archived_analyses".into(),
        json!(crate::storage::scalar(&c, "SELECT COUNT(*) FROM analyses WHERE archived=1").await?),
    );
    for state in ["queued", "running"] {
        counts.insert(
            format!("{state}_jobs"),
            json!(
                crate::storage::scalar(
                    &c,
                    &format!("SELECT COUNT(*) FROM jobs WHERE state='{state}'")
                )
                .await?
            ),
        );
    }
    let schema = crate::storage::scalar(&c, "SELECT version FROM schema_version").await?;
    let mut analyses = vec![];
    let mut refs = BTreeSet::new();
    let mut rows = c.query("SELECT a.id,a.name,a.artifact,a.archived,u.name FROM analyses a JOIN users u ON u.id=a.creator ORDER BY a.name", ()).await?;
    while let Some(row) = rows.next().await? {
        let artifact: String = row.get(2)?;
        let path = format!("artifacts/{artifact}");
        let available = safe_path(&path) && store.root.join(&path).is_file();
        refs.insert(path);
        analyses.push(json!({"id":row.get::<String>(0)?,"name":row.get::<String>(1)?,"artifact":artifact,"archived":row.get::<i64>(3)? != 0,"owner":row.get::<String>(4)?,"available":available}));
    }
    let mut rows = c.query("SELECT input,artifact FROM jobs", ()).await?;
    while let Some(row) = rows.next().await? {
        for (i, dir) in [(0, "inputs"), (1, "artifacts")] {
            if let turso::Value::Text(name) = row.get_value(i)? {
                refs.insert(format!("{dir}/{name}"));
            }
        }
    }
    let root = store.root.canonicalize()?;
    let scan_root = root.clone();
    let scan = tokio::task::spawn_blocking(move || -> Result<Value> {
        let files = dataset_files(&scan_root)?; let mut sizes = serde_json::Map::new(); let mut total = 0; let mut problems = vec![];
        for name in &files { let size = fs::metadata(scan_root.join(name))?.len(); total += size; sizes.insert(name.clone(), json!(size)); }
        for path in &refs {
            if !safe_path(path) || !scan_root.join(path).is_file() { problems.push(format!("Missing or unsafe reference: {path}")); continue; }
            if integrity {
                match file_hash(&scan_root.join(path)) {
                    Err(e) => problems.push(format!("Unreadable {path}: {e}")),
                    Ok((_, hash)) if path.starts_with("artifacts/") && path != &format!("artifacts/{hash}.icas") => problems.push(format!("Artifact checksum mismatch: {path}")),
                    _ => {}
                }
            }
        }
        let unreferenced: Vec<_> = files.iter().filter(|p| p.contains('/') && !refs.contains(*p)).collect();
        Ok(json!({"bytes":total,"installationBytes":disk_usage(&base(&scan_root)?)?,"files":sizes,"problems":problems,"unreferenced":unreferenced,"freeBytes":fs2::available_space(&scan_root)?,"checksumsVerified":integrity}))
    }).await??;
    Ok(
        json!({"application":env!("CARGO_PKG_VERSION"),"engine":"turso-0.7.2","schema":schema,"root":root,"database":root.join("analysis.db"),"measured":crate::storage::now(),"counts":counts,"analyses":analyses,"storage":scan}),
    )
}
pub async fn empty(store: &crate::storage::Store) -> Result<()> {
    let c = store.connect().await?;
    for table in ["analyses", "trend_series", "jobs"] {
        ensure!(
            crate::storage::scalar(&c, &format!("SELECT COUNT(*) FROM {table}")).await? == 0,
            "Destination is not empty: contains {table}"
        );
    }
    Ok(())
}
pub async fn validate_schema(root: &Path) -> Result<()> {
    // Check before Store::open can run migrations on an imported database.
    let db = turso::Builder::new_local(root.join("analysis.db").to_str().context("Invalid path")?)
        .build()
        .await?;
    let c = db.connect()?;
    ensure!(
        crate::storage::scalar(&c, "SELECT version FROM schema_version").await? == 5,
        "Unsupported package database schema"
    );
    ensure!(
        c.query("PRAGMA foreign_key_check", ())
            .await?
            .next()
            .await?
            .is_none(),
        "Package contains broken database relationships"
    );
    Ok(())
}
/// Called only after all server tasks and database handles have been dropped.
pub async fn offline(root: &Path) -> Result<bool> {
    let dir = directory(root)?;
    let status_path = dir.join("status.json");
    let mut status = status(root)?;
    if status["state"] != "draining" && status["state"] != "activating" {
        return Ok(false);
    }
    let id = status["id"]
        .as_str()
        .context("Operation ID missing")?
        .to_owned();
    uuid::Uuid::parse_str(&id)?;
    let operation = status["operation"].as_str().unwrap_or("").to_owned();
    let result: Result<()> = async {
        if operation == "export" {
            let root = resolve(root)?;
            let checked = crate::storage::Store::open(&root, 1).await?;
            let report = inspect(&checked, true).await?;
            ensure!(
                report["storage"]["problems"]
                    .as_array()
                    .is_some_and(|p| p.is_empty()),
                "Export references failed validation"
            );
            status["report"] = report;
            drop(checked);
            let output = dir.join(format!("{id}.icam"));
            let manifest = tokio::task::spawn_blocking(move || pack(&root, &output)).await??;
            status["state"] = json!("export-ready");
            status["manifest"] = serde_json::to_value(manifest)?;
        } else if operation == "activate" {
            let base = base(root)?;
            let stage = dir.join(format!("{id}.restore"));
            let installed = base.join(".stores").join(&id);
            let marker = base.join("active-store.json");
            let rollback = dir.join(format!("{id}.previous.json"));
            status["state"] = json!("activating");
            write_json(&status_path, &status)?;
            // The mounted root never moves. A durable marker selects the verified dataset.
            if !rollback.exists() {
                let original = crate::storage::Store::open(resolve(root)?, 1).await?;
                empty(&original).await?;
                drop(original);
                let previous = if marker.exists() {
                    serde_json::from_slice(&fs::read(&marker)?)?
                } else {
                    json!({"id":null})
                };
                write_json(&rollback, &previous)?;
            }
            if !installed.exists() {
                let current = crate::storage::Store::open(&stage, 1).await?;
                let snapshot = inspect(&current, true).await?;
                ensure!(
                    snapshot["storage"]["problems"]
                        .as_array()
                        .is_some_and(|p| p.is_empty()),
                    "Restored data failed integrity validation"
                );
                let c = current.connect().await?;
                c.execute("DELETE FROM sessions", ()).await?;
                drop(c);
                drop(current);
                fs::create_dir_all(installed.parent().unwrap())?;
                fs::rename(&stage, &installed).context("Installing staged data")?;
                crate::artifacts::sync_directory(installed.parent().unwrap())?;
            }
            let verified = crate::storage::Store::open(&installed, 1).await?;
            let report = inspect(&verified, true).await?;
            drop(verified);
            ensure!(
                report["storage"]["problems"]
                    .as_array()
                    .is_some_and(|p| p.is_empty()),
                "Activated data failed validation; rollback retained"
            );
            write_json(&marker, &json!({"id":id}))?;
            status["state"] = json!("complete");
            status["report"] = report;
        } else {
            anyhow::bail!("Unknown offline operation");
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        status["state"] = json!("failed");
        status["error"] = json!(error.to_string());
        tracing::error!(%error, "Migration failed; original or rollback data retained");
    }
    write_json(&status_path, &status)?;
    resume();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Set MIGRATION_CROSS_INPUT and MIGRATION_CROSS_OUTPUT for a real cross-OS rehearsal"]
    async fn cross_platform_migration_package() -> Result<()> {
        let input = PathBuf::from(std::env::var("MIGRATION_CROSS_INPUT")?);
        let output = PathBuf::from(std::env::var("MIGRATION_CROSS_OUTPUT")?);
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("restored");
        unpack(&input, &root)?;
        validate_schema(&root).await?;
        let store = crate::storage::Store::open(&root, 1).await?;
        let report = inspect(&store, true).await?;
        ensure!(
            report["counts"]["analyses"] == 2,
            "Cross-platform fixture lost saved analyses"
        );
        ensure!(
            report["storage"]["problems"]
                .as_array()
                .is_some_and(|p| p.is_empty()),
            "Cross-platform integrity check failed"
        );
        drop(store);
        pack(&root, &output)?;
        println!(
            "Cross-platform import/export verified: {}",
            report["counts"]
        );
        Ok(())
    }
    #[tokio::test]
    #[ignore = "Build the application binary first; launches two isolated servers and Playwright"]
    async fn browser_sysadmin_migration_between_installations() -> Result<()> {
        struct Server(std::process::Child);
        impl Drop for Server {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        let store = crate::storage::Store::open(&source, 1).await?;
        let user = store
            .user(
                "allowlist:admin@example.invalid",
                "Administrator",
                "admin@example.invalid",
            )
            .await?;
        let artifacts = crate::artifacts::Artifacts::new(source.clone(), 64 * 1024 * 1024);
        let run = std::sync::Arc::new(crate::fixtures::run(20));
        let review = crate::session::ReviewData::new(&run)?;
        let view = crate::session::ViewState::default();
        let artifact = artifacts
            .publish(run, review.clone(), view.clone(), Default::default())
            .await?;
        for name in ["Migration A", "Migration B"] {
            store
                .create(
                    &user,
                    &crate::storage::id(),
                    name,
                    &artifact,
                    crate::storage::InitialReview {
                        review: &review,
                        view: &view,
                        metadata: &Default::default(),
                    },
                )
                .await?;
        }
        drop(store);
        let binary = std::env::current_exe()?
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(if cfg!(windows) {
                "incident-clustering-analyzer.exe"
            } else {
                "incident-clustering-analyzer"
            });
        let mut servers = vec![];
        let mut addresses = vec![];
        for root in [&source, &destination] {
            fs::create_dir_all(root)?;
            fs::write(root.join("allowed-users.json"), br#"[{"email":"admin@example.invalid","name":"Administrator","role":"SysAdmin"},{"email":"reader@example.invalid","name":"Reader","role":"Read Only User"},{"email":"user@example.invalid","name":"User"}]"#)?;
            let socket = std::net::TcpListener::bind("127.0.0.1:0")?;
            let address = socket.local_addr()?;
            drop(socket);
            let logs = fs::File::create(root.join("server.log"))?;
            let child = std::process::Command::new(&binary)
                .env("APP_DATA_DIR", root)
                .env("APP_AUTH_MODE", "allowlist")
                .env("APP_EMAIL_ALLOWLIST", root.join("allowed-users.json"))
                .env("APP_COOKIE_SECURE", "false")
                .env("CLUSTERING_WEB_BIND", address.to_string())
                .env("APP_CACHE_MIB", "64")
                .env("APP_WORKER_THREADS", "1")
                .env("APP_LOG_LEVEL", "WARN")
                .env("RUST_LOG", "warn")
                .env_remove("APP_LOG_FILE")
                .env("APP_LOG_CONSOLE", "true")
                .stdout(logs.try_clone()?)
                .stderr(logs)
                .spawn()?;
            servers.push(Server(child));
            addresses.push(format!("http://{address}"));
        }
        let client = reqwest::Client::new();
        for address in &addresses {
            let mut ready = false;
            for _ in 0..100 {
                if client
                    .get(format!("{address}/healthz"))
                    .send()
                    .await
                    .is_ok_and(|r| r.status().is_success())
                {
                    ready = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            ensure!(ready, "Fixture server did not start");
        }
        let output = tokio::process::Command::new("node")
            .arg("tests/sysadmin-browser.mjs")
            .args(&addresses)
            .output()
            .await?;
        println!("{}", String::from_utf8_lossy(&output.stdout));
        if !output.status.success() {
            for root in [&source, &destination] {
                eprintln!(
                    "{}",
                    fs::read_to_string(root.join("server.log")).unwrap_or_default()
                );
            }
        }
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let active = resolve(&destination)?;
        ensure!(active != destination, "Restored dataset was not selected");
        ensure!(
            destination.join("allowed-users.json").exists(),
            "Destination allowlist lost"
        );
        drop(servers);
        Ok(())
    }
    #[test]
    fn package_paths_are_portable_and_restricted() {
        for path in [
            "analysis.db",
            "analysis.db-log",
            "artifacts/abc.icas",
            "inputs/job-123.json",
        ] {
            assert!(safe_path(path));
        }
        for path in [
            "../analysis.db",
            "artifacts/../secret",
            "artifacts/a/b",
            "C:/data/analysis.db",
            "/analysis.db",
            "inputs/a\\b",
            "allowed-users.json",
            "inputs/.",
            "inputs/..",
            "inputs/a:stream",
        ] {
            assert!(!safe_path(path), "{path}");
        }
    }

    #[test]
    fn malformed_manifests_cannot_create_a_restore_directory() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let package = dir.path().join("bad.icam");
        for paths in [
            vec!["../escape", "analysis.db"],
            vec!["analysis.db", "ANALYSIS.DB"],
            vec!["allowed-users.json", "analysis.db"],
        ] {
            let manifest = Manifest {
                format: 1,
                engine: "turso-0.7.2".into(),
                application: env!("CARGO_PKG_VERSION").into(),
                schema: 5,
                created: 0,
                files: paths
                    .into_iter()
                    .map(|p| Entry {
                        path: p.into(),
                        bytes: 0,
                        sha256: crate::storage::digest(&[]),
                    })
                    .collect(),
            };
            let header = serde_json::to_vec(&manifest)?;
            let mut file = fs::File::create(&package)?;
            file.write_all(b"ICAMIG01")?;
            file.write_all(&(header.len() as u64).to_le_bytes())?;
            file.write_all(&header)?;
            drop(file);
            let output = dir.path().join("rejected");
            assert!(unpack(&package, &output).is_err());
            assert!(!output.exists());
        }
        Ok(())
    }
    #[tokio::test]
    async fn portable_package_restores_complete_dataset_and_rejects_tampering() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("data");
        let store = crate::storage::Store::open(&root, 4).await?;
        let user = store
            .user("fixture:operator", "Operator", "operator@example.invalid")
            .await?;
        store
            .save_dashboard_preferences(&user, &serde_json::json!({"mine":{"search":"Restore"}}))
            .await?;
        let artifacts = crate::artifacts::Artifacts::new(root.clone(), 64 * 1024 * 1024);
        let run = std::sync::Arc::new(crate::fixtures::run(20));
        let review = crate::session::ReviewData::new(&run)?;
        let view = crate::session::ViewState {
            pivot_rows: vec![2],
            ..Default::default()
        };
        let artifact = artifacts
            .publish(run, review.clone(), view.clone(), Default::default())
            .await?;
        let aid = store
            .create(
                &user,
                &crate::storage::id(),
                "Restore rehearsal",
                &artifact,
                crate::storage::InitialReview {
                    review: &review,
                    view: &view,
                    metadata: &Default::default(),
                },
            )
            .await?;
        store
            .command(
                aid.clone(),
                user.clone(),
                crate::storage::Command {
                    command_id: crate::storage::id(),
                    target: "1".into(),
                    expected: 0,
                    parent_version: None,
                    action: serde_json::json!({"type":"rename","label":"Durable review"}),
                },
            )
            .await?;
        store
            .command(
                aid.clone(),
                user.clone(),
                crate::storage::Command {
                    command_id: crate::storage::id(),
                    target: "1".into(),
                    expected: 0,
                    parent_version: None,
                    action: serde_json::json!({"type":"addComment","text":"Preserve ownership"}),
                },
            )
            .await?;
        let jid = format!("job-{}", crate::storage::id());
        crate::jobs::retain_import(&store, &user, &jid, &artifact).await?;
        let run = crate::fixtures::run(20);
        let queued = crate::jobs::enqueue(
            &store,
            &user,
            crate::jobs::Input {
                column_types: vec![],
                source: run.source,
                mapping: run.mapping,
                settings: run.settings,
            },
        )
        .await?;
        store
            .command(
                aid.clone(),
                user.clone(),
                crate::storage::Command {
                    command_id: crate::storage::id(),
                    target: "1:1".into(),
                    expected: 0,
                    parent_version: None,
                    action: json!({"type":"reviewer","userId":user.id}),
                },
            )
            .await?;
        let series = store
            .create_series(&user, &crate::storage::id(), "Restored trends")
            .await?;
        let series_id = series["id"].as_str().unwrap().to_owned();
        let trend_model =
            crate::trends::fit(&crate::fixtures::run(20), "backup-fixture".into(), false);
        let profiles = crate::trends::profiles(&crate::fixtures::run(20), &trend_model);
        store
            .save_trend_profile("backup-profile", &profiles)
            .await?;
        let task = store
            .series_command(
                &user,
                &series_id,
                &crate::trends::Command {
                    command_id: crate::storage::id(),
                    expected: 0,
                    action: crate::trends::Action::Recalculate,
                },
            )
            .await?;
        let auth = crate::auth::Auth::new(store.clone(), None)?;
        let _source_token = auth.issue(&user, 3600).await?;
        drop(auth);
        let second = store
            .create(
                &user,
                &crate::storage::id(),
                "Second saved analysis",
                &artifact,
                crate::storage::InitialReview {
                    review: &review,
                    view: &view,
                    metadata: &Default::default(),
                },
            )
            .await?;
        store
            .command(
                second,
                user.clone(),
                crate::storage::Command {
                    command_id: crate::storage::id(),
                    target: "".into(),
                    expected: 0,
                    parent_version: None,
                    action: json!({"type":"archive"}),
                },
            )
            .await?;
        let expected = store.review(&aid, &user).await?;
        let copy = dir.path().join("backup.icam");
        fs::write(
            root.join("allowed-users.json"),
            b"source operator configuration",
        )?;
        assert!(pack(&root, &copy).is_err());
        drop(store);
        pack(&root, &copy)?;
        let restored = dir.path().join("restored");
        unpack(&copy, &restored)?;
        assert!(!restored.join("allowed-users.json").exists());
        let store = crate::storage::Store::open(&restored, 4).await?;
        assert_eq!(
            store.dashboard_preferences(&user).await?["mine"]["search"],
            "Restore"
        );
        assert_eq!(store.series(&series_id).await?.name, "Restored trends");
        assert_eq!(
            store.trend_profile("backup-profile").await?.unwrap().len(),
            profiles.len()
        );
        assert_eq!(
            store
                .trend_task(task["task"].as_str().unwrap())
                .await?
                .state,
            "queued"
        );
        assert_eq!(store.users().await?[0].id, user.id);
        assert_eq!(store.review(&aid, &user).await?, expected);
        let restored_artifacts =
            crate::artifacts::Artifacts::new(restored.clone(), 64 * 1024 * 1024);
        let restored_run = restored_artifacts.load(&artifact).await?;
        assert_eq!(restored_run.run.source.rows.len(), 20);
        let retained = crate::jobs::get(&store, &jid).await?;
        assert_eq!(retained.owner, user.id);
        assert_eq!(retained.artifact.as_deref(), Some(artifact.as_str()));
        assert_eq!(
            store.review(&aid, &user).await?["view"]["pivotRows"],
            serde_json::json!([2])
        );
        let expires = retained.expires;
        drop(store);
        let destination = dir.path().join("destination");
        let bootstrap = crate::storage::Store::open(&destination, 1).await?;
        empty(&bootstrap).await?;
        drop(bootstrap);
        let external_config = b"destination operator configuration";
        fs::write(destination.join("allowed-users.json"), external_config)?;
        let migration_dir = directory(&destination)?;
        fs::create_dir_all(&migration_dir)?;
        let operation = crate::storage::id();
        unpack(&copy, &migration_dir.join(format!("{operation}.restore")))?;
        validate_schema(&migration_dir.join(format!("{operation}.restore"))).await?;
        write_json(
            &migration_dir.join("status.json"),
            &json!({"state":"draining","operation":"activate","id":operation}),
        )?;
        assert!(offline(&destination).await?);
        assert_eq!(
            status(&destination)?["state"],
            "complete",
            "{}",
            status(&destination)?
        );
        assert_eq!(
            fs::read(destination.join("allowed-users.json"))?,
            external_config
        );
        let activated = crate::storage::Store::open(resolve(&destination)?, 1).await?;
        assert_eq!(activated.review(&aid, &user).await?, expected);
        assert_eq!(crate::jobs::get(&activated, &jid).await?.expires, expires);
        assert_eq!(crate::jobs::get(&activated, &queued).await?.state, "queued");
        assert_eq!(inspect(&activated, true).await?["counts"]["analyses"], 2);
        let c = activated.connect().await?;
        assert_eq!(
            crate::storage::scalar(&c, "SELECT COUNT(*) FROM sessions").await?,
            0
        );
        assert!(empty(&activated).await.is_err());
        drop(c);
        drop(activated);
        assert!(!offline(&destination).await?);
        // Simulate a crash after the pointer switch but before marking completion.
        let mut journal = status(&destination)?;
        journal["state"] = json!("activating");
        write_json(&migration_dir.join("status.json"), &journal)?;
        assert!(offline(&destination).await?);
        assert_eq!(status(&destination)?["state"], "complete");
        // Restore validation failure must leave an empty destination and its config intact.
        let failed_destination = dir.path().join("failed-destination");
        let fresh = crate::storage::Store::open(&failed_destination, 1).await?;
        drop(fresh);
        let failed_dir = directory(&failed_destination)?;
        fs::create_dir_all(&failed_dir)?;
        let failed_id = crate::storage::id();
        let failed_stage = failed_dir.join(format!("{failed_id}.restore"));
        unpack(&copy, &failed_stage)?;
        fs::remove_file(failed_stage.join("artifacts").join(&artifact))?;
        write_json(
            &failed_dir.join("status.json"),
            &json!({"state":"draining","operation":"activate","id":failed_id}),
        )?;
        offline(&failed_destination).await?;
        assert_eq!(status(&failed_destination)?["state"], "failed");
        assert_eq!(resolve(&failed_destination)?, failed_destination);
        let original = crate::storage::Store::open(&failed_destination, 1).await?;
        empty(&original).await?;
        drop(original);
        assert!(discard_packages(&failed_destination)? > 0);
        assert!(failed_destination.join("analysis.db").is_file());
        fs::OpenOptions::new()
            .append(true)
            .open(&copy)?
            .write_all(b"corrupt")?;
        assert!(unpack(&copy, &dir.path().join("invalid")).is_err());
        Ok(())
    }
}
