use crate::model::{AnalysisRun, ClusterId, ColumnMapping, IncidentRecord, LabelTermPolicy, RowIndex, SourceTable};
use crate::text::{is_stopword, normalize_text, tokenize};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub fn summarize_cluster(
    id: ClusterId,
    row_indices: &[RowIndex],
    records: &[IncidentRecord],
    policy: &LabelTermPolicy,
) -> String {
    let keywords = representative_keywords(row_indices, records, policy, 4);
    match keywords.as_slice() {
        [] => format!("Similar incidents in cluster {}", id.0),
        [single] => sentence_case(single),
        [first, second] => sentence_case(&format!("{first} related to {second}")),
        [first, second, third] => {
            sentence_case(&format!("{first}, {second} and {third} related issues"))
        }
        [first, second, third, fourth, ..] => sentence_case(&format!(
            "{first}, {second}, {third} and {fourth} related issues"
        )),
    }
}

pub fn representative_keywords(
    row_indices: &[RowIndex],
    records: &[IncidentRecord],
    policy: &LabelTermPolicy,
    limit: usize,
) -> Vec<String> {
    let rows = row_indices.iter().copied().collect::<HashSet<_>>();
    let mut group_counts = HashMap::<String, usize>::new();
    let mut document_frequency = HashMap::<String, usize>::new();
    let policy = NormalizedLabelTermPolicy::from(policy);
    let mut document_count = 0usize;

    for record in records {
        document_count += 1;
        let tokens = tokenize(&record.analysis_text)
            .into_iter()
            .filter(|token| token.len() >= 3 && !policy.excluded.contains(token))
            .collect::<Vec<_>>();
        let unique = tokens.iter().cloned().collect::<HashSet<_>>();
        for token in unique {
            *document_frequency.entry(token).or_default() += 1;
        }
        if rows.contains(&record.source_row_index) {
            for token in tokens {
                *group_counts.entry(token).or_default() += 1;
            }
        }
    }

    let document_count = document_count.max(1) as f32;
    let mut terms = group_counts
        .into_iter()
        .map(|(term, count)| {
            let df = *document_frequency.get(&term).unwrap_or(&1) as f32;
            let idf = (document_count / df).ln_1p();
            let weight = policy.weight(&term);
            let score = (count as f32) * idf * weight;
            (term, count, score)
        })
        .collect::<Vec<_>>();
    terms.sort_by(
        |(left_term, left_count, left_score), (right_term, right_count, right_score)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| right_count.cmp(left_count))
                .then_with(|| left_term.cmp(right_term))
        },
    );

    terms
        .into_iter()
        .take(limit)
        .map(|(term, _, _)| term)
        .collect()
}

struct NormalizedLabelTermPolicy {
    boosted: HashSet<String>,
    suppressed: HashSet<String>,
    excluded: HashSet<String>,
}

impl NormalizedLabelTermPolicy {
    fn weight(&self, term: &str) -> f32 {
        if self.boosted.contains(term) {
            3.0
        } else if self.suppressed.contains(term) {
            0.2
        } else {
            1.0
        }
    }
}

impl From<&LabelTermPolicy> for NormalizedLabelTermPolicy {
    fn from(policy: &LabelTermPolicy) -> Self {
        Self {
            boosted: normalize_terms(&policy.boosted),
            suppressed: normalize_terms(&policy.suppressed),
            excluded: normalize_terms(&policy.excluded),
        }
    }
}

fn normalize_terms(terms: &[String]) -> HashSet<String> {
    terms
        .iter()
        .flat_map(|term| tokenize(term))
        .filter(|term| term.len() >= 3)
        .collect()
}

fn sentence_case(input: &str) -> String {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return "Similar incidents".to_owned();
    };

    format!("{}{}", first.to_uppercase(), chars.as_str())
}

/// How cluster and theme titles are written. Chosen in the application configuration,
/// not stored with an analysis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LabelStyle {
    /// A typical short description, distinctive key phrases and dominant field values.
    #[default]
    Descriptive,
    /// The original keyword summary ("A, B, C and D related issues").
    Keywords,
}

