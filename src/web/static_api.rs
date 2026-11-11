//! SysAdmin endpoints for static data, and the read-time join onto saved analyses.
use super::*;
use crate::static_data::{JoinConfig, Lookup};
use serde_json::json;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct StaticState {
    uploads: Mutex<HashMap<String, Upload>>,
    /// Lookups keyed by the static data signature, so analyses load without re-reading tables.
    cache: tokio::sync::Mutex<Option<(String, Arc<Vec<Lookup>>)>>,
}
struct Upload {
    owner: String,
    path: PathBuf,
    file_name: String,
    worksheets: Vec<String>,
    created: Instant,
}

/// The analysis with every configured static table joined on, plus the number of joined
/// columns. Stored artifacts are never changed.
pub(super) async fn enriched(
    state: &WebState,
    run: &Arc<crate::model::AnalysisRun>,
) -> Result<(Arc<crate::model::AnalysisRun>, usize)> {
    let lookups = lookups(state).await?;
    let applies = |l: &Lookup| {
        l.key_columns.iter().all(|k| {
            let k = crate::static_data::key_part(k);
            run.source.headers.iter().any(|h| crate::static_data::key_part(h) == k)
        })
    };
    // Most analyses match no table; they are served from the shared cached copy.
    if !lookups.iter().any(applies) {
        return Ok((run.clone(), 0));
    }
    let mut run = (**run).clone();
    tokio::task::spawn_blocking(move || {
        let added = crate::static_data::enrich(&mut run, &lookups);
        (Arc::new(run), added)
    })
    .await
    .map_err(Into::into)
}
async fn lookups(state: &WebState) -> Result<Arc<Vec<Lookup>>> {
    let store = &state.inner.store;
    let signature = store.static_signature().await?;
    let mut cache = state.inner.static_data.cache.lock().await;
    if let Some((cached, lookups)) = cache.as_ref() {
        if *cached == signature {
            return Ok(lookups.clone());
        }
    }
    let lookups = Arc::new(store.static_lookups().await?);
    *cache = Some((signature, lookups.clone()));
    Ok(lookups)
}

fn parse(path: &Path, sheet: Option<&str>) -> Result<crate::model::SourceTable> {
    let source = match sheet {
        Some(sheet) => import_xlsx_sheet(path, sheet)?,
        None => import_source(path)?,
    };
    anyhow::ensure!(
        source.rows.len() <= crate::static_data::MAX_ROWS
            && source.headers.len() <= crate::static_data::MAX_COLUMNS,
        "Static data is limited to {} rows and {} columns",
        crate::static_data::MAX_ROWS,
        crate::static_data::MAX_COLUMNS
    );
    Ok(source)
}
fn preview(id: &str, file_name: &str, worksheets: &[String], worksheet: Option<&str>, source: &crate::model::SourceTable) -> Result<serde_json::Value> {
    Ok(json!({
        "uploadId": id,
        "fileName": file_name,
        "worksheets": worksheets,
        "worksheet": worksheet,
        "rowCount": source.rows.len(),
        "columns": crate::static_data::infer(source)?,
        "preview": source.rows.iter().take(20).collect::<Vec<_>>(),
    }))
}

