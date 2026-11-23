// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::path::{Path, PathBuf};

use ibis_doc_model::query::{
    ParquetBound, ParquetPredicate, QColumn, QTable, QType, SqlitePredicate, SqliteQuerySession,
    SqliteRows, SqliteScalar, read_parquet_filtered, read_sqlite_table_bounded, sqlite_columns,
    sqlite_count, sqlite_tables, validate_sqlite_select,
};
use serde_json::{Value, json};

use crate::catalog::{Access, Tool, boolean, document_id, integer, object, string};
use crate::service::{Args, Limits, Service};
use crate::{ToolError, paths};

pub const DATASET_EXTENSIONS: &[&str] = &["csv", "parquet", "sqlite", "sqlite3", "db"];
const EXTENSIONS: &[&str] = &["sqlite", "sqlite3", "db"];
const PATH: &str = "path";
const MAX_FILTERS: usize = 16;

const TOOLS: &[Tool] = &[
    Tool {
        name: "sqlite_list_tables",
        description: "List ordinary tables in a standalone SQLite database.",
        access: Access::Read,
        returns_content: true,
        schema: || object(&[source_path()], &[PATH]),
        run: list_tables,
    },
    Tool {
        name: "sqlite_describe_table",
        description: "Inspect the column names, declared types, nullability flags and primary key of a SQLite table.",
        access: Access::Read,
        returns_content: true,
        schema: || object(&[source_path(), table_name()], &[PATH, "table"]),
        run: describe_table,
    },
    Tool {
        name: "sqlite_query",
        description: "Run one read-only SQLite SELECT (including WITH) and preview its result. Increase limit for a larger preview.",
        access: Access::Read,
        returns_content: true,
        schema: || {
            let mut props = query_properties();
            props.push((
                "limit",
                integer("Maximum preview rows (default 50, maximum 1000)."),
            ));
            object(&props, &["sql"])
        },
        run: query,
    },
    Tool {
        name: "sqlite_query_to_table",
        description: "Run one read-only SQLite SELECT and write its complete result to a new Excel table at a blank destination. Refuses results above the export limit.",
        access: Access::Edit,
        returns_content: true,
        schema: || {
            let mut props = query_properties();
            props.extend([
                document_id(),
                ("destination_sheet", string("Worksheet for the new table.")),
                ("anchor", string("Top-left cell in A1 form.")),
            ]);
            object(
                &props,
                &["sql", "document_id", "destination_sheet", "anchor"],
            )
        },
        run: query_to_table,
    },
    Tool {
        name: "sqlite_refresh_query_table",
        description: "Re-run the saved query after binding its source files in order, replace the table, and refresh its linked pivots and charts in one edit.",
        access: Access::Edit,
        returns_content: false,
        schema: || {
            object(
                &[
                    document_id(),
                    ("table", string("Table name from xlsx_list_query_reports.")),
                    (
                        "sources",
                        json!({"type":"array","minItems":1,"maxItems":8,"items":string("File path in an allowed folder. Use the order in the saved report.")}),
                    ),
                ],
                &["document_id", "table", "sources"],
            )
        },
        run: refresh_query_table,
    },
    Tool {
        name: "sqlite_refresh_query_reports",
        description: "Preview or atomically refresh workbook query reports from shared file bindings. Preview reports row counts and checks for overlap without editing.",
        access: Access::Edit,
        returns_content: true,
        schema: || {
            object(
                &[
                    document_id(),
                    (
                        "bindings",
                        json!({
                            "type":"object", "minProperties":1, "maxProperties":96,
                            "description":"Map saved source file names to authorized file paths.",
                            "additionalProperties":{"type":"string"}
                        }),
                    ),
                    (
                        "tables",
                        json!({"type":"array","minItems":1,"maxItems":12,"uniqueItems":true,"items":{"type":"string"},
                "description":"Optional query table names. Omit to refresh all reports (up to 12 per call)."}),
                    ),
                    (
                        "metrics",
                        json!({"type":"array","maxItems":48,
                            "description":"Optional numeric columns to sum before and after refresh.",
                            "items": object(&[
                                ("table", string("Query report table to compare.")),
                                ("column", string("Exact numeric result column name.")),
                            ], &["table","column"]),
                        }),
                    ),
                    (
                        "preview",
                        boolean("Validate without editing and report row changes (default false)."),
                    ),
                ],
                &["document_id", "bindings"],
            )
        },
        run: refresh_query_reports,
    },
    Tool {
        name: "sqlite_count_rows",
        description: "Count SQLite table rows with optional filters and grouping by one column. NULL forms its own group. Grouped results are sorted by descending count.",
        access: Access::Read,
        returns_content: true,
        schema: || {
            object(
                &[
                    source_path(),
                    table_name(),
                    (
                        "group_by",
                        string("Optional exact column name to group by."),
                    ),
                    (
                        "filters",
                        json!({
                            "type": "array",
                            "description": "Include only rows that match all conditions.",
                            "maxItems": MAX_FILTERS,
                            "items": object(&[
                                ("column", string("Exact column name.")),
                                ("operator", json!({"enum": ["eq", "ne", "lt", "le", "gt", "ge", "is_null", "is_not_null"]})),
                                ("value", json!({"type": ["string", "number"], "description": "Required for comparisons. Omit for is_null and is_not_null."})),
                            ], &["column", "operator"]),
                        }),
                    ),
                    (
                        "limit",
                        integer(
                            "Maximum returned groups (default 25, maximum 100). Does not limit the rows counted.",
                        ),
                    ),
                ],
                &[PATH, "table"],
            )
        },
        run: count_rows,
    },
];

pub fn sqlite_tools() -> &'static [Tool] {
    TOOLS
}

