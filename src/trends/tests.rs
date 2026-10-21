use super::*;
use crate::{
    model::{Cluster, ClusterId},
    session::{ReviewData, ViewState},
    storage::{InitialReview, PortableMeta, Store},
};
use std::sync::Arc;

fn user() -> User {
    User {
        id: "reviewer".into(),
        name: "Reviewer".into(),
        email: "reviewer@example.invalid".into(),
    }
}
fn run(texts: &[&str]) -> AnalysisRun {
    let mut run = crate::fixtures::run(texts.len());
    for (r, text) in run.processed_incidents.iter_mut().zip(texts) {
        r.analysis_text = (*text).into();
    }
    run.clusters = texts
        .iter()
        .enumerate()
        .map(|(i, _)| Cluster {
            id: ClusterId(i + 1),
            label: format!("Problem {}", i + 1),
            incident_row_indices: vec![i],
            subgroups: vec![],
        })
        .collect();
    run
}
fn fixture() -> (Series, BTreeMap<String, Vec<Profile>>) {
    let a = run(&[
        "payment card checkout rejected bank gateway",
        "printer toner cartridge jammed paper",
    ]);
    let b = run(&["payment card checkout rejected bank gateway"]);
    let model = fit(&a, "first".into(), true);
    let pa = profiles(&a, &model);
    let pb = profiles(&b, &model);
    let mut s = Series::new("Series".into());
    s.model = Some(model);
    s.periods.push(period(
        "a".into(),
        "First".into(),
        "first".into(),
        "2026-01-01".into(),
        "2026-01-31".into(),
        3,
        &pa,
    ));
    s.periods.push(period(
        "b".into(),
        "Second".into(),
        "second".into(),
        "2026-03-01".into(),
        "2026-03-31".into(),
        2,
        &pb,
    ));
    (
        s,
        BTreeMap::from([("first".into(), pa), ("second".into(), pb)]),
    )
}
#[test]
fn chronology_recurrence_and_full_denominator() -> Result<()> {
    let (mut s, p) = fixture();
    s.periods.reverse();
    recalculate(&mut s, &p)?;
    let problem = s.periods[0].occurrences[0].problem.clone().unwrap();
    assert_eq!(s.periods[1].occurrences[0].problem.as_ref(), Some(&problem));
    let cells = cells(&s, &problem);
    assert_eq!(cells[0].count, Some(1));
    assert!((cells[0].percent.unwrap() - 100. / 3.).abs() < 0.001);
    assert_eq!(cells[1].percent, Some(50.));
    let other = s.periods[0].occurrences[1].problem.as_ref().unwrap();
    assert_eq!(super::cells(&s, other)[1].count, None);
    Ok(())
}
#[test]
fn whole_period_splits_are_pending_and_manual_combination_deduplicates() -> Result<()> {
    let (mut s, mut p) = fixture();
    let mut duplicate = p["second"][0].clone();
    duplicate.cluster = 7;
    p.get_mut("second").unwrap().push(duplicate.clone());
    let mut occurrence = s.periods[1].occurrences[0].clone();
    occurrence.cluster = 7;
    s.periods[1].occurrences.push(occurrence);
    recalculate(&mut s, &p)?;
    assert!(s.periods[1]
        .occurrences
        .iter()
        .all(|o| o.problem.is_none() && o.reason.contains("split")));
    let problem = s.periods[0].occurrences[0].problem.clone().unwrap();
    let pid = s.periods[1].id.clone();
    s.apply(
        &Action::Link {
            period: pid,
            clusters: vec![1, 7],
            problem: Some(problem.clone()),
            new_name: None,
            revoke_zero: false,
        },
        &user(),
    )?;
    recalculate(&mut s, &p)?;
    assert_eq!(cells(&s, &problem)[1].count, Some(1));
    assert!(s.periods[1].occurrences.iter().all(|o| o.manual));
    Ok(())
}
#[test]
fn zero_exclusion_replacement_and_manual_protection() -> Result<()> {
    let (mut s, p) = fixture();
    recalculate(&mut s, &p)?;
    let problem = s.periods[0].occurrences[0].problem.clone().unwrap();
    let pid = s.periods[1].id.clone();
    assert!(s
        .clone()
        .apply(
            &Action::Zero {
                period: pid.clone(),
                problem: problem.clone(),
                confirmed: true
            },
            &user()
        )
        .is_err());
    s.apply(
        &Action::Link {
            period: pid.clone(),
            clusters: vec![1],
            problem: None,
            new_name: None,
            revoke_zero: false,
        },
        &user(),
    )?;
    s.apply(
        &Action::Zero {
            period: pid.clone(),
            problem: problem.clone(),
            confirmed: true,
        },
        &user(),
    )?;
    recalculate(&mut s, &p)?;
    assert_eq!(cells(&s, &problem)[1].count, Some(0));
    assert!(s
        .clone()
        .apply(
            &Action::Link {
                period: pid.clone(),
                clusters: vec![1],
                problem: Some(problem.clone()),
                new_name: None,
                revoke_zero: false
            },
            &user()
        )
        .is_err());
    s.apply(
        &Action::Link {
            period: pid,
            clusters: vec![1],
            problem: Some(problem.clone()),
            new_name: None,
            revoke_zero: true,
        },
        &user(),
    )?;
    s.periods[1].occurrences[0].replacement = true;
    recalculate(&mut s, &p)?;
    assert_eq!(s.periods[1].occurrences[0].problem.as_ref(), Some(&problem));
    s.periods[1].occurrences[0].manual = false;
    recalculate(&mut s, &p)?;
    assert!(s.periods[1].occurrences[0].problem.is_none());
    Ok(())
}
#[test]
fn dates_and_archive_guards() -> Result<()> {
    assert!(validate_dates("2026-02-29", "2026-03-01").is_err());
    assert!(validate_dates("2026-03-02", "2026-03-01").is_err());
    validate_dates("2024-02-29", "2024-02-29")?;
    let (mut s, _) = fixture();
    s.periods[1].start = "2026-01-31".into();
    assert!(s.validate().is_err());
    s.periods[1].start = "2026-02-01".into();
    s.validate()?;
    s.archived = true;
    assert!(s
        .apply(
            &Action::Rename {
                name: "Other".into()
            },
            &user()
        )
        .is_err());
    s.apply(&Action::Archive { archived: false }, &user())?;
    Ok(())
}
#[test]
fn ambiguous_new_and_unusable_text_are_distinct() -> Result<()> {
    let (mut s, mut p) = fixture();
    p.get_mut("first").unwrap()[1].vector = p["first"][0].vector.clone();
    recalculate(&mut s, &p)?;
    assert!(s.periods[1].occurrences[0].problem.is_none());
    assert_eq!(s.periods[1].occurrences[0].candidates.len(), 2);
    p.get_mut("second").unwrap()[0].vector.clear();
    recalculate(&mut s, &p)?;
    assert!(s.periods[1].occurrences[0].reason.contains("Insufficient"));
    let (mut s, mut p) = fixture();
    p.get_mut("second").unwrap()[0].vector = BTreeMap::from([("unrelated".into(), 1.)]);
    recalculate(&mut s, &p)?;
    assert_eq!(s.periods[1].occurrences[0].reason, "New tracked problem");
    Ok(())
}
#[test]
fn rejected_candidates_stay_rejected_and_gate_blocks_automatic_links() -> Result<()> {
    let (mut s, p) = fixture();
    s.model.as_mut().unwrap().automatic = false;
    recalculate(&mut s, &p)?;
    assert!(s.periods[1].occurrences[0].problem.is_none());
    let candidate = s.periods[1].occurrences[0].candidates[0].problem.clone();
    let pid = s.periods[1].id.clone();
    s.apply(
        &Action::Reject {
            period: pid,
            cluster: 1,
            problem: candidate.clone(),
            reopen: false,
        },
        &user(),
    )?;
    s.model.as_mut().unwrap().automatic = true;
    recalculate(&mut s, &p)?;
    assert!(s.periods[1].occurrences[0].problem.is_none());
    assert!(s.periods[1].occurrences[0].rejected.contains(&candidate));
    Ok(())
}