/// Each record's short description, which people wrote to summarize the incident; titles quote them.
pub fn short_descriptions<'a>(
    source: &'a SourceTable,
    mapping: &ColumnMapping,
    records: &[IncidentRecord],
) -> Vec<&'a str> {
    mapping
        .short_description
        .map(|column| {
            records
                .iter()
                .map(|record| {
                    source
                        .rows
                        .get(record.source_row_index)
                        .and_then(|row| row.get(column))
                        .map_or("", String::as_str)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Rewrites the generated titles of an existing analysis without clustering it again; the
/// result matches what a new run would produce for the same clusters and themes.
/// Returns how many titles changed.
pub fn relabel(run: &mut AnalysisRun, style: LabelStyle) -> usize {
    let titles = short_descriptions(&run.source, &run.mapping, &run.processed_incidents);
    let labeler = Labeler::new(&run.processed_incidents, titles, &run.settings.label_terms, style);
    let mut labels = Vec::with_capacity(run.clusters.len());
    for cluster in &run.clusters {
        // The unclustered group is derived when an analysis loads and keeps its fixed name.
        if cluster.id == ClusterId::UNCLUSTERED {
            labels.push(None);
            continue;
        }
        let rows = &cluster.incident_row_indices;
        let profile = labeler.profile(rows);
        let themes = cluster
            .subgroups
            .iter()
            .map(|theme| {
                if theme.incident_row_indices.len() >= rows.len() {
                    labeler.cluster_label(cluster.id, &theme.incident_row_indices, &profile, &[])
                } else {
                    let own = labeler.profile(&theme.incident_row_indices);
                    labeler.theme_label(cluster.id, &theme.incident_row_indices, &own, &profile)
                }
            })
            .collect::<Vec<_>>();
        let rows_of_themes = cluster.subgroups.iter().map(|s| s.incident_row_indices.as_slice()).collect::<Vec<_>>();
        let label = labeler.cluster_label(cluster.id, rows, &profile, &rows_of_themes);
        labels.push(Some((label, themes)));
    }
    drop(labeler);
    let mut changed = 0;
    for (cluster, new) in run.clusters.iter_mut().zip(labels) {
        let Some((label, themes)) = new else { continue };
        changed += usize::from(cluster.label != label);
        cluster.label = label;
        for (theme, label) in cluster.subgroups.iter_mut().zip(themes) {
            changed += usize::from(theme.label != label);
            theme.label = label;
        }
    }
    changed
}

/// Share of a group a field value must cover to be named in the title.
const DOMINANT_FIELD_SHARE: f32 = 0.6;
/// Share of a group a phrase must appear in to be a key phrase.
const MIN_PHRASE_COVERAGE: f32 = 0.25;
/// For a mixed group no phrase is common, so the most frequent distinctive ones are named.
const MIN_MIXED_PHRASE_COVERAGE: f32 = 0.03;
const MAX_PHRASES: usize = 2;
/// Share of a group a system name or identifier must cover to stay in a quoted title.
const SHARED_IN_TITLE: f32 = 0.5;
/// Other themes named after a cluster title.
const MAX_ALSO: usize = 2;
/// Below this share of tickets resembling the typical one, a group is described by phrases instead.
const MIN_TYPICAL_SHARE: f32 = 0.15;
const MAX_TITLE_CHARS: usize = 100;
/// Smoothing for phrase rates, so rare phrases do not get extreme scores.
const RATE_SMOOTHING: f32 = 0.01;
/// Words that carry no meaning at the start or end of a key phrase.
const FILLER: &[&str] = &[
    "after", "again", "all", "also", "any", "before", "being", "been", "but", "did", "does", "due",
    "during", "each", "for", "has", "have", "into", "its", "more", "now", "once", "only", "other",
    "over", "some", "still", "than", "then", "this", "under", "unable", "until", "via", "when",
    "while", "will", "would", "should", "could", "since", "there", "their", "these", "those",
    "your", "our", "out", "off", "one", "two", "via", "new", "why", "how", "what", "who", "which",
    "nach", "bei", "aus", "wird", "sind", "kann", "dass", "auch", "noch",
];

/// Writes titles for clusters and themes. Built once per analysis: every record's
/// title phrases are extracted up front, so each group only counts its own members.
pub struct Labeler<'a> {
    records: &'a [IncidentRecord],
    position: HashMap<RowIndex, usize>,
    /// Short description per record, aligned with `records`.
    titles: Vec<&'a str>,
    /// Unique phrase ids (words and adjacent word pairs) of each record's short description.
    terms: Vec<Vec<u32>>,
    vocabulary: Vec<String>,
    corpus_frequency: Vec<u32>,
    /// Lower-case system names (service, category, configuration item) in the data.
    system_names: Vec<String>,
    /// Words of all field values; such words belong in the field context, not in key phrases.
    field_words: HashSet<String>,
    /// Words of system names; a phrase naming a system is about that system, not the problem.
    system_words: HashSet<String>,
    policy: NormalizedLabelTermPolicy,
    source_policy: LabelTermPolicy,
    style: LabelStyle,
}

/// Phrase and field counts for one group of records; kept so themes can be compared with their cluster.
pub struct GroupProfile {
    size: usize,
    frequency: HashMap<u32, u32>,
    /// Lower-case system name → number of group records naming it.
    systems: HashMap<String, usize>,
    dominant: [Option<String>; 4],
}

impl<'a> Labeler<'a> {
    /// `titles` holds each record's short description. When it is empty, the full
    /// analysis text is used instead.
    pub fn new(
        records: &'a [IncidentRecord],
        titles: Vec<&'a str>,
        policy: &LabelTermPolicy,
        style: LabelStyle,
    ) -> Self {
        let titles = if titles.len() == records.len() {
            titles
        } else {
            records.iter().map(|r| r.analysis_text.as_str()).collect()
        };
        let source_policy = policy.clone();
        let policy = NormalizedLabelTermPolicy::from(policy);
        let mut ids = HashMap::<String, u32>::new();
        let mut vocabulary = Vec::new();
        let mut corpus_frequency = Vec::<u32>::new();
        let terms = titles
            .iter()
            .map(|title| {
                if style == LabelStyle::Keywords {
                    return Vec::new();
                }
                let mut own = phrases(title, &policy)
                    .into_iter()
                    .map(|phrase| {
                        *ids.entry(phrase).or_insert_with_key(|phrase| {
                            vocabulary.push(phrase.clone());
                            corpus_frequency.push(0);
                            (vocabulary.len() - 1) as u32
                        })
                    })
                    .collect::<Vec<_>>();
                own.sort_unstable();
                own.dedup();
                for id in &own {
                    corpus_frequency[*id as usize] += 1;
                }
                own
            })
            .collect();
        let mut system_names = HashSet::new();
        let mut field_words = HashSet::new();
        let mut system_words = HashSet::new();
        for record in records {
            let values = &record.filter_values;
            for (value, system) in [
                (&values.service, true),
                (&values.category, true),
                (&values.configuration_item, true),
                (&values.assignment_group, false),
            ] {
                let Some(value) = value.as_deref().map(str::trim).filter(|v| v.chars().count() >= 3) else { continue };
                if system {
                    system_names.insert(value.to_lowercase());
                    system_words.extend(tokenize(value));
                }
                field_words.extend(tokenize(value));
            }
        }
        // Longest first, so "LedgerPoint Finance" is found before "LedgerPoint".
        let mut system_names = system_names.into_iter().collect::<Vec<_>>();
        system_names.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        Self {
            position: records
                .iter()
                .enumerate()
                .map(|(i, r)| (r.source_row_index, i))
                .collect(),
            records,
            titles,
            terms,
            vocabulary,
            corpus_frequency,
            system_names,
            field_words,
            system_words,
            policy,
            source_policy,
            style,
        }
    }

    pub fn profile(&self, rows: &[RowIndex]) -> GroupProfile {
        let mut frequency = HashMap::<u32, u32>::new();
        let mut fields: [HashMap<&str, usize>; 4] = Default::default();
        let mut systems = HashMap::<String, usize>::new();
        let mut size = 0;
        for i in rows.iter().filter_map(|row| self.position.get(row)) {
            size += 1;
            for id in &self.terms[*i] {
                *frequency.entry(*id).or_default() += 1;
            }
            let values = &self.records[*i].filter_values;
            for (index, (counts, value)) in fields
                .iter_mut()
                .zip([
                    &values.category,
                    &values.service,
                    &values.configuration_item,
                    &values.assignment_group,
                ])
                .enumerate()
            {
                if let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                    *counts.entry(value).or_default() += 1;
                    if index < 3 {
                        *systems.entry(value.to_lowercase()).or_default() += 1;
                    }
                }
            }
        }
        let dominant = fields.map(|counts| {
            counts
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
                .filter(|(_, count)| *count as f32 >= DOMINANT_FIELD_SHARE * size.max(1) as f32)
                .map(|(value, _)| value.to_owned())
        });
        GroupProfile { size, frequency, systems, dominant }
    }

    /// Title for a primary cluster, compared with all other records. `themes` are the rows of
    /// its themes, largest first; their problems are named after the title.
    pub fn cluster_label(
        &self,
        id: ClusterId,
        rows: &[RowIndex],
        profile: &GroupProfile,
        themes: &[&[RowIndex]],
    ) -> String {
        if self.style == LabelStyle::Keywords {
            return summarize_cluster(id, rows, self.records, &self.source_policy);
        }
        let background = Background {
            size: self.records.len(),
            frequency: |term: u32| self.corpus_frequency[term as usize],
        };
        let none = [None, None, None, None];
        let also = self.other_problems(rows, profile, themes);
        self.describe(id, rows, profile, &background, &none, Some(also))
    }

    /// Each theme's typical problem, phrased for the whole cluster (system names it does not share removed).
    fn other_problems(&self, rows: &[RowIndex], profile: &GroupProfile, themes: &[&[RowIndex]]) -> Vec<String> {
        let members = self.members(rows);
        themes
            .iter()
            .filter(|theme| theme.len() < profile.size)
            .filter_map(|theme| {
                let own = self.profile(theme);
                let coverage = |term: u32| own.frequency.get(&term).copied().unwrap_or(0) as f32 / own.size.max(1) as f32;
                let rest_rate = |term: u32| {
                    let inside = own.frequency.get(&term).copied().unwrap_or(0);
                    let outside = profile.frequency.get(&term).copied().unwrap_or(0).saturating_sub(inside);
                    outside as f32 / profile.size.saturating_sub(own.size).max(1) as f32
                };
                let best = self.typical_index(theme, &coverage, &rest_rate)?;
                Some(self.generalize(self.titles[best], &members, profile)).filter(|t| !t.is_empty())
            })
            .collect()
    }

    /// Title for a theme, compared with the rest of its cluster so it says what sets it apart.
    pub fn theme_label(
        &self,
        id: ClusterId,
        rows: &[RowIndex],
        profile: &GroupProfile,
        parent: &GroupProfile,
    ) -> String {
        if self.style == LabelStyle::Keywords {
            return summarize_cluster(id, rows, self.records, &self.source_policy);
        }
        if profile.size >= parent.size {
            // A theme covering the whole cluster has nothing to set it apart from.
            return self.cluster_label(id, rows, profile, &[]);
        }
        let background = Background {
            size: parent.size,
            frequency: |term: u32| parent.frequency.get(&term).copied().unwrap_or(0),
        };
        self.describe(id, rows, profile, &background, &parent.dominant, None)
    }

    fn describe(
        &self,
        id: ClusterId,
        rows: &[RowIndex],
        profile: &GroupProfile,
        background: &Background<impl Fn(u32) -> u32>,
        inherited: &[Option<String>; 4],
        // For clusters: their themes' problems. Themes get no subtitle; their title is specific enough.
        also: Option<Vec<String>>,
    ) -> String {
        if profile.size == 0 {
            return format!("Similar incidents in cluster {}", id.0);
        }
        let coverage = |term: u32| {
            profile.frequency.get(&term).copied().unwrap_or(0) as f32 / profile.size as f32
        };
        // Rate among the background records outside this group.
        let rest_rate = |term: u32| {
            let inside = profile.frequency.get(&term).copied().unwrap_or(0);
            let outside = (background.frequency)(term).saturating_sub(inside);
            let others = background.size.saturating_sub(profile.size);
            if others == 0 { 0.0 } else { outside as f32 / others as f32 }
        };
        let members = self.members(rows);
        let typical = self
            .typical_index(rows, &coverage, &rest_rate)
            .map(|i| self.generalize(self.titles[i], &members, profile))
            .filter(|t| !t.is_empty());
        let title_words = typical
            .as_ref()
            .map(|t| tokenize(t).into_iter().collect::<HashSet<_>>())
            .unwrap_or_default();
        // Other problems in the cluster, each clearly different from the title and from each other.
        let mut others = Vec::<String>::new();
        let mut seen = vec![title_words.clone()];
        let shared_prefix = typical.as_deref().and_then(|t| t.split_once(": ")).map(|(p, _)| format!("{p}: "));
        for problem in also.as_deref().unwrap_or_default() {
            // "System: …" is said once, in the title.
            let problem = match &shared_prefix {
                Some(prefix) if problem.starts_with(prefix.as_str()) => sentence_case(&problem[prefix.len()..]),
                _ => problem.clone(),
            };
            let words = tokenize(&problem).into_iter().collect::<HashSet<_>>();
            if typical.is_some() && others.len() < MAX_ALSO && seen.iter().all(|s| jaccard(s, &words) < 0.5) {
                others.push(problem);
                seen.push(words);
            }
        }
        let phrases = match typical {
            Some(_) if also.is_none() || !others.is_empty() => Vec::new(),
            Some(_) => self.key_phrases(profile, &coverage, &rest_rate, &title_words, MAX_PHRASES, MIN_PHRASE_COVERAGE),
            None => self.key_phrases(profile, &coverage, &rest_rate, &title_words, MAX_PHRASES + 1, MIN_MIXED_PHRASE_COVERAGE),
        };
        let mut label = match typical {
            Some(title) => {
                let mut label = title;
                if !others.is_empty() {
                    label.push_str(" — also: ");
                    label.push_str(&others.join("; "));
                } else if !phrases.is_empty() {
                    label.push_str(" — ");
                    label.push_str(&phrases.join(", "));
                }
                label
            }
            // No ticket stands for the group: name what its tickets have in common instead.
            None if !phrases.is_empty() => format!("Mixed incidents — {}", phrases.join(", ")),
            None => format!("Similar incidents in cluster {}", id.0),
        };
        let lower = label.to_lowercase();
        let fields = profile
            .dominant
            .iter()
            .zip(inherited)
            .filter_map(|(value, parent)| value.as_ref().filter(|v| Some(*v) != parent.as_ref()))
            .filter(|value| !lower.contains(&value.to_lowercase()))
            .take(2)
            .cloned()
            .collect::<Vec<_>>();
        if !fields.is_empty() {
            label.push_str(&format!(" ({})", fields.join(" · ")));
        }
        label
    }

    fn members(&self, rows: &[RowIndex]) -> Vec<usize> {
        rows.iter().filter_map(|row| self.position.get(row).copied()).collect()
    }

    /// The record whose short description is most typical of the group and most specific to it,
    /// or `None` when too few tickets resemble even the best candidate.
    fn typical_index(
        &self,
        rows: &[RowIndex],
        coverage: &impl Fn(u32) -> f32,
        rest_rate: &impl Fn(u32) -> f32,
    ) -> Option<usize> {
        let members = self.members(rows);
        let mut candidates = HashMap::<String, (usize, usize)>::new();
        for i in &members {
            let key = normalize_text(self.titles[*i]);
            if !key.is_empty() {
                candidates.entry(key).or_insert((*i, 0)).1 += 1;
            }
        }
        let weight = |term: u32| {
            let c = coverage(term);
            c + (c - rest_rate(term)).max(0.0)
        };
        let words = |i: usize| {
            self.terms[i]
                .iter()
                .copied()
                .filter(|t| !self.vocabulary[*t as usize].contains(' '))
                .collect::<Vec<_>>()
        };
        let (_, best) = candidates
            .into_values()
            .map(|(i, count)| {
                let own = words(i);
                let mean = if own.is_empty() {
                    0.0
                } else {
                    own.iter().map(|t| weight(*t)).sum::<f32>() / own.len() as f32
                };
                // Very short descriptions say little; repeated ones are confirmed by people.
                let length = (own.len() as f32 / 4.0).min(1.0);
                let repeated = 1.0 + 0.1 * (count as f32).ln();
                (mean * length * repeated, i)
            })
            .max_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.1.cmp(&a.1)))?;
        // How many tickets share most of the typical ticket's words.
        let chosen = words(best).into_iter().collect::<HashSet<_>>();
        if !chosen.is_empty() {
            let alike = members
                .iter()
                .filter(|i| {
                    let shared = words(**i).iter().filter(|t| chosen.contains(t)).count();
                    shared as f32 >= 0.6 * chosen.len() as f32
                })
                .count();
            if (alike as f32) < MIN_TYPICAL_SHARE * members.len() as f32 {
                return None;
            }
        }
        Some(best)
    }

    /// Turns one ticket's short description into a title for the whole group: reply prefixes,
    /// references, and system names or identifiers that most of the group does not share are removed.
    fn generalize(&self, title: &str, members: &[usize], profile: &GroupProfile) -> String {
        let shared_system = |name: &str| {
            profile.systems.get(name).copied().unwrap_or(0) as f32 >= SHARED_IN_TITLE * profile.size as f32
        };
        let mut text = title.trim().to_owned();
        loop {
            let lower = text.to_lowercase();
            let Some(prefix) = ["re:", "fw:", "fwd:", "aw:", "wg:"].iter().find(|p| lower.starts_with(**p)) else { break };
            text = text[prefix.len()..].trim_start().to_owned();
        }
        // "System: problem" — keep the system only when most of the group is about it.
        if let Some((prefix, rest)) = text.split_once(": ").filter(|(p, r)| p.chars().count() <= 60 && !r.trim().is_empty()) {
            let lower = prefix.trim().to_lowercase();
            let known = self.system_names.contains(&lower);
            let share = if known {
                profile.systems.get(&lower).copied().unwrap_or(0) as f32 / profile.size.max(1) as f32
            } else {
                phrases(prefix, &self.policy)
                    .into_iter()
                    .filter(|p| !p.contains(' '))
                    .map(|p| self.share_containing(members, &p))
                    .fold(f32::INFINITY, f32::min)
            };
            if share.is_finite() && share < SHARED_IN_TITLE {
                text = rest.trim().to_owned();
            }
        }
        // System names inside the sentence ("… affecting OrbitCRM").
        for name in &self.system_names {
            // Positions found in the lower-case text are only valid when lower-casing keeps the length.
            if shared_system(name) || text.to_lowercase().len() != text.len() {
                continue;
            }
            while let Some(start) = find_word(&text.to_lowercase(), name) {
                text.replace_range(start..start + name.len(), "…");
            }
        }
        // References and identifiers such as host or file names that vary within the group.
        let words = text
            .split_whitespace()
            // References say nothing about the problem; they are dropped, not marked.
            .filter(|word| !is_reference(word))
            .map(|word| {
                let identifier = (word.chars().any(|c| c.is_ascii_digit()) && word.chars().any(char::is_alphabetic))
                    || word.trim_matches(|c: char| !c.is_alphanumeric()).contains('_');
                let varies = identifier && self.share_containing(members, &word.to_lowercase()) < SHARED_IN_TITLE;
                if varies { "…" } else { word }
            })
            .collect::<Vec<_>>();
        let mut joined = String::new();
        for word in words {
            if word == "…" && (joined.ends_with('…') || joined.ends_with("… ")) {
                continue;
            }
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(word);
        }
        for quoted in ["'…'", "\"…\"", "‘…’", "“…”", "(…)"] {
            joined = joined.replace(quoted, "…");
        }
        let joined = balance_quotes(&joined);
        let trimmed = joined
            .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '…' | '-' | ':' | ',' | ';' | '.'))
            .trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '-' | ':' | ',' | ';'));
        let trimmed = trimmed.strip_suffix(" -").unwrap_or(trimmed);
        sentence_case(&truncate_words(trimmed, MAX_TITLE_CHARS))
    }

    /// Share of the group whose short description contains `needle` (lower case).
    fn share_containing(&self, members: &[usize], needle: &str) -> f32 {
        if members.is_empty() {
            return 0.0;
        }
        let hits = members
            .iter()
            .filter(|i| self.titles[**i].to_lowercase().contains(needle))
            .count();
        hits as f32 / members.len() as f32
    }

    fn key_phrases(
        &self,
        profile: &GroupProfile,
        coverage: &impl Fn(u32) -> f32,
        rest_rate: &impl Fn(u32) -> f32,
        title_words: &HashSet<String>,
        limit: usize,
        min_coverage: f32,
    ) -> Vec<String> {
        let mut scored = profile
            .frequency
            .iter()
            .filter(|(_, count)| **count >= 2)
            .filter_map(|(term, _)| {
                let c = coverage(*term);
                let r = rest_rate(*term);
                let phrase = &self.vocabulary[*term as usize];
                // System and team names are shown as field context instead.
                if c < min_coverage
                    || c <= r
                    || phrase.split(' ').all(|w| self.field_words.contains(w))
                    || phrase.split(' ').any(|w| self.system_words.contains(w))
                {
                    return None;
                }
                let pair = phrase.contains(' ');
                let weight = phrase.split(' ').map(|w| self.policy.weight(w)).fold(1.0f32, f32::max);
                let score = c * ((c + RATE_SMOOTHING) / (r + RATE_SMOOTHING)).ln() * weight * if pair { 1.2 } else { 1.0 };
                Some((score, phrase))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        let mut chosen = Vec::<&String>::new();
        let mut used = title_words.clone();
        for (_, phrase) in scored {
            let words = phrase.split(' ').collect::<Vec<_>>();
            // Skip what the title or an earlier phrase already says.
            if words.iter().all(|w| used.contains(*w)) {
                continue;
            }
            // A word pair replaces the single word it contains.
            if words.len() > 1 {
                chosen.retain(|p| !(!p.contains(' ') && words.contains(&p.as_str())));
            }
            used.extend(words.iter().map(|w| (*w).to_owned()));
            chosen.push(phrase);
            if chosen.len() == limit {
                break;
            }
        }
        chosen.into_iter().cloned().collect()
    }
}

struct Background<F> {
    size: usize,
    frequency: F,
}

/// Drops quote marks left without a partner after a quoted identifier was removed.
fn balance_quotes(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    // A quote next to a non-letter is a quote mark; inside a word ("don't") it is an apostrophe.
    let marks = (0..chars.len())
        .filter(|i| matches!(chars[*i], '\'' | '"' | '‘' | '’' | '“' | '”'))
        .filter(|i| {
            let before = i.checked_sub(1).map(|j| chars[j].is_alphanumeric()).unwrap_or(false);
            let after = chars.get(i + 1).is_some_and(|c| c.is_alphanumeric());
            !(before && after)
        })
        .collect::<Vec<_>>();
    if marks.len() % 2 == 0 {
        return text.to_owned();
    }
    let text = chars
        .iter()
        .enumerate()
        .filter(|(i, _)| !marks.contains(i))
        .map(|(_, c)| *c)
        .collect::<String>();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    let union = a.union(b).count();
    if union == 0 { 1.0 } else { a.intersection(b).count() as f32 / union as f32 }
}

/// Meaningful words and pairs of words that are adjacent in the text itself.
fn phrases(text: &str, policy: &NormalizedLabelTermPolicy) -> Vec<String> {
    let normalized = normalize_text(text);
    let content = |w: &str| {
        w.chars().count() >= 3
            // Numbers and identifiers such as host or job names are not phrases.
            && !w.chars().any(|c| c.is_ascii_digit())
            && !is_stopword(w)
            && !FILLER.contains(&w)
            && !policy.excluded.contains(w)
    };
    let words = normalized.split_whitespace().collect::<Vec<_>>();
    let mut out = words.iter().filter(|w| content(w)).map(|w| (*w).to_owned()).collect::<Vec<_>>();
    out.extend(
        words
            .windows(2)
            .filter(|p| p[0] != p[1] && content(p[0]) && content(p[1]))
            .map(|p| format!("{} {}", p[0], p[1])),
    );
    out
}

/// Byte position of `name` in `text` as a whole word or name.
fn find_word(text: &str, name: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = text[from..].find(name) {
        let start = from + found;
        let end = start + name.len();
        let before = text[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = text[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if before && after {
            return Some(start);
        }
        from = end;
    }
    None
}

/// Ticket numbers, email addresses, IP addresses and links say nothing about the problem itself.
fn is_reference(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric());
    let upper = w.to_uppercase();
    w.contains('@')
        || word.contains("://")
        || (w.split('.').count() == 4 && w.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())))
        || ["INC", "REQ", "RITM", "CHG", "PRB", "TASK", "SCTASK"].iter().any(|p| {
            upper.starts_with(p) && upper.len() > p.len() + 3 && upper[p.len()..].chars().all(|c| c.is_ascii_digit())
        })
}

