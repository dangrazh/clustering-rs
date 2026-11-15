//! Row text stays on the server. The browser holds cluster membership and asks here for the
//! rows matching its column filters, the sort order of a column, the text of one page and
//! the values offered by a filter menu.
use crate::columns::{ColumnType, DataType};
use crate::model::{AnalysisRun, Cluster, ClusterId, IgnoredRow, RowIndex, RunSettings, TimingMetrics};
use anyhow::{ensure, Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

/// Rows a single page request may ask for; the largest page size the browser offers fits.
const MAX_PAGE_ROWS: usize = 1_000;

/// Everything about an analysis except the text of its rows.
#[derive(Serialize)]
pub struct Summary<'a> {
    column_types: &'a [ColumnType],
    source: SourceSummary<'a>,
    mapping: &'a crate::model::ColumnMapping,
    settings: &'a RunSettings,
    /// Source row of every processed incident, in processing order.
    processed_row_indices: Vec<RowIndex>,
    ignored_rows: &'a [IgnoredRow],
    clusters: &'a [Cluster],
    unclustered_row_indices: &'a [RowIndex],
    timings: &'a TimingMetrics,
    /// Trailing columns joined from static data; column types cover only the rest.
    static_columns: usize,
}

#[derive(Serialize)]
struct SourceSummary<'a> {
    source_path: &'a Option<std::path::PathBuf>,
    worksheet_name: &'a Option<String>,
    headers: &'a [String],
    row_count: usize,
}

pub fn summary(run: &AnalysisRun, static_columns: usize) -> Summary<'_> {
    Summary {
        column_types: &run.column_types,
        source: SourceSummary {
            source_path: &run.source.source_path,
            worksheet_name: &run.source.worksheet_name,
            headers: &run.source.headers,
            row_count: run.source.rows.len(),
        },
        mapping: &run.mapping,
        settings: &run.settings,
        processed_row_indices: run.processed_incidents.iter().map(|r| r.source_row_index).collect(),
        ignored_rows: &run.ignored_rows,
        clusters: &run.clusters,
        unclustered_row_indices: &run.unclustered_row_indices,
        timings: &run.timings,
        static_columns,
    }
}

/// A column restricted to the selected values, compared the way the filter menu shows them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnFilter {
    column: usize,
    selected: Vec<String>,
    #[serde(default)]
    mode: DateMode,
    #[serde(default)]
    column_type: Option<ColumnType>,
}

/// How a date/time column is grouped in its filter: as written, by day or by month.
#[derive(Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DateMode {
    #[default]
    Exact,
    Date,
    Month,
}

#[derive(Deserialize)]
#[serde(tag = "query", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Query {
    /// Processed rows passing every column filter.
    Match { filters: Vec<ColumnFilter> },
    /// Sort rank of every processed row for one column.
    Order { column: usize },
    /// Text of the given rows.
    Page { row_indices: Vec<RowIndex> },
    /// Values a column takes among the selected cluster's or theme's rows that pass the
    /// other filters.
    Values {
        column: usize,
        #[serde(default)]
        mode: DateMode,
        #[serde(default)]
        column_type: Option<ColumnType>,
        #[serde(default)]
        filters: Vec<ColumnFilter>,
        #[serde(default)]
        cluster: Option<usize>,
        #[serde(default)]
        theme: Option<usize>,
    },
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
pub enum Answer<'a> {
    Match {
        row_indices: Vec<RowIndex>,
    },
    /// Aligned with the processed rows; rows with equal values share a rank.
    Order {
        ranks: Vec<u32>,
    },
    /// Aligned with the request; `None` for a row the analysis does not have.
    Page {
        rows: Vec<Option<&'a [String]>>,
    },
    Values {
        values: Vec<String>,
    },
}

