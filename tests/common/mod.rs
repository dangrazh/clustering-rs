#![allow(dead_code)]
use incident_clustering_analyzer::model::*;
pub fn run(count: usize) -> AnalysisRun {
    let rows:Vec<_>=(0..count).map(|i|vec![format!("INC{i:08}"),format!("Service {} connectivity issue on device {:08x}; office {}. Connection unavailable after update {}.",i%37,i.wrapping_mul(2654435761),i%19,i%113),if i%2==0{"Open".into()}else{"Closed".into()}]).collect();
    let records = rows
        .iter()
        .enumerate()
        .map(|(i, r)| IncidentRecord {
            source_row_index: i,
            incident_number: r[0].clone(),
            analysis_text: r[1].clone(),
            filter_values: FilterValues::default(),
            parsed_date: None,
        })
        .collect();
    let clusters = (0..count.div_ceil(1000))
        .map(|i| {
            let start = i * 1000;
            let end = ((i + 1) * 1000).min(count);
            Cluster {
                id: ClusterId(i + 1),
                label: format!("Connectivity group {}", i + 1),
                incident_row_indices: (start..end).collect(),
                subgroups: vec![
                    Subgroup {
                        id: 1,
                        label: "Network connection".into(),
                        incident_row_indices: (start..end).filter(|r| r % 2 == 0).collect(),
                    },
                    Subgroup {
                        id: 2,
                        label: "Device connection".into(),
                        incident_row_indices: (start..end).filter(|r| r % 2 == 1).collect(),
                    },
                ],
            }
        })
        .collect();
    AnalysisRun {
        source: SourceTable {
            source_path: None,
            worksheet_name: None,
            headers: vec!["Number".into(), "Short Description".into(), "Status".into()],
            rows,
        },
        mapping: ColumnMapping {
            incident_number: Some(0),
            short_description: Some(1),
            ..Default::default()
        },
        settings: RunSettings::default(),
        processed_incidents: records,
        ignored_rows: vec![],
        clusters,
        unclustered_row_indices: vec![],
        timings: TimingMetrics::default(),
    }
}
