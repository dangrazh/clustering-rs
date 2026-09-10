//! Synthetic, non-sensitive session benchmark. Run with: cargo run --release --example session_benchmark -- 150000 6 [output.icas]
#[path = "../tests/common/mod.rs"]
mod common;
use incident_clustering_analyzer::{session::*, workflow::*};
use std::{io::Write, time::Instant};
struct Counter(u64);
impl Write for Counter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0 += b.len() as u64;
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let count = args
        .get(1)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(100_000);
    let level = args
        .get(2)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(COMPRESSION_LEVEL);
    let run = common::run(count);
    let mut review = ReviewData::new(&run)?;
    let keys: Vec<_> = review.annotations.entries.keys().cloned().collect();
    for key in keys {
        for action in [Action::Transition{status:ReviewStatus::Reviewed,assignments:Assignments::default()},Action::AddComment{text:format!("Investigation for {key}: connectivity issue confirmed by service desk; assess recurring pattern and customer impact. Contact service owner before the next review.")}] {
            review.annotations.mutate(Mutation{revision:review.annotations.revision,target:key.clone(),actor:Actor{name:"Benchmark Reviewer".into(),email:"reviewer@example.org".into()},action}).or_else(|e|if e.to_string().contains("transition is not allowed"){Ok(())}else{Err(e)})?;
        }
    }
    #[derive(serde::Serialize)]
    struct Legacy<'a> {
        version: u16,
        run: &'a incident_clustering_analyzer::model::AnalysisRun,
        #[serde(rename = "reviewState")]
        review_state: serde_json::Value,
    }
    let mut json = Counter(0);
    serde_json::to_writer_pretty(
        &mut json,
        &Legacy {
            version: 2,
            run: &run,
            review_state: serde_json::json!({"reviewedClusters":[],"reviewedThemes":[]}),
        },
    )?;
    let start = Instant::now();
    let bytes = encode_session_level(&run, &review, &ViewState::default(), level)?;
    let save_ms = start.elapsed().as_millis();
    let start = Instant::now();
    let loaded = decode_session(&bytes)?;
    let load_ms = start.elapsed().as_millis();
    assert_eq!(loaded.run, run);
    assert_eq!(loaded.review.annotations, review.annotations);
    if let Some(path) = args.get(3) {
        std::fs::write(path, &bytes)?;
    }
    println!(
        "{}",
        serde_json::json!({"records":count,"level":level,"jsonBytes":json.0,"binaryBytes":bytes.len(),"saveMs":save_ms,"loadMs":load_ms,"entities":review.annotations.entries.len()})
    );
    Ok(())
}