pub fn answer(run: &AnalysisRun, query: Query) -> Result<Answer<'_>> {
    let columns = run.source.headers.len();
    let check = |column: usize| {
        ensure!(column < columns, "Column {column} is outside the analysis");
        Ok(())
    };
    let processed = || run.processed_incidents.iter().map(|record| record.source_row_index);
    Ok(match query {
        Query::Match { filters } => {
            let filters = prepare(filters, columns)?;
            Answer::Match {
                row_indices: processed().filter(|row| passes(run, *row, &filters)).collect(),
            }
        }
        Query::Order { column } => {
            check(column)?;
            Answer::Order { ranks: ranks(run, column) }
        }
        Query::Page { row_indices } => {
            ensure!(row_indices.len() <= MAX_PAGE_ROWS, "A page holds at most {MAX_PAGE_ROWS} rows");
            Answer::Page {
                rows: row_indices
                    .iter()
                    .map(|row| run.source.rows.get(*row).map(Vec::as_slice))
                    .collect(),
            }
        }
        Query::Values { column, mode, column_type, filters, cluster, theme } => {
            check(column)?;
            let filters = prepare(filters, columns)?;
            let selected = selection(run, cluster, theme)?;
            let rows: Box<dyn Iterator<Item = RowIndex> + '_> = match selected {
                Some(rows) => Box::new(rows.iter().copied()),
                None => Box::new(processed()),
            };
            let values = rows
                .filter(|row| passes(run, *row, &filters))
                .map(|row| filter_value(cell(run, row, column), column_type.as_ref(), mode))
                .collect::<BTreeSet<_>>();
            let mut values = values.into_iter().map(|value| (SortKey::new(&value), value)).collect::<Vec<_>>();
            values.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            Answer::Values { values: values.into_iter().map(|(_, value)| value).collect() }
        }
    })
}

/// Rows of the selected cluster or theme; `None` selects every processed row.
fn selection(run: &AnalysisRun, cluster: Option<usize>, theme: Option<usize>) -> Result<Option<&[RowIndex]>> {
    let Some(cluster) = cluster else { return Ok(None) };
    let cluster = run
        .clusters
        .iter()
        .find(|candidate| candidate.id == ClusterId(cluster))
        .context("Cluster not found")?;
    Ok(Some(match theme {
        None => &cluster.incident_row_indices,
        Some(theme) => {
            &cluster
                .subgroups
                .iter()
                .find(|candidate| candidate.id == theme)
                .context("Theme not found")?
                .incident_row_indices
        }
    }))
}

struct PreparedFilter {
    column: usize,
    selected: HashSet<String>,
    mode: DateMode,
    column_type: Option<ColumnType>,
}

fn prepare(filters: Vec<ColumnFilter>, columns: usize) -> Result<Vec<PreparedFilter>> {
    filters
        .into_iter()
        .map(|filter| {
            ensure!(filter.column < columns, "Column {} is outside the analysis", filter.column);
            Ok(PreparedFilter {
                column: filter.column,
                selected: filter.selected.into_iter().collect(),
                mode: filter.mode,
                column_type: filter.column_type,
            })
        })
        .collect()
}

fn cell(run: &AnalysisRun, row: RowIndex, column: usize) -> &str {
    run.source
        .rows
        .get(row)
        .and_then(|cells| cells.get(column))
        .map_or("", String::as_str)
}

fn passes(run: &AnalysisRun, row: RowIndex, filters: &[PreparedFilter]) -> bool {
    filters.iter().all(|filter| {
        let value = cell(run, row, filter.column);
        if filter.mode == DateMode::Exact {
            filter.selected.contains(value)
        } else {
            filter.selected.contains(&filter_value(value, filter.column_type.as_ref(), filter.mode))
        }
    })
}

/// The value a cell is filtered by: itself, or its day or month in a date/time column.
/// Text that is not a date stays available as it is written.
fn filter_value(value: &str, column_type: Option<&ColumnType>, mode: DateMode) -> String {
    let date = match column_type {
        Some(kind) if kind.data_type == DataType::DateTime && mode != DateMode::Exact => {
            crate::columns::date(value, kind.date_order)
        }
        _ => None,
    };
    match (date, mode) {
        (Some(date), DateMode::Month) => date[..7].to_owned(),
        (Some(date), _) => date,
        (None, _) => value.to_owned(),
    }
}

fn ranks(run: &AnalysisRun, column: usize) -> Vec<u32> {
    let keys = run
        .processed_incidents
        .par_iter()
        .map(|record| SortKey::new(cell(run, record.source_row_index, column)))
        .collect::<Vec<_>>();
    let mut order = (0..keys.len()).collect::<Vec<_>>();
    order.par_sort_unstable_by(|a, b| keys[*a].cmp(&keys[*b]).then_with(|| a.cmp(b)));
    let mut ranks = vec![0u32; keys.len()];
    let mut rank = 0;
    for pair in 0..order.len() {
        if pair > 0 && keys[order[pair]] != keys[order[pair - 1]] {
            rank += 1;
        }
        ranks[order[pair]] = rank;
    }
    ranks
}

