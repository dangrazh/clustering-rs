//! Operator-uploaded static data: one dedicated table per upload plus its join settings.
use super::*;
use crate::static_data::{self as sd, Column, FieldType, JoinConfig, Lookup};

pub(super) async fn migrate(c: &Connection) -> Result<()> {
    if scalar(c, "SELECT version FROM schema_version").await? >= 6 {
        return Ok(());
    }
    execute(c, "BEGIN").await?;
    let result = async {
        c.execute_batch(
            "CREATE TABLE static_tables(id TEXT PRIMARY KEY, name TEXT NOT NULL, normalized_name TEXT NOT NULL UNIQUE, sql_name TEXT NOT NULL UNIQUE, columns TEXT NOT NULL, row_count INTEGER NOT NULL, source TEXT NOT NULL, config TEXT NOT NULL, created INTEGER NOT NULL, creator TEXT NOT NULL REFERENCES users(id), version INTEGER NOT NULL DEFAULT 0);
             UPDATE schema_version SET version=6;",
        )
        .await?;
        Ok(())
    }
    .await;
    finish(c, result).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaticTable {
    pub id: String,
    pub name: String,
    pub sql_name: String,
    pub columns: Vec<Column>,
    pub row_count: i64,
    pub source: String,
    pub config: JoinConfig,
    pub created: i64,
    pub creator: String,
    pub version: i64,
}

fn text(value: turso::Value) -> String {
    match value {
        turso::Value::Integer(v) => v.to_string(),
        turso::Value::Real(v) => v.to_string(),
        turso::Value::Text(v) => v,
        _ => String::new(),
    }
}

async fn tables(c: &Connection, id: Option<&str>) -> Result<Vec<StaticTable>> {
    let sql = "SELECT t.id,t.name,t.sql_name,t.columns,t.row_count,t.source,t.config,t.created,u.name,t.version FROM static_tables t JOIN users u ON u.id=t.creator";
    let mut rows = match id {
        Some(id) => c.query(format!("{sql} WHERE t.id=?"), [id]).await?,
        None => c.query(format!("{sql} ORDER BY t.name"), ()).await?,
    };
    let mut out = vec![];
    while let Some(r) = rows.next().await? {
        out.push(StaticTable {
            id: r.get(0)?,
            name: r.get(1)?,
            sql_name: r.get(2)?,
            columns: serde_json::from_str(&r.get::<String>(3)?)?,
            row_count: r.get(4)?,
            source: r.get(5)?,
            config: serde_json::from_str(&r.get::<String>(6)?)?,
            created: r.get(7)?,
            creator: r.get(8)?,
            version: r.get(9)?,
        });
    }
    Ok(out)
}

impl Store {
    pub async fn static_tables(&self) -> Result<Vec<StaticTable>> {
        tables(&self.connect().await?, None).await
    }
    pub async fn static_table(&self, id: &str) -> Result<StaticTable> {
        tables(&self.connect().await?, Some(id))
            .await?
            .pop()
            .ok_or_else(|| AppError::Missing("Static data table not found.".into()).into())
    }

    /// Creates a dedicated table from the uploaded rows, or replaces the rows of `replace`.
    /// A replacement keeps the join settings whose columns still exist.
    pub async fn save_static_table(
        &self,
        user: &User,
        name: &str,
        source_label: &str,
        data: &crate::model::SourceTable,
        replace: Option<(&str, i64)>,
    ) -> Result<String> {
        let columns = sd::infer(data)?;
        let (name, normalized) = normalize_name(name)?;
        let _m = self.maintenance.read().await;
        let lock = self.lock("static-data");
        let _l = lock.write().await;
        let _w = self.writers.acquire().await?;
        let c = self.connect().await?;
        execute(&c, "BEGIN").await?;
        let result = async {
            let previous = match replace {
                Some((id, expected)) => {
                    let old = tables(&c, Some(id)).await?.pop().ok_or_else(|| {
                        AppError::Missing("Static data table not found.".into())
                    })?;
                    expect(old.version as u64, expected as u64)?;
                    Some(old)
                }
                None => None,
            };
            if c.query(
                "SELECT id FROM static_tables WHERE normalized_name=? AND id<>?",
                params![normalized.clone(), previous.as_ref().map(|p| p.id.clone()).unwrap_or_default()],
            )
            .await?
            .next()
            .await?
            .is_some()
            {
                return Err(AppError::Conflict("A static data table with this name already exists.".into()).into());
            }
            let sql_name = match &previous {
                Some(old) => old.sql_name.clone(),
                None => {
                    let base = format!("static_{}", sd::identifier(&name).chars().take(48).collect::<String>())
                        .trim_end_matches('_')
                        .to_string();
                    let mut candidate = base.clone();
                    let mut n = 2;
                    while scalar(
                        &c,
                        &format!("SELECT COUNT(*) FROM sqlite_master WHERE name='{candidate}'"),
                    )
                    .await?
                        > 0
                    {
                        candidate = format!("{base}_{n}");
                        n += 1;
                    }
                    candidate
                }
            };
            if previous.is_some() {
                c.execute(format!("DROP TABLE IF EXISTS \"{sql_name}\""), ()).await?;
            }
            let definition = columns
                .iter()
                .map(|col| format!("\"{}\" {}", col.field, col.kind.sql()))
                .collect::<Vec<_>>()
                .join(", ");
            c.execute(
                format!("CREATE TABLE \"{sql_name}\"(row_id INTEGER PRIMARY KEY, {definition})"),
                (),
            )
            .await?;
            let names = columns
                .iter()
                .map(|col| format!("\"{}\"", col.field))
                .collect::<Vec<_>>()
                .join(",");
            let row_marks = format!("({})", vec!["?"; columns.len()].join(","));
            for chunk in data.rows.chunks((4000 / columns.len()).max(1)) {
                let mut values = Vec::with_capacity(chunk.len() * columns.len());
                for row in chunk {
                    for (i, col) in columns.iter().enumerate() {
                        let cell = row.get(i).map(String::as_str).unwrap_or("");
                        values.push(match sd::stored_text(cell, col.kind) {
                            None => turso::Value::Null,
                            Some(v) => match col.kind {
                                FieldType::Integer => v.parse().map(turso::Value::Integer).unwrap_or(turso::Value::Text(v)),
                                FieldType::Real => v.parse().map(turso::Value::Real).unwrap_or(turso::Value::Text(v)),
                                _ => turso::Value::Text(v),
                            },
                        });
                    }
                }
                c.execute(
                    format!(
                        "INSERT INTO \"{sql_name}\"({names}) VALUES {}",
                        vec![row_marks.as_str(); chunk.len()].join(",")
                    ),
                    values,
                )
                .await?;
            }
            let fields: std::collections::HashSet<_> = columns.iter().map(|c| c.field.clone()).collect();
            let mut config = previous.as_ref().map(|p| p.config.clone()).unwrap_or_default();
            config.keys.retain(|k| fields.contains(&k.field));
            config.fields.retain(|f| fields.contains(f));
            let id = previous.as_ref().map(|p| p.id.clone()).unwrap_or_else(id);
            c.execute("DELETE FROM static_tables WHERE id=?", [id.as_str()]).await?;
            c.execute(
                "INSERT INTO static_tables(id,name,normalized_name,sql_name,columns,row_count,source,config,created,creator,version) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    id.clone(),
                    name,
                    normalized,
                    sql_name,
                    serde_json::to_string(&columns)?,
                    data.rows.len() as i64,
                    source_label,
                    serde_json::to_string(&config)?,
                    now(),
                    user.id.clone(),
                    previous.as_ref().map(|p| p.version + 1).unwrap_or(0)
                ],
            )
            .await?;
            Ok(id)
        }
        .await;
        finish(&c, result).await
    }

    pub async fn configure_static_table(&self, id: &str, expected: i64, config: &JoinConfig) -> Result<()> {
        let _m = self.maintenance.read().await;
        let lock = self.lock("static-data");
        let _l = lock.write().await;
        let _w = self.writers.acquire().await?;
        let c = self.connect().await?;
        let table = tables(&c, Some(id))
            .await?
            .pop()
            .ok_or_else(|| AppError::Missing("Static data table not found.".into()))?;
        config.validate(&table.columns)?;
        let changed = c
            .execute(
                "UPDATE static_tables SET config=?,version=version+1 WHERE id=? AND version=?",
                params![serde_json::to_string(config)?, id, expected],
            )
            .await?;
        if changed == 0 {
            return Err(AppError::Conflict("This table changed. Review its current settings and retry.".into()).into());
        }
        Ok(())
    }

    pub async fn delete_static_table(&self, id: &str) -> Result<()> {
        let _m = self.maintenance.read().await;
        let lock = self.lock("static-data");
        let _l = lock.write().await;
        let _w = self.writers.acquire().await?;
        let c = self.connect().await?;
        let table = tables(&c, Some(id))
            .await?
            .pop()
            .ok_or_else(|| AppError::Missing("Static data table not found.".into()))?;
        execute(&c, "BEGIN").await?;
        let result = async {
            c.execute(format!("DROP TABLE IF EXISTS \"{}\"", table.sql_name), ()).await?;
            c.execute("DELETE FROM static_tables WHERE id=?", [id]).await?;
            Ok(())
        }
        .await;
        finish(&c, result).await
    }

    /// A page of stored rows for review, in upload order.
    pub async fn static_rows(&self, id: &str, offset: i64) -> Result<Value> {
        ensure!((0..=10_000_000).contains(&offset), "Invalid offset");
        let table = self.static_table(id).await?;
        let c = self.connect().await?;
        let mut rows = c
            .query(
                format!("SELECT * FROM \"{}\" ORDER BY row_id LIMIT 101 OFFSET ?", table.sql_name),
                [offset],
            )
            .await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            // Column 0 is row_id.
            out.push((1..=table.columns.len()).map(|i| r.get_value(i).map(text)).collect::<Result<Vec<_>, _>>()?);
        }
        let more = out.len() > 100;
        out.truncate(100);
        Ok(json!({"rows":out,"nextOffset":more.then_some(offset+100)}))
    }

    /// Changes whenever any table, its rows or its settings change; keys the lookup cache.
    pub async fn static_signature(&self) -> Result<String> {
        let c = self.connect().await?;
        let mut rows = c.query("SELECT id,version FROM static_tables ORDER BY id", ()).await?;
        let mut out = String::new();
        while let Some(r) = rows.next().await? {
            out.push_str(&format!("{}:{};", r.get::<String>(0)?, r.get::<i64>(1)?));
        }
        Ok(out)
    }

    /// Loads every configured table for joining.
    pub async fn static_lookups(&self) -> Result<Vec<Lookup>> {
        let c = self.connect().await?;
        let mut out = vec![];
        for table in tables(&c, None).await? {
            let JoinConfig { keys, fields } = &table.config;
            if keys.is_empty() || fields.is_empty() {
                continue;
            }
            let column = |field: &str| table.columns.iter().find(|c| c.field == field);
            let selected: Vec<_> = keys
                .iter()
                .map(|k| k.field.as_str())
                .chain(fields.iter().map(String::as_str))
                .map(|f| format!("\"{f}\""))
                .collect();
            let mut rows = c
                .query(format!("SELECT {} FROM \"{}\" ORDER BY row_id", selected.join(","), table.sql_name), ())
                .await?;
            let mut map = HashMap::new();
            while let Some(r) = rows.next().await? {
                let key = (0..keys.len())
                    .map(|i| r.get_value(i).map(|v| sd::key_part(&text(v))))
                    .collect::<Result<Vec<_>, _>>()?;
                if key.iter().all(String::is_empty) {
                    continue;
                }
                let values = (keys.len()..keys.len() + fields.len())
                    .map(|i| r.get_value(i).map(text))
                    .collect::<Result<Vec<_>, _>>()?;
                map.entry(key).or_insert(values);
            }
            out.push(Lookup {
                table: table.name.clone(),
                key_columns: keys.iter().map(|k| k.analysis_column.clone()).collect(),
                fields: fields
                    .iter()
                    .filter_map(|f| column(f).map(|c| (c.name.clone(), c.kind)))
                    .collect(),
                rows: map,
            });
        }
        Ok(out)
    }
}
