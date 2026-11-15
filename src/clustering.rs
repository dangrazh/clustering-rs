use crate::labels::{GroupProfile, Labeler};
use crate::model::{Cluster, ClusterId, IncidentRecord, RowIndex, RunSettings, Subgroup, TextFeatures};
use crate::progress::{ParallelProgressSpec, ProgressReporter};
use crate::text::{extract_features, TermIndex};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Themes shown per cluster, including the one that collects the remaining incidents.
const MAX_THEMES: usize = 10;

/// A theme has at least a tenth of the minimum cluster size, and never fewer than this.
const MIN_THEME_SIZE: usize = 3;

/// Share of a group's incidents that must match its core for it to count as one problem.
const COHESIVE_SHARE: f32 = 0.9;

/// An incident matches a group's core at this fraction of the similarity threshold. The
/// core holds only what most of the group shares, while each incident also carries text of
/// its own, so an incident resembles the core less than it resembles its nearest neighbours.
const CORE_MATCH: f32 = 0.5;

/// Percentage points added to the threshold each time a mixed group is split.
const SPLIT_STEP: u8 = 10;

/// Incidents sampled to find what a large group has in common.
const CORE_SAMPLE: usize = 2_000;

/// Posting lists at least this long are checked for having become a single group.
const FINISHED_CHECK_MIN: usize = 32;

/// Keeps rounding in floating-point sums from ruling out a pair exactly at the threshold.
const ROUNDING_MARGIN: f32 = 0.999;

