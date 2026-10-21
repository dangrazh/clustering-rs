//! Offline matching evaluation. Never modifies application storage or enables production matching.
use anyhow::{ensure, Context, Result};
use incident_clustering_analyzer::{
    model::{Cluster, ClusterId},
    session, storage, trends,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
#[path = "../src/test_fixtures.rs"]
mod fixtures;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Dataset {
    kind: String,
    description: String,
    plausible: f32,
    strong: f32,
    margin: f32,
    periods: Vec<Source>,
}
#[derive(Serialize, Deserialize)]
struct Source {
    file: String,
    start: String,
    end: String,
    truth: BTreeMap<usize, String>,
}
fn generate(dir: &Path) -> Result<()> {
    ensure!(!dir.exists(), "Choose a new directory for synthetic data");
    std::fs::create_dir_all(dir)?;
    let definitions = [
        vec![
            "payment card rejected bank gateway",
            "printer cartridge paper jam",
            "wireless authentication certificate expired",
        ],
        vec![
            "printer cartridge paper jam",
            "payment card rejected bank gateway",
            "database backup disk full",
        ],
        vec![
            "wireless authentication certificate expired",
            "database backup disk full",
        ],
        vec![
            "payment card rejected bank gateway",
            "payment card rejected bank gateway",
            "printer cartridge paper jam",
        ],
    ];
    let mut periods = vec![];
    for (month, texts) in definitions.iter().enumerate() {
        let mut run = fixtures::run(texts.len() * 20);
        run.clusters.clear();
        let mut truth = BTreeMap::new();
        for (i, text) in texts.iter().enumerate() {
            let cid = i + 1;
            truth.insert(cid, (*text).into());
            for row in i * 20..(i + 1) * 20 {
                run.processed_incidents[row].analysis_text = (*text).into();
                run.processed_incidents[row].incident_number =
                    format!("PERIOD{}-INC{}", month + 1, row);
                run.source.rows[row][0] = run.processed_incidents[row].incident_number.clone();
                run.source.rows[row][1] = (*text).into();
            }
            run.clusters.push(Cluster {
                id: ClusterId(cid),
                label: format!("Period {} cluster {}", month + 1, cid),
                incident_row_indices: (i * 20..(i + 1) * 20).collect(),
                subgroups: vec![],
            });
        }
        let file = format!("period-{}.icas", month + 1);
        std::fs::write(
            dir.join(&file),
            session::encode_portable(
                &run,
                &session::ReviewData::new(&run)?,
                &Default::default(),
                &Default::default(),
            )?,
        )?;
        periods.push(Source {
            file,
            start: format!("2026-{:02}-01", month + 1),
            end: format!("2026-{:02}-28", month + 1),
            truth,
        });
    }
    let dataset=Dataset{kind:"synthetic".into(),description:"Mechanics fixtures: new IDs, reordered cluster numbers, recurrence after a gap, novel problem and a split. Not representative quality evidence.".into(),plausible:0.30,strong:0.82,margin:0.12,periods};
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&dataset)?,
    )?;
    println!("Created {}", dir.join("manifest.json").display());
    Ok(())
}
fn evaluate(path: &Path) -> Result<()> {
    let mut dataset: Dataset = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(!dataset.periods.is_empty(), "No periods");
    dataset.periods.sort_by(|a, b| a.start.cmp(&b.start));
    ensure!(
        dataset.plausible > 0.
            && dataset.plausible < dataset.strong
            && dataset.strong <= 1.
            && dataset.margin > 0.
            && dataset.margin <= 1.,
        "Invalid thresholds"
    );
    let began = std::time::Instant::now();
    let mut series = trends::Series::new("Offline evaluation".into());
    let mut profiles = BTreeMap::new();
    let mut truth = BTreeMap::new();
    let mut hashes = vec![];
    for source in &dataset.periods {
        let bytes = std::fs::read(path.parent().unwrap_or(Path::new(".")).join(&source.file))?;
        let loaded = session::decode_session(&bytes)?;
        let fingerprint = session::fingerprint(&loaded.run)?;
        hashes.push(json!({"file":source.file,"sha256":storage::digest(&bytes)}));
        if series.model.is_none() {
            let mut model = trends::fit(&loaded.run, fingerprint.clone(), true);
            model.plausible = dataset.plausible;
            model.strong = dataset.strong;
            model.margin = dataset.margin;
            series.model = Some(model);
        }
        let ps = trends::profiles(&loaded.run, series.model.as_ref().unwrap());
        ensure!(
            ps.iter().all(|p| source.truth.contains_key(&p.cluster)),
            "Every cluster requires a truth label"
        );
        let period = trends::period(
            source.file.clone(),
            source.file.clone(),
            fingerprint.clone(),
            source.start.clone(),
            source.end.clone(),
            loaded.run.processed_incidents.len(),
            &ps,
        );
        for (cluster, label) in &source.truth {
            truth.insert((period.id.clone(), *cluster), label.clone());
        }
        series.periods.push(period);
        profiles.insert(fingerprint, ps);
    }
    trends::recalculate(&mut series, &profiles)?;
    let mut identities = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let (mut automatic, mut incorrect, mut pending, mut new_errors, mut recurring) =
        (0, 0, 0, 0, 0);
    let mut decisions = vec![];
    for p in &series.periods {
        let mut observed = vec![];
        for o in &p.occurrences {
            let expected = &truth[&(p.id.clone(), o.cluster)];
            let prior = seen.contains(expected);
            if prior {
                recurring += 1;
            }
            let predicted = o
                .problem
                .as_ref()
                .and_then(|id| identities.get(id))
                .cloned();
            if o.reason == "Automatic text match" {
                automatic += 1;
                if predicted.as_ref() != Some(expected) {
                    incorrect += 1;
                }
            }
            if o.problem.is_none() {
                pending += 1;
            } else if predicted.is_none() && prior {
                new_errors += 1;
            }
            if let Some(id) = &o.problem {
                identities
                    .entry(id.clone())
                    .or_insert_with(|| expected.clone());
            }
            decisions.push(json!({"period":p.start,"cluster":o.cluster,"expected":expected,"predicted":predicted,"decision":o.reason,"candidates":o.candidates}));
            observed.push(expected.clone());
        }
        seen.extend(observed);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"kind":dataset.kind,"description":dataset.description,"qualityApproved":false,"model":series.model.as_ref().map(|m|&m.version),"thresholds":{"plausible":dataset.plausible,"strong":dataset.strong,"margin":dataset.margin},"sources":hashes,"elapsedMs":began.elapsed().as_millis(),"automaticLinks":automatic,"falseAutomaticLinks":incorrect,"automaticPrecision":if automatic>0{Some(1.-incorrect as f64/automatic as f64)}else{None},"recurringOccurrences":recurring,"automaticCoverage":if recurring>0{Some(automatic as f64/recurring as f64)}else{None},"pending":pending,"newProblemErrors":new_errors,"decisions":decisions})
        )?
    );
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--generate-synthetic") {
        return generate(&PathBuf::from(
            args.get(1).context("Provide a new output directory")?,
        ));
    }
    evaluate(Path::new(args.first().context("Usage: cargo run --example evaluate_trends -- <manifest.json> OR --generate-synthetic <new-directory>")?))
}
