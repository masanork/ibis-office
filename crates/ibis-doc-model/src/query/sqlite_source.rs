// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType};
use crate::CellValue;
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
    sync::Arc,
};
use turso_core::{
    Connection, Database, Numeric, OpenFlags, OpenOptions, PlatformIO, SqliteDialect, Value,
    ValueRef,
};

const MAX_TABLES: usize = 1_024;
const MAX_ROWS: usize = 1_048_575;
const MAX_CELLS: usize = 5_000_000;
const MAX_TEXT_BYTES: usize = 128 * 1024 * 1024;

fn check_sidecars(path: &Path) -> Result<(), String> {
    for suffix in ["-wal", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        match fs::metadata(sidecar) {
            Ok(metadata) if metadata.len() == 0 => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(
                    "SQLite database has a pending journal. Select a standalone SQLite backup."
                        .into(),
                );
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn open(path: &Path) -> Result<(Arc<Database>, Arc<Connection>), String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    if !path.is_file() {
        return Err("Select a local SQLite database file".into());
    }
    check_sidecars(&path)?;
    let mut magic = [0; 16];
    File::open(&path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .map_err(|error| error.to_string())?;
    if &magic != b"SQLite format 3\0" {
        return Err("The selected file is not a SQLite 3 database".into());
    }
    let path = path
        .to_str()
        .ok_or("SQLite database path is not valid UTF-8")?;
    let io = Arc::new(PlatformIO::new().map_err(|error| error.to_string())?);
    let options = OpenOptions::new(Arc::new(SqliteDialect)).flags(OpenFlags::ReadOnly);
    let database = Database::open(io, path, options).map_err(|error| error.to_string())?;
    let connection = database.connect().map_err(|error| error.to_string())?;
    connection.set_query_only(true);
    Ok((database, connection))
}

const TABLE_FILTER: &str =
    "type = 'table' AND rootpage > 0 AND lower(substr(name,1,7)) <> 'sqlite_'";

pub fn sqlite_tables(path: &Path) -> Result<Vec<String>, String> {
    let (_database, connection) = open(path)?;
    let sql = format!(
        "SELECT name FROM main.sqlite_schema WHERE {TABLE_FILTER} ORDER BY name LIMIT {}",
        MAX_TABLES + 1
    );
    let mut stmt = connection.prepare(sql).map_err(|error| error.to_string())?;
    let rows = stmt.run_collect_rows().map_err(|error| error.to_string())?;
    if rows.len() > MAX_TABLES {
        return Err("SQLite database has too many tables to list".into());
    }
    check_sidecars(&path.canonicalize().map_err(|error| error.to_string())?)?;
    rows.into_iter()
        .map(|row| match row.as_slice() {
            [Value::Text(text)] => Ok(text.as_str().to_owned()),
            _ => Err("Invalid table name in SQLite schema".into()),
        })
        .collect()
}

fn checked_table(connection: &Arc<Connection>, table: &str) -> Result<String, String> {
    let mut lookup = connection
        .prepare(format!(
            "SELECT 1 FROM main.sqlite_schema WHERE {TABLE_FILTER} AND name = ?1 LIMIT 1"
        ))
        .map_err(|error| error.to_string())?;
    lookup
        .bind_at(
            std::num::NonZeroUsize::MIN,
            Value::from_text(table.to_owned()),
        )
        .map_err(|error| error.to_string())?;
    if lookup
        .run_collect_rows()
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err(format!("SQLite table does not exist: {table}"));
    }
    Ok(sql_identifier(table))
}

fn sql_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Describe SQLite's declared columns and primary-key order without reading table rows.
pub fn sqlite_columns(path: &Path, table: &str) -> Result<Vec<serde_json::Value>, String> {
    let (_database, connection) = open(path)?;
    let identifier = checked_table(&connection, table)?;
    let mut stmt = connection
        .prepare(format!("PRAGMA main.table_info({identifier})"))
        .map_err(|error| error.to_string())?;
    let rows = stmt.run_collect_rows().map_err(|error| error.to_string())?;
    check_sidecars(path)?;
    rows.into_iter()
        .map(|row| match row.as_slice() {
            [
                _,
                Value::Text(name),
                Value::Text(declared_type),
                Value::Numeric(Numeric::Integer(not_null)),
                _,
                Value::Numeric(Numeric::Integer(primary_key)),
            ] => Ok(serde_json::json!({
                "name": name.as_str(),
                "declared_type": declared_type.as_str(),
                "not_null": *not_null != 0,
                "primary_key_position": (*primary_key != 0).then_some(*primary_key),
            })),
            _ => Err("Invalid SQLite column metadata".into()),
        })
        .collect()
}

/// Count rows by one column using SQLite's grouping and NULL semantics.
pub fn sqlite_count(
    path: &Path,
    table: &str,
    group_by: Option<&str>,
    limit: usize,
) -> Result<serde_json::Value, String> {
    use serde_json::{Value as Json, json};

    let (_database, connection) = open(path)?;
    let table = checked_table(&connection, table)?;
    let limit = limit.clamp(1, 100);
    let sql = match group_by {
        Some(column) => {
            let columns = connection
                .prepare(format!("SELECT * FROM main.{table} LIMIT 0"))
                .map_err(|error| error.to_string())?;
            let index = (0..columns.num_columns())
                .find(|&index| columns.get_column_name(index) == column)
                .ok_or_else(|| format!("SQLite column does not exist: {column}"))?;
            let column = sql_identifier(&columns.get_column_name(index));
            format!(
                "SELECT {column}, COUNT(*) FROM main.{table} GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT {}",
                limit + 1
            )
        }
        None => format!("SELECT COUNT(*) FROM main.{table}"),
    };
    let mut stmt = connection.prepare(sql).map_err(|error| error.to_string())?;
    let mut groups = Vec::new();
    let mut text_budget = 32usize * 1024;
    while let Some(row) = stmt
        .run_one_step_blocking(|| Ok(()), || Ok(()))
        .map_err(|error| error.to_string())?
    {
        if group_by.is_none() {
            let ValueRef::Numeric(Numeric::Integer(count)) = row.get_value(0).as_ref() else {
                return Err("Invalid SQLite count".into());
            };
            check_sidecars(path)?;
            return Ok(json!({"count": count}));
        }
        if groups.len() == limit {
            check_sidecars(path)?;
            return Ok(json!({"groups": groups, "truncated": true}));
        }
        let value = match row.get_value(0).as_ref() {
            ValueRef::Null => Json::Null,
            ValueRef::Numeric(Numeric::Integer(value)) => json!(value),
            ValueRef::Numeric(Numeric::Float(value)) => {
                let value: f64 = value.into();
                json!(serde_json::Number::from_f64(value).ok_or("Non-finite SQLite group value")?)
            }
            ValueRef::Text(value) => {
                text_budget = text_budget
                    .checked_sub(value.as_str().len())
                    .ok_or("SQLite group values exceed the preview size limit")?;
                json!(value.as_str())
            }
            ValueRef::Blob(_) => {
                return Err("SQLite BLOB values cannot be returned as group keys".into());
            }
        };
        let ValueRef::Numeric(Numeric::Integer(count)) = row.get_value(1).as_ref() else {
            return Err("Invalid SQLite count".into());
        };
        groups.push(json!({"value": value, "count": count}));
    }
    check_sidecars(path)?;
    Ok(json!({"groups": groups, "truncated": false}))
}

pub fn read_sqlite_table(path: &Path, table: &str) -> Result<QTable, String> {
    let (_database, connection) = open(path)?;
    let identifier = checked_table(&connection, table)?;
    let mut stmt = connection
        .prepare(format!("SELECT * FROM main.{identifier}"))
        .map_err(|error| error.to_string())?;
    let width = stmt.num_columns();
    if width == 0 || width > 16_384 {
        return Err("SQLite table has an unsupported number of columns".into());
    }
    let columns = (0..width)
        .map(|index| QColumn {
            name: stmt.get_column_name(index).into_owned(),
            ty: QType::Any,
        })
        .collect();
    let mut result = QTable {
        columns,
        rows: Vec::new(),
    };
    let mut types = vec![None; width];
    let mut text_budget = MAX_TEXT_BYTES;
    while let Some(row) = stmt
        .run_one_step_blocking(|| Ok(()), || Ok(()))
        .map_err(|error| error.to_string())?
    {
        let height = result.rows.len() + 1;
        if height > MAX_ROWS || height.saturating_mul(width) > MAX_CELLS {
            return Err("SQLite table exceeds the query editor's table size limit".into());
        }
        let mut values = Vec::with_capacity(width);
        for (index, inferred) in types.iter_mut().enumerate() {
            let value = row.get_value(index).as_ref();
            let (value, kind) = cell(value, &mut text_budget).map_err(|error| {
                format!("Row {height}, {}: {error}", result.columns[index].name)
            })?;
            if let Some(kind) = kind {
                *inferred = Some(match (*inferred, kind) {
                    (None, kind) => kind,
                    (Some(previous), kind) if previous == kind => kind,
                    (Some(QType::Whole), QType::Decimal) | (Some(QType::Decimal), QType::Whole) => {
                        QType::Decimal
                    }
                    _ => QType::Any,
                });
            }
            values.push(value);
        }
        result.rows.push(values);
    }
    for (column, inferred) in result.columns.iter_mut().zip(types) {
        column.ty = inferred.unwrap_or(QType::Any);
    }
    check_sidecars(&path.canonicalize().map_err(|error| error.to_string())?)?;
    Ok(result)
}

fn cell(
    value: ValueRef<'_>,
    budget: &mut usize,
) -> Result<(CellValue, Option<QType>), &'static str> {
    let (cell, ty) = match value {
        ValueRef::Null => return Ok((CellValue::Empty, None)),
        ValueRef::Numeric(Numeric::Integer(value)) => {
            if value.unsigned_abs() >= 1 << 53 {
                return Err("Integer exceeds spreadsheet precision");
            }
            (CellValue::Number(value as f64), QType::Whole)
        }
        ValueRef::Numeric(Numeric::Float(value)) => {
            let value: f64 = value.into();
            if !value.is_finite() {
                return Err("Non-finite SQLite number");
            }
            (CellValue::Number(value), QType::Decimal)
        }
        ValueRef::Text(value) => {
            let text = value.as_str();
            if text.encode_utf16().take(32_768).count() > 32_767 {
                return Err("Text exceeds the spreadsheet cell limit");
            }
            *budget = budget
                .checked_sub(text.len())
                .ok_or("SQLite text exceeds the 128 MiB limit")?;
            (CellValue::Text(text.to_owned()), QType::Text)
        }
        ValueRef::Blob(_) => {
            return Err("SQLite BLOB values cannot be loaded as spreadsheet cells");
        }
    };
    Ok((cell, Some(ty)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_sqlite_format_read_only_and_preserves_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.sqlite3");
        // Generated with SQLite via Python’s sqlite3 module.
        let bytes = include_bytes!("fixtures/external.sqlite3");
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            sqlite_tables(&path).unwrap(),
            ["binary_data", "empty", "sales \"2026\""]
        );
        let table = read_sqlite_table(&path, "sales \"2026\"").unwrap();
        assert_eq!(table.names(), ["code", "amount", "remark"]);
        assert_eq!(table.columns[1].ty, QType::Any);
        assert_eq!(
            table.rows[0],
            [
                CellValue::Text("001".into()),
                CellValue::Number(37.),
                CellValue::Empty
            ]
        );
        assert_eq!(table.rows[1][1], CellValue::Number(12.5));
        assert_eq!(table.rows[2][1], CellValue::Text("pending".into()));
        assert!(read_sqlite_table(&path, "empty").unwrap().rows.is_empty());
        assert!(
            read_sqlite_table(&path, "binary_data")
                .unwrap_err()
                .contains("BLOB")
        );
        assert!(read_sqlite_table(&path, "sales_view").is_err());
        assert!(read_sqlite_table(&path, "sales \"2026\"; DROP TABLE empty; --").is_err());
        let (database, connection) = open(&path).unwrap();
        assert!(database.is_readonly());
        assert!(
            connection
                .prepare("DELETE FROM empty")
                .and_then(|mut stmt| stmt.run_ignore_rows())
                .is_err()
        );
        drop(connection);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        fs::write(dir.path().join("source.sqlite3-wal"), b"pending").unwrap();
        assert!(
            sqlite_tables(&path)
                .unwrap_err()
                .contains("pending journal")
        );
    }
}
