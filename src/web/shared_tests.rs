use super::*;
use crate::fixtures;
use crate::{
    artifacts::Artifacts,
    auth::Auth,
    session::{self, ReviewData, ViewState},
    storage::{id, Store},
};
use serde_json::{json, Value};

async fn fixture() -> Result<(tempfile::TempDir, WebState, String, String, String)> {
    fixture_mode(false).await
}
async fn fixture_mode(
    allowlist: bool,
) -> Result<(tempfile::TempDir, WebState, String, String, String)> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path(), 4).await?;
    let auth = if allowlist {
        let path = dir.path().join("allowed-users.json");
        std::fs::write(
            &path,
            r#"[{"email":"alice@example.invalid","name":"Alice"},{"email":"bob@example.invalid","name":"Bob"}]"#,
        )?;
        Auth::with_allowlist(
            store.clone(),
            crate::auth::AllowlistConfig {
                path,
                session_seconds: 3600,
                secure_cookie: false,
            },
        )
        .await?
    } else {
        Auth::new(store.clone(), None)?
    };
    let alice = store
        .user(
            if allowlist {
                "allowlist:alice@example.invalid"
            } else {
                "fixture:alice"
            },
            "Alice",
            "alice@example.invalid",
        )
        .await?;
    let bob = store
        .user(
            if allowlist {
                "allowlist:bob@example.invalid"
            } else {
                "fixture:bob"
            },
            "Bob",
            "bob@example.invalid",
        )
        .await?;
    let a = auth.issue(&alice, 3600).await?;
    let b = auth.issue(&bob, 3600).await?;
    let state = WebState {
        request_job: None,
        user: None,
        inner: Arc::new(AppState {
            heavy_requests: Arc::new(tokio::sync::Semaphore::new(2)),
            sources: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
            config: AppConfig::default(),
            artifacts: Artifacts::new(store.root.clone(), 128 * 1024 * 1024),
            store,
            auth,
            presence: Mutex::new(HashMap::new()),
        }),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = state.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let server = server.clone();
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(stream),
                        service_fn(move |request| route(request, server.clone())),
                    )
                    .await;
            });
        }
    });
    shared::start_publisher(state.inner.store.clone());
    Ok((dir, state, base, a, b))
}
async fn post(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    csrf: &str,
    value: Value,
) -> Result<reqwest::Response> {
    Ok(client
        .post(url)
        .header("Cookie", format!("app_session={token}"))
        .header("X-CSRF-Token", csrf)
        .json(&value)
        .send()
        .await?)
}

