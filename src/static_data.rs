//! Static reference data uploaded by an operator and joined onto saved analyses.
//!
//! Each upload becomes its own database table whose column names and types are inferred
//! from the spreadsheet. A table joins onto an analysis with a left outer join on one or
//! more key columns; when an analysis lacks a key column the table is skipped. Joined
//! fields are appended to the analysis at read time and never change stored artifacts.
use crate::columns::{self, ColumnType, DataType, DateOrder};
use crate::model::{AnalysisRun, SourceTable};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const MAX_ROWS: usize = 500_000;
pub const MAX_COLUMNS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    Integer,
    Real,
    Date,
    Text,
}
impl FieldType {
    pub fn sql(self) -> &'static str {
        match self {
            Self::Integer => "INTEGER",
            Self::Real => "REAL",
            Self::Date | Self::Text => "TEXT",
        }
    }
    fn column_type(self) -> ColumnType {
        ColumnType {
            data_type: match self {
                Self::Integer | Self::Real => DataType::Number,
                Self::Date => DataType::DateTime,
                Self::Text => DataType::Text,
            },
            date_order: DateOrder::DMY,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    /// Header as it appeared in the spreadsheet; shown to users.
    pub name: String,
    /// Inferred database column name.
    pub field: String,
    #[serde(rename = "type")]
    pub kind: FieldType,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyPair {
    /// Database column of the static table.
    pub field: String,
    /// Header of the analysis source column, matched without regard to case.
    pub analysis_column: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JoinConfig {
    pub keys: Vec<KeyPair>,
    /// Database columns appended to the analysis detail record.
    pub fields: Vec<String>,
}
impl JoinConfig {
    pub fn validate(&self, columns: &[Column]) -> Result<()> {
        let known: HashSet<_> = columns.iter().map(|c| c.field.as_str()).collect();
        ensure!(self.keys.len() <= 8, "Use at most 8 key fields");
        ensure!(self.fields.len() <= MAX_COLUMNS, "Too many fields");
        for key in &self.keys {
            ensure!(known.contains(key.field.as_str()), "Unknown key field {}", key.field);
            ensure!(
                !key.analysis_column.trim().is_empty() && key.analysis_column.len() <= 300,
                "Name the analysis column for key field {}",
                key.field
            );
        }
        for field in &self.fields {
            ensure!(known.contains(field.as_str()), "Unknown field {field}");
        }
        Ok(())
    }
}

/// Lower-case SQL identifier: letters, digits and underscores, never starting with a digit.
pub fn identifier(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    match out.chars().next() {
        None => String::new(),
        Some(c) if c.is_ascii_digit() => format!("c_{out}"),
        Some(_) => out,
    }
}

fn kind(values: &[&str]) -> FieldType {
    if values.is_empty() {
        return FieldType::Text;
    }
    // Leading zeros mark codes such as "00123"; they stay text so joins keep them intact.
    let plain = |v: &&str| !v.starts_with('0') || v.len() == 1 || v.starts_with("0.");
    if values.iter().all(|v| plain(v) && v.parse::<i64>().is_ok()) {
        FieldType::Integer
    } else if values
        .iter()
        .all(|v| plain(v) && v.parse::<f64>().is_ok_and(f64::is_finite))
    {
        FieldType::Real
    } else if values.iter().all(|v| columns::date(v, DateOrder::DMY).is_some()) {
        FieldType::Date
    } else {
        FieldType::Text
    }
}

/// Infers unique database column names and types from the spreadsheet's header and values.
pub fn infer(source: &SourceTable) -> Result<Vec<Column>> {
    ensure!(!source.headers.is_empty(), "The worksheet has no columns");
    ensure!(
        source.headers.len() <= MAX_COLUMNS && source.rows.len() <= MAX_ROWS,
        "Static data is limited to {MAX_COLUMNS} columns and {MAX_ROWS} rows"
    );
    let mut used = HashSet::from(["row_id".to_string()]);
    Ok(source
        .headers
        .iter()
        .enumerate()
        .map(|(i, header)| {
            let base = Some(identifier(header))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("column_{}", i + 1));
            let mut field = base.clone();
            let mut n = 2;
            while !used.insert(field.clone()) {
                field = format!("{base}_{n}");
                n += 1;
            }
            let values: Vec<_> = source
                .rows
                .iter()
                .filter_map(|r| r.get(i))
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .collect();
            Column {
                name: Some(header.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("Column {}", i + 1)),
                field,
                kind: kind(&values),
            }
        })
        .collect())
}

/// Text as stored: dates become ISO dates, numbers keep their spelling, empty cells are NULL.
pub fn stored_text(value: &str, kind: FieldType) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(match kind {
        FieldType::Date => columns::date(value, DateOrder::DMY).unwrap_or_else(|| value.into()),
        _ => value.into(),
    })
}

/// Join keys compare trimmed, case-insensitively and with collapsed inner whitespace.
pub fn key_part(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A configured table loaded for joining.
#[derive(Debug, Clone, Default)]
pub struct Lookup {
    pub table: String,
    pub key_columns: Vec<String>,
    /// Display header and type of each appended field.
    pub fields: Vec<(String, FieldType)>,
    /// Normalized key → field values. The first row wins when a key repeats.
    pub rows: HashMap<Vec<String>, Vec<String>>,
}

/// Appends each lookup's fields to the analysis with a left outer join. Lookups whose key
/// columns are not all present are skipped. Returns the number of columns added.
pub fn enrich(run: &mut AnalysisRun, lookups: &[Lookup]) -> usize {
    let base = run.source.headers.len();
    if run.column_types.len() != base {
        run.column_types = columns::infer(&run.source);
    }
    let position = |headers: &[String], name: &str| {
        let name = key_part(name);
        headers.iter().position(|h| key_part(h) == name)
    };
    for lookup in lookups {
        let Some(keys) = lookup
            .key_columns
            .iter()
            .map(|k| position(&run.source.headers[..base], k))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        if keys.is_empty() || lookup.fields.is_empty() {
            continue;
        }
        let empty = vec![String::new(); lookup.fields.len()];
        for row in &mut run.source.rows {
            let key: Vec<_> = keys
                .iter()
                .map(|&i| key_part(row.get(i).map(String::as_str).unwrap_or("")))
                .collect();
            let values = if key.iter().all(String::is_empty) {
                &empty
            } else {
                lookup.rows.get(&key).unwrap_or(&empty)
            };
            row.resize(run.source.headers.len(), String::new());
            row.extend(values.iter().cloned());
        }
        for (name, kind) in &lookup.fields {
            let taken = run.source.headers.iter().any(|h| key_part(h) == key_part(name));
            run.source.headers.push(if taken {
                format!("{} · {}", lookup.table, name)
            } else {
                name.clone()
            });
            run.column_types.push(kind.column_type());
        }
    }
    run.source.headers.len() - base
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(headers: &[&str], rows: &[&[&str]]) -> SourceTable {
        SourceTable {
            source_path: None,
            worksheet_name: None,
            headers: headers.iter().map(|s| s.to_string()).collect(),
            rows: rows
                .iter()
                .map(|r| r.iter().map(|s| s.to_string()).collect())
                .collect(),
        }
    }

    #[test]
    fn identifiers_are_safe_and_unique() {
        assert_eq!(identifier("Assignment Group"), "assignment_group");
        assert_eq!(identifier("  Cost-Center (EUR) "), "cost_center_eur");
        assert_eq!(identifier("2nd level"), "c_2nd_level");
        assert_eq!(identifier("Ünïcode!"), "n_code");
        let columns = infer(&table(&["Name", "name", "", "Row ID"], &[&["a", "b", "c", "d"]])).unwrap();
        let fields: Vec<_> = columns.iter().map(|c| c.field.as_str()).collect();
        assert_eq!(fields, ["name", "name_2", "column_3", "row_id_2"]);
        assert_eq!(columns[2].name, "Column 3");
    }

    #[test]
    fn types_are_inferred_from_values() {
        let columns = infer(&table(
            &["Count", "Rate", "Since", "Code", "Text", "Empty"],
            &[
                &["1", "1.5", "01.02.2026", "007", "x", ""],
                &["-20", "3", "2026-03-04", "8", "12", ""],
            ],
        ))
        .unwrap();
        let kinds: Vec<_> = columns.iter().map(|c| c.kind).collect();
        use FieldType::*;
        assert_eq!(kinds, [Integer, Real, Date, Text, Text, Text]);
        assert_eq!(stored_text("01.02.2026", Date).as_deref(), Some("2026-02-01"));
        assert_eq!(stored_text("  ", Text), None);
    }

    #[test]
    fn enrich_left_joins_and_skips_tables_without_their_key_column() {
        let mut run = crate::fixtures::run(3);
        run.source.headers.push("Group".into());
        for (row, group) in run.source.rows.iter_mut().zip(["Service Desk", "network ", ""]) {
            row.push(group.into());
        }
        let groups = Lookup {
            table: "Groups".into(),
            key_columns: vec!["group".into()],
            fields: vec![("Manager".into(), FieldType::Text), ("Status".into(), FieldType::Text)],
            rows: HashMap::from([
                (vec!["service desk".into()], vec!["Ana".into(), "Active".into()]),
                (vec!["network".into()], vec!["Ben".into(), "Closed".into()]),
            ]),
        };
        let missing = Lookup {
            table: "Sites".into(),
            key_columns: vec!["Site".into()],
            fields: vec![("Region".into(), FieldType::Text)],
            ..Lookup::default()
        };
        let base = run.source.headers.len();
        assert_eq!(enrich(&mut run, &[groups, missing]), 2);
        // "Status" already exists in the fixture, so the joined field is qualified.
        assert_eq!(run.source.headers[base..], ["Manager", "Groups · Status"]);
        assert_eq!(run.column_types.len(), run.source.headers.len());
        let joined: Vec<_> = run.source.rows.iter().map(|r| r[base..].to_vec()).collect();
        assert_eq!(joined, [vec!["Ana", "Active"], vec!["Ben", "Closed"], vec!["", ""]]);
    }

    #[test]
    fn join_config_rejects_unknown_fields() {
        let columns = infer(&table(&["Group", "Manager"], &[&["a", "b"]])).unwrap();
        let ok = JoinConfig {
            keys: vec![KeyPair { field: "group".into(), analysis_column: "Assignment group".into() }],
            fields: vec!["manager".into()],
        };
        assert!(ok.validate(&columns).is_ok());
        assert!(JoinConfig { fields: vec!["nope".into()], ..ok.clone() }.validate(&columns).is_err());
    }
}
