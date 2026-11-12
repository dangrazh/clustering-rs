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
        role: crate::auth::Role::default(),
        inner: Arc::new(AppState {
            heavy_requests: Arc::new(tokio::sync::Semaphore::new(2)),
            sources: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
            config: AppConfig::default(),
            artifacts: Artifacts::new(store.root.clone(), 128 * 1024 * 1024),
            store,
            auth,
            presence: Mutex::new(HashMap::new()),
            static_data: Default::default(),
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
async fn role_authorization_and_live_demotion_cover_admin_and_read_only_requests() -> Result<()> {
    let (dir,state,base,token,_) = fixture_mode(true).await?;
    let client = reqwest::Client::new();
    let cookie = format!("app_session={token}");
    let session = state.inner.auth.session(&cookie).await?;
    let write_role = |role: &str| std::fs::write(dir.path().join("allowed-users.json"), serde_json::to_vec(&json!([{"email":"alice@example.invalid","name":"Alice","role":role}])).unwrap());
    assert_eq!(client.get(format!("{base}/api/admin/status")).header(header::COOKIE,&cookie).send().await?.status(),403);
    write_role("SysAdmin")?;
    assert_eq!(client.get(format!("{base}/api/admin/diagnostics")).header(header::COOKIE,&cookie).send().await?.status(),200);
    assert_eq!(client.post(format!("{base}/api/admin/export")).header(header::COOKIE,&cookie).send().await?.status(),403);
    write_role("Read Only User")?;
    for path in ["/api/analyze","/api/import","/api/sessions","/api/series","/api/jobs/x/save-central","/api/analyses/x/commands","/api/admin/check"] {
        assert_eq!(post(&client,&format!("{base}{path}"),&token,&session.csrf,json!({})).await?.status(),403,"{path}");
    }
    assert_eq!(post(&client,&format!("{base}/api/dashboard/preferences"),&token,&session.csrf,json!({"mine":{"search":"personal"}})).await?.status(),200);
    assert_eq!(client.get(format!("{base}/api/analyses")).header(header::COOKIE,&cookie).send().await?.status(),200);
    Ok(())
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
            label_style: Default::default(),
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
        // Day/month/year with a time, as exported; the weekly view groups by it.
        run.source.headers.push("Opened".into());
        for (r, row) in run.source.rows.iter_mut().enumerate() {
            row.push(format!("{:02}.0{}.2026 10:15", 1 + r % 14, 1 + i));
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
    let weekly = |column: &str| {
        client
            .get(format!("{base}/api/series/{sid}/weekly?column={column}"))
            .header("Cookie", format!("app_session={bob}"))
            .send()
    };
    let automatic: Value = weekly("").await?.error_for_status()?.json().await?;
    assert_eq!(automatic["column"], "Opened");
    assert_eq!(automatic["automatic"], true);
    assert!(automatic["columns"]
        .as_array()
        .unwrap()
        .contains(&json!({"name":"Opened","date":true})));
    let totals: u64 = automatic["weekly"]["totals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .sum();
    assert_eq!(totals, 21 + 22, "Every processed record of both periods is dated");
    assert_eq!(automatic["weekly"]["undated"], 0);
    // The earliest date, Sunday 1 February, belongs to the week of Monday 26 January.
    assert_eq!(automatic["weekly"]["weeks"][0], "2026-01-26");
    let chosen: Value = weekly("Status").await?.error_for_status()?.json().await?;
    assert_eq!(chosen["automatic"], false);
    assert_eq!(chosen["weekly"]["undated"], 21 + 22);
    assert!(chosen["weekly"]["weeks"].as_array().unwrap().is_empty());
    assert!(!weekly("Missing").await?.status().is_success());
    Ok(())
}

#[tokio::test]
async fn regenerating_titles_keeps_renames_and_respects_owners_and_series() -> Result<()> {
    let (dir, state, base, alice, bob) = fixture_mode(true).await?;
    std::fs::write(
        dir.path().join("allowed-users.json"),
        serde_json::to_vec(&json!([
            {"email":"alice@example.invalid","name":"Alice"},
            {"email":"bob@example.invalid","name":"Bob"}
        ]))?,
    )?;
    let client = reqwest::Client::new();
    let a = state.inner.auth.session(&format!("app_session={alice}")).await?;
    let b = state.inner.auth.session(&format!("app_session={bob}")).await?;
    let saved = |name: &'static str| {
        let state = state.clone();
        let user = a.user.clone();
        async move {
            let run = Arc::new(fixtures::run(20));
            let review = ReviewData::new(&run)?;
            let metadata = crate::storage::PortableMeta {
                summaries: crate::storage::Store::entity_summaries(&run),
                ..Default::default()
            };
            let artifact = state.inner.artifacts.publish(run, review.clone(), ViewState::default(), metadata.clone()).await?;
            let aid = state.inner.store.create(&user, &id(), name, &artifact, crate::storage::InitialReview { review: &review, view: &ViewState::default(), metadata: &metadata }).await?;
            anyhow::Ok((aid, artifact))
        }
    };
    let (aid, old_artifact) = saved("Retitled").await?;
    let url = |aid: &str, op: &str| format!("{base}/api/analyses/{aid}/{op}");
    let regenerate = |token: &str, csrf: &str, aid: &str, expected: u64| {
        let (client, target, token, csrf) = (client.clone(), url(aid, "regenerate-titles"), token.to_owned(), csrf.to_owned());
        async move { post(&client, &target, &token, &csrf, json!({"commandId":id(),"expected":expected})).await }
    };
    post(&client, &url(&aid, "commands"), &alice, &a.csrf, json!({"commandId":id(),"target":"1:2","expected":0,"action":{"type":"rename","label":"Kept by Alice"}}))
        .await?
        .error_for_status()?;

    // Titles are analysis-wide: only the owner or a SysAdmin may regenerate them.
    assert_eq!(regenerate(&bob, &b.csrf, &aid, 0).await?.status(), 403);
    let done: Value = regenerate(&alice, &a.csrf, &aid, 0).await?.error_for_status()?.json().await?;
    assert!(done["changed"].as_u64().unwrap() > 0, "{done}");
    assert_eq!(done["version"], 1);

    let result: Value = client.get(url(&aid, "result")).header("Cookie", format!("app_session={alice}")).send().await?.error_for_status()?.json().await?;
    let cluster = &result["clusters"][0];
    assert_ne!(cluster["label"], "Connectivity group 1");
    assert_ne!(cluster["subgroups"][0]["label"], "Network connection");
    // The review still matches the analysis (new fingerprint) and the rename still wins.
    let review: Value = client.get(url(&aid, "review")).header("Cookie", format!("app_session={alice}")).send().await?.error_for_status()?.json().await?;
    assert_eq!(review["review"]["annotations"]["labels"]["1:2"], "Kept by Alice");
    let dashboard: Value = client.get(format!("{base}/api/dashboard/data?offset=0")).header("Cookie", format!("app_session={alice}")).send().await?.json().await?;
    assert_eq!(dashboard["analyses"][0]["version"], 1);
    // The replaced artifact is no longer referenced, so its file is gone.
    assert!(state.inner.artifacts.load(&old_artifact).await.is_err());
    let again: Value = regenerate(&alice, &a.csrf, &aid, 1).await?.error_for_status()?.json().await?;
    assert_eq!(again["changed"], 0);
    // A stale version is refused.
    assert_eq!(regenerate(&alice, &a.csrf, &aid, 0).await?.status(), 409);

    // Series identify periods by fingerprint, so member analyses keep their titles.
    let (member, _) = saved("Series member").await?;
    let c = state.inner.store.connect().await?;
    c.execute("INSERT INTO trend_series VALUES('s1','Monthly','monthly',0,0,'{}')", ()).await?;
    c.execute("INSERT INTO trend_membership VALUES(?,'s1','2026-01')", [member.as_str()]).await?;
    let refused = regenerate(&alice, &a.csrf, &member, 0).await?;
    assert_eq!(refused.status(), 409);
    assert!(refused.text().await?.contains("Monthly"));
    Ok(())
}

#[tokio::test]
async fn static_data_join_deletion_and_catalog_views() -> Result<()> {
    let (dir, state, base, alice, bob) = fixture_mode(true).await?;
    std::fs::write(
        dir.path().join("allowed-users.json"),
        serde_json::to_vec(&json!([
            {"email":"alice@example.invalid","name":"Alice","role":"SysAdmin"},
            {"email":"bob@example.invalid","name":"Bob"}
        ]))?,
    )?;
    let client = reqwest::Client::new();
    let a = state.inner.auth.session(&format!("app_session={alice}")).await?;
    let b = state.inner.auth.session(&format!("app_session={bob}")).await?;
    let get = |token: &str, path: &str| {
        client
            .get(format!("{base}{path}"))
            .header("Cookie", format!("app_session={token}"))
            .send()
    };

    // A saved analysis whose source has a "Group" column for joining.
    let mut run = fixtures::run(20);
    run.source.headers.push("Group".into());
    for (i, row) in run.source.rows.iter_mut().enumerate() {
        row.push(["Service Desk", " network", "Unknown"][i % 3].into());
    }
    let run = Arc::new(run);
    let review = ReviewData::new(&run)?;
    // As in a central save: dashboard summaries come with the metadata.
    let metadata = crate::storage::PortableMeta {
        summaries: crate::storage::Store::entity_summaries(&run),
        ..Default::default()
    };
    let artifact = state
        .inner
        .artifacts
        .publish(run, review.clone(), ViewState::default(), metadata.clone())
        .await?;
    let aid = state
        .inner
        .store
        .create(
            &a.user,
            &id(),
            "Static join",
            &artifact,
            crate::storage::InitialReview { review: &review, view: &ViewState::default(), metadata: &metadata },
        )
        .await?;

    // Upload, create and configure a static table; it is SysAdmin-only.
    assert_eq!(get(&bob, "/api/static-data").await?.status(), 403);
    let csv = "Group,Manager,Since\nService Desk,Ana,01.02.2026\nNetwork,Ben,2026-03-04\n";
    let upload: Value = client
        .post(format!("{base}/api/static-data/upload?filename=groups.csv"))
        .header("Cookie", format!("app_session={alice}"))
        .header("X-CSRF-Token", &a.csrf)
        .body(csv)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(upload["rowCount"], 2);
    assert_eq!(upload["columns"][2], json!({"name":"Since","field":"since","type":"Date"}));
    let created: Value = post(&client, &format!("{base}/api/static-data/create"), &alice, &a.csrf, json!({"uploadId":upload["uploadId"],"name":"Support groups"}))
        .await?
        .error_for_status()?
        .json()
        .await?;
    let table = created["id"].as_str().unwrap();
    let config = json!({"keys":[{"field":"group","analysisColumn":"group"}],"fields":["manager","since"]});
    post(&client, &format!("{base}/api/static-data/{table}/config"), &alice, &a.csrf, json!({"expected":0,"config":config}))
        .await?
        .error_for_status()?;
    let rows: Value = get(&alice, &format!("/api/static-data/{table}/rows")).await?.json().await?;
    assert_eq!(rows["rows"][0], json!(["Service Desk", "Ana", "2026-02-01"]));

    // The join is left outer, case- and whitespace-insensitive, and visible to every user.
    let result: Value = get(&bob, &format!("/api/analyses/{aid}/result")).await?.error_for_status()?.json().await?;
    assert_eq!(result["static_columns"], 2);
    let headers = result["source"]["headers"].as_array().unwrap();
    assert_eq!(headers[headers.len() - 2..], [json!("Manager"), json!("Since")]);
    let joined: Vec<_> = result["source"]["rows"].as_array().unwrap()[..3]
        .iter()
        .map(|r| r.as_array().unwrap()[4..].to_vec())
        .collect();
    assert_eq!(joined, [vec![json!("Ana"), json!("2026-02-01")], vec![json!("Ben"), json!("2026-03-04")], vec![json!(""), json!("")]]);
    assert_eq!(result["column_types"].as_array().unwrap().len(), headers.len());

    // All assigned work lists every assignee; operators see every user's jobs.
    post(&client, &format!("{base}/api/analyses/{aid}/commands"), &alice, &a.csrf, json!({"commandId":id(),"target":"1","expected":0,"action":{"type":"reviewer","userId":b.user.id}}))
        .await?
        .error_for_status()?;
    let assigned: Value = get(&bob, "/api/dashboard/assigned-all").await?.error_for_status()?.json().await?;
    let entry = assigned["assigned"].as_array().unwrap().iter().find(|i| i["entity"] == "1").unwrap();
    assert_eq!(entry["assignee"]["name"], "Bob");
    assert_eq!(get(&bob, "/api/jobs?scope=all").await?.status(), 403);
    assert_eq!(get(&alice, "/api/jobs?scope=all").await?.status(), 200);

    // Only the owner or a SysAdmin deletes; deletion removes the analysis and its artifact.
    let url = format!("{base}/api/analyses/{aid}/delete");
    let denied = post(&client, &url, &bob, &b.csrf, json!({"commandId":id(),"expected":0})).await?;
    assert_eq!(denied.status(), 403);
    post(&client, &url, &alice, &a.csrf, json!({"commandId":id(),"expected":0}))
        .await?
        .error_for_status()?;
    assert_eq!(get(&alice, &format!("/api/analyses/{aid}/result")).await?.status(), 404);
    assert!(!dir.path().join("artifacts").join(&artifact).exists());
    let assigned: Value = get(&bob, "/api/dashboard/assigned-all").await?.json().await?;
    assert!(assigned["assigned"].as_array().unwrap().is_empty());

    // Replacing rows keeps the join settings whose columns still exist ("Since" is gone).
    let replacement: Value = client
        .post(format!("{base}/api/static-data/upload?filename=groups-v2.csv"))
        .header("Cookie", format!("app_session={alice}"))
        .header("X-CSRF-Token", &a.csrf)
        .body("Group,Manager\nService Desk,Zoe\n")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    post(&client, &format!("{base}/api/static-data/create"), &alice, &a.csrf, json!({"uploadId":replacement["uploadId"],"name":"Support groups","replace":table,"expected":1}))
        .await?
        .error_for_status()?;
    let tables: Value = get(&alice, "/api/static-data").await?.json().await?;
    assert_eq!(tables[0]["rowCount"], 1);
    assert_eq!(tables[0]["config"]["fields"], json!(["manager"]));
    assert_eq!(tables[0]["config"]["keys"][0]["field"], "group");
    post(&client, &format!("{base}/api/static-data/{table}/delete"), &alice, &a.csrf, json!({}))
        .await?
        .error_for_status()?;
    let tables: Value = get(&alice, "/api/static-data").await?.json().await?;
    assert!(tables.as_array().unwrap().is_empty());
    Ok(())
}
