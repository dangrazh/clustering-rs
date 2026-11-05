//! Shared interpretation metadata; never changes source values or clustering identity.
use crate::model::SourceTable;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    #[default]
    Text,
    Number,
    #[serde(rename = "Date/Time")]
    DateTime,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
// Append variants: their order is part of the portable session representation.
pub enum DateOrder {
    #[default]
    DMY,
    MDY,
    #[serde(rename = "YMD_HMS")]
    YmdHms,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ColumnType {
    pub data_type: DataType,
    pub date_order: DateOrder,
}
pub fn date(value: &str, order: DateOrder) -> Option<String> {
    if order == DateOrder::YmdHms {
        let value = value.trim();
        if value.len() != 19
            || !value.bytes().enumerate().all(|(i, b)| match i {
                4 | 7 => b == b'-',
                10 => b == b' ',
                13 | 16 => b == b':',
                _ => b.is_ascii_digit(),
            })
        {
            return None;
        }
        if &value[11..13] > "23" || &value[14..16] > "59" || &value[17..19] > "59" {
            return None;
        }
        return NaiveDate::parse_from_str(&value[..10], "%Y-%m-%d")
            .ok()
            .filter(|d| d.format("%Y").to_string() != "0000")
            .map(|d| d.format("%Y-%m-%d").to_string());
    }
    let value = value.trim();
    let part = value.split(['T', ' ']).next()?;
    let formats = match order {
        DateOrder::DMY => ["%Y-%m-%d", "%d/%m/%Y", "%d.%m.%Y", "%d-%m-%Y"],
        DateOrder::MDY => ["%Y-%m-%d", "%m/%d/%Y", "%m.%d.%Y", "%m-%d-%Y"],
        DateOrder::YmdHms => unreachable!(),
    };
    formats
        .iter()
        .find_map(|f| NaiveDate::parse_from_str(part, f).ok())
        .map(|d| d.format("%Y-%m-%d").to_string())
}
pub fn infer(source: &SourceTable) -> Vec<ColumnType> {
    (0..source.headers.len())
        .map(|column| {
            let values: Vec<_> = source
                .rows
                .iter()
                .filter_map(|r| r.get(column))
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .take(1000)
                .collect();
            let kind = if values.is_empty() {
                DataType::Text
            } else if values.iter().all(|v| date(v, DateOrder::DMY).is_some()) {
                DataType::DateTime
            } else if values
                .iter()
                .all(|v| !v.starts_with('0') || v.len() == 1 || v.starts_with("0."))
                && values
                    .iter()
                    .all(|v| v.parse::<f64>().is_ok_and(|n| n.is_finite()))
            {
                DataType::Number
            } else {
                DataType::Text
            };
            let date_order = if kind == DataType::DateTime
                && values.iter().all(|v| date(v, DateOrder::YmdHms).is_some())
            {
                DateOrder::YmdHms
            } else {
                DateOrder::DMY
            };
            ColumnType {
                data_type: kind,
                date_order,
            }
        })
        .collect()
}
pub fn pivot_value(value: &str, kind: Option<&ColumnType>) -> String {
    match kind {
        Some(k) if k.data_type == DataType::DateTime => {
            date(value, k.date_order).unwrap_or_else(|| value.into())
        }
        _ => value.into(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn year_month_day_timestamp_metadata() {
        let column = ColumnType {
            data_type: DataType::DateTime,
            date_order: DateOrder::YmdHms,
        };
        assert_eq!(
            pivot_value("2026-10-29 14:05:09", Some(&column)),
            "2026-10-29"
        );
        assert_eq!(
            date("2024-02-29 00:00:00", column.date_order).as_deref(),
            Some("2024-02-29")
        );
        for value in [
            "2026-02-29 10:00:00",
            "2026-10-29 24:00:00",
            "2026-10-29 10:60:00",
            "2026-10-29 10:00:60",
            "29/10/2026 10:00:00",
            "2026-10-29",
            "0000-01-01 00:00:00",
        ] {
            assert_eq!(date(value, column.date_order), None, "{value}");
            assert_eq!(pivot_value(value, Some(&column)), value);
        }
        let json = serde_json::to_string(&column).unwrap();
        assert!(json.contains("YMD_HMS"));
        assert_eq!(serde_json::from_str::<ColumnType>(&json).unwrap(), column);
        let bytes = postcard::to_stdvec(&column).unwrap();
        assert_eq!(postcard::from_bytes::<ColumnType>(&bytes).unwrap(), column);
        assert_eq!(postcard::to_stdvec(&DateOrder::DMY).unwrap(), vec![0]);
        assert_eq!(postcard::to_stdvec(&DateOrder::MDY).unwrap(), vec![1]);
        let source = SourceTable {
            source_path: None,
            worksheet_name: None,
            headers: vec!["Created".into()],
            rows: vec![
                vec!["2026-10-29 14:05:09".into()],
                vec!["2026-10-30 01:02:03".into()],
            ],
        };
        assert_eq!(infer(&source), vec![column]);
    }
    #[test]
    fn date_order_and_invalid_dates() {
        assert_eq!(
            date("03/04/2026 13:10", DateOrder::DMY).as_deref(),
            Some("2026-04-03")
        );
        assert_eq!(
            date("03/04/2026 13:10", DateOrder::MDY).as_deref(),
            Some("2026-03-04")
        );
        assert!(date("31/02/2026", DateOrder::DMY).is_none());
        assert_eq!(
            date("2026-04-03T23:59:00-05:00", DateOrder::DMY).as_deref(),
            Some("2026-04-03")
        );
    }
}
