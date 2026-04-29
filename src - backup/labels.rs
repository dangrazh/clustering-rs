use crate::model::{ClusterId, IncidentRecord, RowIndex};
use crate::text::tokenize;
use std::collections::{HashMap, HashSet};

pub fn summarize_cluster(
    id: ClusterId,
    row_indices: &[RowIndex],
    records: &[IncidentRecord],
) -> String {
    let keywords = representative_keywords(row_indices, records, 4);
    match keywords.as_slice() {
        [] => format!("Similar incidents in cluster {}", id.0),
        [single] => sentence_case(single),
        [first, second] => sentence_case(&format!("{first} related to {second}")),
        [first, second, third, ..] => {
            sentence_case(&format!("{first} issues involving {second} and {third}"))
        }
    }
}

pub fn representative_keywords(
    row_indices: &[RowIndex],
    records: &[IncidentRecord],
    limit: usize,
) -> Vec<String> {
    let rows = row_indices.iter().copied().collect::<HashSet<_>>();
    let mut counts = HashMap::<String, usize>::new();

    for record in records
        .iter()
        .filter(|record| rows.contains(&record.source_row_index))
    {
        for token in tokenize(&record.analysis_text) {
            if token.len() >= 3 {
                *counts.entry(token).or_default() += 1;
            }
        }
    }

    let mut terms = counts.into_iter().collect::<Vec<_>>();
    terms.sort_by(|(left_term, left_count), (right_term, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_term.cmp(right_term))
    });

    terms
        .into_iter()
        .take(limit)
        .map(|(term, _)| term)
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

        let label = summarize_cluster(ClusterId(1), &[0], &records);

        assert!(!label.is_empty());
        assert!(label.chars().next().is_some_and(char::is_uppercase));
    }
}