/// Run against the file selected by the desktop user. A model-supplied path cannot select a source.
pub fn run_selected_sqlite_tool(path: &Path, name: &str, args: Value) -> Result<Value, ToolError> {
    run_selected_datasets_tool(std::slice::from_ref(&path.to_path_buf()), name, args)
}

pub fn run_selected_datasets_tool(
    paths: &[PathBuf],
    name: &str,
    mut args: Value,
) -> Result<Value, ToolError> {
    let mut service = selected_sources(paths, &mut args)?;
    let tool = TOOLS
        .iter()
        .find(|tool| tool.name == name && tool.access == Access::Read)
        .ok_or_else(|| {
            ToolError::new("unsupported_operation", "no SQLite read tool by that name")
        })?;
    (tool.run)(&mut service, &args)
}

fn selected_sources(paths: &[PathBuf], args: &mut Value) -> Result<Service, ToolError> {
    let object = args
        .as_object_mut()
        .ok_or_else(|| ToolError::invalid("the arguments are not an object"))?;
    if object.contains_key(PATH) {
        return Err(ToolError::invalid(
            "files are selected in the assistant panel",
        ));
    }
    if let Some(datasets) = object.get_mut("datasets") {
        let entries = datasets
            .as_array_mut()
            .ok_or_else(|| ToolError::invalid("datasets must be an array"))?;
        for entry in entries {
            let source = entry
                .str_arg("source")?
                .parse::<usize>()
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| paths.get(index))
                .ok_or_else(|| ToolError::invalid("source must be a selected file number"))?;
            entry["source"] = json!(
                source
                    .to_str()
                    .ok_or_else(|| ToolError::invalid("path is not valid UTF-8"))?
            );
        }
    } else if let Some(path) = paths.iter().find(|path| is_sqlite(path)) {
        object.insert(
            PATH.into(),
            json!(
                path.to_str()
                    .ok_or_else(|| ToolError::invalid("path is not valid UTF-8"))?
            ),
        );
    }
    Ok(selected_file_service(paths))
}

fn selected_file_service(paths: &[PathBuf]) -> Service {
    let folders = paths
        .iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    Service::new(&folders, true, Limits::default())
}

pub fn is_sqlite(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn source_path() -> (&'static str, Value) {
    (
        PATH,
        string("Path to a standalone .sqlite, .sqlite3 or .db file under an allowed folder."),
    )
}

fn table_name() -> (&'static str, Value) {
    ("table", string("Exact table name from sqlite_list_tables."))
}

fn database(service: &Service, args: &Value) -> Result<PathBuf, ToolError> {
    let path = paths::resolve(&service.allow_dirs, args.str_arg(PATH)?, EXTENSIONS, true)?;
    let metadata = std::fs::metadata(&path).map_err(sqlite_error)?;
    if !metadata.is_file() {
        return Err(ToolError::invalid("select a SQLite database file"));
    }
    if metadata.len() > service.limits.max_file_size {
        return Err(ToolError::new(
            "file_too_large",
            "SQLite database exceeds the file size limit",
        ));
    }
    Ok(path)
}

fn sqlite_error(error: impl ToString) -> ToolError {
    ToolError::new("sqlite_error", error.to_string())
}

fn list_tables(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let tables = sqlite_tables(&database(service, args)?).map_err(sqlite_error)?;
    Ok(json!({"tables": tables}))
}

fn describe_table(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let table = args.str_arg("table")?;
    let columns = sqlite_columns(&database(service, args)?, table).map_err(sqlite_error)?;
    Ok(json!({"table": table, "columns": columns}))
}

fn parse_scalar(value: &Value) -> Result<SqliteScalar<'_>, ToolError> {
    match value {
        Value::String(value) => Ok(SqliteScalar::Text(value)),
        Value::Number(value) if value.is_i64() => {
            Ok(SqliteScalar::Integer(value.as_i64().unwrap()))
        }
        Value::Number(value) if value.is_f64() => Ok(SqliteScalar::Real(value.as_f64().unwrap())),
        Value::Number(_) => Err(ToolError::invalid(
            "integer is outside SQLite's signed 64-bit range",
        )),
        _ => Err(ToolError::invalid("expected a string or number")),
    }
}

fn count_filter(filter: &Value) -> Result<(&str, SqlitePredicate<'_>), ToolError> {
    use SqlitePredicate::*;

    crate::edit::only_known(filter, &["column", "operator", "value"])?;
    let column = filter.str_arg("column")?;
    let operator = filter.str_arg("operator")?;
    let value = filter.get("value");
    let predicate = match operator {
        "is_null" | "is_not_null" if value.is_some() => {
            return Err(ToolError::invalid("omit value for is_null and is_not_null"));
        }
        "is_null" => IsNull,
        "is_not_null" => IsNotNull,
        _ => {
            let value = parse_scalar(
                value.ok_or_else(|| ToolError::invalid("comparison filters require a value"))?,
            )?;
            match operator {
                "eq" => Equal(value),
                "ne" => NotEqual(value),
                "lt" => LessThan(value),
                "le" => AtMost(value),
                "gt" => GreaterThan(value),
                "ge" => AtLeast(value),
                _ => return Err(ToolError::invalid("unknown SQLite filter operator")),
            }
        }
    };
    Ok((column, predicate))
}

fn count_rows(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let table = args.str_arg("table")?;
    let group_by = args.opt_str("group_by")?;
    let limit = args.opt_usize("limit")?.unwrap_or(25).clamp(1, 100);
    let filters: &[Value] = match args.get("filters") {
        None => &[],
        Some(Value::Array(filters)) if filters.len() <= MAX_FILTERS => filters,
        _ => {
            return Err(ToolError::invalid(format!(
                "filters must be an array with at most {MAX_FILTERS} conditions"
            )));
        }
    };
    let predicates = filters
        .iter()
        .map(count_filter)
        .collect::<Result<Vec<_>, _>>()?;
    let mut result = sqlite_count(
        &database(service, args)?,
        table,
        group_by,
        &predicates,
        limit,
    )
    .map_err(sqlite_error)?;
    if !filters.is_empty() {
        result["filters"] = json!(filters);
    }
    result["table"] = json!(table);
    if let Some(group_by) = group_by {
        result["group_by"] = json!(group_by);
    }
    Ok(result)
}