pub fn cluster_incidents(
    records: &[IncidentRecord],
    settings: &RunSettings,
    labeler: &Labeler,
    reporter: Option<&ProgressReporter>,
) -> Vec<Cluster> {
    if records.is_empty() {
        return Vec::new();
    }

    let (features, term_index) = extract_features(records, reporter);
    let index = SimilarityIndex::new(&features, &term_index, settings.minimum_cluster_size, reporter);
    drop(features);
    drop(term_index);

    let everything = (0..index.documents.len() as u32).collect::<Vec<_>>();
    let groups = index.groups(&everything, settings.similarity_threshold_percent, reporter);
    if let Some(reporter) = reporter {
        reporter.substep(
            5,
            8,
            "Building primary clusters",
            3,
            4,
            "Checking cluster cohesion",
            format!("Splitting mixed groups among {} groups of similar incidents", groups.len()),
        );
    }
    let groups = index.cohesive_clusters(groups, settings.similarity_threshold_percent, settings.minimum_cluster_size);

    if let Some(reporter) = reporter {
        reporter.substep(
            5,
            8,
            "Building primary clusters",
            4,
            4,
            "Keeping clusters of the minimum size",
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
        .map(|(position, members)| {
            let rows = index.rows_of(&members);
            let id = ClusterId(position + 1);
            let profile = labeler.profile(&rows);
            let subgroups = build_subgroups(id, &members, &rows, &index, settings, labeler, &profile);
            let themes = subgroups.iter().map(|s| s.incident_row_indices.as_slice()).collect::<Vec<_>>();
            let label = labeler.cluster_label(id, &rows, &profile, &themes);
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

/// One incident's terms, as positions in the corpus-wide rarity order.
struct Document {
    /// Ascending, so the rarest (heaviest) terms come first.
    terms: Vec<u32>,
    /// Total weight of `terms`.
    weight: f32,
}

/// The weighted terms of every incident, prepared once per analysis. Clusters and the
/// themes inside them are found with the same measure, at different thresholds.
///
/// Two incidents are compared by weighted Jaccard similarity: the weight of the terms they
/// share divided by the weight of all terms either one has. A term weighs
/// `ln(1 + N / frequency)²`, the weight a shared term carries in TF-IDF cosine similarity.
/// A term found in a tenth of all incidents (a system name, boilerplate) therefore counts
/// for little next to one found in a few hundred (the actual problem), and no term has to
/// be dropped for being frequent. Frequencies below the rarity floor are treated as equal
/// to it, so an identifier shared by a handful of incidents cannot outweigh the terms that
/// describe a whole group. Terms found in a single incident cannot link anything and are
/// ignored.
///
/// Terms containing a digit (host names, job and ticket numbers, versions) are identifiers:
/// they vary between incidents of the same problem, so they weigh no more than a term found
/// in every incident.
struct SimilarityIndex {
    /// Source row of each document.
    rows: Vec<RowIndex>,
    documents: Vec<Document>,
    /// Weight of each term, by position in the rarity order; never increasing.
    weights: Vec<f32>,
}

impl SimilarityIndex {
    fn new(
        features: &[TextFeatures],
        term_index: &TermIndex,
        rarity_floor: usize,
        reporter: Option<&ProgressReporter>,
    ) -> Self {
        assert!(features.len() < u32::MAX as usize, "too many incidents to cluster");
        if let Some(reporter) = reporter {
            reporter.substep(
                4,
                8,
                SIMILARITY_STAGE,
                1,
                5,
                "Counting term frequencies",
                format!("Reading terms of {} feature vectors", features.len()),
            );
        }
        let term_count = features
            .iter()
            .filter_map(|feature| feature.terms.keys().next_back())
            .max()
            .map_or(0, |id| *id as usize + 1);
        let mut frequency = vec![0u32; term_count];
        for id in features.iter().flat_map(|feature| feature.terms.keys()) {
            frequency[*id as usize] += 1;
        }

        let mut identifier = vec![false; term_count];
        for (term, id) in term_index.terms() {
            if let Some(flag) = identifier.get_mut(id as usize) {
                *flag = term.chars().any(|c| c.is_ascii_digit());
            }
        }
        // Heaviest first: rare words, then common words, then identifiers. The position in
        // this order identifies a term from here on.
        let mut by_rarity = (0..term_count as u32)
            .filter(|id| frequency[*id as usize] >= 2)
            .collect::<Vec<_>>();
        by_rarity.sort_unstable_by_key(|id| (identifier[*id as usize], frequency[*id as usize], *id));
        if let Some(reporter) = reporter {
            reporter.substep(
                4,
                8,
                SIMILARITY_STAGE,
                2,
                5,
                "Weighting terms",
                format!("{} terms occur in more than one incident", by_rarity.len()),
            );
        }
        let mut position = vec![u32::MAX; term_count];
        for (index, id) in by_rarity.iter().enumerate() {
            position[*id as usize] = index as u32;
        }
        let incident_count = features.len() as f32;
        let floor = rarity_floor.max(2) as f32;
        let weights = by_rarity
            .iter()
            .map(|id| {
                let frequency = if identifier[*id as usize] {
                    incident_count
                } else {
                    (frequency[*id as usize] as f32).max(floor)
                };
                (incident_count / frequency).ln_1p().powi(2)
            })
            .collect::<Vec<_>>();

        let documents = features
            .par_iter()
            .map(|feature| {
                let mut terms = feature
                    .terms
                    .keys()
                    .map(|id| position[*id as usize])
                    .filter(|term| *term != u32::MAX)
                    .collect::<Vec<_>>();
                terms.sort_unstable();
                let weight = terms.iter().map(|term| weights[*term as usize]).sum();
                Document { terms, weight }
            })
            .collect();
        Self {
            rows: features.iter().map(|feature| feature.row_index).collect(),
            documents,
            weights,
        }
    }

    /// Source rows of the given documents, ascending.
    fn rows_of(&self, members: &[u32]) -> Vec<RowIndex> {
        let mut rows = members.iter().map(|member| self.rows[*member as usize]).collect::<Vec<_>>();
        rows.sort_unstable();
        rows
    }

    /// Splits `members` into groups whose similarity reaches the threshold, directly or
    /// through a chain of similar incidents. Every member is in exactly one group.
    ///
    /// The order is reproducible: largest group first, equal sizes by their smallest source
    /// row. The same input always yields the same cluster and theme identifiers, whatever
    /// order the worker threads merged in.
    ///
    /// Candidate pairs come from an index of each incident's rarest terms only. A pair at
    /// the threshold must share one of them, so the search is exact, while common terms
    /// never produce long candidate lists. Similar pairs are merged straight into a
    /// concurrent union-find and pairs already in one group are skipped, so thousands of
    /// near-identical incidents cost little and no list of pairs is kept in memory.
    fn groups(&self, members: &[u32], threshold_percent: u8, reporter: Option<&ProgressReporter>) -> Vec<Vec<u32>> {
        let threshold = fraction(threshold_percent);
        let document = |local: u32| &self.documents[members[local as usize] as usize];

        if let Some(reporter) = reporter {
            reporter.substep(
                4,
                8,
                SIMILARITY_STAGE,
                3,
                5,
                "Indexing distinctive terms",
                format!("Indexing the rarest terms of {} incidents", members.len()),
            );
        }
        // A pair at the threshold shares at least `threshold * weight` of each incident. The
        // most common terms adding up to less than that cannot hold the whole overlap, so a
        // similar incident must also share one of the remaining, rarer terms: those are indexed.
        let indexed = members
            .par_iter()
            .map(|member| {
                let document = &self.documents[*member as usize];
                let skippable = threshold * document.weight * ROUNDING_MARGIN;
                let mut skipped = 0.0;
                let mut count = document.terms.len();
                while count > 0 && skipped + self.weights[document.terms[count - 1] as usize] < skippable {
                    skipped += self.weights[document.terms[count - 1] as usize];
                    count -= 1;
                }
                count
            })
            .collect::<Vec<_>>();
        // Posting lists as one sorted array of (term, member); members ascend within a term.
        let mut postings = (0..members.len() as u32)
            .into_par_iter()
            .flat_map_iter(|local| {
                document(local).terms[..indexed[local as usize]].iter().map(move |term| (*term, local))
            })
            .collect::<Vec<_>>();
        postings.par_sort_unstable();
        let mut list_terms = Vec::new();
        let mut list_starts = Vec::new();
        for (index, (term, _)) in postings.iter().enumerate() {
            if list_terms.last() != Some(term) {
                list_terms.push(*term);
                list_starts.push(index);
            }
        }
        list_starts.push(postings.len());
        let list_of = |term: u32| list_terms.binary_search(&term).expect("indexed term has a posting list");
        let posting_list = |list: usize| &postings[list_starts[list]..list_starts[list + 1]];

        let tracker = reporter.map(|reporter| {
            reporter.parallel_substep(ParallelProgressSpec {
                step: 4,
                total_steps: 8,
                stage: SIMILARITY_STAGE.to_owned(),
                substep_current: 4,
                substep_total: 5,
                substep_label: "Comparing incidents".to_owned(),
                detail: "Scoring incidents that share distinctive terms".to_owned(),
                total_units: members.len(),
                unit_label: "incidents compared".to_owned(),
            })
        });
        let components = Components::new(members.len());
        // Set once every incident of a posting list is in one group: nothing is left to find there.
        let finished = (0..list_terms.len()).map(|_| AtomicBool::new(false)).collect::<Vec<_>>();
        (0..members.len() as u32).into_par_iter().for_each_init(
            || (Vec::<usize>::new(), Vec::<u32>::new()),
            |(lists, candidates), left| {
                let own = document(left);
                lists.clear();
                lists.extend(own.terms[..indexed[left as usize]].iter().map(|term| list_of(*term)));
                candidates.clear();
                for list in lists.iter().filter(|list| !finished[**list].load(Ordering::Relaxed)) {
                    let list = posting_list(*list);
                    let later = &list[list.partition_point(|(_, other)| *other <= left)..];
                    candidates.extend(later.iter().map(|(_, other)| *other).filter(|other| {
                        // Total weights too far apart cannot reach the threshold.
                        let weight = document(*other).weight;
                        weight >= threshold * own.weight * ROUNDING_MARGIN
                            && own.weight >= threshold * weight * ROUNDING_MARGIN
                            && !components.same(left, *other)
                    }));
                }
                candidates.sort_unstable();
                candidates.dedup();
                for other in candidates.iter() {
                    if !components.same(left, *other) && self.similar(own, document(*other), threshold) {
                        components.union(left, *other);
                    }
                }
                for list in lists.iter() {
                    let entries = posting_list(*list);
                    if entries.len() >= FINISHED_CHECK_MIN
                        && entries[0].1 == left
                        && entries.iter().all(|(_, other)| components.same(left, *other))
                    {
                        finished[*list].store(true, Ordering::Relaxed);
                    }
                }
                if let Some(tracker) = &tracker {
                    tracker.advance(1);
                }
            },
        );
        if let Some(tracker) = &tracker {
            tracker.finish();
        }
        if let Some(reporter) = reporter {
            reporter.substep(
                4,
                8,
                SIMILARITY_STAGE,
                5,
                5,
                "Finalizing similarity graph",
                format!("{} distinctive term lists searched", list_terms.len()),
            );
            reporter.substep(
                5,
                8,
                "Building primary clusters",
                1,
                4,
                "Collecting connected incidents",
                format!("{} incidents ready for grouping", members.len()),
            );
        }

        let mut group_of_root = vec![usize::MAX; members.len()];
        let mut groups = Vec::<Vec<u32>>::new();
        for (local, member) in members.iter().enumerate() {
            let root = components.find(local as u32) as usize;
            if group_of_root[root] == usize::MAX {
                group_of_root[root] = groups.len();
                groups.push(Vec::new());
            }
            groups[group_of_root[root]].push(*member);
        }
        self.order(&mut groups);
        if let Some(reporter) = reporter {
            reporter.substep(
                5,
                8,
                "Building primary clusters",
                2,
                4,
                "Ordering groups",
                format!("{} groups of similar incidents", groups.len()),
            );
        }
        groups
    }

    /// Largest group first, equal sizes by their smallest source row; rows ascending within a group.
    fn order(&self, groups: &mut [Vec<u32>]) {
        for group in groups.iter_mut() {
            group.sort_unstable_by_key(|member| self.rows[*member as usize]);
        }
        groups.sort_unstable_by(|a, b| {
            b.len().cmp(&a.len()).then_with(|| self.rows[a[0] as usize].cmp(&self.rows[b[0] as usize]))
        });
    }

    /// Turns chains of similar incidents into clusters that each describe one problem.
    ///
    /// Chaining can join unrelated problems through a few in-between incidents. A group is
    /// kept only if nearly all its incidents match its core, the terms most of them share
    /// (see [`CORE_MATCH`]).
    /// A mixed group is split again at a tighter threshold until its parts are cohesive.
    /// Incidents that end up outside any cluster of the minimum size join the cluster whose
    /// core they match best, if they match one at the threshold. Returns only clusters of
    /// the minimum size, in the reproducible order of [`Self::groups`].
    fn cohesive_clusters(&self, groups: Vec<Vec<u32>>, threshold_percent: u8, minimum_size: usize) -> Vec<Vec<u32>> {
        let threshold = fraction(threshold_percent);
        let (mut clusters, rest): (Vec<_>, Vec<_>) = groups
            .into_par_iter()
            .flat_map_iter(|group| self.split_until_cohesive(group, threshold_percent, threshold, minimum_size))
            .partition(|group| group.len() >= minimum_size);

        let cores = clusters.par_iter().map(|cluster| self.core(cluster)).collect::<Vec<_>>();
        let joining = rest
            .into_par_iter()
            .flatten_iter()
            .filter_map(|member| Some((member, self.best_match(member, &cores, threshold)?)))
            .collect::<Vec<_>>();
        for (member, cluster) in joining {
            clusters[cluster].push(member);
        }
        self.order(&mut clusters);
        clusters
    }

    fn split_until_cohesive(
        &self,
        group: Vec<u32>,
        split_percent: u8,
        threshold: f32,
        minimum_size: usize,
    ) -> Vec<Vec<u32>> {
        if group.len() < minimum_size || split_percent >= 100 || self.is_cohesive(&group, threshold) {
            return vec![group];
        }
        let tighter = split_percent.saturating_add(SPLIT_STEP).min(100);
        self.groups(&group, tighter, None)
            .into_iter()
            .flat_map(|part| self.split_until_cohesive(part, tighter, threshold, minimum_size))
            .collect()
    }

    fn is_cohesive(&self, members: &[u32], threshold: f32) -> bool {
        let core = self.core(members);
        let matching = members
            .par_iter()
            .filter(|member| self.similar(&self.documents[**member as usize], &core, threshold * CORE_MATCH))
            .count();
        matching as f32 >= COHESIVE_SHARE * members.len() as f32
    }

    /// What a group has in common: the terms found in at least two thirds of its incidents,
    /// so two problems of similar size joined in one group have no core.
    /// Large groups are sampled at even intervals.
    fn core(&self, members: &[u32]) -> Document {
        let sample = members.iter().step_by(members.len().div_ceil(CORE_SAMPLE).max(1));
        let size = sample.len();
        let mut counts = HashMap::<u32, usize>::new();
        for term in sample.flat_map(|member| &self.documents[*member as usize].terms) {
            *counts.entry(*term).or_default() += 1;
        }
        let mut terms = counts
            .into_iter()
            .filter(|(_, count)| count * 3 >= size * 2)
            .map(|(term, _)| term)
            .collect::<Vec<_>>();
        terms.sort_unstable();
        let weight = terms.iter().map(|term| self.weights[*term as usize]).sum();
        Document { terms, weight }
    }

    /// The core an incident matches best among those it matches at the threshold; the
    /// earlier one on a tie.
    fn best_match(&self, member: u32, cores: &[Document], threshold: f32) -> Option<usize> {
        let document = &self.documents[member as usize];
        cores
            .iter()
            .enumerate()
            .filter(|(_, core)| self.similar(document, core, threshold))
            .map(|(position, core)| (self.similarity(document, core), position))
            .max_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.1.cmp(&a.1)))
            .map(|(_, position)| position)
    }

    /// The core an incident resembles most, whether or not it matches; the earlier one on a tie.
    fn closest(&self, member: u32, cores: &[Document]) -> usize {
        let document = &self.documents[member as usize];
        cores
            .iter()
            .enumerate()
            .map(|(position, core)| (self.similarity(document, core), position))
            .max_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.1.cmp(&a.1)))
            .map_or(0, |(_, position)| position)
    }

    /// Weighted Jaccard similarity of two term sets.
    fn similarity(&self, left: &Document, right: &Document) -> f32 {
        let (mut i, mut j) = (0, 0);
        let mut shared = 0.0;
        while i < left.terms.len() && j < right.terms.len() {
            match left.terms[i].cmp(&right.terms[j]) {
                std::cmp::Ordering::Equal => {
                    shared += self.weights[left.terms[i] as usize];
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
            }
        }
        let either = left.weight + right.weight - shared;
        if either > 0.0 { shared / either } else { 0.0 }
    }

    /// Whether the weighted Jaccard similarity of two incidents reaches the threshold.
    fn similar(&self, left: &Document, right: &Document, threshold: f32) -> bool {
        // shared / (left + right - shared) >= threshold, solved for the shared weight.
        let needed = threshold * (left.weight + right.weight) / (1.0 + threshold);
        let (mut i, mut j) = (0, 0);
        let mut shared = 0.0;
        let (mut left_rest, mut right_rest) = (left.weight, right.weight);
        while i < left.terms.len() && j < right.terms.len() {
            let (a, b) = (left.terms[i], right.terms[j]);
            match a.cmp(&b) {
                std::cmp::Ordering::Equal => {
                    let weight = self.weights[a as usize];
                    shared += weight;
                    if shared >= needed {
                        return true;
                    }
                    left_rest -= weight;
                    right_rest -= weight;
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => {
                    left_rest -= self.weights[a as usize];
                    i += 1;
                }
                std::cmp::Ordering::Greater => {
                    right_rest -= self.weights[b as usize];
                    j += 1;
                }
            }
            // Heavy terms come first, so dissimilar pairs are given up after a few terms.
            if shared + left_rest.min(right_rest) < needed * ROUNDING_MARGIN {
                return false;
            }
        }
        shared >= needed
    }
}

const SIMILARITY_STAGE: &str = "Building similarity graph";

fn fraction(percent: u8) -> f32 {
    f32::from(percent.clamp(1, 100)) / 100.0
}

/// Union-find that worker threads update concurrently without locks. A root only ever
/// links to a root with a smaller index, so links cannot form a cycle.
struct Components {
    parent: Vec<AtomicU32>,
}

impl Components {
    fn new(count: usize) -> Self {
        Self { parent: (0..count as u32).map(AtomicU32::new).collect() }
    }

    fn find(&self, mut index: u32) -> u32 {
        loop {
            let parent = self.parent[index as usize].load(Ordering::Acquire);
            if parent == index {
                return index;
            }
            let grandparent = self.parent[parent as usize].load(Ordering::Acquire);
            if grandparent != parent {
                // Shorten the path; losing the race to another thread is harmless.
                let _ = self.parent[index as usize].compare_exchange_weak(
                    parent,
                    grandparent,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                );
            }
            index = parent;
        }
    }

    /// True only if both are in one group. While other threads are merging, a pair that is
    /// just being joined may still be reported as separate; it is then simply compared again.
    fn same(&self, left: u32, right: u32) -> bool {
        self.find(left) == self.find(right)
    }

    fn union(&self, left: u32, right: u32) {
        loop {
            let (left, right) = (self.find(left), self.find(right));
            if left == right {
                return;
            }
            let (high, low) = if left > right { (left, right) } else { (right, left) };
            if self.parent[high as usize]
                .compare_exchange(high, low, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }
}

/// Themes are the tighter groups inside a cluster: the same similarity measure as for
/// clusters, at the theme threshold. Every incident of the cluster is in exactly one theme.
///
/// Groups below the minimum theme size are not themes of their own. Their incidents join
/// the theme whose core they match best; those matching none form a last theme of the
/// remaining incidents, or join the theme they resemble most when they are too few.
fn build_subgroups(
    cluster_id: ClusterId,
    members: &[u32],
    rows: &[RowIndex],
    index: &SimilarityIndex,
    settings: &RunSettings,
    labeler: &Labeler,
    parent: &GroupProfile,
) -> Vec<Subgroup> {
    let whole_cluster = || {
        vec![Subgroup {
            id: 1,
            label: labeler.cluster_label(cluster_id, rows, parent, &[]),
            incident_row_indices: rows.to_vec(),
        }]
    };
    if rows.len() <= settings.minimum_cluster_size {
        return whole_cluster();
    }

    let minimum_size = (settings.minimum_cluster_size / 10).max(MIN_THEME_SIZE);
    let groups = index.groups(members, settings.subgroup_similarity_threshold_percent, None);
    // Largest first, so the themes are the leading groups; one place stays free for the rest.
    let large_enough = groups.iter().take_while(|group| group.len() >= minimum_size).count();
    let kept = if large_enough == groups.len() && large_enough <= MAX_THEMES {
        large_enough
    } else {
        large_enough.min(MAX_THEMES - 1)
    };
    if kept == 0 {
        return whole_cluster();
    }
    let mut themes = groups;
    let rest = themes.split_off(kept);

    let threshold = fraction(settings.similarity_threshold_percent);
    let cores = themes.iter().map(|theme| index.core(theme)).collect::<Vec<_>>();
    let mut remaining = Vec::new();
    for member in rest.into_iter().flatten() {
        match index.best_match(member, &cores, threshold) {
            Some(theme) => themes[theme].push(member),
            None => remaining.push(member),
        }
    }
    if remaining.len() < minimum_size {
        // Too few for a theme of their own: each joins the theme it resembles most.
        for member in remaining.drain(..) {
            themes[index.closest(member, &cores)].push(member);
        }
    }
    index.order(&mut themes);
    if !remaining.is_empty() {
        themes.push(remaining);
    }

    themes
        .into_iter()
        .enumerate()
        .map(|(position, members)| {
            let rows = index.rows_of(&members);
            Subgroup {
                id: position + 1,
                label: labeler.theme_label(cluster_id, &rows, &labeler.profile(&rows), parent),
                incident_row_indices: rows,
            }
        })
        .collect()
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

        let labeler = Labeler::new(&records, Vec::new(), &settings.label_terms, Default::default());
        let clusters = cluster_incidents(&records, &settings, &labeler, None);

        assert_eq!(clusters.len(), 2);
        assert!(clusters.iter().all(|cluster| cluster.size() == 3));
    }

    #[test]
    fn repeated_runs_give_identical_clusters_and_themes() {
        // Clusters of equal size, each with twenty equally sized groups competing for the
        // theme places: most ordering decisions are ties.
        let problems = [
            "password reset failure sap portal",
            "vpn connection timeout remote access",
            "printer queue stuck floor",
            "mailbox quota exceeded outlook",
            "badge reader offline entrance",
            "license activation rejected server",
            "monitor flickers docking station",
            "sharepoint sync conflict library",
        ];
        let sites = [
            "zurich", "geneva", "basel", "lausanne", "lugano", "lucerne", "winterthur", "fribourg", "neuchatel",
            "schaffhausen", "bellinzona", "montreux", "locarno", "interlaken", "davos", "zermatt", "solothurn",
            "frauenfeld", "rapperswil", "yverdon",
        ];
        let records = (0..problems.len() * sites.len() * 3)
            .map(|index| IncidentRecord {
                source_row_index: index,
                incident_number: format!("INC{index:04}"),
                analysis_text: format!("{} {}", problems[index % problems.len()], sites[index / problems.len() % sites.len()]),
                filter_values: FilterValues::default(),
                parsed_date: None,
            })
            .collect::<Vec<_>>();
        let settings = RunSettings {
            minimum_cluster_size: 10,
            similarity_threshold_percent: 40,
            subgroup_similarity_threshold_percent: 90,
            ..Default::default()
        };
        let labeler = Labeler::new(&records, Vec::new(), &settings.label_terms, Default::default());

        let first = cluster_incidents(&records, &settings, &labeler, None);
        assert_eq!(first.len(), problems.len());
        assert!(first.iter().all(|cluster| (2..=MAX_THEMES).contains(&cluster.subgroups.len())));
        // Largest first; ties are ordered by the smallest row.
        let order = |cluster: &Cluster| (std::cmp::Reverse(cluster.size()), cluster.incident_row_indices[0]);
        assert!(first.windows(2).all(|pair| order(&pair[0]) < order(&pair[1])));
        assert!(first.windows(2).any(|pair| pair[0].size() == pair[1].size()));
        for cluster in &first {
            let order = |theme: &Subgroup| (std::cmp::Reverse(theme.size()), theme.incident_row_indices[0]);
            // The last theme may hold the remaining incidents, whatever their number.
            let themes = &cluster.subgroups[..cluster.subgroups.len() - 1];
            assert!(themes.windows(2).all(|pair| order(&pair[0]) < order(&pair[1])));
        }
        for _ in 0..5 {
            assert_eq!(cluster_incidents(&records, &settings, &labeler, None), first);
        }
    }

    fn incident(index: usize, text: String) -> IncidentRecord {
        IncidentRecord {
            source_row_index: index,
            incident_number: format!("INC{index:05}"),
            analysis_text: text,
            filter_values: FilterValues::default(),
            parsed_date: None,
        }
    }

    fn cluster(records: &[IncidentRecord], settings: &RunSettings) -> Vec<Cluster> {
        let labeler = Labeler::new(records, Vec::new(), &settings.label_terms, Default::default());
        cluster_incidents(records, settings, &labeler, None)
    }

    /// Reproducible pseudo-random numbers for building test corpora.
    fn sequence(mut seed: u64) -> impl FnMut(usize) -> usize {
        move |below| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % below
        }
    }

    const WORDS: [&str; 40] = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet", "kilo", "lima",
        "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra", "tango", "uniform", "victor", "whiskey",
        "xray", "yankee", "zulu", "amber", "birch", "cedar", "dune", "ember", "fjord", "grove", "heath", "inlet",
        "jetty", "knoll", "lagoon", "marsh", "nook",
    ];

    #[test]
    fn a_problem_reported_thousands_of_times_is_one_cluster() {
        let mut next = sequence(7);
        let mut records = (0..2_500)
            .map(|index| incident(index, "Password reset failure on SAP portal login".to_owned()))
            .collect::<Vec<_>>();
        // Unrelated tickets: random word pairs glued together, so no two share a term.
        records.extend((2_500..2_800).map(|index| {
            let text = (0..6).map(|_| format!("{}{}", WORDS[next(40)], WORDS[next(40)])).collect::<Vec<_>>();
            incident(index, text.join(" "))
        }));

        let clusters = cluster(&records, &RunSettings::default());

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].incident_row_indices, (0..2_500).collect::<Vec<_>>());
    }

    #[test]
    fn incidents_differing_only_by_identifiers_are_one_cluster() {
        let mut next = sequence(11);
        let mut records = (0..400)
            .map(|index| {
                let text = format!("Server srv{:05} not responding - node down in rack r{}", next(100_000), next(60));
                incident(index, text)
            })
            .collect::<Vec<_>>();
        records.extend((400..800).map(|index| {
            let text = (0..6).map(|_| format!("{}{}", WORDS[next(40)], WORDS[next(40)])).collect::<Vec<_>>();
            incident(index, text.join(" "))
        }));

        let clusters = cluster(&records, &RunSettings::default());

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].incident_row_indices, (0..400).collect::<Vec<_>>());
    }

    #[test]
    fn a_shared_system_name_does_not_join_different_problems() {
        let systems = ["LedgerPoint Finance", "Helios Treasury", "OrbitCRM Portal"];
        let problems = [
            "users cannot log on database restricted mode",
            "print preview shows blank page",
            "orders allocated wrong warehouse",
            "certificate warning shown browser",
            "attachments missing saved record",
            "scheduled report delivered empty",
            "search returns nothing yesterday",
            "interface queue growing slowly",
            "invoice totals rounded incorrectly",
            "mobile app crashes startup",
            "password email never arrives",
            "export file opens corrupted",
        ];
        let records = (0..systems.len() * problems.len() * 10)
            .map(|index| incident(index, format!("{}: {}", systems[index % 3], problems[index / 3 % 12])))
            .collect::<Vec<_>>();
        let settings = RunSettings { minimum_cluster_size: 10, ..Default::default() };

        let clusters = cluster(&records, &settings);

        // One cluster per problem, each across all three systems.
        assert_eq!(clusters.len(), problems.len());
        for cluster in &clusters {
            assert_eq!(cluster.size(), 30);
            let problem = cluster.incident_row_indices[0] / 3 % 12;
            assert!(cluster.incident_row_indices.iter().all(|row| row / 3 % 12 == problem));
        }
    }

    #[test]
    fn candidate_search_finds_exactly_the_groups_of_an_all_pairs_comparison() {
        let mut next = sequence(3);
        let records = (0..400)
            .map(|index| {
                let text = (0..3 + next(6)).map(|_| WORDS[next(40)]).collect::<Vec<_>>();
                incident(index, text.join(" "))
            })
            .collect::<Vec<_>>();
        let (features, term_index) = extract_features(&records, None);
        let index = SimilarityIndex::new(&features, &term_index, 5, None);
        let everything = (0..records.len() as u32).collect::<Vec<_>>();

        for percent in [15, 30, 42, 58, 75, 100] {
            // Reference: compare every pair and merge the similar ones.
            let threshold = f32::from(percent) / 100.0;
            let mut group = (0..records.len()).collect::<Vec<_>>();
            for left in 0..records.len() {
                for right in left + 1..records.len() {
                    if index.similar(&index.documents[left], &index.documents[right], threshold) {
                        let (from, to) = (group[right], group[left]);
                        group.iter_mut().filter(|g| **g == from).for_each(|g| *g = to);
                    }
                }
            }
            let mut expected = std::collections::BTreeMap::<usize, Vec<u32>>::new();
            for (member, g) in group.iter().enumerate() {
                expected.entry(*g).or_default().push(member as u32);
            }
            let mut expected = expected.into_values().collect::<Vec<_>>();
            expected.sort_unstable_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));

            assert_eq!(index.groups(&everything, percent, None), expected, "threshold {percent}%");
        }
    }

    #[test]
    fn problems_joined_only_through_in_between_incidents_are_separate_clusters() {
        let first = "password reset email never arrives mailbox";
        let second = "warehouse scanner loses wireless connection dock";
        let mut next = sequence(5);
        let mut records = Vec::new();
        for index in 0..300 {
            let text = match index {
                0..40 => first.to_owned(),
                40..80 => second.to_owned(),
                // Tickets reporting both problems at once link the two groups.
                80..84 => format!("{first} {second}"),
                _ => (0..6).map(|_| format!("{}{}", WORDS[next(40)], WORDS[next(40)])).collect::<Vec<_>>().join(" "),
            };
            records.push(incident(index, text));
        }
        let settings = RunSettings { minimum_cluster_size: 20, ..Default::default() };

        let clusters = cluster(&records, &settings);

        assert_eq!(clusters.len(), 2);
        let holding = |row: usize| clusters.iter().position(|cluster| cluster.incident_row_indices.contains(&row));
        let (one, other) = (holding(0), holding(40));
        assert!(one.is_some() && other.is_some() && one != other);
        assert!((0..40).all(|row| holding(row) == one));
        assert!((40..80).all(|row| holding(row) == other));
        // The in-between tickets are not lost: each joined one of the two clusters.
        assert_eq!(clusters.iter().map(Cluster::size).sum::<usize>(), 84);
    }

    #[test]
    fn every_incident_of_a_cluster_is_in_one_theme_of_the_minimum_size() {
        let problems = ["password reset failure sap portal", "vpn connection timeout remote access"];
        let sites = [
            "zurich", "geneva", "basel", "lausanne", "lugano", "lucerne", "winterthur", "fribourg", "neuchatel",
            "schaffhausen", "bellinzona", "montreux", "locarno", "interlaken",
        ];
        // Per problem: fourteen sites, six of them with fewer tickets than a theme needs.
        let mut records = Vec::new();
        for problem in problems {
            for (s, site) in sites.iter().enumerate() {
                for _ in 0..if s < 8 { 12 } else { 2 } {
                    records.push(incident(records.len(), format!("{problem} {site}")));
                }
            }
        }
        let settings = RunSettings {
            minimum_cluster_size: 30,
            similarity_threshold_percent: 30,
            subgroup_similarity_threshold_percent: 90,
            ..Default::default()
        };

        let clusters = cluster(&records, &settings);

        assert_eq!(clusters.len(), 2);
        for cluster in &clusters {
            assert_eq!(cluster.size(), 108);
            assert!(cluster.subgroups.len() > 1 && cluster.subgroups.len() <= MAX_THEMES);
            assert!(cluster.subgroups.iter().all(|theme| theme.size() >= 3));
            let mut themed = cluster.subgroups.iter().flat_map(|theme| theme.incident_row_indices.clone()).collect::<Vec<_>>();
            themed.sort_unstable();
            assert_eq!(themed, cluster.incident_row_indices);
        }
    }
}