#[tokio::test]
async fn email_sign_in_uses_allowlist_name_and_protects_session_routes() -> Result<()> {
    let (_dir, state, base, _, _) = fixture_mode(true).await?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let form = client.get(format!("{base}/login")).send().await?;
    assert_eq!(form.status(), 200);
    assert!(form.text().await?.contains("name=\"email\""));
    let login_url = format!("{base}/auth/login");
    assert_eq!(
        client
            .post(&login_url)
            .json(&json!({"email":"alice@example.invalid"}))
            .send()
            .await?
            .status(),
        403
    );
    for (payload, expected) in [
        (json!({"email":"unknown@example.invalid"}), 403),
        (
            json!({"email":"alice@example.invalid","name":"Impersonated Name"}),
            400,
        ),
    ] {
        assert_eq!(
            client
                .post(&login_url)
                .header("X-Requested-With", "same-origin")
                .json(&payload)
                .send()
                .await?
                .status(),
            expected
        );
    }
    let signed_in = client
        .post(&login_url)
        .header("X-Requested-With", "same-origin")
        .json(&json!({"email":" ALICE@EXAMPLE.INVALID "}))
        .send()
        .await?;
    assert_eq!(signed_in.status(), 200);
    let cookies = signed_in.headers()[header::SET_COOKIE].to_str()?.to_owned();
    assert!(!cookies.contains("; Secure"));
    let me: Value = client
        .get(format!("{base}/api/me"))
        .header(header::COOKIE, &cookies)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(me["user"]["name"], "Alice");
    assert_eq!(
        client
            .post(format!("{base}/auth/logout"))
            .header(header::COOKIE, &cookies)
            .send()
            .await?
            .status(),
        403
    );
    let logged_out = client
        .post(format!("{base}/auth/logout"))
        .header(header::COOKIE, &cookies)
        .header("X-CSRF-Token", me["csrf"].as_str().unwrap())
        .send()
        .await?;
    assert_eq!(logged_out.status(), 303);
    assert!(logged_out.headers()[header::SET_COOKIE]
        .to_str()?
        .contains("Max-Age=0"));
    assert!(state.inner.auth.session(&cookies).await.is_err());
    Ok(())
}
#[tokio::test]
async fn authenticated_import_central_save_two_user_review_and_export() -> Result<()> {
    let (_dir, state, base, alice, bob) = fixture().await?;
    let client = reqwest::Client::new();
    let ac = format!("app_session={alice}");
    let bc = format!("app_session={bob}");
    let a = state.inner.auth.session(&ac).await?;
    let b = state.inner.auth.session(&bc).await?;
    assert_eq!(
        client
            .get(format!("{base}/api/analyses"))
            .send()
            .await?
            .status(),
        401
    );
    assert_eq!(
        client
            .post(format!("{base}/api/sessions"))
            .header("Cookie", &ac)
            .send()
            .await?
            .status(),
        403
    );
    let run = fixtures::run(20);
    let review = ReviewData::new(&run)?;
    let bytes = session::encode_session(&run, &review, &ViewState::default())?;
    let imported: Value = client
        .post(format!("{base}/api/sessions"))
        .header("Cookie", &ac)
        .header("X-CSRF-Token", &a.csrf)
        .body(bytes)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let jid = imported["jobId"].as_str().unwrap();
    assert_eq!(
        client
            .get(format!("{base}/api/jobs/{jid}/result"))
            .header("Cookie", &bc)
            .send()
            .await?
            .status(),
        403
    );
    let saved: Value = post(
        &client,
        &format!("{base}/api/jobs/{jid}/save-central"),
        &alice,
        &a.csrf,
        json!({"commandId":id(),"name":"Team review","view":ViewState::default()}),
    )
    .await?
    .error_for_status()?
    .json()
    .await?;
    let aid = saved["id"].as_str().unwrap();
    assert_ne!(aid, review.analysis_id);
    // Emulate a retained job from before summaries were recorded. Preserve its central link.
    let c = state.inner.store.connect().await?;
    c.execute(
        "UPDATE jobs SET metadata=? WHERE id=?",
        turso::params![json!({"imported":true,"analysisId":aid}).to_string(), jid],
    )
    .await?;
    let jobs: Vec<Value> = client
        .get(format!("{base}/api/jobs"))
        .header("Cookie", &ac)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(jobs[0]["metadata"]["summary"]["sourceRows"], 20);
    assert_eq!(jobs[0]["metadata"]["summary"]["processedRows"], 20);
    assert_eq!(jobs[0]["metadata"]["summary"]["columnCount"], 3);
    assert_eq!(jobs[0]["metadata"]["analysisId"], aid);
    let catalog: Vec<Value> = client
        .get(format!("{base}/api/analyses"))
        .header("Cookie", &bc)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(catalog.len(), 1);
    let url = format!("{base}/api/analyses/{aid}/commands");
    let command = json!({"commandId":id(),"target":"1","expected":0,"action":{"type":"rename","label":"Shared label"},"actor":{"name":"Impersonator","email":"fake@example.invalid"}});
    let edited: Value = post(&client, &url, &bob, &b.csrf, command)
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(edited["snapshot"]["audit"][0]["actor"]["name"], "Bob");
    let stale=post(&client,&url,&alice,&a.csrf,json!({"commandId":id(),"target":"1","expected":0,"action":{"type":"rename","label":"Stale"}})).await?;
    assert_eq!(stale.status(), 409);
    let stale: Value = stale.json().await?;
    assert_eq!(
        stale["snapshot"]["review"]["annotations"]["labels"]["1"],
        "Shared label"
    );
    let export = post(
        &client,
        &format!("{base}/api/analyses/{aid}/session/save"),
        &bob,
        &b.csrf,
        json!(ViewState::default()),
    )
    .await?
    .error_for_status()?
    .bytes()
    .await?;
    assert_eq!(export[8], 3);
    let decoded = session::decode_session(&export)?;
    assert_eq!(decoded.metadata.audit[0]["actor"]["name"], "Bob");
    let pivot = post(
        &client,
        &format!("{base}/api/analyses/{aid}/pivot"),
        &bob,
        &b.csrf,
        json!({"rowIndices":[0,1,2],"rowColumns":[2],"columnColumns":[]}),
    )
    .await?;
    assert_eq!(pivot.status(), 200);
    Ok(())
}