pub(super) async fn route(request: Request<Incoming>, state: WebState) -> Result<Response<BoxBody>> {
    let uri = request.uri().clone();
    let method = request.method().clone();
    let query = parse_query(uri.query());
    let user = state.user.clone().context("Sign in required")?;
    let store = &state.inner.store;
    let parts: Vec<&str> = uri
        .path()
        .trim_start_matches("/api/static-data")
        .trim_matches('/')
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
    let value = match (method, parts.as_slice()) {
        (Method::GET, []) => json!(store.static_tables().await?),
        (Method::POST, ["upload"]) => {
            let file_name = query
                .get("filename")
                .cloned()
                .filter(|n| !n.trim().is_empty())
                .context("filename query parameter is required")?;
            let extension = Path::new(&file_name)
                .extension()
                .and_then(|v| v.to_str())
                .map(str::to_ascii_lowercase)
                .filter(|e| matches!(e.as_str(), "xlsx" | "xlsm" | "xls" | "csv"))
                .context("Choose an Excel (.xlsx, .xls) or CSV file")?;
            let body = http_body_util::Limited::new(request.into_body(), 128 * 1024 * 1024)
                .collect()
                .await
                .map_err(|e| anyhow::anyhow!("Upload is too large or incomplete: {e}"))?
                .to_bytes();
            anyhow::ensure!(!body.is_empty(), "The uploaded file is empty");
            let id = state.next_id("static");
            let path = store.root.join("inputs").join(format!("{id}.{extension}"));
            tokio::fs::write(&path, &body).await?;
            let parse_path = path.clone();
            let parsed = tokio::task::spawn_blocking(move || {
                let worksheets = if is_excel(&parse_path) { list_worksheets(&parse_path)? } else { vec![] };
                let sheet = worksheets.first().cloned();
                let source = parse(&parse_path, sheet.as_deref())?;
                Ok::<_, anyhow::Error>((worksheets, sheet, source))
            })
            .await?;
            let (worksheets, sheet, source) = match parsed {
                Ok(v) => v,
                Err(e) => {
                    let _ = tokio::fs::remove_file(&path).await;
                    return Err(e);
                }
            };
            let upload = Upload { owner: user.id.clone(), path, file_name, worksheets, created: Instant::now() };
            let response = preview(&id, &upload.file_name, &upload.worksheets, sheet.as_deref(), &source)?;
            let mut uploads = state.inner.static_data.uploads.lock().unwrap();
            // Abandoned uploads are discarded after an hour.
            uploads.retain(|_, u| {
                let keep = u.created.elapsed() < Duration::from_secs(3600);
                if !keep {
                    let _ = std::fs::remove_file(&u.path);
                }
                keep
            });
            uploads.insert(id, upload);
            response
        }
        (Method::POST, ["upload", id, "worksheet"]) => {
            let sheet = query.get("sheet").context("sheet query parameter is required")?.clone();
            let (path, file_name, worksheets) = {
                let uploads = state.inner.static_data.uploads.lock().unwrap();
                let upload = uploads.get(*id).filter(|u| u.owner == user.id).context("Upload expired. Upload the file again.")?;
                anyhow::ensure!(upload.worksheets.contains(&sheet), "Unknown worksheet");
                (upload.path.clone(), upload.file_name.clone(), upload.worksheets.clone())
            };
            let chosen = sheet.clone();
            let source = tokio::task::spawn_blocking(move || parse(&path, Some(&chosen))).await??;
            preview(id, &file_name, &worksheets, Some(&sheet), &source)?
        }
        (Method::POST, ["create"]) => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Create {
                upload_id: String,
                name: String,
                worksheet: Option<String>,
                replace: Option<String>,
                expected: Option<i64>,
            }
            let payload: Create = shared::body(request).await?;
            let (path, file_name) = {
                let uploads = state.inner.static_data.uploads.lock().unwrap();
                let upload = uploads
                    .get(&payload.upload_id)
                    .filter(|u| u.owner == user.id)
                    .context("Upload expired. Upload the file again.")?;
                if let Some(sheet) = &payload.worksheet {
                    anyhow::ensure!(upload.worksheets.contains(sheet), "Unknown worksheet");
                }
                (upload.path.clone(), upload.file_name.clone())
            };
            let sheet = payload.worksheet.clone();
            let source = tokio::task::spawn_blocking(move || parse(&path, sheet.as_deref())).await??;
            let label = match &payload.worksheet {
                Some(sheet) => format!("{file_name} · {sheet}"),
                None => file_name,
            };
            let replace = match &payload.replace {
                Some(id) => Some((id.as_str(), payload.expected.context("Missing expected version")?)),
                None => None,
            };
            let id = store.save_static_table(&user, &payload.name, &label, &source, replace).await?;
            if let Some(upload) = state.inner.static_data.uploads.lock().unwrap().remove(&payload.upload_id) {
                let _ = std::fs::remove_file(upload.path);
            }
            json!({"id":id})
        }
        (Method::POST, [id, "config"]) => {
            #[derive(Deserialize)]
            struct Configure {
                expected: i64,
                config: JoinConfig,
            }
            let payload: Configure = shared::body(request).await?;
            store.configure_static_table(id, payload.expected, &payload.config).await?;
            json!({"saved":true})
        }
        (Method::POST, [id, "delete"]) => {
            store.delete_static_table(id).await?;
            json!({"deleted":true})
        }
        (Method::GET, [id, "rows"]) => {
            let offset = query.get("offset").map(|v| v.parse::<i64>()).transpose()?.unwrap_or(0);
            store.static_rows(id, offset).await?
        }
        _ => anyhow::bail!("Unknown static data operation"),
    };
    json_response(StatusCode::OK, &value)
}
