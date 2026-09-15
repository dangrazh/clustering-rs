//! Stopped-service backups. A manifest becomes visible only after every copied file is durable.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    engine: String,
    created: i64,
    files: Vec<Entry>,
}
#[derive(Serialize, Deserialize)]
struct Entry {
    path: String,
    bytes: u64,
    sha256: String,
}
fn hash(path: &Path) -> Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        bytes += n as u64;
    }
    Ok((bytes, format!("{:x}", hash.finalize())))
}
fn exclusive(root: &Path) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("instance.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file)
        .context("Stop the application before backup or restore")?;
    Ok(file)
}
fn create_destination(source: &Path, destination: &Path) -> Result<PathBuf> {
    ensure!(!destination.exists(), "Destination must be a new directory");
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let resolved = parent.canonicalize()?.join(
        destination
            .file_name()
            .context("Missing destination name")?,
    );
    ensure!(
        !resolved.starts_with(source),
        "Backup/restore destination must be outside its source"
    );
    fs::create_dir(&resolved)?;
    Ok(resolved)
}
fn copy_tree(
    source: &Path,
    relative: &Path,
    destination: &Path,
    entries: &mut Vec<Entry>,
) -> Result<()> {
    for file in fs::read_dir(source.join(relative))? {
        let file = file?;
        let rel = relative.join(file.file_name());
        if relative.as_os_str().is_empty()
            && matches!(file.file_name().to_str(), Some("instance.lock" | "staging"))
        {
            continue;
        }
        let kind = file.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "Symbolic links are not allowed in application storage"
        );
        if kind.is_dir() {
            fs::create_dir(destination.join(&rel))?;
            copy_tree(source, &rel, destination, entries)?;
            crate::artifacts::sync_directory(&destination.join(&rel))?;
        } else if kind.is_file() {
            fs::copy(file.path(), destination.join(&rel))?;
            fs::OpenOptions::new()
                .write(true)
                .open(destination.join(&rel))?
                .sync_all()?;
            let (bytes, sha256) = hash(&destination.join(&rel))?;
            entries.push(Entry {
                path: rel
                    .to_str()
                    .context("Invalid file path")?
                    .replace('\\', "/"),
                bytes,
                sha256,
            });
        }
    }
    Ok(())
}
pub fn backup(source: &Path, destination: &Path) -> Result<()> {
    let source = source.canonicalize()?;
    let _lock = exclusive(&source)?;
    ensure!(
        source.join("analysis.db").is_file(),
        "No application database found"
    );
    let destination = create_destination(&source, destination)?;
    let mut files = vec![];
    copy_tree(&source, Path::new(""), &destination, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let manifest = Manifest {
        version: 1,
        engine: "turso-0.7.2".into(),
        created: crate::storage::now(),
        files,
    };
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination.join("manifest.json"))?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    file.sync_all()?;
    crate::artifacts::sync_directory(&destination)?;
    tracing::info!(
        files = manifest.files.len(),
        "Application backup completed and manifest committed"
    );
    Ok(())
}
pub fn restore(source: &Path, destination: &Path) -> Result<()> {
    let source = source.canonicalize()?;
    let manifest: Manifest =
        serde_json::from_reader(fs::File::open(source.join("manifest.json"))?)?;
    ensure!(
        manifest.version == 1 && manifest.engine == "turso-0.7.2",
        "Unsupported backup version"
    );
    ensure!(
        manifest.files.iter().any(|e| e.path == "analysis.db"),
        "Backup has no database"
    );
    for entry in &manifest.files {
        let relative = Path::new(&entry.path);
        ensure!(
            !relative.as_os_str().is_empty()
                && relative
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))),
            "Unsafe backup path"
        );
        let path = source.join(relative).canonicalize()?;
        ensure!(
            path.starts_with(&source),
            "Backup file escapes source directory"
        );
        ensure!(
            hash(&path)? == (entry.bytes, entry.sha256.clone()),
            "Checksum mismatch: {}",
            entry.path
        );
    }
    let destination = create_destination(&source, destination)?;
    let _lock = exclusive(&destination)?;
    for entry in &manifest.files {
        let target = destination.join(&entry.path);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(source.join(&entry.path), &target)?;
        fs::OpenOptions::new()
            .write(true)
            .open(&target)?
            .sync_all()?;
        ensure!(
            hash(&target)? == (entry.bytes, entry.sha256.clone()),
            "Copied file verification failed"
        );
        crate::artifacts::sync_directory(target.parent().unwrap())?;
    }
    fs::create_dir_all(destination.join("staging"))?;
    crate::artifacts::sync_directory(&destination)?;
    tracing::info!(
        files = manifest.files.len(),
        "Application restore completed and verified"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stopped_backup_restores_database_and_rejects_tampering() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("data");
        let store = crate::storage::Store::open(&root, 4).await?;
        let user = store
            .user("fixture:operator", "Operator", "operator@example.invalid")
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
        let expected = store.review(&aid, &user).await?;
        let copy = dir.path().join("backup");
        assert!(backup(&root, &copy).is_err());
        drop(store);
        backup(&root, &copy)?;
        let restored = dir.path().join("restored");
        restore(&copy, &restored)?;
        let store = crate::storage::Store::open(&restored, 4).await?;
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
        drop(store);
        fs::OpenOptions::new()
            .append(true)
            .open(copy.join("analysis.db"))?
            .write_all(b"corrupt")?;
        assert!(restore(&copy, &dir.path().join("invalid")).is_err());
        Ok(())
    }
}
