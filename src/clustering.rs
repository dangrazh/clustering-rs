use crate::labels::summarize_cluster;
use crate::model::{Cluster, ClusterId, IncidentRecord, RowIndex, RunSettings, Subgroup, TermId};
use crate::progress::{ParallelProgressSpec, ProgressReporter};
use crate::text::extract_features;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

/// Maximum number of documents a term may appear in before it is excluded
/// from candidate pair generation. Very common terms are poor discriminators
/// and generate O(n^2) pairs within their posting list.
const MAX_TERM_DOC_FREQUENCY: usize = 2_000;

/// Very large primary clusters are already useful as a top-level result, but
/// recursively re-clustering them for subgroups can recreate the most expensive
/// parts of the pipeline. Cap that work so low-memory systems stay responsive.
const MAX_SUBGROUP_RECLUSTER_SIZE: usize = 5_000;

pub fn cluster_incidents(
    records: &[IncidentRecord],
    settings: &RunSettings,
    reporter: Option<&ProgressReporter>,
) -> Vec<Cluster> {
    if records.is_empty() {
        return Vec::new();
    }

    let (features, _term_index) = extract_features(records, reporter);

    let edges =
        build_edges_from_shared_counts(&features, settings.similarity_threshold_percent, reporter);

    if let Some(reporter) = reporter {
        reporter.substep(
            5,
            8,
            "Building primary clusters",
            1,
            4,
            "Initializing cluster union-find",
            format!("{} incidents ready for graph merging", records.len()),
        );
    }
    let mut dsu = DisjointSet::new(records.iter().map(|record| record.source_row_index));

    if let Some(reporter) = reporter {
        reporter.substep(
            5,
            8,
            "Building primary clusters",
            2,
            4,
            "Merging similarity edges",
            format!("Applying {} similar incident links", edges.len()),
        );
    }
    for (left, right) in edges {
        dsu.union(left, right);
    }

    let mut groups = dsu.groups();
    if let Some(reporter) = reporter {
        reporter.substep(
            5,
            8,
            "Building primary clusters",
            3,
            4,
            "Filtering undersized groups",
            format!(
                "{} raw graph components before minimum cluster size filter",
                groups.len()
            ),
        );
    }
    groups.retain(|group| group.len() >= settings.minimum_cluster_size);
    groups.sort_by_key(|group| std::cmp::Reverse(group.len()));

    if let Some(reporter) = reporter {
        reporter.substep(
            5,
            8,
            "Building primary clusters",
            4,
            4,
            "Sorting retained clusters",
            format!("{} clusters meet the minimum size", groups.len()),
        );
    }

    if let Some(reporter) = reporter {
        reporter.substep(
            6,
            8,
            "Generating summaries and subgroups",
            1,
            3,
            "Preparing cluster labeling",
            format!("Creating labels and themes for {} clusters", groups.len()),
        );
    }

    let cluster_tracker = reporter.map(|reporter| {
        reporter.parallel_substep(ParallelProgressSpec {
            step: 6,
            total_steps: 8,
            stage: "Generating summaries and subgroups".to_owned(),
            substep_current: 2,
            substep_total: 3,
            substep_label: "Building cluster summaries".to_owned(),
            detail: "Generating labels and subgroup themes".to_owned(),
            total_units: groups.len(),
            unit_label: "clusters summarized".to_owned(),
        })
    });

    let clusters = groups
        .into_par_iter()
        .enumerate()
        .map(|(index, mut rows)| {
            rows.sort_unstable();
            let id = ClusterId(index + 1);
            let label = summarize_cluster(id, &rows, records);
            let subgroups = build_subgroups(id, &rows, records, settings);
            if let Some(tracker) = &cluster_tracker {
                tracker.advance(1);
            }
            Cluster {
                id,
                label,
                incident_row_indices: rows,
                subgroups,
            }
        })
        .collect::<Vec<_>>();

    if let Some(tracker) = &cluster_tracker {
        tracker.finish();
    }
    if let Some(reporter) = reporter {
        reporter.substep(
            6,
            8,
            "Generating summaries and subgroups",
            3,
            3,
            "Finalizing cluster presentation",
            "Ordering cluster summaries for result display",
        );
    }

    clusters
}

