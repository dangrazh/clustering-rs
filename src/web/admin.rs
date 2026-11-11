use super::*;
use crate::migration as m;
use serde_json::{json, Value};

pub(super) async fn route(
    request: Request<Incoming>,
    state: WebState,
) -> Result<Response<BoxBody>> {
    let root = state.inner.store.root.clone();
    let dir = m::directory(&root)?;
    tokio::fs::create_dir_all(&dir).await?;
    let path = request.uri().path().to_owned();
    if request.method() == Method::GET && path == "/api/admin/status" {
        return json_response(
            StatusCode::OK,
            &json!({"maintenance":m::paused(),"operation":m::status(&root)?}),
        );
    }
    if request.method() == Method::GET && path == "/api/admin/download" {
        let status = m::status(&root)?;
        anyhow::ensure!(
            status["state"] == "export-ready",
            "No completed export available"
        );
        let id = status["id"].as_str().context("Missing operation")?;
        uuid::Uuid::parse_str(id)?;
        let file = tokio::fs::File::open(dir.join(format!("{id}.icam"))).await?;
        let size = file.metadata().await?.len();
        let stream = stream::unfold(file, |mut file| async move {
            use tokio::io::AsyncReadExt;
            let mut bytes = vec![0; 65536];
            match file.read(&mut bytes).await {
                Ok(0) => None,
                Ok(n) => {
                    bytes.truncate(n);
                    Some((Ok::<_, Infallible>(Frame::data(Bytes::from(bytes))), file))
                }
                Err(error) => {
                    tracing::error!(%error,"Migration download interrupted");
                    None
                }
            }
        });
        return Ok(Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"migration-{id}.icam\""),
            )
            .header(header::CONTENT_LENGTH, size)
            .header(header::CACHE_CONTROL, "no-store")
            .body(BodyExt::boxed(StreamBody::new(stream)))?);
    }
    let _operation = m::coordinator()
        .operation
        .try_lock()
        .context("Another administrator operation is in progress")?;
    anyhow::ensure!(
        !m::paused(),
        "Maintenance is draining; wait for the service to reopen"
    );
    if request.method() == Method::GET && path == "/api/admin/diagnostics" {
        return json_response(
            StatusCode::OK,
            &m::inspect(&state.inner.store, false).await?,
        );
    }
    anyhow::ensure!(
        request.method() == Method::POST,
        "Unknown administrator operation"
    );
    match path.as_str() {
        "/api/admin/discard" => {
            let count = tokio::task::spawn_blocking(move || m::discard_packages(&root)).await??;
            json_response(StatusCode::OK, &json!({"discarded":count}))
        }
        "/api/admin/check" => {
            json_response(StatusCode::OK, &m::inspect(&state.inner.store, true).await?)
        }
        "/api/admin/export" => {
            let report = m::inspect(&state.inner.store, true).await?;
            anyhow::ensure!(
                report["storage"]["problems"]
                    .as_array()
                    .is_some_and(|p| p.is_empty()),
                "Resolve missing or damaged references before exporting"
            );
            let status = json!({"state":"draining","operation":"export","id":crate::storage::id(),"actor":state.user.as_ref().unwrap().id,"created":crate::storage::now(),"report":report});
            m::write_json(&dir.join("status.json"), &status)?;
            m::pause();
            json_response(StatusCode::ACCEPTED, &status)
        }
        "/api/admin/upload" => {
            m::empty(&state.inner.store).await?;
            let id = crate::storage::id();
            let package = dir.join(format!("{id}.upload"));
            let stage = dir.join(format!("{id}.restore"));
            let mut status = json!({"state":"validating","operation":"import","id":id,"actor":state.user.as_ref().unwrap().id,"created":crate::storage::now()});
            m::write_json(&dir.join("status.json"), &status)?;
            let result = async {
                use tokio::io::AsyncWriteExt;
                let mut file = tokio::fs::File::create(&package).await?;
                let mut body = request.into_body();
                let mut size = 0u64;
                while let Some(frame) = body.frame().await {
                    if let Ok(bytes) = frame?.into_data() {
                        size = size
                            .checked_add(bytes.len() as u64)
                            .context("Upload size overflow")?;
                        anyhow::ensure!(
                            size <= m::limit()? + 16 * 1024 * 1024,
                            "Upload exceeds migration limit"
                        );
                        anyhow::ensure!(
                            fs2::available_space(&dir)? > bytes.len() as u64 + 16 * 1024 * 1024,
                            "Insufficient upload disk space"
                        );
                        file.write_all(&bytes).await?;
                    }
                }
                file.sync_all().await?;
                drop(file);
                let package_copy = package.clone();
                let stage_copy = stage.clone();
                let manifest =
                    tokio::task::spawn_blocking(move || m::unpack(&package_copy, &stage_copy))
                        .await??;
                m::validate_schema(&stage).await?;
                let restored = crate::storage::Store::open(&stage, 1).await?;
                let report = m::inspect(&restored, true).await?;
                drop(restored);
                anyhow::ensure!(
                    report["schema"] == crate::storage::SCHEMA_VERSION
                        && report["storage"]["problems"]
                            .as_array()
                            .is_some_and(|p| p.is_empty()),
                    "Package database or references failed validation"
                );
                let package_copy = package.clone();
                let (_, digest) =
                    tokio::task::spawn_blocking(move || m::file_hash(&package_copy)).await??;
                status["state"] = json!("import-ready");
                status["digest"] = json!(digest);
                status["report"] = report;
                status["manifest"] = serde_json::to_value(manifest)?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                status["state"] = json!("failed");
                status["error"] = json!(error.to_string());
            }
            m::write_json(&dir.join("status.json"), &status)?;
            json_response(StatusCode::OK, &status)
        }
        "/api/admin/activate" => {
            let confirmation: Value = shared::body(request).await?;
            let mut status = m::status(&root)?;
            anyhow::ensure!(
                status["state"] == "import-ready"
                    && confirmation["id"] == status["id"]
                    && confirmation["digest"] == status["digest"],
                "Validated package has changed; refresh the preview"
            );
            m::empty(&state.inner.store).await?;
            status["state"] = json!("draining");
            status["operation"] = json!("activate");
            m::write_json(&dir.join("status.json"), &status)?;
            m::pause();
            json_response(StatusCode::ACCEPTED, &status)
        }
        _ => Ok(json_error(
            StatusCode::NOT_FOUND,
            "Unknown administrator operation",
        )),
    }
}