fn query_properties() -> Vec<(&'static str, Value)> {
    vec![
        source_path(),
        (
            "sql",
            string("One SQLite SELECT or WITH query. Use positional ? parameters."),
        ),
        (
            "datasets",
            json!({
                "type":"array","minItems":1,"maxItems":8,
                "description":"Use an in-memory database containing only these CSV, Parquet and/or SQLite tables.",
                "items":object(&[
                    ("source", string("Path to a CSV, Parquet or SQLite file in an allowed folder.")),
                    ("alias", string("SQL table name to register for this dataset.")),
                    ("table", string("Source table name, required for SQLite files.")),
                    ("where", json!({
                        "type":"object","minProperties":1,"maxProperties":MAX_FILTERS,
                        "additionalProperties":{"anyOf":[
                            {"type":["string","number","boolean","null"]},
                            {"type":"object","minProperties":1,"additionalProperties":false,"properties":{
                                "lt":{"type":["string","number"]},"lte":{"type":["string","number"]},
                                "gt":{"type":["string","number"]},"gte":{"type":["string","number"]}
                            }}
                        ]},
                        "description":"Parquet only: equality or range conditions, combined with AND. Scalar null matches missing values."
                    })),
                    ("columns", json!({
                        "type":"array","minItems":1,"maxItems":256,"uniqueItems":true,
                        "items":{"type":"string","minLength":1},
                        "description":"Parquet only: exact column names to load, in order. Omit to load all columns."
                    })),
                ], &["source","alias"]),
            }),
        ),
        (
            "params",
            json!({"type":"array","maxItems":100,"items":{"type":["string","number","null"]}}),
        ),
    ]
}

fn sql_params(args: &Value) -> Result<Vec<Option<SqliteScalar<'_>>>, ToolError> {
    match args.get("params") {
        None => Ok(Vec::new()),
        Some(Value::Array(values)) if values.len() <= 100 => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    Ok(None)
                } else {
                    parse_scalar(value).map(Some)
                }
            })
            .collect::<Result<Vec<_>, _>>(),
        _ => Err(ToolError::invalid(
            "params must be an array of at most 100 values",
        )),
    }
}

fn query_session(service: &Service, args: &Value) -> Result<SqliteQuerySession, ToolError> {
    if let Some(specs) = args.get("datasets") {
        if args.opt_str(PATH)?.is_some() {
            return Err(ToolError::invalid(
                "use path for a SQLite database or datasets for registered tables",
            ));
        }
        SqliteQuerySession::from_datasets(&load_datasets(service, specs)?).map_err(sqlite_error)
    } else {
        SqliteQuerySession::from_file(&database(service, args)?).map_err(sqlite_error)
    }
}

fn run_session_query(
    session: &SqliteQuerySession,
    args: &Value,
    max_rows: usize,
    max_bytes: usize,
) -> Result<SqliteRows, ToolError> {
    session
        .query(
            args.str_arg("sql")?,
            &sql_params(args)?,
            max_rows,
            max_bytes,
        )
        .map_err(sqlite_error)
}

fn run_query(
    service: &Service,
    args: &Value,
    max_rows: usize,
    max_bytes: usize,
) -> Result<SqliteRows, ToolError> {
    run_session_query(&query_session(service, args)?, args, max_rows, max_bytes)
}

fn query(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let limit = args.opt_usize("limit")?.unwrap_or(50).clamp(1, 1000);
    let result = run_query(service, args, limit, service.limits.page_bytes)?;
    Ok(json!({
        "sql": args["sql"], "params": args.get("params").cloned().unwrap_or_else(|| json!([])),
        "columns": result.columns, "rows": result.rows, "truncated": result.truncated,
    }))
}

fn complete_query(service: &Service, args: &Value) -> Result<SqliteRows, ToolError> {
    require_complete(run_query(service, args, 10_000, 4 * 1024 * 1024)?)
}

fn require_complete(rows: SqliteRows) -> Result<SqliteRows, ToolError> {
    if rows.truncated {
        return Err(ToolError::new(
            "result_too_large",
            "The complete result exceeds the table export limit. Narrow the query.",
        ));
    }
    Ok(rows)
}

fn query_to_table(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let rows = complete_query(service, args)?;
    let definition = saved_definition(args)?;
    crate::tools_pivot::write_table(service, args, rows, definition)
}

/// The complete database result stays in-process between the worker and the draft.
pub struct SqliteTableExport {
    arguments: Value,
    rows: SqliteRows,
    definition: Value,
    refresh: bool,
    checks: Vec<Value>,
}

impl SqliteTableExport {
    pub fn stage(
        self,
        document: &mut ibis_doc_model::Document,
        context: crate::DraftToolContext,
    ) -> Result<(Value, Vec<ibis_doc_model::Command>), ToolError> {
        crate::service::run_draft_operation(
            ibis_formats::Kind::Workbook,
            document,
            context,
            self.arguments,
            |service, args| {
                if self.refresh {
                    apply_single_refresh(service, args, self.rows, &self.definition, &self.checks)
                } else {
                    crate::tools_pivot::write_table(service, args, self.rows, self.definition)
                }
            },
        )
    }
}

pub fn prepare_selected_sqlite_export(
    path: &Path,
    args: Value,
) -> Result<SqliteTableExport, ToolError> {
    prepare_selected_datasets_export(std::slice::from_ref(&path.to_path_buf()), args)
}