pub fn unclustered_rows(
    records: &[IncidentRecord],
    clusters: &[Cluster],
    reporter: Option<&ProgressReporter>,
) -> Vec<RowIndex> {
    if let Some(reporter) = reporter {
        reporter.substep(
            7,
            8,
            "Assigning unclustered incidents",
            1,
            2,
            "Collecting clustered row ids",
            format!("{} clusters selected for final output", clusters.len()),
        );
    }
    let clustered = clusters
        .iter()
        .flat_map(|cluster| cluster.incident_row_indices.iter().copied())
        .collect::<HashSet<_>>();

    let tracker = reporter.map(|reporter| {
        reporter.parallel_substep(ParallelProgressSpec {
            step: 7,
            total_steps: 8,
            stage: "Assigning unclustered incidents".to_owned(),
            substep_current: 2,
            substep_total: 2,
            substep_label: "Scanning processed incidents".to_owned(),
            detail: "Collecting rows that did not meet cluster thresholds".to_owned(),
            total_units: records.len(),
            unit_label: "incidents scanned".to_owned(),
        })
    });

    let rows = records
        .par_iter()
        .filter_map(|record| {
            if let Some(tracker) = &tracker {
                tracker.advance(1);
            }
            (!clustered.contains(&record.source_row_index)).then_some(record.source_row_index)
        })
        .collect();

    if let Some(tracker) = &tracker {
        tracker.finish();
    }

    rows
}

/// Builds similarity edges by accumulating shared-term counts via a parallel
/// inverted index traversal, then computing Jaccard from the counts.
///
/// This avoids materializing all candidate pairs into a HashSet and eliminates
/// the separate per-pair set intersection/union pass.
fn build_edges_from_shared_counts(
    features: &[crate::model::TextFeatures],
    threshold_percent: u8,
    reporter: Option<&ProgressReporter>,
) -> Vec<(RowIndex, RowIndex)> {
    if let Some(reporter) = reporter {
        reporter.substep(
            4,
            8,
            "Building similarity graph",
            1,
            5,
            "Building inverted index",
            format!("Indexing terms from {} feature vectors", features.len()),
        );
    }

    // Build inverted index: term_id -> list of row indices
    let mut inverted: HashMap<TermId, Vec<RowIndex>> = HashMap::new();
    for f in features {
        for &term_id in f.terms.keys() {
            inverted.entry(term_id).or_default().push(f.row_index);
        }
    }

    if let Some(reporter) = reporter {
        reporter.substep(
            4,
            8,
            "Building similarity graph",
            2,
            5,
            "Filtering noisy terms",
            format!("Evaluating {} posting lists", inverted.len()),
        );
    }

    // Identify which terms pass the document-frequency filter.
    // Compute per-document term set sizes counting ONLY these terms so that
    // the Jaccard denominator is consistent with the shared-count numerator.
    let valid_terms: HashSet<TermId> = inverted
        .iter()
        .filter(|(_, rows)| (2..=MAX_TERM_DOC_FREQUENCY).contains(&rows.len()))
        .map(|(&term_id, _)| term_id)
        .collect();

    let term_set_sizes: HashMap<RowIndex, usize> = features
        .par_iter()
        .map(|f| {
            let count = f.terms.keys().filter(|id| valid_terms.contains(id)).count();
            (f.row_index, count)
        })
        .collect();

    // Collect only the filtered posting lists
    let posting_lists: Vec<&Vec<RowIndex>> = inverted
        .values()
        .filter(|rows| (2..=MAX_TERM_DOC_FREQUENCY).contains(&rows.len()))
        .collect();

    let candidate_tracker = reporter.map(|reporter| {
        reporter.parallel_substep(ParallelProgressSpec {
            step: 4,
            total_steps: 8,
            stage: "Building similarity graph".to_owned(),
            substep_current: 3,
            substep_total: 5,
            substep_label: "Scanning feature vectors".to_owned(),
            detail: "Building row-local candidate pairs from shared terms".to_owned(),
            total_units: features.len(),
            unit_label: "feature vectors scanned".to_owned(),
        })
    });

    // Evaluate candidate pairs one source row at a time. This avoids holding a
    // global shared-count map for all candidate pairs in memory at once.
    let edges = features
        .par_iter()
        .map(|feature| {
            let left = feature.row_index;
            let left_size = term_set_sizes.get(&left).copied().unwrap_or(0);
            let mut shared_counts = HashMap::<RowIndex, u32>::new();

            for term_id in feature.terms.keys().filter(|id| valid_terms.contains(id)) {
                if let Some(rows) = inverted.get(term_id) {
                    for &right in rows {
                        if right > left {
                            *shared_counts.entry(right).or_default() += 1;
                        }
                    }
                }
            }

            if let Some(tracker) = &candidate_tracker {
                tracker.advance(1);
            }

            shared_counts
                .into_iter()
                .filter_map(|(right, shared)| {
                    let right_size = term_set_sizes.get(&right).copied().unwrap_or(0);
                    let union = left_size + right_size - shared as usize;
                    if union == 0 {
                        return None;
                    }
                    let jaccard = (shared as usize * 100) / union;
                    (jaccard >= threshold_percent as usize).then_some((left, right))
                })
                .collect::<Vec<_>>()
        })
        .reduce(Vec::new, |mut left_edges, mut right_edges| {
            left_edges.append(&mut right_edges);
            left_edges
        });

    if let Some(tracker) = &candidate_tracker {
        tracker.finish();
    }

    if let Some(reporter) = reporter {
        reporter.substep(
            4,
            8,
            "Building similarity graph",
            4,
            5,
            "Applying similarity threshold",
            format!("{} similarity edges retained after row-local scoring", edges.len()),
        );
    }

    if let Some(reporter) = reporter {
        reporter.substep(
            4,
            8,
            "Building similarity graph",
            5,
            5,
            "Finalizing similarity edges",
            format!("{} filtered posting lists contributed to graph construction", posting_lists.len()),
        );
    }

    edges
}

