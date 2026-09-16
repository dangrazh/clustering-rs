mod common;
use chrono::NaiveDate;
use incident_clustering_analyzer::{session::*, workflow::*};

fn reviewed(run: &incident_clustering_analyzer::model::AnalysisRun) -> ReviewData {
    let mut review = ReviewData::new(run).unwrap();
    for action in [
        Action::Transition {
            status: ReviewStatus::Reviewed,
            assignments: Assignments::default(),
        },
        Action::AddComment {
            text: "Unicode café 中文 🛠️\n<script>plain text</script>".into(),
        },
    ] {
        review
            .annotations
            .mutate(Mutation {
                revision: review.annotations.revision,
                target: "1".into(),
                actor: Actor {
                    name: "A Reviewer".into(),
                    email: "a@example.org".into(),
                },
                action,
            })
            .unwrap();
    }
    for (target, label) in [("1", "Renamed cluster"), ("1:1", "Renamed theme")] {
        review
            .annotations
            .mutate(Mutation {
                revision: review.annotations.revision,
                target: target.into(),
                actor: Actor {
                    name: "Reviewer".into(),
                    email: "reviewer@example.org".into(),
                },
                action: Action::Rename {
                    label: label.into(),
                },
            })
            .unwrap();
    }
    review
}
#[test]
fn binary_session_and_review_round_trip() {
    let mut run = common::run(50);
    run.processed_incidents[0].parsed_date = NaiveDate::from_ymd_opt(2020, 2, 29);
    run.timings.clustering_ms = u64::MAX as u128 + 1;
    let review = reviewed(&run);
    let view = ViewState {
        version: 2,
        selection: Some(Selection {
            r#type: "theme".into(),
            cluster: Some(1),
            theme: Some(1),
        }),
        expanded_clusters: vec!["1".into()],
        detail_column_filters: vec![ColumnFilter {
            selected: Some(vec!["INC00000000".into()]),
            query: "INC".into(),
            search_deselected: false,
        }],
        detail_sort: Some(Sort {
            column: 0,
            direction: "desc".into(),
        }),
        pivot_rows: vec![2],
        detail_drilldown_row_indices: Some(vec![0, 2]),
        workflow_states: vec![ReviewStatus::Reviewed],
        ..Default::default()
    };
    let bytes = encode_session(&run, &review, &view).unwrap();
    let loaded = decode_session(&bytes).unwrap();
    assert_eq!(loaded.run, run);
    assert_eq!(loaded.review.annotations, review.annotations);
    assert_eq!(loaded.review.analysis_id, review.analysis_id);
    assert_eq!(loaded.view, view);
    let restored = decode_review(&encode_review(&review).unwrap()).unwrap();
    assert_eq!(restored.annotations, review.annotations);
    restored.matches(&review).unwrap();
    assert!(decode_review(&bytes).is_err());
    assert!(decode_session(&encode_review(&review).unwrap()).is_err());
    assert!(restored.matches(&ReviewData::new(&run).unwrap()).is_err());
}
#[test]
fn invalid_files_are_rejected() {
    let run = common::run(10);
    let review = reviewed(&run);
    let bytes = encode_session(&run, &review, &ViewState::default()).unwrap();
    assert!(decode_session(br#"{"version":2,"run":{}}"#).is_err());
    for length in [0, 8, 51, bytes.len() - 1] {
        assert!(decode_session(&bytes[..length]).is_err());
    }
    for index in [8, 10, 11, 12, 20, bytes.len() - 3] {
        let mut bad = bytes.clone();
        bad[index] ^= 0xff;
        assert!(decode_session(&bad).is_err(), "byte {index}");
    }
    let mut bad = run.clone();
    bad.clusters[0].incident_row_indices.push(999);
    let bytes =
        encode_session(&bad, &ReviewData::new(&bad).unwrap(), &ViewState::default()).unwrap();
    assert!(decode_session(&bytes).is_err());
    let mut bad = review.clone();
    bad.annotations.entries.get_mut("1").unwrap().history[0].target = "99".into();
    assert!(decode_session(&encode_session(&run, &bad, &ViewState::default()).unwrap()).is_err());
}
#[test]
fn mapping_profiles_and_invalid_optional_view_references() {
    let run = common::run(2);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mapping.json");
    save_mapping_profile(&file, &run.mapping).unwrap();
    assert_eq!(load_mapping_profile(&file).unwrap(), run.mapping);
    let mut view = ViewState {
        pivot_rows: vec![999],
        expanded_clusters: vec!["999".into()],
        detail_drilldown_row_indices: Some(vec![0, 999]),
        ..Default::default()
    };
    view.sanitize(&run);
    assert!(view.pivot_rows.is_empty());
    assert!(view.expanded_clusters.is_empty());
    assert_eq!(view.detail_drilldown_row_indices, Some(vec![0]));
}