pub fn prepare_selected_datasets_export(
    paths: &[PathBuf],
    mut args: Value,
) -> Result<SqliteTableExport, ToolError> {
    let service = selected_sources(paths, &mut args)?;
    let rows = complete_query(&service, &args)?;
    let definition = saved_definition(&args)?;
    Ok(SqliteTableExport {
        arguments: args,
        rows,
        definition,
        refresh: false,
        checks: Vec::new(),
    })
}

fn parquet_filter_value(value: &Value) -> Result<ibis_doc_model::CellValue, ToolError> {
    use ibis_doc_model::CellValue;
    match value {
        Value::String(value) => Ok(CellValue::Text(value.clone())),
        Value::Bool(value) => Ok(CellValue::Bool(*value)),
        Value::Null => Ok(CellValue::Empty),
        Value::Number(value) => value
            .as_f64()
            .filter(|n| n.is_finite() && (value.is_f64() || n.abs() < (1_u64 << 53) as f64))
            .map(CellValue::Number)
            .ok_or_else(|| ToolError::invalid("Parquet filter number exceeds exact numeric range")),
        _ => Err(ToolError::invalid(
            "Parquet filter values must be scalar or null",
        )),
    }
}

fn parquet_predicate(value: &Value) -> Result<ParquetPredicate, ToolError> {
    let Some(bounds) = value.as_object() else {
        return parquet_filter_value(value).map(ParquetPredicate::Equal);
    };
    if bounds.is_empty() {
        return Err(ToolError::invalid(
            "A Parquet range must specify at least one bound",
        ));
    }
    bounds
        .iter()
        .map(|(key, value)| {
            let bound = match key.as_str() {
                "lt" => ParquetBound::Lt,
                "lte" => ParquetBound::Le,
                "gt" => ParquetBound::Gt,
                "gte" => ParquetBound::Ge,
                _ => {
                    return Err(ToolError::invalid(
                        "Parquet range operators are lt, lte, gt and gte",
                    ));
                }
            };
            if !(value.is_string() || value.is_number()) {
                return Err(ToolError::invalid(
                    "Parquet range bounds must be strings or numbers",
                ));
            }
            Ok((bound, parquet_filter_value(value)?))
        })
        .collect::<Result<Vec<_>, ToolError>>()
        .map(ParquetPredicate::Range)
}

fn load_datasets(service: &Service, specs: &Value) -> Result<Vec<(String, QTable)>, ToolError> {
    use ibis_doc_model::CellValue;
    let specs = specs
        .as_array()
        .filter(|items| (1..=8).contains(&items.len()))
        .ok_or_else(|| ToolError::invalid("datasets must have 1 to 8 entries"))?;
    let mut aliases = std::collections::HashSet::new();
    let mut cells = 100_000usize;
    let mut text = 8 * 1024 * 1024usize;
    specs.iter().map(|spec| {
        crate::edit::only_known(spec, &["source", "alias", "table", "columns", "where"])?;
        let alias = spec.str_arg("alias")?;
        if alias.is_empty() || alias.len() > 128 || !alias.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || alias.as_bytes()[0].is_ascii_digit() || !aliases.insert(alias.to_ascii_lowercase()) {
            return Err(ToolError::invalid("dataset aliases must be distinct SQL identifiers using letters, digits or underscores"));
        }
        let path = paths::resolve(&service.allow_dirs, spec.str_arg("source")?, DATASET_EXTENSIONS, true)?;
        let metadata = std::fs::metadata(&path).map_err(sqlite_error)?;
        if !metadata.is_file() || metadata.len() > service.limits.max_file_size.min(32 * 1024 * 1024) {
            return Err(ToolError::invalid("dataset must be a regular file within the input size limit"));
        }
        let table_name = spec.opt_str("table")?;
        let kind = source_kind(&path);
        let columns = match spec.get("columns") {
            None => Vec::new(),
            Some(_) if kind != "parquet" => {
                return Err(ToolError::invalid("columns is only used for Parquet datasets"));
            }
            Some(value) => value.as_array()
                .filter(|items| (1..=256).contains(&items.len()))
                .and_then(|items| items.iter().map(|item| {
                    item.as_str().filter(|name| !name.is_empty()).map(str::to_owned)
                }).collect::<Option<Vec<_>>>())
                .ok_or_else(|| ToolError::invalid("columns must contain 1 to 256 non-empty Parquet column names"))?,
        };
        let filters = match spec.get("where") {
            None => Vec::new(),
            Some(_) if kind != "parquet" => {
                return Err(ToolError::invalid("where is only used for Parquet datasets"));
            }
            Some(value) => value.as_object()
                .filter(|items| (1..=MAX_FILTERS).contains(&items.len()))
                .ok_or_else(|| ToolError::invalid(format!("where must have 1 to {MAX_FILTERS} conditions")))?
                .iter().map(|(name, value)| Ok((name.as_str(), parquet_predicate(value)?)))
                .collect::<Result<Vec<_>, ToolError>>()?,
        };
        let table = if is_sqlite(&path) {
            let name = table_name.ok_or_else(|| ToolError::invalid("SQLite datasets require a source table"))?;
            read_sqlite_table_bounded(&path, name, cells, text).map_err(sqlite_error)?
        } else {
            if table_name.is_some() {
                return Err(ToolError::invalid("table is only used for SQLite datasets"));
            }
            if kind == "parquet" {
                read_parquet_filtered(&path, &columns, &filters, cells, text).map_err(sqlite_error)?
            } else {
                read_csv_dataset(&path, cells, text)?
            }
        };
        if table.columns.is_empty() || table.columns.len() > 256 {
            return Err(ToolError::invalid("datasets require 1 to 256 columns"));
        }
        cells = cells.checked_sub(table.rows.len() * table.columns.len())
            .ok_or_else(|| ToolError::invalid("datasets exceed 100000 cells"))?;
        for value in table.rows.iter().flatten() {
            if let CellValue::Text(value) = value {
                text = text.checked_sub(value.len()).ok_or_else(|| ToolError::invalid("datasets exceed the text size limit"))?;
            }
        }
        Ok((alias.to_owned(), table))
    }).collect()
}

