use super::*;
use crate::fixtures;

fn input() -> Input {
    let mut run = fixtures::run(12);
    run.source.source_path = Some("incident-export.xlsx".into());
    Input {
        source: run.source,
        mapping: run.mapping,
        settings: run.settings,
    }
}
#[tokio::test]
async fn fifo_restart_cancellation_and_ownership() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path(), 4).await?;
    let alice = store
        .user("fixture:alice", "Alice", "alice@example.invalid")
        .await?;
    let bob = store
        .user("fixture:bob", "Bob", "bob@example.invalid")
        .await?;
    let first = enqueue(&store, &alice, input()).await?;
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let second = enqueue(&store, &bob, input()).await?;
    assert_eq!(next(&store).await?.unwrap().id, first);
    assert!(change(&store, &first, "queued", "running", None, None).await?);
    assert!(cancel(&store, &alice, &second).await.is_err());
    drop(store);
    let store = Store::open(dir.path(), 4).await?;
    interrupted(&store).await?;
    let summary = get(&store, &first).await?.metadata["summary"].clone();
    assert_eq!(summary["sourceFileName"], "incident-export.xlsx");
    assert_eq!(summary["sourceRows"], 12);
    assert_eq!(summary["columnCount"], 3);
    assert!(summary["processedRows"].is_null());
    assert_eq!(get(&store, &first).await?.state, "failed");
    assert_eq!(get(&store, &second).await?.state, "queued");
    assert_eq!(next(&store).await?.unwrap().id, second);
    cancel(&store, &bob, &second).await?;
    assert!(next(&store).await?.is_none());
    assert_eq!(list(&store, &alice.id).await?.len(), 1);
    Ok(())
}
#[test]
fn worker_output_reopens_with_immutable_membership() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("input.json");
    let output = dir.path().join("output.icas");
    serde_json::to_writer(std::fs::File::create(&source)?, &input())?;
    worker(&source, &output)?;
    let session = session::decode_session(&std::fs::read(output)?)?;
    assert_eq!(session.run.source.rows.len(), 12);
    session.review.validate(&session.run)?;
    Ok(())
}