#[tokio::test]
#[ignore = "Requires npm dependencies and Playwright Chromium; run explicitly"]
async fn browser_collaboration_acceptance() -> Result<()> {
    run_browser_fixture(false).await
}

#[tokio::test]
#[ignore = "Large Chromium readiness measurement; requires Playwright"]
async fn large_browser_reopen_acceptance() -> Result<()> {
    run_browser_fixture(true).await
}

async fn run_browser_fixture(large: bool) -> Result<()> {
    let (_dir, state, base, alice, bob) = fixture_mode(true).await?;
    let user = state
        .inner
        .auth
        .session(&format!("app_session={alice}"))
        .await?
        .user;
    let mut run = fixtures::run(if large { 150_000 } else { 1200 });
    run.source.source_path = Some("browser-incidents.xlsx".into());
    if large {
        for column in 3..30 {
            run.source.headers.push(format!("Field {column}"));
        }
        for (index, row) in run.source.rows.iter_mut().enumerate() {
            for column in 3..30 {
                row.push(format!(
                    "{index:06}-{column:02}-{:016x}-{:016x}",
                    index.wrapping_mul(2654435761),
                    column * 7919
                ));
            }
        }
    }
    let run = Arc::new(run);
    let review = ReviewData::new(&run)?;
    let artifact = state
        .inner
        .artifacts
        .publish(
            run,
            review.clone(),
            ViewState::default(),
            Default::default(),
        )
        .await?;
    state
        .inner
        .store
        .create(
            &user,
            &id(),
            "Browser acceptance",
            &artifact,
            crate::storage::InitialReview {
                review: &review,
                view: &ViewState::default(),
                metadata: &Default::default(),
            },
        )
        .await?;
    let output = tokio::process::Command::new("node")
        .arg(if large {
            "web_tests/browser-load.js"
        } else {
            "web_tests/browser-acceptance.js"
        })
        .env("FIXTURE_URL", base)
        .env("FIXTURE_ALICE", alice)
        .env("FIXTURE_BOB", bob)
        .env("FIXTURE_EMAIL_LOGIN", "1")
        .output()
        .await?;
    anyhow::ensure!(
        output.status.success(),
        "Browser acceptance failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[tokio::test]
#[ignore = "Build the application binary first; exercises actual child-process execution"]
async fn supervised_child_publishes_reopenable_result() -> Result<()> {
    let (_dir, mut state, _base, alice, _bob) = fixture().await?;
    let user = state
        .inner
        .auth
        .session(&format!("app_session={alice}"))
        .await?
        .user;
    let run = fixtures::run(20);
    let jid = crate::jobs::enqueue(
        &state.inner.store,
        &user,
        crate::jobs::Input {
            source: run.source,
            mapping: run.mapping,
            settings: run.settings,
        },
    )
    .await?;
    crate::jobs::change(&state.inner.store, &jid, "queued", "running", None, None).await?;
    let record = crate::jobs::get(&state.inner.store, &jid).await?;
    let executable = std::env::current_exe()?
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!(
            "incident-clustering-analyzer{}",
            std::env::consts::EXE_SUFFIX
        ));
    anyhow::ensure!(
        executable.is_file(),
        "Build the application binary before this test"
    );
    jobs_runtime::run_child(&state, &record, executable).await?;
    assert_eq!(
        crate::jobs::get(&state.inner.store, &jid).await?.state,
        "finished"
    );
    state.user = Some(user);
    jobs_runtime::prepare(&state, &jid).await?;
    let (run, review, _) = review_api::snapshot(&review_api::find_job(&state, &jid)?)?;
    assert_eq!(run.source.rows.len(), 20);
    review.validate(&run)?;
    Ok(())
}
