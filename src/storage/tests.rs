use super::*;

async fn fixture() -> Result<(tempfile::TempDir, Arc<Store>, User, User, String)> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path(), 4).await?;
    let alice = store
        .user("fixture:alice", "Alice", "alice@example.invalid")
        .await?;
    let bob = store
        .user("fixture:bob", "Bob", "bob@example.invalid")
        .await?;
    let mut annotations = Annotations::default();
    annotations.entries.insert(
        "1".into(),
        Annotation {
            workflow: Some(Workflow::default()),
            ..Default::default()
        },
    );
    annotations
        .entries
        .insert("1:1".into(), Annotation::default());
    let review = ReviewData {
        analysis_id: id(),
        fingerprint: "fixture-fingerprint".into(),
        annotations,
    };
    let aid = store
        .create(
            &alice,
            &id(),
            "Shared fixture",
            "fixture.icas",
            InitialReview {
                review: &review,
                view: &ViewState::default(),
                metadata: &PortableMeta::default(),
            },
        )
        .await?;
    Ok((dir, store, alice, bob, aid))
}
fn command(target: &str, action: Value, expected: u64) -> Command {
    Command {
        command_id: id(),
        target: target.into(),
        action,
        expected,
        parent_version: None,
    }
}

#[tokio::test]
async fn archived_names_remain_reserved_and_imports_preserve_read_only_comments() -> Result<()> {
    let (_dir, s, alice, bob, aid) = fixture().await?;
    s.command(
        aid.clone(),
        alice.clone(),
        command("", json!({"type":"archive"}), 0),
    )
    .await?;
    let snapshot = s.review(&aid, &alice).await?;
    let review: ReviewData = serde_json::from_value(snapshot["review"].clone())?;
    assert!(matches!(
        s.create(
            &bob,
            &id(),
            " SHARED FIXTURE ",
            "fixture.icas",
            InitialReview {
                review: &review,
                view: &ViewState::default(),
                metadata: &Default::default()
            }
        )
        .await
        .unwrap_err()
        .downcast_ref::<AppError>(),
        Some(AppError::Conflict(_))
    ));
    let mut imported = review.clone();
    imported.annotations.mutate(Mutation {
        revision: 0,
        target: "1".into(),
        actor: alice.actor(),
        action: Action::AddComment {
            text: "Recorded externally".into(),
        },
    })?;
    let cid = imported.annotations.entries["1"].comments[0].id.clone();
    let mut meta = PortableMeta::default();
    meta.read_only_comments.insert(cid.clone());
    let imported_id = s
        .create(
            &bob,
            &id(),
            "Imported fixture",
            "fixture.icas",
            InitialReview {
                review: &imported,
                view: &ViewState::default(),
                metadata: &meta,
            },
        )
        .await?;
    assert_ne!(aid, imported_id);
    assert!(
        !s.review(&imported_id, &bob).await?["commentAccess"][&cid]["canEdit"]
            .as_bool()
            .unwrap()
    );
    let edit = command(
        "1",
        json!({"type":"editComment","id":cid,"text":"Replace attribution"}),
        0,
    );
    assert!(matches!(
        s.command(imported_id.clone(), bob.clone(), edit)
            .await
            .unwrap_err()
            .downcast_ref::<AppError>(),
        Some(AppError::Forbidden)
    ));
    s.command(
        imported_id,
        bob,
        command(
            "1",
            json!({"type":"addComment","text":"New owned comment"}),
            0,
        ),
    )
    .await?;
    Ok(())
}
#[tokio::test]
async fn independent_edits_retries_ownership_and_stale_values() -> Result<()> {
    let (_dir, s, alice, bob, aid) = fixture().await?;
    let label = command("1", json!({"type":"rename","label":"Reviewed"}), 0);
    let comment = command("1", json!({"type":"addComment","text":"Check this"}), 0);
    let (l, c) = tokio::join!(
        s.command(aid.clone(), alice.clone(), label.clone()),
        s.command(aid.clone(), bob.clone(), comment.clone())
    );
    l?;
    let added = c?;
    assert_eq!(s.command(aid.clone(), bob.clone(), comment).await?, added);
    let view = s.review(&aid, &alice).await?;
    assert_eq!(view["review"]["annotations"]["labels"]["1"], "Reviewed");
    assert_eq!(
        view["review"]["annotations"]["entries"]["1"]["comments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let stale = command("1", json!({"type":"rename","label":"Stale"}), 0);
    assert!(matches!(
        s.command(aid.clone(), bob.clone(), stale)
            .await
            .unwrap_err()
            .downcast_ref::<AppError>(),
        Some(AppError::Conflict(_))
    ));
    let cid = added["commentId"].as_str().unwrap();
    let edit = command(
        "1",
        json!({"type":"editComment","id":cid,"text":"Changed"}),
        0,
    );
    assert!(matches!(
        s.command(aid.clone(), alice.clone(), edit.clone())
            .await
            .unwrap_err()
            .downcast_ref::<AppError>(),
        Some(AppError::Forbidden)
    ));
    s.command(aid.clone(), bob.clone(), edit).await?;
    let mut changed = label;
    changed.action["label"] = json!("Reused id");
    assert!(s
        .command(aid.clone(), alice.clone(), changed)
        .await
        .is_err());
    s.publish().await?;
    let events = s.events(0).await?;
    assert_eq!(events.len(), 4);
    s.publish().await?;
    assert!(s
        .events(events.last().unwrap()["sequence"].as_i64().unwrap())
        .await?
        .is_empty());
    Ok(())
}
#[tokio::test]
async fn archive_and_parent_versions_protect_shared_work() -> Result<()> {
    let (_dir, s, alice, bob, aid) = fixture().await?;
    s.command(
        aid.clone(),
        alice.clone(),
        command(
            "1",
            json!({"type":"assign","assignments":{"smeEmail":"sme@example.invalid"}}),
            0,
        ),
    )
    .await?;
    let mut theme = command(
        "1:1",
        json!({"type":"assign","assignments":{"smeEmail":"other@example.invalid"}}),
        0,
    );
    theme.parent_version = Some(0);
    assert!(matches!(
        s.command(aid.clone(), bob.clone(), theme)
            .await
            .unwrap_err()
            .downcast_ref::<AppError>(),
        Some(AppError::Conflict(_))
    ));
    s.command(
        aid.clone(),
        alice.clone(),
        command("", json!({"type":"archive"}), 0),
    )
    .await?;
    assert!(matches!(
        s.command(
            aid.clone(),
            bob.clone(),
            command("1", json!({"type":"addComment","text":"Late"}), 0)
        )
        .await
        .unwrap_err()
        .downcast_ref::<AppError>(),
        Some(AppError::Archived)
    ));
    s.command(
        aid.clone(),
        alice.clone(),
        command("", json!({"type":"restore"}), 1),
    )
    .await?;
    s.command(
        aid.clone(),
        bob.clone(),
        command("1", json!({"type":"reviewer","userId":bob.id}), 0),
    )
    .await?;
    let view = s.review(&aid, &alice).await?;
    assert_eq!(view["reviewers"]["1"]["value"]["userId"], bob.id);
    assert_eq!(
        view["review"]["annotations"]["entries"]["1"]["workflow"]["assignments"]["smeEmail"],
        "sme@example.invalid"
    );
    Ok(())
}
#[tokio::test]
async fn restart_retains_records_and_personal_views() -> Result<()> {
    let (dir, s, alice, bob, aid) = fixture().await?;
    assert!(Store::open(dir.path(), 4).await.is_err());
    let view = ViewState {
        detail_drilldown_label: "Alice's view".into(),
        ..Default::default()
    };
    s.save_view(&aid, &alice, &view).await?;
    assert_eq!(
        s.review(&aid, &bob).await?["view"]["detailDrilldownLabel"],
        ""
    );
    drop(s);
    let reopened = Store::open(dir.path(), 4).await?;
    assert_eq!(
        reopened.review(&aid, &alice).await?["view"]["detailDrilldownLabel"],
        "Alice's view"
    );
    reopened.publish().await?;
    assert_eq!(reopened.events(0).await?.len(), 1);
    assert_eq!(reopened.list(" SHARED ", false).await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn concurrent_claims_preserve_workflow_fields_and_deleted_comments_stay_deleted() -> Result<()>
{
    let (_dir, s, alice, bob, aid) = fixture().await?;
    let assignments = json!({"smeEmail":"sme@example.invalid","ownerEmail":"owner@example.invalid","eta":"2026-12-01"});
    s.command(
        aid.clone(),
        alice.clone(),
        command("1", json!({"type":"assign","assignments":assignments}), 0),
    )
    .await?;
    let (a, b) = tokio::join!(
        s.command(
            aid.clone(),
            alice.clone(),
            command("1", json!({"type":"reviewer","userId":alice.id}), 0)
        ),
        s.command(
            aid.clone(),
            bob.clone(),
            command("1", json!({"type":"reviewer","userId":bob.id}), 0)
        )
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let snapshot = s.review(&aid, &alice).await?;
    assert_eq!(
        snapshot["review"]["annotations"]["entries"]["1"]["workflow"]["assignments"],
        assignments
    );
    let assigned = snapshot["reviewers"].clone();
    s.command(
        aid.clone(),
        alice.clone(),
        command(
            "1",
            json!({"type":"assign","assignments":{"smeEmail":"changed@example.invalid"}}),
            1,
        ),
    )
    .await?;
    assert_eq!(s.review(&aid, &alice).await?["reviewers"], assigned);
    let add = command(
        "1",
        json!({"type":"addComment","text":"Delete then retry the old delivery"}),
        0,
    );
    let outcome = s.command(aid.clone(), alice.clone(), add.clone()).await?;
    s.command(
        aid.clone(),
        alice.clone(),
        command(
            "1",
            json!({"type":"deleteComment","id":outcome["commentId"]}),
            0,
        ),
    )
    .await?;
    assert_eq!(s.command(aid.clone(), alice.clone(), add).await?, outcome);
    assert!(
        s.review(&aid, &alice).await?["review"]["annotations"]["entries"]["1"]["comments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "Internal crash-test child; invoked by acknowledged_commit_survives_process_kill"]
async fn crash_child() -> Result<()> {
    let root = std::env::var("CRASH_FIXTURE_ROOT")?;
    let s = Store::open(&root, 4).await?;
    let user = s
        .user("fixture:crash", "Reviewer", "reviewer@example.invalid")
        .await?;
    let mut annotations = Annotations::default();
    annotations.entries.insert(
        "1".into(),
        Annotation {
            workflow: Some(Workflow::default()),
            ..Default::default()
        },
    );
    let review = ReviewData {
        analysis_id: id(),
        fingerprint: "fixture".into(),
        annotations,
    };
    let aid = s
        .create(
            &user,
            &id(),
            "Crash fixture",
            "fixture.icas",
            InitialReview {
                review: &review,
                view: &ViewState::default(),
                metadata: &Default::default(),
            },
        )
        .await?;
    s.command(
        aid.clone(),
        user.clone(),
        command("1", json!({"type":"rename","label":"Acknowledged"}), 0),
    )
    .await?;
    let c = s.connect().await?;
    execute(&c, "BEGIN CONCURRENT").await?;
    c.execute("INSERT INTO users(id,external_id,name,email) VALUES('uncommitted','uncommitted','Never committed','none@example.invalid')",()).await?;
    let mut file = std::fs::File::create(std::path::Path::new(&root).join("ack.json"))?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec(&json!({"aid":aid,"user":user}))?)?;
    file.sync_all()?;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[tokio::test]
async fn acknowledged_commit_survives_process_kill() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut child = tokio::process::Command::new(std::env::current_exe()?)
        .args([
            "--ignored",
            "--exact",
            "storage::tests::crash_child",
            "--nocapture",
        ])
        .env("CRASH_FIXTURE_ROOT", dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while !dir.path().join("ack.json").exists() {
            ensure!(
                child.try_wait()?.is_none(),
                "Crash fixture exited before acknowledgment"
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    child.kill().await?;
    child.wait().await?;
    let ack: Value = serde_json::from_reader(std::fs::File::open(dir.path().join("ack.json"))?)?;
    let user: User = serde_json::from_value(ack["user"].clone())?;
    let s = Store::open(dir.path(), 4).await?;
    let snapshot = s.review(ack["aid"].as_str().unwrap(), &user).await?;
    assert_eq!(
        snapshot["review"]["annotations"]["labels"]["1"],
        "Acknowledged"
    );
    assert_eq!(s.users().await?.len(), 1);
    s.publish().await?;
    assert_eq!(s.events(0).await?.len(), 2);
    Ok(())
}