#[test]
fn manual_correction_cannot_create_a_gradual_automatic_drift_chain() -> Result<()> {
    let (mut s, mut profiles) = fixture();
    // A -> B passes the reference check, but C only resembles B, not the founding A.
    let a = BTreeMap::from([("x".into(), 1.)]);
    let b = BTreeMap::from([("x".into(), 0.9), ("y".into(), (1_f32 - 0.81).sqrt())]);
    let c = BTreeMap::from([("x".into(), 0.65), ("y".into(), (1_f32 - 0.4225).sqrt())]);
    profiles.get_mut("first").unwrap()[0].vector = a;
    profiles.get_mut("second").unwrap()[0].vector = b;
    profiles.get_mut("second").unwrap()[0].examples.clear();
    let mut third = profiles["second"].clone();
    third[0].vector = c;
    let next = period(
        "c".into(),
        "Third".into(),
        "third".into(),
        "2026-04-01".into(),
        "2026-04-30".into(),
        1,
        &third,
    );
    profiles.insert("third".into(), third);
    s.periods.push(next);
    recalculate(&mut s, &profiles)?;
    assert_eq!(s.periods[1].occurrences[0].reason, "Automatic text match");
    assert!(s.periods[2].occurrences[0].problem.is_none());
    Ok(())
}

