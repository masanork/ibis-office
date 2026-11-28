// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType};
use crate::CellValue;
use std::{
    collections::BTreeMap,
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

pub(super) fn sql_identifier(name: &str) -> String {
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
    read_sqlite_table_impl(path, table, Some((max_cells, max_text)))
}

/// Read the columns exposed by SELECT * without stepping through any rows.
pub fn sqlite_dataset_columns(path: &Path, table: &str) -> Result<Vec<QColumn>, String> {
    Ok(read_sqlite_table_impl(path, table, None)?.columns)
}

fn read_sqlite_table_impl(
    path: &Path,
    table: &str,
    limits: Option<(usize, usize)>,
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
    let Some((max_cells, max_text)) = limits else {
        check_sidecars(path)?;
        return Ok(result);
    };
    scan_sqlite_rows(&mut stmt, &mut result.columns, Some((max_cells, max_text)), |row| {
        result.rows.push(row);
        Ok(())
    })?;
    check_sidecars(&path.canonicalize().map_err(|error| error.to_string())?)?;
    Ok(result)
}

fn scan_sqlite_rows(
    stmt: &mut turso_core::Statement,
    columns: &mut [QColumn],
    limits: Option<(usize, usize)>,
    mut visit: impl FnMut(Vec<CellValue>) -> Result<(), String>,
) -> Result<(), String> {
    let width = columns.len();
    let mut types = vec![None; width];
    let mut text_budget = limits.map_or(MAX_TEXT_BYTES, |(_, text)| text);
    let mut height = 0usize;
    while let Some(row) = stmt
        .run_one_step_blocking(|| Ok(()), || Ok(()))
        .map_err(|error| error.to_string())?
    {
        height += 1;
        if limits.is_some_and(|(max_cells, _)| {
            height > MAX_ROWS || height.saturating_mul(width) > max_cells
        }) {
            return Err("SQLite table exceeds the query editor's table size limit".into());
        }
        if limits.is_none() {
            text_budget = MAX_TEXT_BYTES;
        }
        let mut values = Vec::with_capacity(width);
        for (index, (inferred, column)) in types.iter_mut().zip(columns.iter()).enumerate() {
            let (value, kind) = cell(row.get_value(index).as_ref(), &mut text_budget)
                .map_err(|error| format!("Row {height}, {}: {error}", column.name))?;
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
        visit(values)?;
    }
    for (column, inferred) in columns.iter_mut().zip(types) {
        column.ty = inferred.unwrap_or(QType::Any);
    }
    Ok(())
}

fn visit_sqlite_table(
    path: &Path,
    table: &str,
    schema_only: bool,
    mut visit: impl FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
) -> Result<(), String> {
    if schema_only {
        return visit(&sqlite_dataset_columns(path, table)?, None);
    }
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let (_database, connection) = open(&path)?;
    let identifier = checked_table(&connection, table)?;
    // Keep type inference and copying on the same read snapshot.
    connection.prepare("BEGIN").and_then(|mut stmt| stmt.run_ignore_rows())
        .map_err(|error| error.to_string())?;
    let mut stmt = connection.prepare(format!("SELECT * FROM main.{identifier}"))
        .map_err(|error| error.to_string())?;
    let width = stmt.num_columns();
    if !(1..=256).contains(&width) {
        return Err("datasets require 1 to 256 columns".into());
    }
    let mut columns = (0..width).map(|index| QColumn {
        name: stmt.get_column_name(index).into_owned(),
        ty: QType::Any,
    }).collect::<Vec<_>>();
    scan_sqlite_rows(&mut stmt, &mut columns, None, |_| Ok(()))?;
    stmt.reset().map_err(|error| error.to_string())?;
    visit(&columns, None)?;
    scan_sqlite_rows(&mut stmt, &mut columns.clone(), None, |row| visit(&columns, Some(row)))?;
    check_sidecars(&path)
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

/// SQL table name, column name, then the staged storage type or declared SQLite type.
pub type SqliteInputSchema = BTreeMap<String, BTreeMap<String, String>>;

pub fn dataset_sql_type(ty: QType) -> &'static str {
    match ty {
        QType::Any => "DYNAMIC",
        QType::Text => "TEXT",
        QType::Whole | QType::Int32 | QType::Logical => "INTEGER",
        _ => "REAL",
    }
}

fn staged_column_types(columns: &[QColumn]) -> BTreeMap<String, String> {
    columns.iter().map(|column| (column.name.clone(), dataset_sql_type(column.ty).into())).collect()
}

/// A read-only SQL connection over one source snapshot.
pub struct SqliteQuerySession {
    _database: Arc<Database>,
    connection: Arc<Connection>,
    path: Option<std::path::PathBuf>,
    temp: Option<tempfile::TempDir>,
    staged_schema: SqliteInputSchema,
}

impl SqliteQuerySession {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let (database, connection) = open(path)?;
        connection
            .prepare("BEGIN")
            .and_then(|mut stmt| stmt.run_ignore_rows())
            .map_err(|error| error.to_string())?;
        Ok(Self {
            _database: database,
            connection,
            path: Some(path.to_owned()),
            temp: None,
            staged_schema: SqliteInputSchema::new(),
        })
    }

    /// Inspect the same read transaction or staged tables used for the report SQL.
    pub fn input_schema(&self, byte_limit: usize) -> Result<Option<SqliteInputSchema>, String> {
        let Some(path) = &self.path else {
            let bytes = serde_json::to_vec(&self.staged_schema).map_err(|error| error.to_string())?;
            return Ok((bytes.len() <= byte_limit).then(|| self.staged_schema.clone()));
        };
        let mut statement = self.connection.prepare(format!(
            "SELECT name FROM main.sqlite_schema WHERE {TABLE_FILTER} ORDER BY name LIMIT {}",
            MAX_TABLES + 1
        )).map_err(|error| error.to_string())?;
        let mut result = SqliteInputSchema::new();
        let mut remaining = byte_limit;
        while let Some(row) = statement.run_one_step_blocking(|| Ok(()), || Ok(()))
            .map_err(|error| error.to_string())?
        {
            let ValueRef::Text(name) = row.get_value(0).as_ref() else {
                return Err("Invalid table name in SQLite schema".into());
            };
            let name = name.as_str();
            let table = self.connection.prepare(format!("SELECT * FROM main.{}", sql_identifier(name)))
                .map_err(|error| error.to_string())?;
            let mut columns = BTreeMap::new();
            for index in 0..table.num_columns() {
                let column = table.get_column_name(index).into_owned();
                let ty = table.get_column_decltype(index)
                    .filter(|ty| !ty.is_empty()).map_or_else(|| "DYNAMIC".into(), |ty| ty.to_ascii_uppercase());
                let Some(budget) = remaining.checked_sub(column.len() + ty.len() + 6) else {
                    return Ok(None);
                };
                remaining = budget;
                columns.insert(column, ty);
            }
            let Some(budget) = remaining.checked_sub(name.len() + 6) else {
                return Ok(None);
            };
            remaining = budget;
            if result.len() == MAX_TABLES {
                return Ok(None);
            }
            result.insert(name.to_owned(), columns);
        }
        check_sidecars(path)?;
        let bytes = serde_json::to_vec(&result).map_err(|error| error.to_string())?;
        Ok((bytes.len() <= byte_limit).then_some(result))
    }

    pub fn query(
        &self,
        sql: &str,
        params: &[Option<SqliteScalar<'_>>],
        max_rows: usize,
        max_bytes: usize,
    ) -> Result<SqliteRows, String> {
        validate_sqlite_select(sql)?;
        let parameters = params
            .iter()
            .map(|value| value.as_ref().map_or(Value::Null, SqliteScalar::to_sql))
            .collect();
        execute_query(
            self.path.as_deref(),
            &self.connection,
            sql,
            parameters,
            max_rows,
            max_bytes,
        )
    }
}

pub fn sqlite_query(
    path: &Path,
    sql: &str,
    params: &[Option<SqliteScalar<'_>>],
    max_rows: usize,
    max_bytes: usize,
) -> Result<SqliteRows, String> {
    SqliteQuerySession::from_file(path)?.query(sql, params, max_rows, max_bytes)
}

pub fn validate_sqlite_select(sql: &str) -> Result<(), String> {
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
    SqliteQuerySession::from_datasets(tables)?.query(sql, params, max_rows, max_bytes)
}

impl SqliteQuerySession {
    pub fn from_datasets(tables: &[(String, QTable)]) -> Result<Self, String> {
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
                    let value = dataset_value(value.clone(), table.columns[index].ty)?;
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
        Ok(Self {
            _database: database,
            connection,
            path: None,
            temp: None,
            staged_schema: tables.iter().map(|(alias, table)| (alias.clone(), staged_column_types(&table.columns))).collect(),
        })
    }
}

pub(super) fn dataset_value(value: CellValue, ty: QType) -> Result<Value, String> {
    Ok(match value {
        CellValue::Empty => Value::Null,
        CellValue::Text(text) => Value::from_text(text),
        CellValue::Number(value)
            if matches!(ty, QType::Whole | QType::Int32)
                && value.is_finite()
                && value.fract() == 0.0
                && value.abs() < (1u64 << 53) as f64 =>
        {
            Value::Numeric(Numeric::Integer(value as i64))
        }
        CellValue::Number(value) if value.is_finite() => Value::from_f64(value),
        CellValue::Bool(value) => Value::Numeric(Numeric::Integer(i64::from(value))),
        _ => return Err("Unsupported dataset value".into()),
    })
}

pub enum DiskDataset {
    ArrowIpc(std::path::PathBuf, Vec<String>),
    JsonLines(std::path::PathBuf, Vec<String>),
    RecoverableJsonLines(std::path::PathBuf, Vec<String>),
    JsonLinesSample(std::path::PathBuf, Vec<String>, usize),
    JsonLinesPaths(std::path::PathBuf, Vec<String>),
    Avro(std::path::PathBuf, Vec<String>),
    Sqlite(std::path::PathBuf, String),
    Jet(std::path::PathBuf, String, Vec<String>),
    Ese(std::path::PathBuf, String, Vec<String>),
    Pst(std::path::PathBuf, String, Vec<String>),
    Csv(std::path::PathBuf),
    Log(std::path::PathBuf),
    Parquet(std::path::PathBuf, Vec<String>, Vec<(String, super::ParquetPredicate)>),
    Table(QTable),
}

pub const DATASET_MEMORY_LIMIT: &str = "Dataset exceeds the in-memory input budget";

pub(super) fn charge_dataset_text(row: &[CellValue], budget: &mut usize) -> Result<(), String> {
    for value in row {
        if let CellValue::Text(text) = value {
            *budget = budget.checked_sub(text.len()).ok_or(DATASET_MEMORY_LIMIT)?;
        }
    }
    Ok(())
}

impl DiskDataset {
    pub fn columns(&self) -> Result<Vec<QColumn>, String> {
        let mut result = Vec::new();
        self.visit(true, |columns, _| {
            result = columns.to_vec();
            Ok(())
        })?;
        Ok(result)
    }

    fn visit(
        &self,
        schema_only: bool,
        mut visit: impl FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
    ) -> Result<(), String> {
        match self {
            Self::ArrowIpc(path, selected) => {
                super::arrow_ipc_source::visit_arrow_ipc(path, selected, schema_only, visit)
            }
            Self::JsonLines(path, selected) => {
                super::json_lines_source::visit_json_lines(path, selected, schema_only, visit)
            }
            Self::RecoverableJsonLines(path, selected) => {
                super::json_lines_source::visit_recoverable_json_lines(path, selected, false, usize::MAX, None, None, schema_only, visit).map(|_| ())
            }
            Self::JsonLinesSample(path, selected, max_lines) => {
                super::json_lines_source::visit_recoverable_json_lines(path, selected, false, *max_lines, None, None, schema_only, visit).map(|_| ())
            }
            Self::JsonLinesPaths(path, selected) => {
                super::json_lines_source::visit_recoverable_json_lines(path, selected, true, usize::MAX, None, None, schema_only, visit).map(|_| ())
            }
            Self::Avro(path, selected) => super::avro_source::visit_avro(path, selected, schema_only, visit),
            Self::Parquet(path, selected, filters) => {
                let filters = filters.iter().map(|(name, value)| (name.as_str(), value.clone())).collect::<Vec<_>>();
                super::parquet_source::visit_parquet(path, selected, &filters, schema_only, visit)
            }
            Self::Sqlite(path, table) => visit_sqlite_table(path, table, schema_only, visit),
            Self::Jet(path, table, selected) => {
                super::jet_source::visit_jet_table(path, table, selected, schema_only, visit)
            }
            Self::Ese(path, table, selected) => {
                super::ese_source::visit_ese_table(path, table, selected, schema_only, visit)
            }
            Self::Pst(path, table, selected) => {
                super::pst_source::visit_pst_table(path, table, selected, schema_only, visit)
            }
            Self::Log(path) => super::log_source::visit_log(path, schema_only, visit),
            Self::Csv(path) => super::csv_source::visit_csv(path, schema_only, visit),
            Self::Table(table) => {
                visit(&table.columns, None)?;
                if schema_only {
                    return Ok(());
                }
                for row in &table.rows {
                    visit(&table.columns, Some(row.clone()))?;
                }
                Ok(())
            }
        }
    }

    pub fn collect_bounded(&self, max_cells: usize, max_text: usize) -> Result<QTable, String> {
        let mut table = QTable::default();
        self.visit(false, bounded_rows(&mut table, max_cells, max_text))?;
        Ok(table)
    }

    pub fn collect_json_lines_range(
        path: &Path,
        selected: &[String],
        range: super::JsonLinesRange,
    ) -> Result<QTable, String> {
        if range.first == 0 || range.limit == 0 || range.last.is_some_and(|last| last < range.first) {
            return Err("Invalid JSON Lines range".into());
        }
        let mut table = QTable::default();
        super::json_lines_source::visit_recoverable_json_lines(
            path, selected, true, usize::MAX, None, Some(range), false,
            bounded_rows(&mut table, 256_000, 8 * 1024 * 1024),
        )?;
        Ok(table)
    }

    pub(super) fn collect_json_lines_page(
        path: &Path,
        max_lines: usize,
        cursor: Option<&super::JsonLinesPreviewCursor>,
    ) -> Result<(QTable, super::JsonLinesPreviewCursor), String> {
        let mut table = QTable::default();
        let cursor = super::json_lines_source::visit_recoverable_json_lines(
            path, &[], false, max_lines, cursor, None, false, bounded_rows(&mut table, 100_000, 8 * 1024 * 1024),
        )?;
        Ok((table, cursor))
    }

    pub fn collect_json_lines_sample(
        path: &Path,
        max_lines: usize,
    ) -> Result<(QTable, super::JsonLinesPreviewCursor), String> {
        Self::collect_json_lines_page(path, max_lines, None)
    }
}

fn bounded_rows(
    table: &mut QTable,
    max_cells: usize,
    mut max_text: usize,
) -> impl FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String> + '_ {
    move |columns, row| {
        if let Some(row) = row {
            if columns.len().saturating_mul(table.rows.len() + 1) > max_cells {
                return Err(DATASET_MEMORY_LIMIT.into());
            }
            charge_dataset_text(&row, &mut max_text)?;
            table.rows.push(row);
        } else {
            table.columns = columns.to_vec();
        }
        Ok(())
    }
}

const STAGED_BATCH_TEXT_BYTES: usize = 64 * 1024;

struct StagedInsert {
    single: turso_core::Statement,
    multi: Option<turso_core::Statement>,
    values: Vec<Value>,
    batch_rows: usize,
    batch_text_bytes: usize,
}

impl StagedInsert {
    fn new(connection: &Arc<Connection>, table: &str, width: usize, batch: bool) -> Result<Self, String> {
        let placeholders = format!("({})", vec!["?"; width].join(", "));
        let prepare = |rows| {
            let values = vec![placeholders.as_str(); rows].join(", ");
            connection.prepare(format!("INSERT INTO {table} VALUES {values}"))
                .map_err(|error| error.to_string())
        };
        let batch_rows = if batch { (64 / width).clamp(1, 32) } else { 1 };
        Ok(Self {
            single: prepare(1)?,
            multi: (batch_rows > 1).then(|| prepare(batch_rows)).transpose()?,
            values: Vec::with_capacity(batch_rows * width),
            batch_rows,
            batch_text_bytes: 0,
        })
    }

    fn push(&mut self, columns: &[QColumn], row: Vec<CellValue>) -> Result<(), String> {
        if row.len() != columns.len() {
            return Err("Dataset row does not match its schema".into());
        }
        if self.multi.is_none() {
            let values = row
                .into_iter()
                .zip(columns)
                .map(|(value, column)| dataset_value(value, column.ty));
            return Self::run(&mut self.single, values);
        }
        let row_text_bytes = row.iter().map(|value| match value {
            CellValue::Text(text) => text.len(),
            _ => 0,
        }).sum::<usize>();
        if self.batch_text_bytes + row_text_bytes > STAGED_BATCH_TEXT_BYTES {
            self.flush(columns.len())?;
        }
        let values = row
            .into_iter()
            .zip(columns)
            .map(|(value, column)| dataset_value(value, column.ty));
        if row_text_bytes > STAGED_BATCH_TEXT_BYTES {
            return Self::run(&mut self.single, values);
        }
        for value in values {
            self.values.push(value?);
        }
        self.batch_text_bytes += row_text_bytes;
        if self.values.len() == self.batch_rows * columns.len() {
            let stmt = self.multi.as_mut().ok_or("Missing batched dataset insert")?;
            Self::run(stmt, self.values.drain(..).map(Ok))?;
            self.batch_text_bytes = 0;
        }
        Ok(())
    }

    fn flush(&mut self, width: usize) -> Result<(), String> {
        let mut values = self.values.drain(..);
        while values.len() > 0 {
            Self::run(&mut self.single, values.by_ref().take(width).map(Ok))?;
        }
        self.batch_text_bytes = 0;
        Ok(())
    }

    fn run(
        stmt: &mut turso_core::Statement,
        values: impl Iterator<Item = Result<Value, String>>,
    ) -> Result<(), String> {
        for (index, value) in values.enumerate() {
            stmt.bind_at(NonZeroUsize::new(index + 1).unwrap(), value?)
                .map_err(|error| error.to_string())?;
        }
        stmt.run_ignore_rows().and_then(|_| stmt.reset()).map_err(|error| error.to_string())
    }
}

const STAGING_DISK_RESERVE: u64 = 512 * 1024 * 1024;
const STAGING_CHECK_TEXT_BYTES: usize = 8 * 1024 * 1024;

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn available_disk_bytes(path: &Path) -> std::io::Result<u64> {
    let volume = rustix::fs::statvfs(path)?;
    Ok(volume.f_bavail.saturating_mul(volume.f_frsize))
}

#[cfg(target_os = "windows")]
fn available_disk_bytes(path: &Path) -> std::io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let path = path.as_os_str().encode_wide().chain([0]).collect::<Vec<_>>();
    let mut available = 0;
    // The path is NUL-terminated and the output pointer is valid until the call returns.
    if unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(available)
}

fn staged_file_bytes(path: &Path) -> std::io::Result<u64> {
    fs::read_dir(path)?.try_fold(0u64, |sum, entry| {
        Ok(sum.saturating_add(entry?.metadata()?.len()))
    })
}

fn staging_disk_budget(path: &Path, estimated_bytes: u64) -> Result<u64, String> {
    let available = available_disk_bytes(path).map_err(|e| format!("Cannot check temporary disk space: {e}"))?;
    let budget = available.saturating_sub(STAGING_DISK_RESERVE);
    if available <= STAGING_DISK_RESERVE || estimated_bytes > budget {
        return Err(format!(
            "Not enough space for a temporary SQL database (available: {} MiB, estimated staging: {} MiB, reserve: 512 MiB)",
            available / (1024 * 1024), estimated_bytes.div_ceil(1024 * 1024)
        ));
    }
    Ok(budget)
}

fn check_staged_disk(path: &Path, budget: u64) -> Result<(), String> {
    let available = available_disk_bytes(path).map_err(|e| format!("Cannot check temporary disk space: {e}"))?;
    let used = staged_file_bytes(path).map_err(|e| e.to_string())?;
    if available <= STAGING_DISK_RESERVE || used > budget {
        return Err("Temporary SQL database reached its disk budget".into());
    }
    Ok(())
}

impl SqliteQuerySession {
    /// Stage Arrow sources in a private temporary database for reuse by several SELECTs.
    pub fn from_arrow_disk(
        sources: &[(String, std::path::PathBuf, Vec<String>)],
    ) -> Result<Self, String> {
        let datasets = sources
            .iter()
            .map(|(alias, path, selected)| {
                (
                    alias.clone(),
                    DiskDataset::ArrowIpc(path.clone(), selected.clone()),
                )
            })
            .collect::<Vec<_>>();
        Self::from_disk_datasets(&datasets)
    }

    pub fn from_disk_datasets(sources: &[(String, DiskDataset)]) -> Result<Self, String> {
        Self::from_disk_datasets_with_disk_estimate(sources, 0)
    }

    pub fn from_disk_datasets_with_disk_estimate(
        sources: &[(String, DiskDataset)],
        estimated_bytes: u64,
    ) -> Result<Self, String> {
        #[cfg(feature = "report-profiling")]
        use super::report_profile::{Phase, Timer};
        #[cfg(feature = "report-profiling")]
        let setup_timer = Timer::start(Phase::Write);
        let temp = tempfile::Builder::new()
            .prefix("ibis-datasets-")
            .tempdir()
            .map_err(|error| error.to_string())?;
        let disk_budget = staging_disk_budget(temp.path(), estimated_bytes)?;
        let path = temp.path().join("datasets.sqlite");
        let io = Arc::new(PlatformIO::new().map_err(|error| error.to_string())?);
        let database = Database::open_file(
            io,
            path.to_str().ok_or("Invalid temporary database path")?,
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
        execute("PRAGMA temp_store = FILE")?;
        #[cfg(feature = "report-profiling")]
        drop(setup_timer);
        let mut staged_schema = SqliteInputSchema::new();
        for (alias, source) in sources {
            let identifier = sql_identifier(alias);
            let mut insert: Option<StagedInsert> = None;
            let mut width = 0;
            let mut rows = 0usize;
            let mut text_bytes = 0usize;
            {
                #[cfg(feature = "report-profiling")]
                let _timer = Timer::start(Phase::Write);
                execute("BEGIN")?;
            }
            source.visit(false, |columns, row| {
                #[cfg(feature = "report-profiling")]
                let _timer = Timer::start(Phase::Write);
                if let Some(row) = row {
                    let insert = insert.as_mut().ok_or("Missing dataset schema")?;
                    text_bytes += row.iter().map(|value| match value {
                        CellValue::Text(text) => text.len(),
                        _ => 0,
                    }).sum::<usize>();
                    insert.push(columns, row)?;
                    rows += 1;
                    if rows == 4096 || text_bytes >= STAGING_CHECK_TEXT_BYTES {
                        insert.flush(width)?;
                        execute("COMMIT")?;
                        check_staged_disk(temp.path(), disk_budget)?;
                        execute("BEGIN")?;
                        rows = 0;
                        text_bytes = 0;
                    }
                } else {
                    let names = columns
                        .iter()
                        .map(|column| sql_identifier(&column.name))
                        .collect::<Vec<_>>();
                    execute(&format!("CREATE TABLE {identifier} ({})", names.join(", ")))?;
                    staged_schema.insert(alias.clone(), staged_column_types(columns));
                    width = names.len();
                    let batch = matches!(
                        source,
                        DiskDataset::ArrowIpc(..) | DiskDataset::JsonLines(..) | DiskDataset::RecoverableJsonLines(..) | DiskDataset::JsonLinesPaths(..) | DiskDataset::Avro(..) | DiskDataset::Parquet(..) | DiskDataset::Sqlite(..) | DiskDataset::Jet(..) | DiskDataset::Ese(..) | DiskDataset::Pst(..) | DiskDataset::Csv(..) | DiskDataset::Log(..)
                    );
                    #[cfg(feature = "report-profiling")]
                    let batch = batch && !super::report_profile::single_row_inserts_forced();
                    insert = Some(StagedInsert::new(&connection, &identifier, width, batch)?);
                }
                Ok(())
            })?;
            #[cfg(feature = "report-profiling")]
            let _timer = Timer::start(Phase::Write);
            if let Some(insert) = &mut insert {
                insert.flush(width)?;
            }
            execute("COMMIT")?;
            check_staged_disk(temp.path(), disk_budget)?;
        }
        connection.set_query_only(true);
        Ok(Self {
            _database: database,
            connection,
            path: None,
            temp: Some(temp),
            staged_schema,
        })
    }

    /// Size of the staged database and its visible sidecar files while the session is open.
    pub fn staged_disk_bytes(&self) -> std::io::Result<u64> {
        self.temp.as_ref().map_or(Ok(0), |temp| staged_file_bytes(temp.path()))
    }
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
    fn staging_rejects_unavailable_capacity_before_and_during_copy() {
        let dir = tempfile::tempdir().unwrap();
        assert!(staging_disk_budget(dir.path(), u64::MAX).unwrap_err().contains("Not enough space"));
        fs::write(dir.path().join("datasets.sqlite-wal"), b"1234").unwrap();
        assert!(check_staged_disk(dir.path(), 3).unwrap_err().contains("disk budget"));
    }

    #[test]
    fn stages_sqlite_above_cell_budget_preserving_type_and_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.sqlite");
        {
            let database = Database::open_file(
                Arc::new(PlatformIO::new().unwrap()), path.to_str().unwrap(), Arc::new(SqliteDialect),
            ).unwrap();
            let connection = database.connect().unwrap();
            for sql in [
                "CREATE TABLE facts (n, code)",
                "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM seq WHERE n<50002) INSERT INTO facts SELECT n, CASE WHEN n%2=0 THEN '001' ELSE NULL END FROM seq",
                "PRAGMA wal_checkpoint(TRUNCATE)",
            ] {
                connection.prepare(sql).unwrap().run_ignore_rows().unwrap();
            }
        }
        let before = fs::read(&path).unwrap();
        let source = DiskDataset::Sqlite(path.clone(), "facts".into());
        let session = SqliteQuerySession::from_disk_datasets(&[("facts".into(), source)]).unwrap();
        let result = session.query(
            "SELECT count(*), sum(n), count(code), min(code), min(typeof(n)) FROM facts",
            &[], 1, 1024,
        ).unwrap();
        assert_eq!(serde_json::json!(result.rows), serde_json::json!([[50002,1250125003_i64,25001,"001","integer"]]));
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn json_lines_batches_preserve_row_order_and_large_values() {
        use std::io::{BufWriter, Write};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.jsonl");
        let mut writer = BufWriter::new(File::create(&path).unwrap());
        let large_text = "x".repeat(30_000);
        for seq in 0..4_102 {
            let mut row = serde_json::json!({"seq":seq, "amount":seq as f64 / 2.0, "a":"001"});
            if matches!(seq, 34 | 35 | 37) {
                row["a"] = serde_json::json!(large_text);
                row["b"] = serde_json::json!(large_text);
            }
            if seq == 37 {
                row["c"] = serde_json::json!(large_text);
            }
            writeln!(writer, "{row}").unwrap();
        }
        writer.flush().unwrap();
        drop(writer);
        let source = DiskDataset::JsonLines(
            path,
            ["seq", "amount", "a", "b", "c"].map(String::from).to_vec(),
        );
        let table = source.collect_bounded(30_000, 1_000_000).unwrap();
        let memory = SqliteQuerySession::from_datasets(&[("facts".into(), table)]).unwrap();
        let disk = SqliteQuerySession::from_disk_datasets(&[("facts".into(), source)]).unwrap();
        let sql = "SELECT seq, amount, length(a), length(b), length(c) FROM facts ORDER BY rowid";
        let expected = memory.query(sql, &[], 5_000, 1_048_576).unwrap();
        let actual = disk.query(sql, &[], 5_000, 1_048_576).unwrap();
        assert!(!actual.truncated);
        assert_eq!(actual.rows.len(), 4_102);
        assert_eq!(actual.rows, expected.rows);
    }

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
