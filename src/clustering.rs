use crate::labels::summarize_cluster;
use crate::model::{Cluster, ClusterId, IncidentRecord, RowIndex, RunSettings, Subgroup, TermId};
use crate::text::extract_features;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

/// Maximum number of documents a term may appear in before it is excluded
/// from candidate pair generation. Very common terms are poor discriminators
/// and generate O(n^2) pairs within their posting list.
const MAX_TERM_DOC_FREQUENCY: usize = 2_000;

pub fn cluster_incidents(
    records: &[IncidentRecord],
    settings: &RunSettings,
    mut progress: impl FnMut(&'static str, String),
) -> Vec<Cluster> {
    if records.is_empty() {
        return Vec::new();
    }

    progress(
        "Extracting text features",
        format!("Vectorizing {} processed incidents", records.len()),
    );
    let (features, _term_index) = extract_features(records);

    progress(
        "Building similarity graph",
        format!("Finding similar pairs among {} incidents", features.len()),
    );
    let edges = build_edges_from_shared_counts(&features, settings.similarity_threshold_percent);

    progress(
        "Building primary clusters",
        format!("Merging {} similar incident links", edges.len()),
    );
    let mut dsu = DisjointSet::new(records.iter().map(|record| record.source_row_index));
    for (left, right) in edges {
        dsu.union(left, right);
    }

    let mut groups = dsu.groups();
    groups.retain(|group| group.len() >= settings.minimum_cluster_size);
    groups.sort_by_key(|group| std::cmp::Reverse(group.len()));

    progress(
        "Generating summaries and subgroups",
        format!("Creating labels and themes for {} clusters", groups.len()),
    );
    groups
        .into_par_iter()
        .enumerate()
        .map(|(index, mut rows)| {
            rows.sort_unstable();
            let id = ClusterId(index + 1);
            let label = summarize_cluster(id, &rows, records);
            let subgroups = build_subgroups(id, &rows, records, settings);
            Cluster {
                id,
                label,
                incident_row_indices: rows,
                subgroups,
            }
        })
        .collect()
}

pub fn unclustered_rows(records: &[IncidentRecord], clusters: &[Cluster]) -> Vec<RowIndex> {
    let clustered = clusters
        .iter()
        .flat_map(|cluster| cluster.incident_row_indices.iter().copied())
        .collect::<HashSet<_>>();

    records
        .par_iter()
        .map(|record| record.source_row_index)
        .filter(|row_index| !clustered.contains(row_index))
        .collect()
}

/// Builds similarity edges by accumulating shared-term counts via a parallel
/// inverted index traversal, then computing Jaccard from the counts.
///
/// This avoids materializing all candidate pairs into a HashSet and eliminates
/// the separate per-pair set intersection/union pass.
fn build_edges_from_shared_counts(
    features: &[crate::model::TextFeatures],
    threshold_percent: u8,
) -> Vec<(RowIndex, RowIndex)> {
    // Build inverted index: term_id -> list of row indices
    let mut inverted: HashMap<TermId, Vec<RowIndex>> = HashMap::new();
    for f in features {
        for &term_id in f.terms.keys() {
            inverted.entry(term_id).or_default().push(f.row_index);
        }
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

    // Parallel traversal: each posting list accumulates shared counts into a
    // thread-local map, then we merge and filter by Jaccard threshold.
    let shared_counts: HashMap<(RowIndex, RowIndex), u32> = posting_lists
        .par_iter()
        .fold(
            HashMap::new,
            |mut acc: HashMap<(RowIndex, RowIndex), u32>, rows| {
                for (i, &left) in rows.iter().enumerate() {
                    for &right in &rows[i + 1..] {
                        let pair = if left < right {
                            (left, right)
                        } else {
                            (right, left)
                        };
                        *acc.entry(pair).or_default() += 1;
                    }
                }
                acc
            },
        )
        .reduce(HashMap::new, |mut a, b| {
            for (pair, count) in b {
                *a.entry(pair).or_default() += count;
            }
            a
        });

    // Filter pairs by Jaccard threshold computed from shared counts and
    // filtered term set sizes (both use the same subset of terms).
    shared_counts
        .into_par_iter()
        .filter(|&((left, right), shared)| {
            let left_size = term_set_sizes.get(&left).copied().unwrap_or(0);
            let right_size = term_set_sizes.get(&right).copied().unwrap_or(0);
            let union = left_size + right_size - shared as usize;
            if union == 0 {
                return false;
            }
            let jaccard = (shared as usize * 100) / union;
            jaccard >= threshold_percent as usize
        })
        .map(|(pair, _)| pair)
        .collect()
}

fn build_subgroups(
    cluster_id: ClusterId,
    rows: &[RowIndex],
    records: &[IncidentRecord],
    settings: &RunSettings,
) -> Vec<Subgroup> {
    if rows.len() <= settings.minimum_cluster_size {
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
    let (features, _term_index) = extract_features(&cluster_records);
    let edges =
        build_edges_from_shared_counts(&features, settings.subgroup_similarity_threshold_percent);

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

        let clusters = cluster_incidents(&records, &settings, |_, _| {});

        assert_eq!(clusters.len(), 2);
        assert!(clusters.iter().all(|cluster| cluster.size() == 3));
    }
}