#[test]
#[ignore = "Explicit 150,000-row trend-profile workload measurement"]
fn trend_profiles_expected_large_analysis() -> Result<()> {
    let run = crate::fixtures::run(150_000);
    let start = std::time::Instant::now();
    let model = fit(&run, "large-fixture".into(), false);
    let fit_ms = start.elapsed().as_millis();
    let ps = super::profiles(&run, &model);
    let total_ms = start.elapsed().as_millis();
    let bytes = serde_json::to_vec(&ps)?;
    println!("trend workload: rows={}, clusters={}, vocabulary={}, fit_ms={}, total_ms={}, profile_bytes={}",run.processed_incidents.len(),ps.len(),model.weights.len(),fit_ms,total_ms,bytes.len());
    assert_eq!(ps.len(), 150);
    assert_eq!(ps.iter().map(|p| p.rows.len()).sum::<usize>(), 150_000);
    Ok(())
}
async fn storage_fixture() -> Result<(tempfile::TempDir, Arc<Store>, User, String)> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path(), 4).await?;
    let user = store.user("test", "Reviewer", "r@example.invalid").await?;
    let review = ReviewData {
        analysis_id: id(),
        fingerprint: "source".into(),
        annotations: Default::default(),
    };
    let aid = store
        .create(
            &user,
            &id(),
            "Analysis",
            "fixture.icas",
            InitialReview {
                review: &review,
                view: &ViewState::default(),
                metadata: &PortableMeta::default(),
            },
        )
        .await?;
    Ok((dir, store, user, aid))
}
#[tokio::test]
async fn durable_preview_conflicts_membership_receipts_and_typed_events() -> Result<()> {
    let (_dir, store, user, aid) = storage_fixture().await?;
    let created = store.create_series(&user, &id(), "Series").await?;
    let sid = created["id"].as_str().unwrap();
    let command = Command {
        command_id: id(),
        expected: 0,
        action: Action::Recalculate,
    };
    let result = store.series_command(&user, sid, &command).await?;
    assert_eq!(result, store.series_command(&user, sid, &command).await?);
    let mut task = store.trend_task(result["task"].as_str().unwrap()).await?;
    let mut proposed = store.series(sid).await?;
    let (mut sample, p) = fixture();
    recalculate(&mut sample, &p)?;
    proposed.problems = sample.problems;
    proposed.model = sample.model;
    proposed.periods = vec![sample.periods.remove(0)];
    proposed.periods[0].analysis = aid.clone();
    proposed.periods[0].fingerprint = "source".into();
    task.proposed = Some(proposed);
    task.state = "preview".into();
    store.save_trend_task(&task).await?;
    let apply_id = id();
    let applied = store
        .apply_trend_preview(&user, sid, &task.id, &apply_id)
        .await?;
    assert_eq!(
        applied,
        store
            .apply_trend_preview(&user, sid, &task.id, &apply_id)
            .await?
    );
    let other = store.create_series(&user, &id(), "Other").await?;
    let otherid = other["id"].as_str().unwrap();
    let conflict = store
        .series_command(
            &user,
            otherid,
            &Command {
                command_id: id(),
                expected: 0,
                action: Action::AddPeriod {
                    analysis: aid.clone(),
                    start: "2026-03-01".into(),
                    end: "2026-03-31".into(),
                    replace: None,
                },
            },
        )
        .await
        .expect_err("An already assigned analysis must be rejected before preparing a proposal");
    assert!(conflict.to_string().contains("Series"), "{conflict}");
    let result = store.series_command(&user, otherid, &command).await;
    assert!(result.is_err(), "Receipt scope includes series");
    let result = store
        .series_command(
            &user,
            otherid,
            &Command {
                command_id: id(),
                ..command.clone()
            },
        )
        .await?;
    let mut duplicate = store.trend_task(result["task"].as_str().unwrap()).await?;
    let mut copied = store.series(sid).await?;
    copied.id = otherid.into();
    copied.name = "Other".into();
    duplicate.proposed = Some(copied);
    duplicate.state = "preview".into();
    store.save_trend_task(&duplicate).await?;
    let conflict = store
        .apply_trend_preview(&user, otherid, &duplicate.id, &id())
        .await
        .expect_err("A competing proposal must still be checked when saving");
    assert!(conflict.to_string().contains("Series"), "{conflict}");
    let memberships = store.trend_memberships().await?;
    assert_eq!(memberships[0]["series"], sid);
    assert_eq!(memberships[0]["analysis"], aid);
    assert!(
        store.series(otherid).await?.periods.is_empty(),
        "Failed publication is atomic"
    );
    let mut stale = task.clone();
    stale.id = id();
    stale.proposed = Some(store.series(sid).await?);
    // Existing task, now stale because its expected revision is still zero.
    task.proposed = stale.proposed;
    task.state = "preview".into();
    store.save_trend_task(&task).await?;
    assert!(store
        .apply_trend_preview(&user, sid, &task.id, &id())
        .await
        .is_err());
    store.publish().await?;
    let events = store.events(0).await?;
    assert!(events
        .iter()
        .any(|e| e["seriesId"] == sid && e["analysisId"].is_null()));
    assert!(events.iter().any(|e| e["analysisId"] == aid));
    assert!(!store
        .trend_history(sid, 0)
        .await?
        .as_array()
        .unwrap()
        .is_empty());
    assert!(
        store.delete_series(&user, sid, &id(), 0).await.is_err(),
        "Stale deletion must not remove the series"
    );
    assert_eq!(store.series(sid).await?.periods.len(), 1);
    let delete_id = id();
    let result = store.delete_series(&user, sid, &delete_id, 1).await?;
    assert_eq!(
        result,
        store.delete_series(&user, sid, &delete_id, 1).await?
    );
    assert!(store.series(sid).await.is_err());
    assert_eq!(store.trend_memberships().await?, serde_json::json!([]));
    assert_eq!(store.trend_tasks(sid).await?, serde_json::json!([]));
    assert_eq!(store.trend_history(sid, 0).await?, serde_json::json!([]));
    // A worker holding an old task cannot recreate the deleted series or proposal.
    store.save_trend_task(&task).await?;
    assert!(store.trend_task(&task.id).await.is_err());
    assert!(store
        .apply_trend_preview(&user, sid, &task.id, &id())
        .await
        .is_err());
    // Saved source and name can both be reused after deletion.
    store
        .apply_trend_preview(&user, otherid, &duplicate.id, &id())
        .await?;
    assert_eq!(store.series(otherid).await?.periods[0].analysis, aid);
    store.create_series(&user, &id(), "Series").await?;
    store.publish().await?;
    assert!(store
        .events(0)
        .await?
        .iter()
        .any(|e| e["seriesId"] == sid && e["change"]["deleted"] == true));
    Ok(())
}
#[tokio::test]
async fn migration_reopen_profiles_and_task_recovery() -> Result<()> {
    let (dir, store, user, _) = storage_fixture().await?;
    let s = store.create_series(&user, &id(), "Persisted").await?;
    let sid = s["id"].as_str().unwrap();
    let result = store
        .series_command(
            &user,
            sid,
            &Command {
                command_id: id(),
                expected: 0,
                action: Action::Recalculate,
            },
        )
        .await?;
    let mut task = store.trend_task(result["task"].as_str().unwrap()).await?;
    task.state = "running".into();
    store.save_trend_task(&task).await?;
    let (_, p) = fixture();
    store.save_trend_profile("cache", &p["first"]).await?;
    drop(store);
    let reopened = Store::open(dir.path(), 4).await?;
    reopened.recover_trend_tasks().await?;
    assert_eq!(reopened.trend_task(&task.id).await?.state, "queued");
    assert_eq!(reopened.trend_profile("cache").await?.unwrap().len(), 2);
    assert_eq!(reopened.series(sid).await?.name, "Persisted");
    Ok(())
}