fn read_csv_dataset(path: &Path, max_cells: usize, max_text: usize) -> Result<QTable, ToolError> {
    use ibis_doc_model::CellValue;
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(max_text as u64 + 1).read_to_end(&mut bytes))
        .map_err(sqlite_error)?;
    if bytes.len() > max_text {
        return Err(ToolError::invalid("CSV exceeds the import size limit"));
    }
    let decoded = ibis_formats::csv::decode(&bytes).map_err(sqlite_error)?;
    if decoded.len() > max_text {
        return Err(ToolError::invalid("CSV exceeds the decoded size limit"));
    }
    let mut records = ibis_formats::csv::CsvRecordIter::new(&decoded, ',');
    let headers = records
        .next()
        .ok_or_else(|| ToolError::invalid("CSV must have headers"))?;
    if headers.len() > 256 || headers.iter().any(|header| header.trim().is_empty()) {
        return Err(ToolError::invalid("CSV needs 1 to 256 nonblank headers"));
    }
    let width = headers.len();
    let mut rows = Vec::new();
    for record in records {
        if record.len() != width || (rows.len() + 1) * width > max_cells {
            return Err(ToolError::invalid(
                "CSV rows must match their header width and fit the cell limit",
            ));
        }
        rows.push(record.into_iter().map(CellValue::Text).collect());
    }
    let columns = headers
        .into_iter()
        .map(|name| QColumn {
            name,
            ty: QType::Text,
        })
        .collect();
    Ok(QTable { columns, rows })
}

pub(crate) const LIST_QUERY_REPORTS: Tool = Tool {
    name: "xlsx_list_query_reports",
    description: "List saved query tables and the source slots needed to refresh them.",
    access: Access::Read,
    returns_content: true,
    schema: || {
        object(
            &[
                document_id(),
                (
                    "table",
                    string(
                        "Optional table name to inspect its saved SQL, parameters and source slots.",
                    ),
                ),
                ("offset", integer("First report to return (0-based).")),
                (
                    "limit",
                    integer("Maximum reports to return (default 20, maximum 25)."),
                ),
            ],
            &["document_id"],
        )
    },
    run: list_query_reports,
};

fn list_query_reports(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let offset = args.opt_usize("offset")?.unwrap_or(0);
    let limit = args.opt_usize("limit")?.unwrap_or(20).clamp(1, 25);
    let open = crate::tools_xlsx::workbook_document(service, args.str_arg("document_id")?)?;
    let workbook = open.document.presentation();
    if args.opt_str("table")?.is_some() {
        let definition = query_report_definition(workbook, args)?;
        return Ok(json!({"table":args["table"],"definition":definition}));
    }
    let book = workbook
        .workbook
        .as_ref()
        .ok_or_else(|| ToolError::invalid("Workbook is missing"))?;
    let mut reports = crate::tools_xlsx::workbook_sheets(workbook)
        .into_iter()
        .flat_map(|(_, sheet, model)| {
            model.tables.iter().filter_map(move |table| {
                let definition = book.query_reports.get(&table.name)?;
                Some(json!({"table":table.name,"sheet":sheet,"sources":definition["sources"]}))
            })
        })
        .skip(offset);
    let results: Vec<_> = reports.by_ref().take(limit).collect();
    let mut result = json!({"reports":results});
    if reports.next().is_some() {
        result["next_offset"] = json!(offset + results.len());
    }
    Ok(result)
}

fn source_kind(path: &Path) -> &'static str {
    if is_sqlite(path) {
        "sqlite"
    } else if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("parquet"))
    {
        "parquet"
    } else if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
    {
        "csv"
    } else {
        ""
    }
}

fn saved_definition(args: &Value) -> Result<Value, ToolError> {
    let source = |path: &str, spec: &Value| {
        let path = Path::new(path);
        let mut source = json!({
            "file": path.file_name().unwrap_or_default().to_string_lossy(),
            "kind": source_kind(path),
            "alias": spec["alias"], "table": spec["table"],
        });
        for key in ["columns", "where"] {
            if let Some(value) = spec.get(key) {
                source[key] = value.clone();
            }
        }
        source
    };
    let sources = if let Some(specs) = args["datasets"].as_array() {
        specs
            .iter()
            .map(|spec| Ok(source(spec.str_arg("source")?, spec)))
            .collect::<Result<Vec<_>, ToolError>>()?
    } else {
        vec![source(args.str_arg(PATH)?, &Value::Null)]
    };
    let definition = json!({
        "sql":args["sql"], "params":args.get("params").cloned().unwrap_or_else(|| json!([])),
        "sources":sources,
    });
    if definition.to_string().len() > 32 * 1024 {
        return Err(ToolError::invalid(
            "The saved report definition exceeds 32 KiB",
        ));
    }
    Ok(definition)
}

pub fn query_report_definition(
    workbook: &ibis_doc_model::Presentation,
    args: &Value,
) -> Result<Value, ToolError> {
    let table = args.str_arg("table")?;
    workbook
        .workbook
        .as_ref()
        .and_then(|book| {
            book.query_reports
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(table))
                .map(|(_, definition)| definition.clone())
        })
        .ok_or_else(|| {
            ToolError::new(
                "no_such_query_report",
                format!("No query report named {table}"),
            )
        })
}

