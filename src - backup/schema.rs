use crate::model::{
    ColumnIndex, ColumnMapping, FilterValues, IgnoredRow, IncidentRecord, SourceTable,
};
use crate::progress::{ParallelProgressSpec, ProgressReporter};
use chrono::NaiveDate;
use rayon::prelude::*;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MappingError {
    #[error("incident number column is required")]
    MissingIncidentNumber,
    #[error("short description column is required")]
    MissingShortDescription,
    #[error("column index {0} is outside the source header range")]
    ColumnOutOfRange(ColumnIndex),
}

pub fn suggest_mapping(headers: &[String]) -> ColumnMapping {
    let mut mapping = ColumnMapping::default();

    for (index, header) in headers.iter().enumerate() {
        let normalized = normalize_header(header);
        match normalized.as_str() {
            "incnumber" | "incidentnumber" | "number" => mapping.incident_number = Some(index),
            "incshortdescription" | "shortdescription" | "summary" => {
                mapping.short_description = Some(index)
            }
            "category" => mapping.category = Some(index),
            "priority" => {}
            "assignmentgroup" | "assignedgroup" => mapping.assignment_group = Some(index),
            "service" | "businessservice" => mapping.service = Some(index),
            "configurationitem" | "ci" | "cmdbci" => mapping.configuration_item = Some(index),
            "created" | "createddate" | "opened" | "openedat" | "date" => {
                mapping.date = Some(index)
            }
            _ => {}
        }
    }

    mapping
}

pub fn validate_mapping(mapping: &ColumnMapping, table: &SourceTable) -> Result<(), MappingError> {
    let Some(incident_number) = mapping.incident_number else {
        return Err(MappingError::MissingIncidentNumber);
    };
    let Some(short_description) = mapping.short_description else {
        return Err(MappingError::MissingShortDescription);
    };

    let mut indices = vec![incident_number, short_description];
    indices.extend(mapping.additional_text.iter().copied());
    indices.extend(
        [
            mapping.assignment_group,
            mapping.service,
            mapping.category,
            mapping.configuration_item,
            mapping.date,
        ]
        .into_iter()
        .flatten(),
    );

    let column_count = table.headers.len();
    if let Some(index) = indices.into_iter().find(|index| *index >= column_count) {
        return Err(MappingError::ColumnOutOfRange(index));
    }

    Ok(())
}

pub fn build_records(
    table: &SourceTable,
    mapping: &ColumnMapping,
    reporter: Option<&ProgressReporter>,
) -> Result<(Vec<IncidentRecord>, Vec<IgnoredRow>), MappingError> {
    validate_mapping(mapping, table)?;
    let incident_number_index = mapping.incident_number.expect("validated incident number");
    let short_description_index = mapping
        .short_description
        .expect("validated short description");

    if let Some(reporter) = reporter {
        reporter.substep(
            2,
            8,
            "Building incident records",
            1,
            3,
            "Preparing row builders",
            format!("{} source rows queued", table.rows.len()),
        );
    }

    let row_tracker = reporter.map(|reporter| {
        reporter.parallel_substep(ParallelProgressSpec {
            step: 2,
            total_steps: 8,
            stage: "Building incident records".to_owned(),
            substep_current: 2,
            substep_total: 3,
            substep_label: "Processing source rows".to_owned(),
            detail: "Checking mandatory fields and composing incident text".to_owned(),
            total_units: table.rows.len(),
            unit_label: "rows processed".to_owned(),
        })
    });

    let row_results = table
        .rows
        .par_iter()
        .enumerate()
        .map(|(source_row_index, row)| {
            let result = build_row_result(
                source_row_index,
                row,
                mapping,
                incident_number_index,
                short_description_index,
            );
            if let Some(tracker) = &row_tracker {
                tracker.advance(1);
            }
            result
        })
        .collect::<Vec<_>>();

    if let Some(tracker) = &row_tracker {
        tracker.finish();
    }

    if let Some(reporter) = reporter {
        reporter.substep(
            2,
            8,
            "Building incident records",
            3,
            3,
            "Separating valid and ignored rows",
            "Consolidating parallel row-build output",
        );
    }

    let mut records = Vec::with_capacity(table.rows.len());
    let mut ignored = Vec::new();
    for result in row_results {
        match result {
            RowBuildResult::Record(record) => records.push(record),
            RowBuildResult::Ignored(row) => ignored.push(row),
        }
    }

    Ok((records, ignored))
}

enum RowBuildResult {
    Record(IncidentRecord),
    Ignored(IgnoredRow),
}

fn build_row_result(
    source_row_index: usize,
    row: &[String],
    mapping: &ColumnMapping,
    incident_number_index: ColumnIndex,
    short_description_index: ColumnIndex,
) -> RowBuildResult {
    let incident_number = cell(row, incident_number_index).trim();
    let short_description = cell(row, short_description_index).trim();
    let missing_incident_number = incident_number.is_empty();
    let missing_short_description = short_description.is_empty();

    if missing_incident_number || missing_short_description {
        return RowBuildResult::Ignored(IgnoredRow {
            source_row_index,
            missing_incident_number,
            missing_short_description,
        });
    }

    let mut text_parts = Vec::with_capacity(1 + mapping.additional_text.len());
    text_parts.push(short_description.to_owned());
    text_parts.extend(
        mapping
            .additional_text
            .iter()
            .map(|index| cell(row, *index).trim())
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
    );

    RowBuildResult::Record(IncidentRecord {
        source_row_index,
        incident_number: incident_number.to_owned(),
        analysis_text: text_parts.join(" "),
        filter_values: FilterValues {
            assignment_group: optional_cell(row, mapping.assignment_group),
            service: optional_cell(row, mapping.service),
            category: optional_cell(row, mapping.category),
            configuration_item: optional_cell(row, mapping.configuration_item),
        },
        parsed_date: optional_cell(row, mapping.date).and_then(|value| parse_date(&value)),
    })
}

fn cell(row: &[String], index: ColumnIndex) -> &str {
    row.get(index).map(String::as_str).unwrap_or_default()
}

fn optional_cell(row: &[String], index: Option<ColumnIndex>) -> Option<String> {
    index
        .and_then(|index| row.get(index))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn normalize_header(header: &str) -> String {
    header
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    let value = value.trim();
    ["%Y-%m-%d", "%d.%m.%Y", "%d/%m/%Y", "%m/%d/%Y"]
        .into_iter()
        .find_map(|format| NaiveDate::parse_from_str(value, format).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_common_columns() {
        let headers = vec![
            "INC Number".to_owned(),
            "INC Short Description".to_owned(),
            "Category".to_owned(),
            "Assignment Group".to_owned(),
        ];

        let mapping = suggest_mapping(&headers);

        assert_eq!(mapping.incident_number, Some(0));
        assert_eq!(mapping.short_description, Some(1));
        assert_eq!(mapping.category, Some(2));
        assert_eq!(mapping.assignment_group, Some(3));
    }

    #[test]
    fn ignores_rows_missing_required_fields() {
        let table = SourceTable {
            source_path: None,
            worksheet_name: None,
            headers: vec!["INC Number".to_owned(), "INC Short Description".to_owned()],
            rows: vec![
                vec!["INC001".to_owned(), "VPN is down".to_owned()],
                vec!["".to_owned(), "Missing number".to_owned()],
                vec!["INC003".to_owned(), "".to_owned()],
            ],
        };
        let mapping = suggest_mapping(&table.headers);

        let (records, ignored) = build_records(&table, &mapping, None).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(ignored.len(), 2);
    }
}
