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
    assert_eq!(edited["snapshot"]["analysis"]["owner"]["id"], a.user.id);
    assert_eq!(edited["snapshot"]["analysis"]["owner"]["name"], "Alice");
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
    assert_eq!(export[8], 6);
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
    run_browser_fixture(false, false).await
}

#[tokio::test]
#[ignore = "Large Chromium readiness measurement; requires Playwright"]
async fn large_browser_reopen_acceptance() -> Result<()> {
    run_browser_fixture(true, false).await
}

async fn run_browser_fixture(large: bool, series: bool) -> Result<()> {
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
    state
        .inner
        .store
        .backfill_dashboard(&state.inner.artifacts)
        .await?;
    let trend_worker = tokio::spawn(trends::supervise(state.clone()));
    let output = tokio::process::Command::new("node")
        .arg(if series {
            "web_tests/browser-series.js"
        } else if large {
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
    trend_worker.abort();
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
            column_types: vec![],
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

#[tokio::test]
#[ignore = "Requires Playwright Chromium; exercises the full series UI"]
async fn browser_series_acceptance() -> Result<()> {
    run_browser_fixture(false, true).await
}

#[tokio::test]
async fn series_http_historical_insertion_replacement_and_access() -> Result<()> {
    let (_dir, state, base, alice, bob) = fixture().await?;
    let client = reqwest::Client::new();
    let cookie = format!("app_session={alice}");
    let session = state.inner.auth.session(&cookie).await?;
    assert_eq!(
        client
            .get(format!("{base}/api/series"))
            .send()
            .await?
            .status(),
        401
    );
    assert_eq!(
        client
            .post(format!("{base}/api/series"))
            .header("Cookie", &cookie)
            .send()
            .await?
            .status(),
        403
    );
    let mut sources = vec![];
    for i in 0..3 {
        let mut run = fixtures::run(20 + i);
        for r in &mut run.processed_incidents {
            r.analysis_text = "payment card checkout rejected bank gateway".into();
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
        sources.push(
            state
                .inner
                .store
                .create(
                    &session.user,
                    &id(),
                    &format!("Period source {i}"),
                    &artifact,
                    crate::storage::InitialReview {
                        review: &review,
                        view: &ViewState::default(),
                        metadata: &Default::default(),
                    },
                )
                .await?,
        );
    }
    state
        .inner
        .store
        .command(
            sources[0].clone(),
            session.user.clone(),
            crate::storage::Command {
                command_id: id(),
                target: "1".into(),
                expected: 0,
                parent_version: None,
                action: json!({"type":"rename","label":"Current source label"}),
            },
        )
        .await?;
    state
        .inner
        .store
        .command(
            sources[0].clone(),
            session.user.clone(),
            crate::storage::Command {
                command_id: id(),
                target: "".into(),
                expected: 0,
                parent_version: None,
                action: json!({"type":"archive"}),
            },
        )
        .await?;
    let created: Value = post(
        &client,
        &format!("{base}/api/series"),
        &alice,
        &session.csrf,
        json!({"commandId":id(),"name":"HTTP series"}),
    )
    .await?
    .error_for_status()?
    .json()
    .await?;
    let sid = created["id"].as_str().unwrap();
    for (index, month) in [(0, 2), (1, 1)] {
        let value:Value=post(&client,&format!("{base}/api/series/{sid}/commands"),&alice,&session.csrf,json!({"commandId":id(),"expected":index,"action":{"type":"addPeriod","analysis":sources[index],"start":format!("2026-0{month}-01"),"end":format!("2026-0{month}-28"),"replace":null}})).await?.error_for_status()?.json().await?;
        let mut task = state
            .inner
            .store
            .trend_task(value["task"].as_str().unwrap())
            .await?;
        trends::compute(&state, &mut task).await?;
        state.inner.store.save_trend_task(&task).await?;
        assert_eq!(
            state.inner.store.series(sid).await?.revision,
            index as u64,
            "Preview cannot publish effective counts"
        );
        post(
            &client,
            &format!("{base}/api/series/{sid}/previews/{}/apply", task.id),
            &alice,
            &session.csrf,
            json!({"commandId":id()}),
        )
        .await?
        .error_for_status()?;
    }
    let snapshot = state.inner.store.series(sid).await?;
    assert_eq!(snapshot.periods[0].analysis, sources[1]);
    assert_eq!(
        snapshot.periods[1].occurrences[0].label,
        "Current source label"
    );
    let period = snapshot.periods[1].id.clone();
    let value = state
        .inner
        .store
        .series_command(
            &session.user,
            sid,
            &crate::trends::Command {
                command_id: id(),
                expected: 2,
                action: crate::trends::Action::AddPeriod {
                    analysis: sources[2].clone(),
                    start: "2026-02-01".into(),
                    end: "2026-02-28".into(),
                    replace: Some(period),
                },
            },
        )
        .await?;
    let mut task = state
        .inner
        .store
        .trend_task(value["task"].as_str().unwrap())
        .await?;
    trends::compute(&state, &mut task).await?;
    assert!(task.proposed.as_ref().unwrap().periods[1].occurrences[0].replacement);
    assert!(task.proposed.as_ref().unwrap().periods[1].occurrences[0]
        .problem
        .is_none());
    state.inner.store.save_trend_task(&task).await?;
    state
        .inner
        .store
        .apply_trend_preview(&session.user, sid, &task.id, &id())
        .await?;
    let bob_view: Value = client
        .get(format!("{base}/api/series/{sid}"))
        .header("Cookie", format!("app_session={bob}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(bob_view["periods"][1]["analysis"], sources[2]);
    assert!(bob_view["model"]["weights"].is_null());
    assert!(bob_view["periods"][1]["occurrences"][0]["rows"].is_null());
    let before_zero = state.inner.store.series(sid).await?;
    let task_id = state
        .inner
        .store
        .series_command(
            &session.user,
            sid,
            &crate::trends::Command {
                command_id: id(),
                expected: before_zero.revision,
                action: crate::trends::Action::Zero {
                    period: before_zero.periods[1].id.clone(),
                    problem: before_zero.problems[0].id.clone(),
                    confirmed: true,
                },
            },
        )
        .await?;
    let mut zero_task = state
        .inner
        .store
        .trend_task(task_id["task"].as_str().unwrap())
        .await?;
    trends::compute(&state, &mut zero_task).await?;
    state.inner.store.save_trend_task(&zero_task).await?;
    let bob_session = state
        .inner
        .auth
        .session(&format!("app_session={bob}"))
        .await?;
    state
        .inner
        .store
        .apply_trend_preview(&bob_session.user, sid, &zero_task.id, &id())
        .await?;
    assert_eq!(
        state.inner.store.series(sid).await?.zeros[0].actor.id,
        bob_session.user.id,
        "Zero attribution follows the confirming reviewer"
    );
    assert!(
        state.inner.store.analysis(&sources[0]).await?.archived,
        "Archived source remains unchanged"
    );
    Ok(())
}