/// Orders cell values: blanks, then numbers by value, then text without regard to case or
/// accents, with runs of digits compared as numbers ("item 9" before "item 10").
#[derive(Debug, PartialEq)]
enum SortKey {
    Blank,
    Number(f64),
    Text(String),
}

impl SortKey {
    fn new(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty() {
            return Self::Blank;
        }
        match value.parse::<f64>() {
            Ok(number) if number.is_finite() => Self::Number(number),
            _ => Self::Text(
                value
                    .nfkd()
                    .filter(|c| !is_combining_mark(*c))
                    .flat_map(char::to_lowercase)
                    .collect(),
            ),
        }
    }

    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Blank, Self::Blank) => Ordering::Equal,
            (Self::Blank, _) => Ordering::Less,
            (_, Self::Blank) => Ordering::Greater,
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Number(_), Self::Text(_)) => Ordering::Less,
            (Self::Text(_), Self::Number(_)) => Ordering::Greater,
            (Self::Text(a), Self::Text(b)) => natural(a, b),
        }
    }
}

fn natural(left: &str, right: &str) -> Ordering {
    let (mut a, mut b) = (left.chars().peekable(), right.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let digits = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut run = String::new();
                    while let Some(c) = chars.next_if(char::is_ascii_digit) {
                        run.push(c);
                    }
                    run.trim_start_matches('0').to_owned()
                };
                let (x, y) = (digits(&mut a), digits(&mut b));
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(&y));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columns::DateOrder;
    use crate::model::{ColumnMapping, FilterValues, IncidentRecord, SourceTable, Subgroup};

    /// Six rows; row 4 was ignored, so five are processed. Cluster 1 holds rows 0–2.
    fn run() -> AnalysisRun {
        let rows = [
            ["INC1", "Open", "Zürich", "03/04/2026 10:00", "item 10"],
            ["INC2", "Closed", "zurich", "04/04/2026 11:00", "Item 9"],
            ["INC3", "Open", "Bern", "15/05/2026 12:00", "12"],
            ["INC4", "Open", "Zürich", "not a date", "2.5"],
            ["", "Open", "Bern", "", ""],
            ["INC6", "Closed", "Aarau", "16/05/2026 09:00", ""],
        ];
        let record = |row: usize| IncidentRecord {
            source_row_index: row,
            incident_number: format!("INC{}", row + 1),
            analysis_text: String::new(),
            filter_values: FilterValues::default(),
            parsed_date: None,
        };
        AnalysisRun {
            column_types: vec![],
            source: SourceTable {
                source_path: None,
                worksheet_name: None,
                headers: ["Number", "State", "Site", "Opened", "Note"].map(String::from).to_vec(),
                rows: rows.iter().map(|row| row.map(String::from).to_vec()).collect(),
            },
            mapping: ColumnMapping::default(),
            settings: RunSettings::default(),
            processed_incidents: [0, 1, 2, 3, 5].map(record).to_vec(),
            ignored_rows: vec![],
            clusters: vec![Cluster {
                id: ClusterId(1),
                label: "Cluster".into(),
                incident_row_indices: vec![0, 1, 2],
                subgroups: vec![Subgroup { id: 2, label: "Theme".into(), incident_row_indices: vec![1, 2] }],
            }],
            unclustered_row_indices: vec![3, 5],
            timings: TimingMetrics::default(),
        }
    }

    fn ask(run: &AnalysisRun, query: serde_json::Value) -> serde_json::Value {
        serde_json::to_value(answer(run, serde_json::from_value(query).unwrap()).unwrap()).unwrap()
    }

    #[test]
    fn summary_describes_the_analysis_without_row_text() {
        let run = run();
        let summary = serde_json::to_value(summary(&run, 1)).unwrap();
        assert_eq!(summary["processed_row_indices"], serde_json::json!([0, 1, 2, 3, 5]));
        assert_eq!(summary["source"]["row_count"], 6);
        assert_eq!(summary["source"]["headers"][2], "Site");
        assert_eq!(summary["clusters"][0]["subgroups"][0]["incident_row_indices"], serde_json::json!([1, 2]));
        assert_eq!(summary["static_columns"], 1);
        assert!(summary["source"].get("rows").is_none());
        assert!(!summary.to_string().contains("Zürich"));
    }

    #[test]
    fn matching_rows_pass_every_filter_and_exclude_ignored_rows() {
        let run = run();
        let open = serde_json::json!({"column": 1, "selected": ["Open"]});
        assert_eq!(
            ask(&run, serde_json::json!({"query": "match", "filters": [open]}))["rowIndices"],
            serde_json::json!([0, 2, 3])
        );
        let site = serde_json::json!({"column": 2, "selected": ["Bern", "Zürich"]});
        assert_eq!(
            ask(&run, serde_json::json!({"query": "match", "filters": [open, site]}))["rowIndices"],
            serde_json::json!([0, 2, 3])
        );
        assert_eq!(
            ask(&run, serde_json::json!({"query": "match", "filters": [{"column": 4, "selected": [""]}]}))["rowIndices"],
            serde_json::json!([5])
        );
        assert_eq!(ask(&run, serde_json::json!({"query": "match", "filters": []}))["rowIndices"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn date_columns_filter_by_day_or_month_and_keep_other_text() {
        let run = run();
        let kind = serde_json::json!({"dataType": "Date/Time", "dateOrder": "DMY"});
        let filter = |mode: &str, selected: serde_json::Value| {
            serde_json::json!({"query": "match", "filters": [{"column": 3, "selected": selected, "mode": mode, "columnType": kind}]})
        };
        assert_eq!(ask(&run, filter("month", serde_json::json!(["2026-04"])))["rowIndices"], serde_json::json!([0, 1]));
        assert_eq!(ask(&run, filter("date", serde_json::json!(["2026-05-15", "not a date"])))["rowIndices"], serde_json::json!([2, 3]));
        assert_eq!(ask(&run, filter("exact", serde_json::json!(["03/04/2026 10:00"])))["rowIndices"], serde_json::json!([0]));
        let values = ask(&run, serde_json::json!({"query": "values", "column": 3, "mode": "month", "columnType": kind}));
        assert_eq!(values["values"], serde_json::json!(["2026-04", "2026-05", "not a date"]));
        assert_eq!(filter_value("03/04/2026", Some(&ColumnType { data_type: DataType::DateTime, date_order: DateOrder::MDY }), DateMode::Date), "2026-03-04");
    }

    #[test]
    fn filter_values_come_from_the_selection_and_the_other_filters() {
        let run = run();
        let values = |query: serde_json::Value| ask(&run, query)["values"].clone();
        assert_eq!(values(serde_json::json!({"query": "values", "column": 2})), serde_json::json!(["Aarau", "Bern", "Zürich", "zurich"]));
        assert_eq!(values(serde_json::json!({"query": "values", "column": 2, "cluster": 1})), serde_json::json!(["Bern", "Zürich", "zurich"]));
        assert_eq!(values(serde_json::json!({"query": "values", "column": 2, "cluster": 1, "theme": 2})), serde_json::json!(["Bern", "zurich"]));
        let open = serde_json::json!([{"column": 1, "selected": ["Open"]}]);
        assert_eq!(values(serde_json::json!({"query": "values", "column": 2, "cluster": 1, "filters": open})), serde_json::json!(["Bern", "Zürich"]));
        assert!(answer(&run, serde_json::from_value(serde_json::json!({"query": "values", "column": 2, "cluster": 9})).unwrap()).is_err());
    }

    #[test]
    fn ranks_order_blanks_numbers_then_text_naturally() {
        let run = run();
        // Notes of the processed rows: "item 10", "Item 9", "12", "2.5", "".
        assert_eq!(ask(&run, serde_json::json!({"query": "order", "column": 4}))["ranks"], serde_json::json!([4, 3, 2, 1, 0]));
        // Sites: Zürich, zurich, Bern, Zürich, Aarau: case and accents do not separate values.
        assert_eq!(ask(&run, serde_json::json!({"query": "order", "column": 2}))["ranks"], serde_json::json!([2, 2, 1, 2, 0]));
        assert!(answer(&run, serde_json::from_value(serde_json::json!({"query": "order", "column": 5})).unwrap()).is_err());
    }

    #[test]
    fn pages_return_the_requested_rows_in_order() {
        let run = run();
        let page = ask(&run, serde_json::json!({"query": "page", "rowIndices": [5, 0, 99]}));
        assert_eq!(page["rows"][0][2], "Aarau");
        assert_eq!(page["rows"][1][0], "INC1");
        assert!(page["rows"][2].is_null());
        let too_many = serde_json::json!({"query": "page", "rowIndices": vec![0; MAX_PAGE_ROWS + 1]});
        assert!(answer(&run, serde_json::from_value(too_many).unwrap()).is_err());
    }
}