fn build_subgroups(
    cluster_id: ClusterId,
    rows: &[RowIndex],
    records: &[IncidentRecord],
    settings: &RunSettings,
) -> Vec<Subgroup> {
    if rows.len() <= settings.minimum_cluster_size || rows.len() > MAX_SUBGROUP_RECLUSTER_SIZE {
        return vec![Subgroup {
            id: 1,
            label: summarize_cluster(cluster_id, rows, records),
            incident_row_indices: rows.to_vec(),
        }];
    }

    let row_set = rows.iter().copied().collect::<HashSet<_>>();
    let cluster_records = records
        .iter()
        .filter(|record| row_set.contains(&record.source_row_index))
        .cloned()
        .collect::<Vec<_>>();
    let (features, _term_index) = extract_features(&cluster_records, None);
    let edges = build_edges_from_shared_counts(
        &features,
        settings.subgroup_similarity_threshold_percent,
        None,
    );

    let mut dsu = DisjointSet::new(rows.iter().copied());
    for (left, right) in edges {
        dsu.union(left, right);
    }

    let mut groups = dsu.groups();
    groups.sort_by_key(|group| std::cmp::Reverse(group.len()));
    groups.truncate(10);

    groups
        .into_iter()
        .enumerate()
        .map(|(index, mut rows)| {
            rows.sort_unstable();
            Subgroup {
                id: index + 1,
                label: summarize_cluster(cluster_id, &rows, records),
                incident_row_indices: rows,
            }
        })
        .collect()
}

#[derive(Debug)]
struct DisjointSet {
    parent: HashMap<RowIndex, RowIndex>,
}

impl DisjointSet {
    fn new(rows: impl IntoIterator<Item = RowIndex>) -> Self {
        let parent = rows.into_iter().map(|row| (row, row)).collect();
        Self { parent }
    }

    fn find(&mut self, row: RowIndex) -> RowIndex {
        let parent = *self.parent.get(&row).unwrap_or(&row);
        if parent == row {
            row
        } else {
            let root = self.find(parent);
            self.parent.insert(row, root);
            root
        }
    }

    fn union(&mut self, left: RowIndex, right: RowIndex) {
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root != right_root {
            self.parent.insert(right_root, left_root);
        }
    }

    fn groups(mut self) -> Vec<Vec<RowIndex>> {
        let rows = self.parent.keys().copied().collect::<Vec<_>>();
        let mut groups = HashMap::<RowIndex, Vec<RowIndex>>::new();
        for row in rows {
            let root = self.find(row);
            groups.entry(root).or_default().push(row);
        }
        groups.into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FilterValues;

    #[test]
    fn clusters_similar_records_without_target_count() {
        let records = (0..6)
            .map(|index| IncidentRecord {
                source_row_index: index,
                incident_number: format!("INC{index:03}"),
                analysis_text: if index < 3 {
                    "password reset failure sap portal".to_owned()
                } else {
                    "vpn connection timeout remote access".to_owned()
                },
                filter_values: FilterValues::default(),
                parsed_date: None,
            })
            .collect::<Vec<_>>();
        let settings = RunSettings {
            minimum_cluster_size: 3,
            ..Default::default()
        };

        let clusters = cluster_incidents(&records, &settings, None);

        assert_eq!(clusters.len(), 2);
        assert!(clusters.iter().all(|cluster| cluster.size() == 3));
    }
}
