//! Immutable, checksummed session artifacts with bounded coalesced loading.
use crate::{
    session::{self, ReviewData, ViewState},
    storage::{self, PortableMeta},
};
use anyhow::{ensure, Context, Result};
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

pub struct Artifacts {
    root: PathBuf,
    cache: Mutex<Cache>,
    capacity: usize,
}
struct Cache {
    entries: HashMap<String, (Arc<CachedArtifact>, usize, u64)>,
    bytes: usize,
    clock: u64,
}
pub struct CachedArtifact {
    pub run: Arc<crate::model::AnalysisRun>,
    pub review: ReviewData,
    pub view: ViewState,
    pub metadata: PortableMeta,
}
impl Artifacts {
    pub async fn cached_bytes(&self) -> usize {
        self.cache.lock().await.bytes
    }
    pub fn new(root: PathBuf, capacity: usize) -> Self {
        Self {
            root,
            capacity,
            cache: Mutex::new(Cache {
                entries: HashMap::new(),
                bytes: 0,
                clock: 0,
            }),
        }
    }
    pub async fn publish(
        &self,
        run: Arc<crate::model::AnalysisRun>,
        review: ReviewData,
        view: ViewState,
        meta: PortableMeta,
    ) -> Result<String> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let bytes = session::encode_portable(&run, &review, &view, &meta)?;
            // Verify compatibility before the database may reference this artifact.
            let _ = session::decode_session(&bytes)?;
            let name = format!("{}.icas", storage::digest(&bytes));
            let target = root.join("artifacts").join(&name);
            if target.exists() {
                ensure!(
                    storage::digest(&std::fs::read(&target)?) == storage::digest(&bytes),
                    "Existing artifact integrity failure"
                );
                return Ok(name);
            }
            let staged = root.join("staging").join(format!("{}.part", storage::id()));
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&staged)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&staged, &target)?;
            sync_directory(&root.join("artifacts"))?;
            sync_directory(&root.join("staging"))?;
            Ok(name)
        })
        .await?
    }
    pub async fn load(&self, name: &str) -> Result<Arc<CachedArtifact>> {
        ensure!(
            name.len() == 69
                && name.is_ascii()
                && name.ends_with(".icas")
                && name[..64].bytes().all(|c| c.is_ascii_hexdigit()),
            "Invalid artifact identifier"
        );
        // Serialize cache fills: at most one large decoder allocates at a time.
        let mut cache = self.cache.lock().await;
        cache.clock += 1;
        let clock = cache.clock;
        if let Some((run, _, used)) = cache.entries.get_mut(name) {
            *used = clock;
            return Ok(run.clone());
        }
        let path = self.root.join("artifacts").join(name);
        let expected = name[..64].to_owned();
        let (loaded, size) = tokio::task::spawn_blocking(move || {
            ensure!(
                std::fs::metadata(&path)?.len() <= session::MAX_FILE_BYTES as u64,
                "Artifact too large"
            );
            let bytes = std::fs::read(path)?;
            ensure!(
                storage::digest(&bytes) == expected,
                "Artifact checksum mismatch"
            );
            let run = session::decode_session(&bytes)?;
            let source_bytes = run
                .run
                .source
                .rows
                .iter()
                .flatten()
                .map(|s| s.capacity() + std::mem::size_of::<String>())
                .sum::<usize>();
            let processed = run
                .run
                .processed_incidents
                .iter()
                .map(|r| {
                    r.analysis_text.capacity()
                        + r.incident_number.capacity()
                        + std::mem::size_of_val(r)
                })
                .sum::<usize>();
            let loaded = CachedArtifact {
                run: Arc::new(run.run),
                review: run.review,
                view: run.view,
                metadata: run.metadata,
            };
            Ok::<_, anyhow::Error>((
                Arc::new(loaded),
                (source_bytes + processed).saturating_mul(2),
            ))
        })
        .await??;
        while cache.bytes + size > self.capacity && !cache.entries.is_empty() {
            let oldest = cache
                .entries
                .iter()
                .min_by_key(|(_, (_, _, used))| *used)
                .map(|(k, _)| k.clone())
                .context("Cache entry missing")?;
            let (_, bytes, _) = cache.entries.remove(&oldest).unwrap();
            cache.bytes -= bytes;
        }
        if size <= self.capacity {
            cache.bytes += size;
            cache
                .entries
                .insert(name.into(), (loaded.clone(), size, clock));
        }
        Ok(loaded)
    }
}
pub fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
