//! Cluster occurrences are immutable evidence; series links are independent review decisions.
use crate::{
    model::AnalysisRun,
    storage::{id, User},
};
#[cfg(test)]
mod tests;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type Vector = BTreeMap<String, f32>;

pub fn validate_source(run: &AnalysisRun) -> Result<()> {
    let rows: BTreeSet<_> = run
        .processed_incidents
        .iter()
        .map(|r| r.source_row_index)
        .collect();
    ensure!(
        rows.len() == run.processed_incidents.len(),
        "Source contains duplicate processed row identities"
    );
    ensure!(
        rows.iter().all(|r| *r < run.source.rows.len()),
        "Processed row is outside source data"
    );
    let mut clusters = BTreeSet::new();
    for cluster in &run.clusters {
        if cluster.id.0 == 0 {
            continue;
        }
        ensure!(
            cluster.id.0 != 0 && clusters.insert(cluster.id.0),
            "Invalid or duplicate source cluster identity"
        );
        ensure!(
            cluster
                .incident_row_indices
                .iter()
                .all(|r| rows.contains(r)),
            "Cluster references an unprocessed source row"
        );
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub version: String,
    pub reference: String,
    pub weights: Vector,
    pub plausible: f32,
    pub strong: f32,
    pub margin: f32,
    pub automatic: bool,
    #[serde(default)]
    pub validation: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub cluster: usize,
    pub label: String,
    pub rows: BTreeSet<usize>,
    pub vector: Vector,
    pub examples: Vec<Vector>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Period {
    pub id: String,
    pub analysis: String,
    pub name: String,
    pub fingerprint: String,
    pub start: String,
    pub end: String,
    pub total: usize,
    pub occurrences: Vec<Occurrence>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub problem: String,
    pub score: f32,
    pub reference: f32,
    #[serde(default)]
    pub period: String,
    #[serde(default)]
    pub cluster: usize,
    #[serde(default)]
    pub evidence: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    pub cluster: usize,
    pub label: String,
    pub rows: BTreeSet<usize>,
    pub problem: Option<String>,
    pub manual: bool,
    pub reason: String,
    pub candidates: Vec<Candidate>,
    pub rejected: BTreeSet<String>,
    pub replacement: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub id: String,
    pub name: String,
    pub origin: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Zero {
    pub period: String,
    pub problem: String,
    pub actor: User,
    pub time: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub id: String,
    pub name: String,
    pub archived: bool,
    pub revision: u64,
    pub periods: Vec<Period>,
    pub problems: Vec<Problem>,
    pub zeros: Vec<Zero>,
    pub model: Option<Model>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Action {
    Rename {
        name: String,
    },
    Archive {
        archived: bool,
    },
    AddPeriod {
        analysis: String,
        start: String,
        end: String,
        replace: Option<String>,
    },
    RenameProblem {
        problem: String,
        name: String,
    },
    Link {
        period: String,
        clusters: Vec<usize>,
        problem: Option<String>,
        new_name: Option<String>,
        revoke_zero: bool,
    },
    Reopen {
        period: String,
        cluster: usize,
    },
    Reject {
        period: String,
        cluster: usize,
        problem: String,
        reopen: bool,
    },
    Zero {
        period: String,
        problem: String,
        confirmed: bool,
    },
    Recalculate,
    RebuildModel,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub command_id: String,
    pub expected: u64,
    pub action: Action,
}

impl Series {
    pub fn new(name: String) -> Self {
        Self {
            id: id(),
            name,
            archived: false,
            revision: 0,
            periods: vec![],
            problems: vec![],
            zeros: vec![],
            model: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        let mut analyses = BTreeSet::new();
        let mut previous = None;
        for p in &self.periods {
            validate_dates(&p.start, &p.end)?;
            if let Some(end) = previous {
                ensure!(end < p.start.as_str(), "Periods overlap");
            }
            previous = Some(p.end.as_str());
            ensure!(
                analyses.insert(&p.analysis),
                "An analysis can belong to only one period"
            );
            for o in &p.occurrences {
                if let Some(problem) = &o.problem {
                    ensure!(
                        self.problems.iter().any(|v| &v.id == problem),
                        "Unknown tracked problem"
                    );
                    ensure!(
                        !self
                            .zeros
                            .iter()
                            .any(|z| z.period == p.id && &z.problem == problem),
                        "Revoke the zero confirmation before linking a cluster"
                    );
                }
            }
        }
        for z in &self.zeros {
            ensure!(
                self.periods.iter().any(|p| p.id == z.period)
                    && self.problems.iter().any(|p| p.id == z.problem),
                "Invalid zero confirmation"
            );
        }
        Ok(())
    }
    pub fn apply(&mut self, action: &Action, user: &User) -> Result<()> {
        ensure!(
            !self.archived || matches!(action, Action::Archive { archived: false }),
            "Series is archived. Restore it before editing."
        );
        match action {
            Action::Rename { name } => self.name = crate::storage::normalize_name(name)?.0,
            Action::Archive { archived } => self.archived = *archived,
            Action::RenameProblem { problem, name } => {
                let name = crate::storage::normalize_name(name)?.0;
                self.problems
                    .iter_mut()
                    .find(|p| &p.id == problem)
                    .context("Unknown tracked problem")?
                    .name = name;
            }
            Action::Link {
                period,
                clusters,
                problem,
                new_name,
                revoke_zero,
            } => {
                ensure!(!clusters.is_empty(), "Select at least one cluster");
                let target = if let Some(name) = new_name {
                    let p = Problem {
                        id: id(),
                        name: crate::storage::normalize_name(name)?.0,
                        origin: format!("manual:{}", period),
                    };
                    let target = p.id.clone();
                    self.problems.push(p);
                    Some(target)
                } else {
                    problem.clone()
                };
                if let Some(t) = &target {
                    ensure!(
                        self.problems.iter().any(|p| &p.id == t),
                        "Unknown tracked problem"
                    );
                }
                let p = self
                    .periods
                    .iter_mut()
                    .find(|p| &p.id == period)
                    .context("Unknown period")?;
                ensure!(
                    clusters
                        .iter()
                        .all(|c| p.occurrences.iter().any(|o| o.cluster == *c)),
                    "Unknown cluster"
                );
                if *revoke_zero {
                    self.zeros
                        .retain(|z| !(&z.period == period && Some(&z.problem) == target.as_ref()));
                }
                for o in &mut p.occurrences {
                    if clusters.contains(&o.cluster) {
                        o.problem = target.clone();
                        o.manual = true;
                        o.replacement = false;
                        o.reason = if target.is_some() {
                            "Manually linked"
                        } else {
                            "Manually left pending"
                        }
                        .into();
                    }
                }
            }
            Action::Reopen { period, cluster } => {
                let o = self.occurrence_mut(period, *cluster)?;
                o.manual = false;
                o.replacement = false;
                o.problem = None;
            }
            Action::Reject {
                period,
                cluster,
                problem,
                reopen,
            } => {
                ensure!(
                    self.problems.iter().any(|p| &p.id == problem),
                    "Unknown tracked problem"
                );
                let o = self.occurrence_mut(period, *cluster)?;
                if *reopen {
                    o.rejected.remove(problem);
                } else {
                    ensure!(
                        !o.manual,
                        "Reopen the manual decision before rejecting candidates"
                    );
                    o.rejected.insert(problem.clone());
                    if o.problem.as_ref() == Some(problem) {
                        o.problem = None;
                    }
                }
            }
            Action::Zero {
                period,
                problem,
                confirmed,
            } => {
                let p = self
                    .periods
                    .iter()
                    .find(|p| &p.id == period)
                    .context("Unknown period")?;
                ensure!(
                    self.problems.iter().any(|p| &p.id == problem),
                    "Unknown tracked problem"
                );
                if *confirmed {
                    ensure!(
                        !p.occurrences
                            .iter()
                            .any(|o| o.problem.as_ref() == Some(problem)),
                        "Unlink the contributing clusters before confirming zero"
                    );
                }
                self.zeros
                    .retain(|z| !(&z.period == period && &z.problem == problem));
                if *confirmed {
                    self.zeros.push(Zero {
                        period: period.clone(),
                        problem: problem.clone(),
                        actor: user.clone(),
                        time: crate::storage::now(),
                    });
                }
            }
            Action::Recalculate | Action::RebuildModel => {}
            Action::AddPeriod { .. } => {
                anyhow::bail!("Period addition requires verified source data")
            }
        }
        self.validate()
    }
    fn occurrence_mut(&mut self, period: &str, cluster: usize) -> Result<&mut Occurrence> {
        self.periods
            .iter_mut()
            .find(|p| p.id == period)
            .context("Unknown period")?
            .occurrences
            .iter_mut()
            .find(|o| o.cluster == cluster)
            .context("Unknown cluster")
    }
}
pub fn validate_dates(start: &str, end: &str) -> Result<()> {
    let parse = |s: &str| -> Result<chrono::NaiveDate> {
        let d = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")?;
        ensure!(
            d.format("%Y-%m-%d").to_string() == s,
            "Use YYYY-MM-DD dates"
        );
        Ok(d)
    };
    ensure!(parse(start)? <= parse(end)?, "Period end precedes start");
    Ok(())
}
fn common_terms(a: &Vector, b: &Vector) -> Vec<String> {
    let mut terms: Vec<_> = a
        .iter()
        .filter(|(t, _)| !t.starts_with("char:"))
        .filter_map(|(t, v)| b.get(t).map(|w| (t, v * w)))
        .collect();
    terms.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    terms.into_iter().take(8).map(|(t, _)| t.clone()).collect()
}
fn tokens(text: &str) -> Vector {
    let mut out = Vector::new();
    for term in crate::text::feature_terms(text) {
        *out.entry(term).or_default() += 1.;
    }
    out
}
fn normalize(v: &mut Vector) {
    let n = v.values().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0. {
        for x in v.values_mut() {
            *x /= n;
        }
    }
}
pub fn fit(run: &AnalysisRun, reference: String, automatic: bool) -> Model {
    let mut weights = Vector::new();
    for r in &run.processed_incidents {
        for term in tokens(&r.analysis_text).keys() {
            *weights.entry(term.clone()).or_default() += 1.;
        }
    }
    let n = run.processed_incidents.len() as f32;
    for df in weights.values_mut() {
        *df = (1. + n / (1. + *df)).ln().clamp(0.1, 4.);
    }
    Model {
        version: "lexical-v1".into(),
        reference,
        weights,
        plausible: 0.30,
        strong: 0.82,
        margin: 0.12,
        automatic,
        validation: None,
    }
}

/// Operator-supplied validation evidence and calibrated thresholds; never inferred from a score.
pub fn configure_model(model: &mut Model) -> Result<()> {
    let Ok(path) = std::env::var("APP_TRENDS_VALIDATION") else {
        return Ok(());
    };
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Validation {
        report: String,
        approved_by: String,
        plausible: f32,
        strong: f32,
        margin: f32,
    }
    let bytes = std::fs::read(path).context("Cannot read trends validation configuration")?;
    ensure!(
        bytes.len() <= 16384,
        "Trends validation configuration is too large"
    );
    let v: Validation = serde_json::from_slice(&bytes)?;
    ensure!(
        !v.report.trim().is_empty() && !v.approved_by.trim().is_empty(),
        "Validation report and approver are required"
    );
    ensure!(
        v.plausible.is_finite()
            && v.strong.is_finite()
            && v.margin.is_finite()
            && v.plausible > 0.
            && v.plausible < v.strong
            && v.strong <= 1.
            && v.margin > 0.
            && v.margin <= 1.,
        "Invalid calibrated matching thresholds"
    );
    model.plausible = v.plausible;
    model.strong = v.strong;
    model.margin = v.margin;
    model.automatic = true;
    model.validation = Some(format!("{} — approved by {}", v.report, v.approved_by));
    Ok(())
}
pub fn profiles(run: &AnalysisRun, model: &Model) -> Vec<Profile> {
    let rows: BTreeMap<_, _> = run
        .processed_incidents
        .iter()
        .map(|r| (r.source_row_index, r))
        .collect();
    run.clusters
        .iter()
        .filter(|c| c.id.0 != 0)
        .map(|c| {
            let mut vector = Vector::new();
            let mut examples = vec![];
            for row in &c.incident_row_indices {
                if let Some(r) = rows.get(row) {
                    let mut v = tokens(&r.analysis_text);
                    for (term, value) in &mut v {
                        *value = (1. + value.ln()) * model.weights.get(term).copied().unwrap_or(4.);
                    }
                    normalize(&mut v);
                    for (t, x) in &v {
                        *vector.entry(t.clone()).or_default() += x;
                    }
                    if !v.is_empty()
                        && (examples.is_empty() || examples.iter().all(|e| cosine(e, &v) < 0.85))
                        && examples.len() < 8
                    {
                        examples.push(v);
                    }
                }
            }
            // Bound persisted aggregates; selection is deterministic for tied weights.
            let mut ranked: Vec<_> = vector.into_iter().collect();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            ranked.truncate(4096);
            let mut vector = ranked.into_iter().collect();
            normalize(&mut vector);
            Profile {
                cluster: c.id.0,
                label: c.label.clone(),
                rows: c.incident_row_indices.iter().copied().collect(),
                vector,
                examples,
            }
        })
        .collect()
}
pub fn cosine(a: &Vector, b: &Vector) -> f32 {
    let (a, b) = if a.len() > b.len() { (b, a) } else { (a, b) };
    a.iter()
        .map(|(k, v)| v * b.get(k).unwrap_or(&0.))
        .sum::<f32>()
        .clamp(0., 1.)
}
pub fn period(
    analysis: String,
    name: String,
    fingerprint: String,
    start: String,
    end: String,
    total: usize,
    profiles: &[Profile],
) -> Period {
    Period {
        id: id(),
        analysis,
        name,
        fingerprint,
        start,
        end,
        total,
        occurrences: profiles
            .iter()
            .map(|p| Occurrence {
                cluster: p.cluster,
                label: p.label.clone(),
                rows: p.rows.clone(),
                problem: None,
                manual: false,
                reason: String::new(),
                candidates: vec![],
                rejected: BTreeSet::new(),
                replacement: false,
            })
            .collect(),
    }
}
pub fn recalculate(series: &mut Series, profiles: &BTreeMap<String, Vec<Profile>>) -> Result<()> {
    series.periods.sort_by(|a, b| a.start.cmp(&b.start));
    series.validate()?;
    let model = series
        .model
        .as_ref()
        .context("Missing matching model")?
        .clone();
    let mut history: BTreeMap<String, Vec<(String, &Profile)>> = BTreeMap::new();
    let mut trusted: BTreeMap<String, Vec<&Profile>> = BTreeMap::new();
    let mut index_by_term: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for (index, period) in series.periods.iter_mut().enumerate() {
        let ps = profiles
            .get(&period.fingerprint)
            .context("Missing source profiles")?;
        for o in &mut period.occurrences {
            let p = ps
                .iter()
                .find(|p| p.cluster == o.cluster)
                .context("Missing cluster profile")?;
            if o.manual {
                continue;
            }
            o.problem = None;
            o.candidates.clear();
            if p.vector.is_empty() {
                o.reason = "Insufficient text — review required".into();
                continue;
            }
            // Exact inverted retrieval: any nonzero cosine must share a feature.
            // Keep every such problem so pruning cannot hide the runner-up.
            let eligible: BTreeSet<_> = p
                .vector
                .keys()
                .filter_map(|t| index_by_term.get(t.as_str()))
                .flat_map(|ids| ids.iter())
                .collect();
            for problem in eligible {
                let prior = &history[problem];
                if o.rejected.contains(problem) {
                    continue;
                }
                let best = prior
                    .iter()
                    .max_by(|a, b| {
                        cosine(&p.vector, &a.1.vector).total_cmp(&cosine(&p.vector, &b.1.vector))
                    })
                    .context("Empty history")?;
                let score = cosine(&p.vector, &best.1.vector);
                if score >= model.plausible {
                    o.candidates.push(Candidate {
                        problem: problem.clone(),
                        score,
                        reference: trusted
                            .get(problem)
                            .map(|values| {
                                values
                                    .iter()
                                    .map(|v| cosine(&p.vector, &v.vector))
                                    .fold(0., f32::max)
                            })
                            .unwrap_or(0.),
                        period: best.0.clone(),
                        cluster: best.1.cluster,
                        evidence: common_terms(&p.vector, &best.1.vector),
                    });
                }
            }
            o.candidates
                .sort_by(|a, b| b.score.total_cmp(&a.score).then(a.problem.cmp(&b.problem)));
            if o.replacement {
                o.reason = "Replacement source — review required".into();
                continue;
            }
            if o.candidates.is_empty() && o.rejected.is_empty() {
                let origin = format!("{}:{}", period.fingerprint, o.cluster);
                let problem = if let Some(p) = series.problems.iter().find(|p| p.origin == origin) {
                    p.id.clone()
                } else {
                    let p = Problem {
                        id: id(),
                        name: o.label.clone(),
                        origin,
                    };
                    let key = p.id.clone();
                    series.problems.push(p);
                    key
                };
                o.problem = Some(problem);
                o.reason = if index == 0 {
                    "Initial tracked problem"
                } else {
                    "New tracked problem"
                }
                .into();
            } else if let Some(best) = o.candidates.first() {
                let runner = o.candidates.get(1).map(|c| c.score).unwrap_or(0.);
                let mixed = p.examples.iter().any(|example| {
                    history.iter().any(|(key, values)| {
                        key != &best.problem
                            && values
                                .iter()
                                .any(|v| cosine(example, &v.1.vector) >= model.strong)
                    })
                });
                if best.score >= model.strong
                    && best.score - runner >= model.margin
                    && best.reference >= model.strong
                    && !mixed
                    && model.automatic
                {
                    o.problem = Some(best.problem.clone());
                    o.reason = "Automatic text match".into();
                } else {
                    o.reason = if !model.automatic {
                        "Candidate — automatic linking awaits validation"
                    } else {
                        "Ambiguous or mixed evidence — review required"
                    }
                    .into();
                }
            } else {
                o.reason = "Candidates rejected — review required".into();
            }
        }
        // Resolve whole-period competition before admitting any new automatic evidence.
        let mut claims: BTreeMap<String, usize> = BTreeMap::new();
        for o in &period.occurrences {
            let mut keys: BTreeSet<_> = if o.manual {
                BTreeSet::new()
            } else {
                o.candidates.iter().map(|c| c.problem.clone()).collect()
            };
            if let Some(p) = &o.problem {
                keys.insert(p.clone());
            }
            for p in keys {
                *claims.entry(p).or_default() += 1;
            }
        }
        for o in &mut period.occurrences {
            if !o.manual
                && o.candidates
                    .iter()
                    .any(|c| claims.get(&c.problem).copied().unwrap_or(0) > 1)
            {
                o.problem = None;
                o.reason = "Possible split — review required".into();
            }
            if let Some(problem) = &o.problem {
                if series
                    .zeros
                    .iter()
                    .any(|z| z.period == period.id && &z.problem == problem)
                {
                    o.problem = None;
                    o.reason = "Conflicts with confirmed zero — review required".into();
                    continue;
                }
                let profile = ps
                    .iter()
                    .find(|p| p.cluster == o.cluster)
                    .context("Missing profile")?;
                for term in profile.vector.keys() {
                    index_by_term
                        .entry(term)
                        .or_default()
                        .insert(problem.clone());
                }
                let refs = trusted.entry(problem.clone()).or_default();
                if refs.is_empty() || o.manual {
                    refs.push(profile);
                }
                history
                    .entry(problem.clone())
                    .or_default()
                    .push((period.id.clone(), profile));
            }
        }
    }
    series.validate()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    pub period: String,
    pub count: Option<usize>,
    pub percent: Option<f64>,
    pub clusters: Vec<usize>,
    pub zero: Option<Zero>,
}
pub fn cells(series: &Series, problem: &str) -> Vec<Cell> {
    series
        .periods
        .iter()
        .map(|p| {
            let contributors: Vec<_> = p
                .occurrences
                .iter()
                .filter(|o| o.problem.as_deref() == Some(problem))
                .collect();
            let zero = series
                .zeros
                .iter()
                .find(|z| z.period == p.id && z.problem == problem)
                .cloned();
            let count = if !contributors.is_empty() {
                Some(
                    contributors
                        .iter()
                        .flat_map(|o| o.rows.iter())
                        .collect::<BTreeSet<_>>()
                        .len(),
                )
            } else {
                zero.as_ref().map(|_| 0)
            };
            Cell {
                period: p.id.clone(),
                percent: count
                    .filter(|_| p.total > 0)
                    .map(|n| 100. * n as f64 / p.total as f64),
                count,
                clusters: contributors.iter().map(|o| o.cluster).collect(),
                zero,
            }
        })
        .collect()
}
