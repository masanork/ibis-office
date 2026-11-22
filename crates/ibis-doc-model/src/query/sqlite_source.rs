// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType};
use crate::CellValue;
use std::{
    fs::{self, File},
    io::Read,
    num::NonZeroUsize,
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

pub enum SqliteScalar<'a> {
    Integer(i64),
    Real(f64),
    Text(&'a str),
}

impl SqliteScalar<'_> {
    fn to_sql(&self) -> Value {
        match self {
            Self::Integer(value) => Value::Numeric(Numeric::Integer(*value)),
            Self::Real(value) => Value::from_f64(*value),
            Self::Text(value) => Value::from_text((*value).to_owned()),
        }
    }
}

pub enum SqlitePredicate<'a> {
    Equal(SqliteScalar<'a>),
    NotEqual(SqliteScalar<'a>),
    LessThan(SqliteScalar<'a>),
    AtMost(SqliteScalar<'a>),
    GreaterThan(SqliteScalar<'a>),
    AtLeast(SqliteScalar<'a>),
    IsNull,
    IsNotNull,
}

impl SqlitePredicate<'_> {
    fn sql(&self) -> (&'static str, Option<Value>) {
        let (op, value) = match self {
            Self::Equal(value) => ("= ?", value),
            Self::NotEqual(value) => ("<> ?", value),
            Self::LessThan(value) => ("< ?", value),
            Self::AtMost(value) => ("<= ?", value),
            Self::GreaterThan(value) => ("> ?", value),
            Self::AtLeast(value) => (">= ?", value),
            Self::IsNull => return ("IS NULL", None),
            Self::IsNotNull => return ("IS NOT NULL", None),
        };
        (op, Some(value.to_sql()))
    }
}

/// Count rows by one column using SQLite's grouping and NULL semantics.
pub fn sqlite_count(
    path: &Path,
    table: &str,
    group_by: Option<&str>,
    filters: &[(&str, SqlitePredicate<'_>)],
    limit: usize,
) -> Result<serde_json::Value, String> {
    use serde_json::json;

    let (_database, connection) = open(path)?;
    let table = checked_table(&connection, table)?;
    let limit = limit.clamp(1, 100);
    let column_identifier = |name: &str, columns: &[String]| {
        columns
            .iter()
            .find(|column| column.as_str() == name)
            .map(|column| sql_identifier(column))
            .ok_or_else(|| format!("SQLite column does not exist: {name}"))
    };
    let mut columns = Vec::new();
    if group_by.is_some() || !filters.is_empty() {
        let stmt = connection
            .prepare(format!("SELECT * FROM main.{table} LIMIT 0"))
            .map_err(|error| error.to_string())?;
        columns
            .extend((0..stmt.num_columns()).map(|index| stmt.get_column_name(index).into_owned()));
    }
    let mut conditions = Vec::new();
    let mut parameters = Vec::new();
    for (column, predicate) in filters {
        let column = column_identifier(column, &columns)?;
        let (op, bound) = predicate.sql();
        conditions.push(format!("{column} {op}"));
        parameters.extend(bound);
    }
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };
    let sql = match group_by {
        Some(column) => {
            let column = column_identifier(column, &columns)?;
            format!(
                "SELECT {column}, COUNT(*) FROM main.{table}{where_clause} GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT {}",
                limit + 1
            )
        }
        None => format!("SELECT COUNT(*) FROM main.{table}{where_clause}"),
    };
    let mut result = execute_query(Some(path), &connection, &sql, parameters, limit, 256 * 1024)?;
    if group_by.is_none() {
        let count = result
            .rows
            .first()
            .and_then(|row| row[0].as_i64())
            .ok_or("Invalid SQLite count")?;
        return Ok(json!({"count": count}));
    }
    let mut text_budget = 32usize * 1024;
    let groups = result
        .rows
        .drain(..)
        .map(|mut row| {
            let count = row
                .pop()
                .and_then(|value| value.as_i64())
                .ok_or("Invalid SQLite count")?;
            let value = row.pop().ok_or("Invalid SQLite group")?;
            if let Some(text) = value.as_str() {
                text_budget = text_budget
                    .checked_sub(text.len())
                    .ok_or("SQLite group values exceed the preview size limit")?;
            }
            Ok::<_, String>(json!({"value": value, "count": count}))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if result.truncated && groups.len() < limit {
        return Err("SQLite group values exceed the preview size limit".into());
    }
    Ok(json!({"groups": groups, "truncated": result.truncated}))
}

pub fn read_sqlite_table(path: &Path, table: &str) -> Result<QTable, String> {
    read_sqlite_table_bounded(path, table, MAX_CELLS, MAX_TEXT_BYTES)
}

pub fn read_sqlite_table_bounded(
    path: &Path,
    table: &str,
    max_cells: usize,
    max_text: usize,
) -> Result<QTable, String> {
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
    let mut text_budget = max_text;
    while let Some(row) = stmt
        .run_one_step_blocking(|| Ok(()), || Ok(()))
        .map_err(|error| error.to_string())?
    {
        let height = result.rows.len() + 1;
        if height > MAX_ROWS || height.saturating_mul(width) > max_cells {
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
                .ok_or("SQLite text exceeds the import limit")?;
            (CellValue::Text(text.to_owned()), QType::Text)
        }
        ValueRef::Blob(_) => {
            return Err("SQLite BLOB values cannot be loaded as spreadsheet cells");
        }
    };
    Ok((cell, Some(ty)))
}

/// A bounded SQL result. SQLite integers remain exact until worksheet conversion.
pub struct SqliteRows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub truncated: bool,
}

pub fn sqlite_query(
    path: &Path,
    sql: &str,
    params: &[Option<SqliteScalar<'_>>],
    max_rows: usize,
    max_bytes: usize,
) -> Result<SqliteRows, String> {
    validate_select(sql)?;
    let (_database, connection) = open(path)?;
    let parameters = params
        .iter()
        .map(|value| value.as_ref().map_or(Value::Null, SqliteScalar::to_sql))
        .collect();
    execute_query(
        Some(path),
        &connection,
        sql,
        parameters,
        max_rows,
        max_bytes,
    )
}

fn validate_select(sql: &str) -> Result<(), String> {
    use turso_parser::{
        ast::{Cmd, Stmt},
        parser::Parser,
    };

    if sql.len() > 16 * 1024 {
        return Err("SQLite query text exceeds 16 KiB".into());
    }
    let mut parser = Parser::new(sql.as_bytes());
    let first = parser.next_cmd().map_err(|error| error.to_string())?;
    if !matches!(first, Some(Cmd::Stmt(Stmt::Select(_))))
        || parser
            .next_cmd()
            .map_err(|error| error.to_string())?
            .is_some()
    {
        return Err("Supply a single read-only SELECT or WITH … SELECT query".into());
    }
    Ok(())
}

/// Query bounded tabular snapshots without writing any source file.
pub fn sqlite_query_datasets(
    tables: &[(String, QTable)],
    sql: &str,
    params: &[Option<SqliteScalar<'_>>],
    max_rows: usize,
    max_bytes: usize,
) -> Result<SqliteRows, String> {
    validate_select(sql)?;
    let database = Database::open_file(
        Arc::new(turso_core::MemoryIO::new()),
        ":memory:",
        Arc::new(SqliteDialect),
    )
    .map_err(|error| error.to_string())?;
    let connection = database.connect().map_err(|error| error.to_string())?;
    let execute = |sql: &str| {
        connection
            .prepare(sql)
            .and_then(|mut stmt| stmt.run_ignore_rows())
            .map_err(|error| error.to_string())
    };
    execute("BEGIN")?;
    for (name, table) in tables {
        let name = sql_identifier(name);
        let columns = table
            .columns
            .iter()
            .map(|column| sql_identifier(&column.name))
            .collect::<Vec<_>>();
        execute(&format!("CREATE TABLE {name} ({})", columns.join(", ")))?;
        let placeholders = vec!["?"; columns.len()].join(", ");
        let mut stmt = connection
            .prepare(format!("INSERT INTO {name} VALUES ({placeholders})"))
            .map_err(|error| error.to_string())?;
        for row in &table.rows {
            for (index, value) in row.iter().enumerate() {
                let value = match value {
                    CellValue::Empty => Value::Null,
                    CellValue::Text(text) => Value::from_text(text.clone()),
                    CellValue::Number(value)
                        if matches!(table.columns[index].ty, QType::Whole | QType::Int32)
                            && value.is_finite()
                            && value.fract() == 0.0
                            && value.abs() < (1u64 << 53) as f64 =>
                    {
                        Value::Numeric(Numeric::Integer(*value as i64))
                    }
                    CellValue::Number(value) if value.is_finite() => Value::from_f64(*value),
                    CellValue::Bool(value) => Value::Numeric(Numeric::Integer(i64::from(*value))),
                    _ => return Err("Unsupported dataset value".into()),
                };
                stmt.bind_at(NonZeroUsize::new(index + 1).unwrap(), value)
                    .map_err(|error| error.to_string())?;
            }
            stmt.run_ignore_rows()
                .and_then(|_| stmt.reset())
                .map_err(|error| error.to_string())?;
        }
    }
    execute("COMMIT")?;
    connection.set_query_only(true);
    let parameters = params
        .iter()
        .map(|value| value.as_ref().map_or(Value::Null, SqliteScalar::to_sql))
        .collect();
    execute_query(None, &connection, sql, parameters, max_rows, max_bytes)
}

fn execute_query(
    path: Option<&Path>,
    connection: &Arc<Connection>,
    sql: &str,
    parameters: Vec<Value>,
    max_rows: usize,
    max_bytes: usize,
) -> Result<SqliteRows, String> {
    use serde_json::{Value as Json, json};
    use std::time::{Duration, Instant};

    connection.set_load_extension_enabled(false);
    let deadline = Instant::now() + Duration::from_secs(3);
    connection.set_progress_handler(1_000, Some(Box::new(move || Instant::now() >= deadline)));
    let mut stmt = connection.prepare(sql).map_err(|error| error.to_string())?;
    if stmt.parameters_count() != parameters.len() {
        return Err(format!(
            "SQLite query requires {} positional parameters",
            stmt.parameters_count()
        ));
    }
    for (index, value) in parameters.into_iter().enumerate() {
        stmt.bind_at(NonZeroUsize::new(index + 1).unwrap(), value)
            .map_err(|error| error.to_string())?;
    }
    let width = stmt.num_columns();
    if !(1..=256).contains(&width) {
        return Err("SQLite query must return between 1 and 256 columns".into());
    }
    let columns = (0..width)
        .map(|index| stmt.get_column_name(index).into_owned())
        .collect::<Vec<_>>();
    let mut remaining = max_bytes
        .checked_sub(json!(columns).to_string().len())
        .ok_or("SQLite column names exceed the result size limit")?;
    let mut result = SqliteRows {
        columns,
        rows: Vec::new(),
        truncated: false,
    };
    loop {
        if Instant::now() >= deadline {
            return Err("SQLite query exceeded the execution time limit".into());
        }
        let row = stmt
            .run_one_step_blocking(|| Ok(()), || Ok(()))
            .map_err(|error| {
                if Instant::now() >= deadline {
                    "SQLite query exceeded the execution time limit".into()
                } else {
                    error.to_string()
                }
            })?;
        let Some(row) = row else { break };
        if result.rows.len() >= max_rows || (result.rows.len() + 2) * width > 100_000 {
            result.truncated = true;
            break;
        }
        let mut values = Vec::with_capacity(width);
        let mut text_room = remaining;
        for index in 0..width {
            values.push(match row.get_value(index).as_ref() {
                ValueRef::Null => Json::Null,
                ValueRef::Numeric(Numeric::Integer(value)) => json!(value),
                ValueRef::Numeric(Numeric::Float(value)) => {
                    let value: f64 = value.into();
                    json!(serde_json::Number::from_f64(value).ok_or("Non-finite SQLite number")?)
                }
                ValueRef::Text(value) => {
                    let text = value.as_str();
                    let Some(room) = text_room.checked_sub(text.len()) else {
                        break;
                    };
                    text_room = room;
                    json!(text)
                }
                ValueRef::Blob(_) => {
                    return Err(
                        "SQLite BLOB results must be converted explicitly, for example with hex()"
                            .into(),
                    );
                }
            });
        }
        let size = serde_json::to_string(&values)
            .map_err(|e| e.to_string())?
            .len()
            + 1;
        if values.len() != width || size > remaining {
            result.truncated = true;
            break;
        }
        remaining -= size;
        result.rows.push(values);
    }
    if let Some(path) = path {
        check_sidecars(path)?;
    }
    Ok(result)
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