fn truncate_words(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    for word in text.split_whitespace() {
        if out.chars().count() + word.chars().count() + 1 > max - 1 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() {
        out = text.chars().take(max - 1).collect();
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FilterValues, IncidentRecord};

    #[test]
    fn creates_non_empty_sentence_summary() {
        let records = vec![IncidentRecord {
            source_row_index: 0,
            incident_number: "INC001".to_owned(),
            analysis_text: "Password reset failure for SAP".to_owned(),
            filter_values: FilterValues::default(),
            parsed_date: None,
        }];

        let label = summarize_cluster(ClusterId(1), &[0], &records, &LabelTermPolicy::default());

        assert!(!label.is_empty());
        assert!(label.chars().next().is_some_and(char::is_uppercase));
    }

    #[test]
    fn boosted_terms_rank_ahead_of_more_frequent_terms() {
        let records = vec![
            incident(0, "generic generic vpn"),
            incident(1, "generic generic vpn"),
        ];
        let policy = LabelTermPolicy {
            boosted: vec!["vpn".to_owned()],
            ..Default::default()
        };

        let keywords = representative_keywords(&[0, 1], &records, &policy, 2);

        assert_eq!(keywords.first().map(String::as_str), Some("vpn"));
    }

    #[test]
    fn excluded_terms_are_not_used_in_labels() {
        let records = vec![incident(0, "password password vpn")];
        let policy = LabelTermPolicy {
            excluded: vec!["password".to_owned()],
            ..Default::default()
        };

        let keywords = representative_keywords(&[0], &records, &policy, 2);

        assert_eq!(keywords, vec!["vpn"]);
    }

    #[test]
    fn tf_idf_prefers_more_distinctive_terms() {
        let records = vec![
            incident(0, "common rare"),
            incident(1, "common rare"),
            incident(2, "common"),
            incident(3, "common"),
            incident(4, "common"),
            incident(5, "common"),
        ];

        let keywords = representative_keywords(&[0, 1], &records, &LabelTermPolicy::default(), 2);

        assert_eq!(keywords.first().map(String::as_str), Some("rare"));
    }

    #[test]
    fn formats_four_keyword_summary_with_requested_template() {
        let records = vec![incident(0, "alpha beta gamma delta epsilon")];

        let label = summarize_cluster(ClusterId(1), &[0], &records, &LabelTermPolicy::default());

        assert_eq!(label, "Alpha, beta, delta and epsilon related issues");
    }

    fn ticket(row: RowIndex, service: &str, group: &str) -> IncidentRecord {
        IncidentRecord {
            filter_values: FilterValues {
                service: Some(service.to_owned()),
                assignment_group: Some(group.to_owned()),
                ..Default::default()
            },
            ..incident(row, "")
        }
    }

    /// Clusters of "System: problem" tickets like typical ITSM exports.
    fn fixture() -> (Vec<IncidentRecord>, Vec<String>) {
        let mut records = Vec::new();
        let mut titles = Vec::new();
        let systems = ["LedgerPoint Finance", "Helios Treasury", "OrbitCRM", "ProcureFlow"];
        for i in 0..40 {
            let system = systems[i % 4];
            let (title, group) = if i < 24 {
                (format!("{system}: Users cannot log on - database in restricted mode"), "Oracle DBA Team")
            } else if i < 36 {
                (format!("Blocking chain detected on prd{i:02}db01 - {system} batch contention"), "Oracle DBA Team")
            } else {
                (format!("Report '{system}' shows wrong totals INC{i:08}"), "Finance Apps L2")
            };
            records.push(ticket(i, system, group));
            titles.push(title);
        }
        // Unrelated tickets, so cluster words are distinctive against the rest.
        for i in 40..100 {
            records.push(ticket(i, "Workplace", "Service Desk"));
            titles.push(format!("Laptop {} keyboard not working", ["dock", "screen", "battery"][i % 3]));
        }
        (records, titles)
    }

    fn labeler<'a>(records: &'a [IncidentRecord], titles: &'a [String]) -> Labeler<'a> {
        let titles = titles.iter().map(String::as_str).collect();
        Labeler::new(records, titles, &LabelTermPolicy::default(), LabelStyle::Descriptive)
    }

    #[test]
    fn cluster_title_quotes_a_typical_ticket_without_systems_it_does_not_share() {
        let (records, titles) = fixture();
        let labeler = labeler(&records, &titles);
        let rows = (0..40).collect::<Vec<_>>();
        let themes: [&[RowIndex]; 2] = [&rows[..24], &rows[24..36]];
        let label = labeler.cluster_label(ClusterId(1), &rows, &labeler.profile(&rows), &themes);
        assert_eq!(
            label,
            "Users cannot log on - database in restricted mode — also: Blocking chain detected on … - … batch contention (Oracle DBA Team)"
        );
    }

    #[test]
    fn theme_title_keeps_its_own_system_and_names_new_field_values() {
        let (records, titles) = fixture();
        let labeler = labeler(&records, &titles);
        let cluster = (0..40).collect::<Vec<_>>();
        let parent = labeler.profile(&cluster);
        // One system's restricted-mode tickets.
        let theme = (0..24).step_by(4).collect::<Vec<_>>();
        let label = labeler.theme_label(ClusterId(1), &theme, &labeler.profile(&theme), &parent);
        // The team is the cluster's own, so it is not repeated.
        assert_eq!(label, "LedgerPoint Finance: Users cannot log on - database in restricted mode");
        let reports = (36..40).collect::<Vec<_>>();
        let label = labeler.theme_label(ClusterId(1), &reports, &labeler.profile(&reports), &parent);
        assert_eq!(label, "Report … shows wrong totals (Finance Apps L2)");
    }

    #[test]
    fn mixed_groups_are_described_by_phrases() {
        let mut records = Vec::new();
        let mut titles = Vec::new();
        // Ten unrelated problems: none stands for the group.
        let problems = [
            "vpn tunnel drops", "printer jams paper", "mailbox quota full", "badge reader offline",
            "monitor flickers randomly", "teams calls freeze", "sharepoint sync stuck",
            "license activation fails", "wifi roaming slow", "headset microphone muted",
        ];
        for i in 0..50 {
            records.push(ticket(i, "Workplace", "Service Desk"));
            titles.push(format!("{} {}", problems[i % 10], ["today", "again", "since monday", "after update", "for team"][i / 10]));
        }
        let labeler = labeler(&records, &titles);
        let rows = (0..50).collect::<Vec<_>>();
        let label = labeler.cluster_label(ClusterId(3), &rows, &labeler.profile(&rows), &[]);
        assert!(label.starts_with("Mixed incidents — "), "{label}");
    }

    #[test]
    fn generalized_titles_drop_references_and_keep_quotes_balanced() {
        assert_eq!(balance_quotes("Cannot save - … unable to extend table'"), "Cannot save - … unable to extend table");
        assert_eq!(balance_quotes("Users can't see 'Calculate' button"), "Users can't see 'Calculate' button");
        assert!(is_reference("INC0012345"));
        assert!(is_reference("jane.doe@example.com"));
        assert!(is_reference("10.1.2.3"));
        assert!(!is_reference("ORA-01653"));
        assert_eq!(phrases("Inserts into PICK_TASKS failing on prd01db", &NormalizedLabelTermPolicy::from(&LabelTermPolicy::default())),
            ["inserts", "pick", "tasks", "failing", "pick tasks", "tasks failing"]);
    }

    #[test]
    fn regenerating_a_fresh_run_matches_its_titles() {
        use crate::model::{RunSettings, SourceTable};
        let problems = ["VPN tunnel drops after update", "Printer jams on floor three", "Mailbox quota full warning"];
        let rows = (0..300)
            .map(|i| {
                vec![
                    format!("INC{i:05}"),
                    format!("{} {}", problems[i % 3], ["for team", "since monday", "again"][i / 3 % 3]),
                    ["Workplace", "Network"][i % 2].to_owned(),
                ]
            })
            .collect();
        let source = SourceTable {
            source_path: None,
            worksheet_name: None,
            headers: vec!["Number".into(), "Short Description".into(), "Service Impacted".into()],
            rows,
        };
        let mapping = crate::schema::suggest_mapping(&source.headers);
        let settings = RunSettings { minimum_cluster_size: 20, ..Default::default() };
        let mut run = crate::worker::run_analysis_labeled(source, mapping, settings, LabelStyle::Descriptive, |_| {}).unwrap();
        assert!(run.clusters.iter().filter(|c| c.id != ClusterId::UNCLUSTERED).count() >= 3);
        assert_eq!(relabel(&mut run, LabelStyle::Descriptive), 0);
        assert!(relabel(&mut run, LabelStyle::Keywords) > 0);
    }

    #[test]
    fn keyword_style_keeps_the_original_titles() {
        let records = vec![incident(0, "alpha beta gamma delta epsilon")];
        let labeler = Labeler::new(&records, vec!["Alpha beta"], &LabelTermPolicy::default(), LabelStyle::Keywords);
        let label = labeler.cluster_label(ClusterId(1), &[0], &labeler.profile(&[0]), &[]);
        assert_eq!(label, "Alpha, beta, delta and epsilon related issues");
        let style: LabelStyle = serde_json::from_str(r#""keywords""#).unwrap();
        assert_eq!(style, LabelStyle::Keywords);
    }

    fn incident(source_row_index: RowIndex, analysis_text: &str) -> IncidentRecord {
        IncidentRecord {
            source_row_index,
            incident_number: format!("INC{source_row_index:03}"),
            analysis_text: analysis_text.to_owned(),
            filter_values: FilterValues::default(),
            parsed_date: None,
        }
    }
}
