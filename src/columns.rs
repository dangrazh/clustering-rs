//! Shared interpretation metadata; never changes source values or clustering identity.
use crate::model::SourceTable;
use chrono::NaiveDate;
use serde::{Serialize,Deserialize};
#[derive(Debug,Clone,Copy,Default,PartialEq,Eq,Serialize,Deserialize)]
pub enum DataType { #[default] Text, Number, #[serde(rename="Date/Time")] DateTime }
#[derive(Debug,Clone,Copy,Default,PartialEq,Eq,Serialize,Deserialize)]
pub enum DateOrder { #[default] DMY, MDY }
#[derive(Debug,Clone,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,rename_all="camelCase")]
pub struct ColumnType { pub data_type:DataType,pub date_order:DateOrder }
pub fn date(value:&str,order:DateOrder)->Option<String>{
 let value=value.trim();let part=value.split(['T',' ']).next()?;
 let formats=match order {DateOrder::DMY=>["%Y-%m-%d","%d/%m/%Y","%d.%m.%Y","%d-%m-%Y"],DateOrder::MDY=>["%Y-%m-%d","%m/%d/%Y","%m.%d.%Y","%m-%d-%Y"]};
 formats.iter().find_map(|f|NaiveDate::parse_from_str(part,f).ok()).map(|d|d.format("%Y-%m-%d").to_string())
}
pub fn infer(source:&SourceTable)->Vec<ColumnType>{
 (0..source.headers.len()).map(|column|{
   let values:Vec<_>=source.rows.iter().filter_map(|r|r.get(column)).map(|v|v.trim()).filter(|v|!v.is_empty()).take(1000).collect();
   let kind=if values.is_empty(){DataType::Text}else if values.iter().all(|v|date(v,DateOrder::DMY).is_some()){DataType::DateTime}else if values.iter().all(|v|!v.starts_with('0')||v.len()==1||v.starts_with("0."))&&values.iter().all(|v|v.parse::<f64>().is_ok_and(|n|n.is_finite())){DataType::Number}else{DataType::Text};
   ColumnType{data_type:kind,date_order:DateOrder::DMY}
 }).collect()
}
pub fn pivot_value(value:&str,kind:Option<&ColumnType>)->String{
 match kind {Some(k) if k.data_type==DataType::DateTime=>date(value,k.date_order).unwrap_or_else(||value.into()),_=>value.into()}
}
#[cfg(test)]mod tests{use super::*;
#[test]fn date_order_and_invalid_dates(){assert_eq!(date("03/04/2026 13:10",DateOrder::DMY).as_deref(),Some("2026-04-03"));assert_eq!(date("03/04/2026 13:10",DateOrder::MDY).as_deref(),Some("2026-03-04"));assert!(date("31/02/2026",DateOrder::DMY).is_none());assert_eq!(date("2026-04-03T23:59:00-05:00",DateOrder::DMY).as_deref(),Some("2026-04-03"));}
}