fn bound_query(definition: &Value, args: &Value) -> Result<Value, ToolError> {
    let expected = definition["sources"]
        .as_array()
        .filter(|sources| (1..=8).contains(&sources.len()))
        .ok_or_else(|| ToolError::invalid("Invalid saved report sources"))?;
    let sources = args["sources"]
        .as_array()
        .filter(|sources| sources.len() == expected.len())
        .ok_or_else(|| {
            ToolError::invalid(format!(
                "Supply {} source files in saved order",
                expected.len()
            ))
        })?;
    let mut query = json!({"sql":definition["sql"],"params":definition["params"]});
    let datasets = sources
        .iter()
        .zip(expected)
        .map(|(value, source)| {
            let path = value
                .as_str()
                .ok_or_else(|| ToolError::invalid("A source binding must be a file path"))?;
            if source_kind(Path::new(path)) != source.str_arg("kind")?
                || source_kind(Path::new(path)).is_empty()
            {
                return Err(ToolError::invalid("A source binding has the wrong format"));
            }
            let mut dataset =
                json!({"source":path,"alias":source["alias"],"table":source["table"]});
            for key in ["columns", "where"] {
                if let Some(value) = source.get(key) {
                    dataset[key] = value.clone();
                }
            }
            Ok(dataset)
        })
        .collect::<Result<Vec<_>, ToolError>>()?;
    if expected.len() == 1 && expected[0]["alias"].is_null() && expected[0]["kind"] == "sqlite" {
        query[PATH] = sources[0].clone();
    } else {
        query["datasets"] = json!(datasets);
    }
    Ok(query)
}

fn refresh_query_table(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let open = crate::tools_xlsx::workbook_document(service, args.str_arg("document_id")?)?;
    let definition = query_report_definition(open.document.presentation(), args)?;
    let (rows, checks) =
        refresh_query_result(service, &bound_query(&definition, args)?, &definition)?;
    apply_single_refresh(service, args, rows, &definition, &checks)
}

fn apply_single_refresh(
    service: &mut Service,
    args: &Value,
    rows: SqliteRows,
    definition: &Value,
    checks: &[Value],
) -> Result<Value, ToolError> {
    enforce_checks(checks)?;
    let id = args.str_arg("document_id")?;
    let open = crate::tools_xlsx::workbook_document(service, id)?;
    let (mut summary, commands) = crate::tools_pivot::plan_query_table_refresh(
        open.document.presentation(),
        args,
        rows,
        definition,
        &[],
    )?;
    if !checks.is_empty() {
        summary["checks"] = json!(checks);
    }
    crate::edit::execute(open, commands)?;
    Ok(crate::edit::edited(id, open, summary, Vec::new()))
}

pub fn prepare_selected_query_refresh(
    paths: &[PathBuf],
    mut args: Value,
    definition: Value,
) -> Result<SqliteTableExport, ToolError> {
    let bindings = args["sources"]
        .as_array_mut()
        .ok_or_else(|| ToolError::invalid("sources must be a list of selected file numbers"))?;
    for entry in bindings {
        *entry = json!(selected_file(paths, entry)?);
    }
    let service = selected_file_service(paths);
    let (rows, checks) =
        refresh_query_result(&service, &bound_query(&definition, &args)?, &definition)?;
    Ok(SqliteTableExport {
        arguments: args,
        rows,
        definition,
        refresh: true,
        checks,
    })
}

const MAX_REPORTS_PER_BATCH: usize = 12;

/// Snapshot the report definitions before the assistant dispatches file I/O.
pub fn batch_query_report_definitions(
    workbook: &ibis_doc_model::Presentation,
    args: &Value,
) -> Result<Vec<(String, Value)>, ToolError> {
    let book = workbook
        .workbook
        .as_ref()
        .ok_or_else(|| ToolError::invalid("Workbook is missing"))?;
    let sheets = crate::tools_xlsx::workbook_sheets(workbook);
    let available = |name: &str| {
        sheets.iter().any(|(_, _, sheet)| {
            sheet
                .tables
                .iter()
                .any(|table| table.name.eq_ignore_ascii_case(name))
        })
    };
    let mut reports = Vec::new();
    if let Some(tables) = args.get("tables") {
        let tables = tables
            .as_array()
            .filter(|names| (1..=MAX_REPORTS_PER_BATCH).contains(&names.len()))
            .ok_or_else(|| ToolError::invalid("Specify 1 to 12 query tables"))?;
        let mut seen = std::collections::HashSet::new();
        for table in tables {
            let name = table
                .as_str()
                .filter(|name| seen.insert(name.to_ascii_lowercase()))
                .ok_or_else(|| ToolError::invalid("Query table names must be distinct strings"))?;
            if !available(name) {
                return Err(ToolError::new(
                    "no_such_query_report",
                    format!("No query table named {name}"),
                ));
            }
            reports.push((
                name.to_owned(),
                query_report_definition(workbook, &json!({"table":name}))?,
            ));
        }
    } else {
        for (name, definition) in &book.query_reports {
            if available(name) {
                if reports.len() == MAX_REPORTS_PER_BATCH {
                    return Err(ToolError::invalid(
                        "Choose up to 12 query tables for this batch",
                    ));
                }
                reports.push((name.clone(), definition.clone()));
            }
        }
    }
    if reports.is_empty() {
        return Err(ToolError::new(
            "no_such_query_report",
            "Workbook has no saved query reports",
        ));
    }
    Ok(reports)
}

struct QueryReportResult {
    name: String,
    definition: Value,
    rows: SqliteRows,
    checks: Vec<Value>,
}

fn batch_bindings(args: &Value) -> Result<&serde_json::Map<String, Value>, ToolError> {
    args["bindings"]
        .as_object()
        .filter(|values| (1..=96).contains(&values.len()))
        .ok_or_else(|| {
            ToolError::invalid("Provide 1 to 96 source bindings keyed by saved file name")
        })
}

