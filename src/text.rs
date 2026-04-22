use crate::model::{IncidentRecord, RowIndex, TermId, TextFeatures};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Maps term strings to unique integer IDs for compact representation.
#[derive(Debug)]
pub struct TermIndex {
    map: HashMap<String, TermId>,
    next_id: TermId,
}

impl TermIndex {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            next_id: 0,
        }
    }

    fn get_or_insert(&mut self, term: String) -> TermId {
        let next = &mut self.next_id;
        *self.map.entry(term).or_insert_with(|| {
            let id = *next;
            *next += 1;
            id
        })
    }
}

/// Extracts sparse TF-IDF feature vectors for all records.
///
/// Returns the feature vectors and the term index used to map term strings to IDs.
pub fn extract_features(records: &[IncidentRecord]) -> (Vec<TextFeatures>, TermIndex) {
    // Step 1: tokenize all documents in parallel (produces String terms)
    let document_tokens: Vec<(RowIndex, Vec<String>)> = records
        .par_iter()
        .map(|record| {
            (
                record.source_row_index,
                feature_terms(&record.analysis_text),
            )
        })
        .collect();

    // Step 2: build term index (sequential — populates the ID map)
    let mut term_index = TermIndex::new();
    let document_term_ids: Vec<(RowIndex, Vec<TermId>)> = document_tokens
        .into_iter()
        .map(|(row_index, terms)| {
            let ids = terms
                .into_iter()
                .map(|term| term_index.get_or_insert(term))
                .collect();
            (row_index, ids)
        })
        .collect();
    // Step 3: parallel document-frequency counting
    let document_frequency: HashMap<TermId, usize> = document_term_ids
        .par_iter()
        .fold(
            HashMap::new,
            |mut acc: HashMap<TermId, usize>, (_, ids)| {
                let unique: HashSet<TermId> = ids.iter().copied().collect();
                for id in unique {
                    *acc.entry(id).or_default() += 1;
                }
                acc
            },
        )
        .reduce(HashMap::new, |mut a, b| {
            for (id, count) in b {
                *a.entry(id).or_default() += count;
            }
            a
        });

    // Step 4: parallel TF-IDF weight computation
    let document_count = document_term_ids.len().max(1) as f32;
    let features = document_term_ids
        .into_par_iter()
        .map(|(row_index, ids)| {
            let mut term_counts = HashMap::<TermId, usize>::new();
            for id in ids {
                *term_counts.entry(id).or_default() += 1;
            }

            let terms = term_counts
                .into_iter()
                .map(|(id, count)| {
                    let df = *document_frequency.get(&id).unwrap_or(&1) as f32;
                    let idf = (document_count / df).ln_1p();
                    (id, count as f32 * idf)
                })
                .collect::<BTreeMap<_, _>>();

            TextFeatures { row_index, terms }
        })
        .collect();

    (features, term_index)
}

pub fn normalize_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut previous_space = true;

    for ch in input.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            output.push(ch);
            previous_space = false;
        } else if !previous_space {
            output.push(' ');
            previous_space = true;
        }
    }

    output.trim().to_owned()
}

pub fn tokenize(input: &str) -> Vec<String> {
    normalize_text(input)
        .split_whitespace()
        .filter(|token| token.len() > 1)
        .filter(|token| !is_stopword(token))
        .map(str::to_owned)
        .collect()
}

pub fn feature_terms(input: &str) -> Vec<String> {
    let tokens = tokenize(input);
    let mut terms = Vec::with_capacity(tokens.len() * 2);

    terms.extend(tokens.iter().cloned());
    terms.extend(
        tokens
            .windows(2)
            .map(|window| format!("{} {}", window[0], window[1])),
    );

    for token in &tokens {
        if token.chars().count() >= 5 {
            let chars = token.chars().collect::<Vec<_>>();
            terms.extend(
                chars
                    .windows(4)
                    .map(|window| window.iter().collect::<String>())
                    .map(|ngram| format!("char:{ngram}")),
            );
        }
    }

    terms
}

fn is_stopword(token: &str) -> bool {
    STOPWORDS.contains(&token)
}

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "in", "is", "it", "of",
    "on", "or", "that", "the", "to", "was", "were", "with", "without", "not", "no", "can",
    "cannot", "user", "users", "ticket", "incident", "issue", "problem", "error", "der", "die",
    "das", "den", "dem", "ein", "eine", "und", "oder", "ist", "im", "in", "mit", "nicht", "auf",
    "zu", "von", "pour", "avec", "sans", "dans", "des", "les", "une", "est", "pas", "sur", "per",
    "con", "senza", "non", "gli", "del", "della", "che",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_mixed_punctuation() {
        assert_eq!(normalize_text("VPN_DOWN!! Für User"), "vpn down für user");
    }

    #[test]
    fn emits_word_and_character_terms() {
        let terms = feature_terms("Password reset failure");
        assert!(terms.iter().any(|term| term == "password reset"));
        assert!(terms.iter().any(|term| term == "char:pass"));
    }
}
