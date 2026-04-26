use crate::model::{AnalysisRun, Cluster, ClusterId, SourceTable, Subgroup};
use anyhow::{Context, Result};
use calamine::{open_workbook_auto, Data, Reader};
use rust_xlsxwriter::Workbook;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn import_source(path: impl AsRef<Path>) -> Result<SourceTable> {
    let path = path.as_ref();
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("csv") => import_csv(path),
        Some("xlsx") | Some("xlsm") | Some("xls") => {
            let sheets = list_worksheets(path)?;
            let sheet_name = sheets
                .first()
                .cloned()
                .context("workbook does not contain worksheets")?;
            import_xlsx_sheet(path, &sheet_name)
        }
        _ => anyhow::bail!("unsupported input file type"),
    }
}

pub fn list_worksheets(path: impl AsRef<Path>) -> Result<Vec<String>> {
    let workbook = open_workbook_auto(path.as_ref())
        .with_context(|| format!("failed to open workbook {}", path.as_ref().display()))?;
    Ok(workbook.sheet_names().to_vec())
}

pub fn import_csv(path: impl AsRef<Path>) -> Result<SourceTable> {
    let path = path.as_ref();
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .with_context(|| format!("failed to open CSV {}", path.display()))?;

    let headers = reader
        .headers()
        .context("failed to read CSV headers")?
        .iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();

    let rows = reader
        .records()
        .map(|record| {
            record
                .map(|record| record.iter().map(str::to_owned).collect::<Vec<_>>())
                .context("failed to read CSV record")
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(SourceTable {
        source_path: Some(path.to_path_buf()),
        worksheet_name: None,
        headers,
        rows,
    })
}

pub fn import_xlsx_sheet(path: impl AsRef<Path>, sheet_name: &str) -> Result<SourceTable> {
    let path = path.as_ref();
    let mut workbook = open_workbook_auto(path)
        .with_context(|| format!("failed to open workbook {}", path.display()))?;
    let range = workbook
        .worksheet_range(sheet_name)
        .with_context(|| format!("failed to read worksheet {sheet_name}"))?;

    let mut rows = range.rows();
    let headers = rows
        .next()
        .map(|row| row.iter().map(cell_to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    let rows = rows
        .map(|row| row.iter().map(cell_to_string).collect::<Vec<_>>())
        .collect::<Vec<_>>();

    Ok(SourceTable {
        source_path: Some(path.to_path_buf()),
        worksheet_name: Some(sheet_name.to_owned()),
        headers,
        rows,
    })
}

pub fn export_analysis(run: &AnalysisRun, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();

    let mut headers = run.source.headers.clone();
    headers.extend([
        "Cluster ID".to_owned(),
        "Cluster Label".to_owned(),
        "Cluster Size".to_owned(),
        "Theme ID".to_owned(),
        "Theme Label".to_owned(),
        "Theme Size".to_owned(),
    ]);

    for (column, header) in headers.iter().enumerate() {
        worksheet.write_string(0, column as u16, header)?;
    }

    let cluster_lookup = build_cluster_lookup(&run.clusters);
    for (export_row_index, source_row_index) in run
        .processed_incidents
        .iter()
        .map(|record| record.source_row_index)
        .enumerate()
    {
        let excel_row = (export_row_index + 1) as u32;
        let source_row = run
            .source
            .rows
            .get(source_row_index)
            .with_context(|| format!("source row {source_row_index} is missing"))?;

        for (column, value) in source_row.iter().enumerate() {
            worksheet.write_string(excel_row, column as u16, value)?;
        }

        let metadata = cluster_lookup
            .get(&source_row_index)
            .cloned()
            .unwrap_or_else(ExportRowMetadata::unclustered);

        let base_column = run.source.headers.len() as u16;
        worksheet.write_string(excel_row, base_column, &metadata.cluster_id)?;
        worksheet.write_string(excel_row, base_column + 1, &metadata.cluster_label)?;
        worksheet.write_number(excel_row, base_column + 2, metadata.cluster_size as f64)?;
        worksheet.write_string(excel_row, base_column + 3, &metadata.theme_id)?;
        worksheet.write_string(excel_row, base_column + 4, &metadata.theme_label)?;
        worksheet.write_number(excel_row, base_column + 5, metadata.theme_size as f64)?;
    }

    workbook
        .save(path)
        .with_context(|| format!("failed to save export {}", path.display()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExportRowMetadata {
    cluster_id: String,
    cluster_label: String,
    cluster_size: usize,
    theme_id: String,
    theme_label: String,
    theme_size: usize,
}

impl ExportRowMetadata {
    fn for_cluster(cluster: &Cluster, theme: Option<&Subgroup>) -> Self {
        Self {
            cluster_id: cluster.id.to_string(),
            cluster_label: cluster.label.clone(),
            cluster_size: cluster.size(),
            theme_id: theme.map(|theme| theme.id.to_string()).unwrap_or_default(),
            theme_label: theme.map(|theme| theme.label.clone()).unwrap_or_default(),
            theme_size: theme.map(Subgroup::size).unwrap_or_default(),
        }
    }

    fn unclustered() -> Self {
        Self {
            cluster_id: ClusterId::UNCLUSTERED.to_string(),
            cluster_label: "Unclustered".to_owned(),
            cluster_size: 0,
            theme_id: String::new(),
            theme_label: String::new(),
            theme_size: 0,
        }
    }
}

fn build_cluster_lookup(clusters: &[Cluster]) -> HashMap<usize, ExportRowMetadata> {
    let mut lookup = HashMap::new();

    for cluster in clusters {
        let cluster_metadata = ExportRowMetadata::for_cluster(cluster, None);
        for row_index in &cluster.incident_row_indices {
            lookup.insert(*row_index, cluster_metadata.clone());
        }

        for theme in &cluster.subgroups {
            let theme_metadata = ExportRowMetadata::for_cluster(cluster, Some(theme));
            for row_index in &theme.incident_row_indices {
                lookup.insert(*row_index, theme_metadata.clone());
            }
        }
    }

    lookup
}

fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(value) => value.clone(),
        Data::Float(value) => value.to_string(),
        Data::Int(value) => value.to_string(),
        Data::Bool(value) => value.to_string(),
        Data::DateTime(value) => value.to_string(),
        Data::DateTimeIso(value) => value.clone(),
        Data::DurationIso(value) => value.clone(),
        Data::Error(value) => value.to_string(),
    }
}

#[allow(dead_code)]
pub fn default_export_path(source_path: Option<&Path>) -> PathBuf {
    source_path
        .map(|path| {
            let mut output = path.to_path_buf();
            output.set_file_name(format!(
                "{}_clustered.xlsx",
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("incidents")
            ));
            output
        })
        .unwrap_or_else(|| PathBuf::from("clustered_incidents.xlsx"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cluster, ClusterId, Subgroup};

    #[test]
    fn cluster_lookup_includes_theme_metadata() {
        let clusters = vec![Cluster {
            id: ClusterId(7),
            label: "Network incidents".to_owned(),
            incident_row_indices: vec![1, 2, 3],
            subgroups: vec![Subgroup {
                id: 2,
                label: "Router failures".to_owned(),
                incident_row_indices: vec![2, 3],
            }],
        }];

        let lookup = build_cluster_lookup(&clusters);

        assert_eq!(
            lookup.get(&2),
            Some(&ExportRowMetadata {
                cluster_id: "C0007".to_owned(),
                cluster_label: "Network incidents".to_owned(),
                cluster_size: 3,
                theme_id: "2".to_owned(),
                theme_label: "Router failures".to_owned(),
                theme_size: 2,
            })
        );
        assert_eq!(
            lookup.get(&1),
            Some(&ExportRowMetadata {
                cluster_id: "C0007".to_owned(),
                cluster_label: "Network incidents".to_owned(),
                cluster_size: 3,
                theme_id: String::new(),
                theme_label: String::new(),
                theme_size: 0,
            })
        );
    }
}