fn report_error(name: &str, error: ToolError) -> ToolError {
    ToolError::new(error.code, format!("{name}: {}", error.message))
}

fn batch_query_results(
    service: &Service,
    args: &Value,
    reports: Vec<(String, Value)>,
) -> Result<Vec<QueryReportResult>, ToolError> {
    let bindings = batch_bindings(args)?;
    reports
        .into_iter()
        .map(|(name, definition)| {
            let (rows, checks) = (|| {
                let sources = definition["sources"]
                    .as_array()
                    .filter(|sources| (1..=8).contains(&sources.len()))
                    .ok_or_else(|| ToolError::invalid("Invalid saved report sources"))?;
                let sources = sources
                    .iter()
                    .map(|source| {
                        let file = source.str_arg("file")?;
                        bindings
                            .get(file)
                            .filter(|path| path.is_string())
                            .ok_or_else(|| {
                                ToolError::invalid(format!("Missing file binding for {file}"))
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                refresh_query_result(
                    service,
                    &bound_query(&definition, &json!({"sources":sources}))?,
                    &definition,
                )
            })()
            .map_err(|error| report_error(&name, error))?;
            Ok(QueryReportResult {
                name,
                definition,
                rows,
                checks,
            })
        })
        .collect()
}

fn batch_metrics<'a>(
    args: &'a Value,
    results: &[QueryReportResult],
) -> Result<std::collections::HashMap<String, Vec<&'a str>>, ToolError> {
    let mut columns = std::collections::HashMap::<String, Vec<&str>>::new();
    let Some(metrics) = args.get("metrics") else {
        return Ok(columns);
    };
    let metrics = metrics
        .as_array()
        .filter(|metrics| metrics.len() <= 48)
        .ok_or_else(|| ToolError::invalid("metrics must be an array of up to 48 entries"))?;
    for metric in metrics {
        crate::edit::only_known(metric, &["table", "column"])?;
        let table = metric.str_arg("table")?;
        let column = metric.str_arg("column")?;
        let result = results
            .iter()
            .find(|report| report.name.eq_ignore_ascii_case(table))
            .ok_or_else(|| {
                ToolError::invalid(format!(
                    "Metric table {table} is outside this refresh batch"
                ))
            })?;
        let selected = columns.entry(result.name.clone()).or_default();
        if selected.contains(&column) {
            return Err(ToolError::invalid(format!(
                "Duplicate metric {table}.{column}"
            )));
        }
        selected.push(column);
    }
    Ok(columns)
}

fn apply_batch_refresh(
    service: &mut Service,
    args: &Value,
    results: Vec<QueryReportResult>,
) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let preview = args.opt_bool("preview")?.unwrap_or(false);
    let metrics = batch_metrics(args, &results)?;
    let open = crate::tools_xlsx::workbook_document(service, id)?;
    let mut scratch = ibis_doc_model::Document::new(open.document.presentation().clone());
    let mut reports = Vec::with_capacity(results.len());
    let mut commands = Vec::new();
    let mut blocked = false;
    for result in results {
        let check_result = enforce_checks(&result.checks);
        blocked |= check_result.is_err();
        if !preview {
            check_result.map_err(|error| report_error(&result.name, error))?;
        }
        let (mut summary, update) = crate::tools_pivot::plan_query_table_refresh(
            scratch.presentation(),
            &json!({"table":result.name}),
            result.rows,
            &result.definition,
            metrics.get(&result.name).map_or(&[], Vec::as_slice),
        )
        .map_err(|error| report_error(&result.name, error))?;
        scratch.execute(update.clone()).map_err(|error| {
            ToolError::new("edit_rejected", format!("{}: {error:?}", result.name))
        })?;
        if !result.checks.is_empty() {
            summary["checks"] = json!(result.checks);
        }
        reports.push(summary);
        commands.extend(update);
    }
    if !preview {
        crate::edit::execute(open, commands)?;
    }
    Ok(crate::edit::edited(
        id,
        open,
        json!({"preview":preview,"blocked":blocked,"reports":reports}),
        Vec::new(),
    ))
}

fn refresh_query_reports(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let open = crate::tools_xlsx::workbook_document(service, args.str_arg("document_id")?)?;
    let reports = batch_query_report_definitions(open.document.presentation(), args)?;
    let results = batch_query_results(service, args, reports)?;
    apply_batch_refresh(service, args, results)
}

pub struct SqliteReportBatch {
    arguments: Value,
    results: Vec<QueryReportResult>,
}

impl SqliteReportBatch {
    pub fn stage(
        self,
        document: &mut ibis_doc_model::Document,
        context: crate::DraftToolContext,
    ) -> Result<(Value, Vec<ibis_doc_model::Command>), ToolError> {
        crate::service::run_draft_operation(
            ibis_formats::Kind::Workbook,
            document,
            context,
            self.arguments,
            |service, args| apply_batch_refresh(service, args, self.results),
        )
    }
}

fn selected_file<'a>(paths: &'a [PathBuf], entry: &Value) -> Result<&'a str, ToolError> {
    entry
        .as_str()
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(|number| number.checked_sub(1))
        .and_then(|index| paths.get(index))
        .and_then(|path| path.to_str())
        .ok_or_else(|| ToolError::invalid("Use only file numbers selected in the assistant panel"))
}

pub fn prepare_selected_report_batch(
    paths: &[PathBuf],
    mut args: Value,
    reports: Vec<(String, Value)>,
) -> Result<SqliteReportBatch, ToolError> {
    batch_bindings(&args)?;
    for binding in args["bindings"].as_object_mut().unwrap().values_mut() {
        *binding = json!(selected_file(paths, binding)?);
    }
    let service = selected_file_service(paths);
    let results = batch_query_results(&service, &args, reports)?;
    Ok(SqliteReportBatch {
        arguments: args,
        results,
    })
}

