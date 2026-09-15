//! Explicit workload checks; print evidence rather than pretending local hardware is production.
use crate::{
    artifacts::Artifacts,
    fixtures,
    session::{ReviewData, ViewState},
    storage::{id, Command, InitialReview, Store},
};
use anyhow::Result;
use serde_json::json;
use std::{sync::Arc, time::Instant};

#[tokio::test]
#[ignore = "Large synthetic acceptance workload; run explicitly, preferably --release"]
async fn large_artifact_save_open_and_capacity() -> Result<()> {
    for count in [20_000, 150_000, 200_000] {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path(), 4).await?;
        let user = store
            .user(
                "fixture:volume",
                "Volume Reviewer",
                "volume@example.invalid",
            )
            .await?;
        let mut run = fixtures::run(count);
        let mut random = 0x51ea9dca74_u64;
        for col in 3..30 {
            run.source.headers.push(format!("Field {col}"));
        }
        for (index, row) in run.source.rows.iter_mut().enumerate() {
            for col in 3..30 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                let first = random;
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                row.push(format!("{index:06}-{col:02}-{first:016x}-{random:016x}"));
            }
        }
        let run = Arc::new(run);
        let review = ReviewData::new(&run)?;
        let artifacts = Artifacts::new(store.root.clone(), 1024 * 1024 * 1024);
        let start = Instant::now();
        let artifact = artifacts
            .publish(
                run,
                review.clone(),
                ViewState::default(),
                Default::default(),
            )
            .await?;
        let aid = store
            .create(
                &user,
                &id(),
                "Volume fixture",
                &artifact,
                InitialReview {
                    review: &review,
                    view: &ViewState::default(),
                    metadata: &Default::default(),
                },
            )
            .await?;
        let save_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let loaded = artifacts.load(&artifact).await?;
        let _snapshot = store.review(&aid, &user).await?;
        let loaded_bytes = serde_json::to_vec(&loaded.run)?.len();
        let open_ms = start.elapsed().as_millis();
        assert_eq!(loaded.run.source.rows.len(), count);
        assert_eq!(loaded.run.source.headers.len(), 30);
        println!(
            "{}",
            json!({"rows":count,"columns":30,"artifactBytes":std::fs::metadata(store.root.join("artifacts").join(artifact))?.len(),"resultJsonBytes":loaded_bytes,"initialSaveMs":save_ms,"coldOpenAndSerializeMs":open_ms,"optimized":!cfg!(debug_assertions)})
        );
    }
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "Explicit 50-user mixed storage workload"]
async fn fifty_users_review_with_readers() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path(), 4).await?;
    let run = fixtures::run(1000);
    let review = ReviewData::new(&run)?;
    let mut users = vec![];
    for i in 0..500 {
        users.push(
            store
                .user(
                    &format!("fixture:{i}"),
                    &format!("Reviewer {i}"),
                    &format!("reviewer{i}@example.invalid"),
                )
                .await?,
        );
    }
    let mut analyses = vec![];
    for i in 0..1000 {
        analyses.push(
            store
                .create(
                    &users[0],
                    &id(),
                    &format!("Team {i}"),
                    "fixture.icas",
                    InitialReview {
                        review: &review,
                        view: &ViewState::default(),
                        metadata: &Default::default(),
                    },
                )
                .await?,
        );
    }
    let mut tasks = tokio::task::JoinSet::new();
    assert_eq!(store.users().await?.len(), 500);
    assert_eq!(store.list("", false).await?.len(), 1000);
    for (i, user) in users.into_iter().take(50).enumerate() {
        let store = store.clone();
        let aid = analyses[i % 5].clone();
        tasks.spawn(async move{
        let mut latencies=vec![];
        for iteration in 0..20{let started=Instant::now();
            store.command(aid.clone(),user.clone(),Command{command_id:id(),target:"1".into(),action:json!({"type":"addComment","text":format!("User {i}, review {iteration}")}),expected:0,parent_version:None}).await?;
            latencies.push(started.elapsed().as_millis());let snapshot=store.review(&aid,&user).await?;assert!(!snapshot["review"]["annotations"]["entries"]["1"]["comments"].as_array().unwrap().is_empty());
        }
        Ok::<_,anyhow::Error>(latencies)
    });
    }
    let mut latencies = vec![];
    while let Some(result) = tasks.join_next().await {
        latencies.extend(result??);
    }
    latencies.sort_unstable();
    loop {
        store.publish().await?;
        let c = store.connect().await?;
        if crate::storage::scalar(&c, "SELECT COUNT(*) FROM outbox WHERE published=0").await? == 0 {
            break;
        }
    }
    let c = store.connect().await?;
    assert_eq!(
        crate::storage::scalar(&c, "SELECT COUNT(*) FROM comments").await?,
        1000
    );
    assert_eq!(
        crate::storage::scalar(&c, "SELECT COUNT(*) FROM deliveries").await?,
        2000
    );
    println!(
        "{}",
        json!({"directoryUsers":500,"savedAnalyses":1000,"activeUsers":50,"activeAnalyses":5,"writers":4,"edits":latencies.len(),"p50Ms":latencies[latencies.len()/2],"p95Ms":latencies[latencies.len()*95/100],"p99Ms":latencies[latencies.len()*99/100],"maximumMs":latencies.last(),"optimized":!cfg!(debug_assertions)})
    );
    Ok(())
}
