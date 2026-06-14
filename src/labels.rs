use crate::model::{ClusterId, IncidentRecord, LabelTermPolicy, RowIndex};
use crate::text::tokenize;
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
        [first, second, third, fourth, ..] => {
            sentence_case(&format!("{first}, {second}, {third} and {fourth} related issues"))
        }
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
    terms.sort_by(|(left_term, left_count, left_score), (right_term, right_count, right_score)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| right_count.cmp(left_count))
            .then_with(|| left_term.cmp(right_term))
    });

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