pub(crate) const SET_QUERY_REPORT_CHECKS: Tool = Tool {
    name: "xlsx_set_query_report_checks",
    description: "Save read-only count checks on a query report. Checks run with its source bindings on every refresh. An empty list removes its checks.",
    access: Access::Edit,
    returns_content: false,
    schema: || {
        object(
            &[
                document_id(),
                ("table", string("Name of an existing query report.")),
                (
                    "checks",
                    json!({
                        "type":"array","maxItems":4,
                        "items":object(&[
                            ("name", string("Name to identify the check in preview results.")),
                            ("sql", string("One SELECT using the report's bound tables, returning one nonnegative integer.")),
                            ("params", json!({"type":"array","maxItems":100,"items":{"type":["string","number","null"]}})),
                            ("max", integer("Optional highest accepted count. Omit for an informational check.")),
                        ], &["name","sql"]),
                    }),
                ),
            ],
            &["document_id", "table", "checks"],
        )
    },
    run: set_query_report_checks,
};

fn validate_report_checks(value: &Value) -> Result<&[Value], ToolError> {
    let checks = value
        .as_array()
        .filter(|checks| checks.len() <= 4)
        .ok_or_else(|| ToolError::invalid("checks must be an array of up to four count queries"))?;
    let mut names = std::collections::HashSet::new();
    for check in checks {
        crate::edit::only_known(check, &["name", "sql", "params", "max"])?;
        let name = check.str_arg("name")?;
        if name.trim().is_empty() || name.chars().count() > 80 || !names.insert(name) {
            return Err(ToolError::invalid(
                "Check names must be distinct, nonblank and at most 80 characters",
            ));
        }
        validate_sqlite_select(check.str_arg("sql")?)
            .map_err(|error| ToolError::invalid(format!("{name}: {error}")))?;
        sql_params(check)?;
        if let Some(max) = check.get("max")
            && !max.as_i64().is_some_and(|max| max >= 0)
        {
            return Err(ToolError::invalid(
                "max must be a nonnegative signed 64-bit integer",
            ));
        }
    }
    Ok(checks)
}

pub(crate) fn check_report_budget(
    workbook: &ibis_doc_model::Presentation,
    replacing: Option<&str>,
    definition: &Value,
) -> Result<(), ToolError> {
    if workbook.workbook.as_ref().is_some_and(|book| {
        book.query_reports
            .iter()
            .filter(|(name, _)| Some(name.as_str()) != replacing)
            .map(|(_, definition)| definition.to_string().len())
            .sum::<usize>()
            .saturating_add(definition.to_string().len())
            > 512 * 1024
    }) {
        return Err(ToolError::invalid(
            "The workbook's saved query definitions exceed 512 KiB.",
        ));
    }
    Ok(())
}

fn set_query_report_checks(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    use ibis_doc_model::{Command, SheetCommand};

    let checks = validate_report_checks(&args["checks"])?;
    let id = args.str_arg("document_id")?;
    let open = crate::tools_xlsx::workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let name = args.str_arg("table")?;
    let mut definition = query_report_definition(workbook, args)?;
    let table = crate::tools_xlsx::workbook_sheets(workbook)
        .iter()
        .flat_map(|(_, _, sheet)| &sheet.tables)
        .find(|table| table.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| ToolError::invalid("The report table is missing."))?
        .name
        .clone();
    definition
        .as_object_mut()
        .ok_or_else(|| ToolError::invalid("Invalid report definition"))?
        .remove("checks");
    if !checks.is_empty() {
        definition["checks"] = json!(checks);
    }
    if definition.to_string().len() > 32 * 1024 {
        return Err(ToolError::invalid(
            "The saved report definition exceeds 32 KiB",
        ));
    }
    check_report_budget(workbook, Some(&table), &definition)?;
    let summary = json!({"table":table,"checks":checks.len()});
    crate::edit::execute(
        open,
        vec![Command::Sheet(SheetCommand::SetQueryReport {
            table,
            definition,
        })],
    )?;
    Ok(crate::edit::edited(id, open, summary, Vec::new()))
}

fn refresh_query_result(
    service: &Service,
    args: &Value,
    definition: &Value,
) -> Result<(SqliteRows, Vec<Value>), ToolError> {
    let checks = definition
        .get("checks")
        .map(validate_report_checks)
        .transpose()?
        .unwrap_or_default();
    let session = query_session(service, args)?;
    let checks = checks
        .iter()
        .map(|check| {
            let name = check.str_arg("name")?;
            let result = run_session_query(&session, check, 1, 1024)
                .map_err(|error| report_error(name, error))?;
            let count = match result.rows.as_slice() {
                [row] if !result.truncated && row.len() == 1 => {
                    row[0].as_i64().filter(|&count| count >= 0)
                }
                _ => None,
            }
            .ok_or_else(|| {
                ToolError::invalid(format!("{name}: check must return one nonnegative integer"))
            })?;
            let max = check["max"].as_i64();
            let passed = max.map(|max| count <= max);
            Ok(json!({"name":name,"count":count,"max":max,"passed":passed}))
        })
        .collect::<Result<Vec<_>, ToolError>>()?;
    let rows = require_complete(run_session_query(&session, args, 10_000, 4 * 1024 * 1024)?)?;
    Ok((rows, checks))
}

fn enforce_checks(checks: &[Value]) -> Result<(), ToolError> {
    if let Some(check) = checks.iter().find(|check| check["passed"] == false) {
        return Err(ToolError::new(
            "query_check_failed",
            format!(
                "Check {} returned {}, exceeding its maximum {}",
                check["name"], check["count"], check["max"],
            ),
        ));
    }
    Ok(())
}