#[tokio::test]
async fn concurrent_series_edits_allow_one_revision_and_keep_archived_names_reserved() -> Result<()>
{
    let (_dir, store, user, _) = storage_fixture().await?;
    let bob = store
        .user("second", "Second reviewer", "second@example.invalid")
        .await?;
    let created = store.create_series(&user, &id(), "Concurrent").await?;
    let sid = created["id"].as_str().unwrap();
    let a = Command {
        command_id: id(),
        expected: 0,
        action: Action::Rename {
            name: "First name".into(),
        },
    };
    let b = Command {
        command_id: id(),
        expected: 0,
        action: Action::Rename {
            name: "Second name".into(),
        },
    };
    let (left, right) = tokio::join!(
        store.series_command(&user, sid, &a),
        store.series_command(&bob, sid, &b)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    assert_eq!(store.series(sid).await?.revision, 1);
    store
        .series_command(
            &user,
            sid,
            &Command {
                command_id: id(),
                expected: 1,
                action: Action::Archive { archived: true },
            },
        )
        .await?;
    let name = store.series(sid).await?.name;
    assert!(store
        .create_series(&bob, &id(), &format!(" {} ", name.to_uppercase()))
        .await
        .is_err());
    assert!(store
        .series_command(
            &bob,
            sid,
            &Command {
                command_id: id(),
                expected: 2,
                action: Action::Recalculate
            }
        )
        .await
        .is_err());
    store
        .series_command(
            &bob,
            sid,
            &Command {
                command_id: id(),
                expected: 2,
                action: Action::Archive { archived: false },
            },
        )
        .await?;
    Ok(())
}
