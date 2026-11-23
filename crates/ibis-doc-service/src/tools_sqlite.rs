// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::path::{Path, PathBuf};

use ibis_doc_model::query::{
    DATASET_MEMORY_LIMIT, DiskDataset, ParquetBound, ParquetPredicate, QColumn, QTable, QType, SqliteInputSchema, SqlitePredicate, SqliteQuerySession,
    SqliteRows, SqliteScalar, dataset_sql_type, read_arrow_ipc_dataset, parquet_dataset_columns, read_parquet_filtered, read_sqlite_table_bounded, sqlite_columns,
    sqlite_count, sqlite_dataset_columns, sqlite_tables, validate_sqlite_select,
};
use serde_json::{Value, json};

use crate::catalog::{Access, Tool, boolean, document_id, integer, object, string};
use crate::service::{Args, Limits, Service};
use crate::{ToolError, paths};

pub const DESKTOP_TEXT_DATASET_LIMIT: u64 = 512 * 1024 * 1024;

pub const DATASET_EXTENSIONS: &[&str] = &["csv", "parquet", "arrow", "ipc", "feather", "jsonl", "ndjson", "avro", "sqlite", "sqlite3", "db", "log", "syslog"];
const EXTENSIONS: &[&str] = &["sqlite", "sqlite3", "db"];
const PATH: &str = "path";
const MAX_FILTERS: usize = 16;
const REPORT_DEFINITION_BYTES: usize = 32 * 1024;

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
        name: "sqlite_describe_datasets",
        description: "Inspect SQL column names and storage types before writing a query. Use the same datasets array as sqlite_query. Returns schema without rows.",
        access: Access::Read,
        returns_content: true,
        schema: || object(&[datasets_property()], &["datasets"]),
        run: describe_datasets,
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
                ("parameter_controls", json!({"type":"array","maxItems":12,"items":object(&[
                    ("name", string("Shared parameter name across reports.")),
                    ("label", string("Selector label.")),
                    ("slot", integer("One-based position in params.")),
                    ("kind", json!({"type":"string","enum":["text","integer","number"]})),
                    ("sheet", string("Worksheet containing choices.")),
                    ("range", string("Two columns, database value followed by display label. Exclude headers.")),
                ], &["name","label","slot","kind","sheet","range"])})),
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
        description: "Re-run the saved query after binding its source files in order, replace the table, and refresh its linked pivots and charts in one edit. Returns input schema changes as warnings.",
        access: Access::Edit,
        returns_content: false,
        schema: || {
            object(
                &[
                    document_id(),
                    ("table", string("Table name from xlsx_list_query_reports.")),
                    parameter_values_property(),
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
        description: "Preview or atomically refresh workbook query reports from shared file bindings. Preview reports row counts and input schema changes and checks for overlap without editing.",
        access: Access::Edit,
        returns_content: true,
        schema: || {
            object(
                &[
                    document_id(),
                    parameter_values_property(),
                    ("snapshot_id", string("Apply the most recent successful preview without rereading its sources.")),
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
                        "arrow_disk",
                        boolean("Experimental: stage identical Arrow IPC dataset bindings once in a private temporary database for this batch."),
                    ),
                    (
                        "preview",
                        boolean("Validate without editing and report row changes (default false)."),
                    ),
                ],
                &["document_id"],
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
    Service::new(&folders, true, Limits {
        max_text_dataset_file_size: Some(DESKTOP_TEXT_DATASET_LIMIT),
        ..Limits::default()
    })
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
        datasets_property(),
        (
            "params",
            json!({"type":"array","maxItems":100,"items":{"type":["string","number","null"]}}),
        ),
    ]
}

fn datasets_property() -> (&'static str, Value) {
    (
        "datasets",
        json!({
            "type":"array","minItems":1,"maxItems":8,
            "description":"Register CSV, Parquet, Arrow IPC, JSON Lines, Avro, SQLite and UTF-8 log files. Logs expose line_number and message.",
            "items":object(&[
                ("source", string("Path to a supported dataset file in an allowed folder.")),
                ("alias", string("SQL table name to register for this dataset.")),
                ("table", string("Source table name, required for SQLite files.")),
                ("recover", json!({"type":"boolean","description":"JSON Lines only: retain invalid records with _jsonl_line and _jsonl_error. Defaults to false."})),
                ("paths", json!({
                    "type":"array","minItems":1,"maxItems":254,"uniqueItems":true,
                    "items":{"type":"string","minLength":1},
                    "description":"JSON Lines only: select nested fields by JSON Pointer, in order. Includes recovery diagnostics. Mutually exclusive with columns."
                })),
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
                    "description":"Parquet, Arrow IPC, JSON Lines and Avro: exact column names to load, in order. Omit to load all columns."
                })),
            ], &["source","alias"]),
        }),
    )
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
    if args.get("datasets").is_some() && args.opt_str(PATH)?.is_some() {
        return Err(ToolError::invalid("use path for a SQLite database or datasets for registered tables"));
    }
    batch_session(service, args, false)
}

fn memory_query_session(service: &Service, args: &Value) -> Result<SqliteQuerySession, ToolError> {
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
    #[cfg(feature = "report-profiling")]
    let _timer = ibis_doc_model::query::report_profile::Timer::start(
        ibis_doc_model::query::report_profile::Phase::Sql,
    );
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

fn prepare_export(service: &Service, args: &Value) -> Result<(SqliteRows, Value), ToolError> {
    let session = query_session(service, args)?;
    let rows = require_complete(run_session_query(&session, args, 10_000, 4 * 1024 * 1024)?)?;
    let mut definition = saved_definition(args)?;
    if let Some(schema) = report_input_schema(&session)? {
        set_input_schema(&mut definition, &schema);
    }
    Ok((rows, definition))
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
    let (rows, definition) = prepare_export(service, args)?;
    crate::tools_pivot::write_table(service, args, rows, definition)
}

/// The complete database result stays in-process between the worker and the draft.
pub struct SqliteTableExport {
    arguments: Value,
    rows: SqliteRows,
    definition: Value,
    refresh: Option<RefreshReview>,
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
                if let Some(review) = self.refresh {
                    apply_single_refresh(service, args, self.rows, &self.definition, &review)
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
    let (rows, definition) = prepare_export(&service, &args)?;
    Ok(SqliteTableExport {
        arguments: args,
        rows,
        definition,
        refresh: None,
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

fn dataset_alias<'a>(
    spec: &'a Value,
    aliases: &mut std::collections::HashSet<String>,
) -> Result<&'a str, ToolError> {
    let alias = spec.str_arg("alias")?;
    if alias.is_empty()
        || alias.len() > 128
        || !alias.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || alias.as_bytes()[0].is_ascii_digit()
        || !aliases.insert(alias.to_ascii_lowercase())
    {
        return Err(ToolError::invalid(
            "dataset aliases must be distinct SQL identifiers using letters, digits or underscores",
        ));
    }
    Ok(alias)
}

fn parquet_filters<'a>(spec: &'a Value, kind: &str) -> Result<Vec<(&'a str, ParquetPredicate)>, ToolError> {
    let Some(filters) = spec.get("where") else { return Ok(Vec::new()) };
    if kind != "parquet" {
        return Err(ToolError::invalid("where is only used for Parquet datasets"));
    }
    filters.as_object()
        .filter(|items| (1..=MAX_FILTERS).contains(&items.len()))
        .ok_or_else(|| ToolError::invalid(format!("where must have 1 to {MAX_FILTERS} conditions")))?
        .iter()
        .map(|(name, value)| Ok((name.as_str(), parquet_predicate(value)?)))
        .collect()
}

fn json_lines_dataset(
    spec: &Value,
    path: PathBuf,
    columns: Vec<String>,
) -> Result<DiskDataset, ToolError> {
    if let Some(paths) = spec.get("paths") {
        if spec.get("columns").is_some() {
            return Err(ToolError::invalid("Use either paths or columns for JSON Lines"));
        }
        let paths = dataset_names(paths, 254)
            .ok_or_else(|| ToolError::invalid("paths must contain 1 to 254 non-empty JSON Pointers"))?;
        return Ok(DiskDataset::JsonLinesPaths(path, paths));
    }
    if spec.opt_bool("recover")?.unwrap_or(false) {
        return Ok(DiskDataset::RecoverableJsonLines(path, columns));
    }
    Ok(DiskDataset::JsonLines(path, columns))
}

fn dataset_names(value: &Value, limit: usize) -> Option<Vec<String>> {
    value.as_array()
        .filter(|items| (1..=limit).contains(&items.len()))?
        .iter()
        .map(|item| item.as_str().filter(|name| !name.is_empty()).map(str::to_owned))
        .collect()
}

fn dataset_columns(spec: &Value, kind: &str) -> Result<Vec<String>, ToolError> {
    match spec.get("columns") {
        None => Ok(Vec::new()),
        Some(_) if !matches!(kind, "parquet" | "arrow_ipc" | "json_lines" | "avro") => Err(ToolError::invalid(
            "columns is only used for Parquet, Arrow IPC, JSON Lines and Avro datasets",
        )),
        Some(value) => dataset_names(value, 256)
            .ok_or_else(|| ToolError::invalid("columns must contain 1 to 256 non-empty column names")),
    }
}

fn describe_datasets(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    crate::edit::only_known(args, &["datasets"])?;
    let mut result = Vec::new();
    let envelope_bytes = r#"{"datasets":[]}"#.len();
    let mut remaining = service.limits.page_bytes.saturating_sub(envelope_bytes);
    visit_datasets(service, &args["datasets"], true, |alias, kind, source_table, table| {
        let columns: Vec<_> = table.columns.into_iter().map(|column| {
            json!({"name": column.name, "sql_type": dataset_sql_type(column.ty)})
        }).collect();
        let mut entry = json!({"alias": alias, "format": kind, "columns": columns});
        if let Some(name) = source_table {
            entry["table"] = json!(name);
        }
        let bytes = entry.to_string().len() + usize::from(!result.is_empty());
        remaining = remaining.checked_sub(bytes)
            .ok_or_else(|| ToolError::new("result_too_large", "Dataset schema exceeds the response limit"))?;
        result.push(entry);
        Ok(())
    })?;
    Ok(json!({"datasets": result}))
}

fn load_datasets(service: &Service, specs: &Value) -> Result<Vec<(String, QTable)>, ToolError> {
    let mut datasets = Vec::new();
    visit_datasets(service, specs, false, |alias, _, _, table| {
        datasets.push((alias.to_owned(), table));
        Ok(())
    })?;
    Ok(datasets)
}

fn visit_datasets(
    service: &Service,
    specs: &Value,
    schema_only: bool,
    mut visit: impl FnMut(&str, &str, Option<&str>, QTable) -> Result<(), ToolError>,
) -> Result<(), ToolError> {
    use ibis_doc_model::CellValue;
    let specs = specs
        .as_array()
        .filter(|items| (1..=8).contains(&items.len()))
        .ok_or_else(|| ToolError::invalid("datasets must have 1 to 8 entries"))?;
    let mut aliases = std::collections::HashSet::new();
    let mut cells = 100_000usize;
    let mut text = 8 * 1024 * 1024usize;
    for spec in specs {
        crate::edit::only_known(spec, &["source", "alias", "table", "columns", "where", "recover", "paths"])?;
        let alias = dataset_alias(spec, &mut aliases)?;
        let path = paths::resolve(&service.allow_dirs, spec.str_arg("source")?, DATASET_EXTENSIONS, true)?;
        let metadata = std::fs::metadata(&path).map_err(sqlite_error)?;
        let kind = source_kind(&path);
        let limit = dataset_file_limit(&service.limits, kind);
        let full_schema_scan = kind != "json_lines"
            || spec["recover"] == true
            || spec.get("paths").is_some()
            || service.limits.max_text_dataset_file_size.is_some();
        let limit = if !schema_only || !full_schema_scan {
            limit.min(32 * 1024 * 1024)
        } else {
            limit
        };
        if !metadata.is_file() || metadata.len() > limit {
            return Err(ToolError::invalid("dataset must be a regular file within the input size limit"));
        }
        let table_name = spec.opt_str("table")?;
        if kind != "json_lines" && (spec.get("recover").is_some() || spec.get("paths").is_some()) {
            return Err(ToolError::invalid("recover and paths are only used for JSON Lines datasets"));
        }
        let columns = dataset_columns(spec, kind)?;
        let filters = parquet_filters(spec, kind)?;
        if kind != "sqlite" && table_name.is_some() {
            return Err(ToolError::invalid("table is only used for SQLite datasets"));
        }
        let schema = |columns| QTable { columns, rows: Vec::new() };
        let table = if kind == "csv" {
            match read_csv_dataset(&path, cells, text, schema_only) {
                Err(error) if schema_only && csv_size_limit(&error.message) => {
                    QTable { columns: DiskDataset::Csv(path).columns().map_err(sqlite_error)?, rows: Vec::new() }
                }
                result => result?,
            }
        } else {
            match (kind, schema_only) {
                ("sqlite", _) => {
                    let name = table_name.ok_or_else(|| ToolError::invalid("SQLite datasets require a source table"))?;
                    if schema_only {
                        sqlite_dataset_columns(&path, name).map(schema)
                    } else {
                        read_sqlite_table_bounded(&path, name, cells, text)
                    }
                }
                ("parquet", true) => parquet_dataset_columns(&path, &columns, &filters).map(schema),
                ("parquet", false) => read_parquet_filtered(&path, &columns, &filters, cells, text),
                ("arrow_ipc", true) => DiskDataset::ArrowIpc(path, columns).columns().map(schema),
                ("arrow_ipc", false) => read_arrow_ipc_dataset(&path, &columns, cells, text),
                ("log", _) => {
                    let source = DiskDataset::Log(path);
                    if schema_only { source.columns().map(schema) } else { source.collect_bounded(cells, text) }
                }
                ("json_lines" | "avro", _) => {
                    let source = if kind == "json_lines" {
                        json_lines_dataset(spec, path, columns)?
                    } else {
                        DiskDataset::Avro(path, columns)
                    };
                    if schema_only {
                        source.columns().map(schema)
                    } else {
                        source.collect_bounded(cells, text)
                    }
                }
                _ => return Err(ToolError::invalid("unsupported dataset format")),
            }.map_err(sqlite_error)?
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
        visit(alias, kind, table_name, table)?;
    }
    Ok(())
}

fn csv_size_limit(message: &str) -> bool {
    matches!(message, "CSV exceeds the import size limit" | "CSV exceeds the decoded size limit")
}

fn read_csv_dataset(path: &Path, max_cells: usize, max_text: usize, schema_only: bool) -> Result<QTable, ToolError> {
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
    let columns: Vec<_> = headers
        .into_iter()
        .map(|name| QColumn {
            name,
            ty: QType::Text,
        })
        .collect();
    let mut rows = Vec::new();
    if !schema_only {
        let width = columns.len();
        for record in records {
            if record.len() != width {
                return Err(ToolError::invalid("CSV rows must match their header width"));
            }
            if (rows.len() + 1) * width > max_cells {
                return Err(ToolError::invalid(DATASET_MEMORY_LIMIT));
            }
            rows.push(record.into_iter().map(CellValue::Text).collect());
        }
    }
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
    } else if path.extension().is_some_and(|ext| {
        ["jsonl", "ndjson"].iter().any(|name| ext.eq_ignore_ascii_case(name))
    }) {
        "json_lines"
    } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("log") || ext.eq_ignore_ascii_case("syslog")) {
        "log"
    } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("avro")) {
        "avro"
    } else if path.extension().is_some_and(|ext| {
        ["arrow", "ipc", "feather"].iter().any(|name| ext.eq_ignore_ascii_case(name))
    }) {
        "arrow_ipc"
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

fn parameter_values_property() -> (&'static str, Value) {
    ("parameter_values", json!({"type":"object","maxProperties":12,
        "description":"Override typed control values by name for the selected reports.",
        "additionalProperties":{"type":["string","number"]}}))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportParameterKind {
    Text,
    Integer,
    Number,
}

impl ReportParameterKind {
    pub fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Text => value.as_str().is_some_and(|text| text.len() <= 1024),
            Self::Integer => value.as_i64().is_some(),
            Self::Number => value.as_i64().is_some() || (value.is_f64() && value.as_f64().is_some_and(f64::is_finite)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportParameterControl {
    pub name: String,
    pub label: String,
    pub slot: usize,
    pub kind: ReportParameterKind,
    pub sheet: String,
    pub range: String,
}

pub fn report_parameter_controls(definition: &Value) -> Result<Vec<ReportParameterControl>, ToolError> {
    let Some(raw) = definition.get("parameter_controls") else { return Ok(Vec::new()) };
    let controls: Vec<ReportParameterControl> = serde_json::from_value(raw.clone())
        .map_err(|_| ToolError::invalid("Invalid report parameter controls"))?;
    let mut names = std::collections::BTreeSet::new();
    let mut slots = std::collections::BTreeSet::new();
    if controls.len() > 12 {
        return Err(ToolError::invalid("Too many report parameter controls"));
    }
    for control in &controls {
        let range = ibis_doc_model::ai_sheet::parse_cell_range(&control.range);
        if control.name.is_empty() || control.name.len() > 64
            || !control.name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || control.label.trim().is_empty() || control.label.len() > 256
            || control.sheet.trim().is_empty() || !names.insert(&control.name)
            || !slots.insert(control.slot) || !(1..=100).contains(&control.slot)
            || !control.kind.accepts(&definition["params"][control.slot.saturating_sub(1)])
            || !range.is_some_and(|r| r.last_column - r.first_column == 1 && r.last_row - r.first_row < 10_000)
        {
            return Err(ToolError::invalid(format!("Invalid report parameter control: {}", control.name)));
        }
    }
    Ok(controls)
}

fn validate_parameter_values<'a>(
    args: &Value,
    definitions: impl IntoIterator<Item = &'a Value>,
) -> Result<(), ToolError> {
    let Some(values) = args.get("parameter_values") else { return Ok(()) };
    let values = values.as_object().filter(|v| v.len() <= 12)
        .ok_or_else(|| ToolError::invalid("Invalid report parameter values"))?;
    let mut known = std::collections::BTreeSet::new();
    for definition in definitions {
        for control in report_parameter_controls(definition)? {
            if let Some(value) = values.get(&control.name) {
                if !control.kind.accepts(value) {
                    return Err(ToolError::invalid(format!("Wrong type for report parameter {}", control.name)));
                }
                known.insert(control.name);
            }
        }
    }
    if let Some(name) = values.keys().find(|name| !known.contains(*name)) {
        return Err(ToolError::invalid(format!("Unknown report parameter {name}")));
    }
    Ok(())
}

fn saved_definition(args: &Value) -> Result<Value, ToolError> {
    let source = |path: &str, spec: &Value| {
        let path = Path::new(path);
        let mut source = json!({
            "file": path.file_name().unwrap_or_default().to_string_lossy(),
            "kind": source_kind(path),
            "alias": spec["alias"], "table": spec["table"],
        });
        for key in ["columns", "where", "recover", "paths"] {
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
    let mut definition = json!({
        "sql":args["sql"], "params":args.get("params").cloned().unwrap_or_else(|| json!([])),
        "sources":sources,
    });
    if let Some(controls) = args.get("parameter_controls") {
        definition["parameter_controls"] = controls.clone();
        report_parameter_controls(&definition)?;
    }
    if definition.to_string().len() > REPORT_DEFINITION_BYTES {
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
    let mut params = definition["params"].clone();
    for control in report_parameter_controls(definition)? {
        if let Some(value) = args["parameter_values"].get(&control.name) {
            params[control.slot - 1] = value.clone();
        }
    }
    let mut query = json!({"sql":definition["sql"],"params":params});
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
            for key in ["columns", "where", "recover", "paths"] {
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

fn report_input_schema(session: &SqliteQuerySession) -> Result<Option<SqliteInputSchema>, ToolError> {
    session.input_schema(REPORT_DEFINITION_BYTES).map_err(sqlite_error)
}

fn set_input_schema(definition: &mut Value, schema: &SqliteInputSchema) -> bool {
    let previous = definition["input_schema"].clone();
    definition["input_schema"] = json!(schema);
    if definition.to_string().len() <= REPORT_DEFINITION_BYTES {
        return true;
    }
    if previous.is_null() {
        definition.as_object_mut().unwrap().remove("input_schema");
    } else {
        definition["input_schema"] = previous;
    }
    false
}

struct RefreshReview {
    checks: Vec<Value>,
    input_schema: Option<SqliteInputSchema>,
    changed_params: Option<Value>,
}

impl RefreshReview {
    fn summarize(
        &self,
        definition: &Value,
        summary: &mut Value,
        commands: &mut Vec<ibis_doc_model::Command>,
    ) {
        use ibis_doc_model::{Command, SheetCommand};

        if !self.checks.is_empty() {
            summary["checks"] = json!(self.checks);
        }
        let mut next = definition.clone();
        if let Some(params) = &self.changed_params {
            next["params"] = params.clone();
        }
        let previous = definition.get("input_schema")
            .and_then(|schema| serde_json::from_value::<SqliteInputSchema>(schema.clone()).ok());
        let current = self.input_schema.as_ref();
        let schema_changed = current.is_some() && previous.as_ref() != current;
        match current {
            Some(schema) if schema_changed => {
                if !set_input_schema(&mut next, schema) {
                    summary["input_schema_status"] = json!("unavailable");
                }
            }
            None => summary["input_schema_status"] = json!("unavailable"),
            _ => {}
        }
        if next != *definition {
            commands.push(Command::Sheet(SheetCommand::SetQueryReport {
                table: summary["table"].as_str().expect("refresh summary has a table name").to_owned(),
                definition: next,
            }));
        }
        let Some(current) = current.filter(|_| schema_changed) else { return };
        if let Some(previous) = previous {
            let mut changes = Vec::new();
            let mut count = 0;
            let mut record = |source: &str, column: &str, from: Option<&str>, to: Option<&str>| {
                count += 1;
                if changes.len() < 20 {
                    changes.push(json!({"source":source,"column":column,"previous":from,"current":to}));
                }
            };
            for (source, columns) in &previous {
                for (column, from) in columns {
                    let to = current.get(source).and_then(|columns| columns.get(column));
                    if to != Some(from) {
                        record(source, column, Some(from), to.map(String::as_str));
                    }
                }
            }
            for (source, columns) in current {
                for (column, to) in columns {
                    if !previous.get(source).is_some_and(|columns| columns.contains_key(column)) {
                        record(source, column, None, Some(to));
                    }
                }
            }
            if count != 0 {
                summary["schema_changes"] = json!(changes);
                summary["schema_change_count"] = json!(count);
            }
        }
    }
}

fn refresh_query_table(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let open = crate::tools_xlsx::workbook_document(service, args.str_arg("document_id")?)?;
    let definition = query_report_definition(open.document.presentation(), args)?;
    validate_parameter_values(args, [&definition])?;
    let query = bound_query(&definition, args)?;
    let (rows, review) = refresh_query_result(service, &query, &definition)?;
    apply_single_refresh(service, args, rows, &definition, &review)
}

fn apply_single_refresh(
    service: &mut Service,
    args: &Value,
    rows: SqliteRows,
    definition: &Value,
    review: &RefreshReview,
) -> Result<Value, ToolError> {
    enforce_checks(&review.checks)?;
    let id = args.str_arg("document_id")?;
    let open = crate::tools_xlsx::workbook_document(service, id)?;
    let (mut summary, mut commands) = crate::tools_pivot::plan_query_table_refresh(
        open.document.presentation(),
        args,
        rows,
        definition,
        &[],
    )?;
    review.summarize(definition, &mut summary, &mut commands);
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
    validate_parameter_values(&args, [&definition])?;
    let (rows, review) =
        refresh_query_result(&service, &bound_query(&definition, &args)?, &definition)?;
    Ok(SqliteTableExport {
        arguments: args,
        rows,
        definition,
        refresh: Some(review),
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
    review: RefreshReview,
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

fn is_text_dataset(kind: &str) -> bool {
    matches!(kind, "csv" | "json_lines" | "log")
}

fn dataset_file_limit(limits: &Limits, kind: &str) -> u64 {
    if is_text_dataset(kind) {
        limits.max_text_dataset_file_size.unwrap_or(limits.max_file_size)
    } else {
        limits.max_file_size
    }
}

fn streamable_kind(kind: &str) -> bool {
    matches!(kind, "arrow_ipc" | "json_lines" | "avro" | "parquet" | "sqlite" | "csv" | "log")
}

fn disk_dataset_session(service: &Service, specs: &[Value], arrow_only: bool) -> Result<SqliteQuerySession, ToolError> {
    if !(1..=8).contains(&specs.len()) {
        return Err(ToolError::invalid("datasets must have 1 to 8 entries"));
    }
    let mut aliases = std::collections::HashSet::new();
    let mut sources = Vec::new();
    let mut estimated_bytes = 0u64;
    let mut bounded = Vec::new();
    for spec in specs {
        crate::edit::only_known(spec, &["source", "alias", "table", "columns", "where", "recover", "paths"])?;
        let alias = dataset_alias(spec, &mut aliases)?.to_owned();
        let path = paths::resolve(&service.allow_dirs, spec.str_arg("source")?, DATASET_EXTENSIONS, true)?;
        let kind = source_kind(&path);
        if arrow_only && kind != "arrow_ipc" {
            return Err(ToolError::invalid("Explicit Arrow disk staging requires Arrow IPC sources"));
        }
        if !streamable_kind(kind) {
            bounded.push(spec.clone());
            continue;
        }
        let filters = parquet_filters(spec, kind)?;
        if kind != "sqlite" && !spec["table"].is_null() {
            return Err(ToolError::invalid("table is only used for SQLite datasets"));
        }
        let metadata = std::fs::metadata(&path).map_err(sqlite_error)?;
        let limit = dataset_file_limit(&service.limits, kind);
        if !metadata.is_file() || metadata.len() > limit {
            return Err(ToolError::invalid(format!(
                "Dataset must be a regular file within the {} MiB input limit", limit / (1024 * 1024),
            )));
        }
        if kind != "json_lines" && (spec.get("recover").is_some() || spec.get("paths").is_some()) {
            return Err(ToolError::invalid("recover and paths are only used for JSON Lines datasets"));
        }
        let estimated = metadata.len().saturating_mul(if is_text_dataset(kind) { 3 } else { 1 });
        estimated_bytes = estimated_bytes.saturating_add(estimated);
        let columns = dataset_columns(spec, kind)?;
        let source = match kind {
            "arrow_ipc" => DiskDataset::ArrowIpc(path, columns),
            "json_lines" => json_lines_dataset(spec, path, columns)?,
            "parquet" => DiskDataset::Parquet(path, columns, filters.into_iter().map(|(name, value)| (name.to_owned(), value)).collect()),
            "sqlite" => DiskDataset::Sqlite(path, spec.opt_str("table")?
                .ok_or_else(|| ToolError::invalid("SQLite datasets require a source table"))?.to_owned()),
            "csv" => DiskDataset::Csv(path),
            "log" => DiskDataset::Log(path),
            _ => DiskDataset::Avro(path, columns),
        };
        sources.push((alias, source));
    }
    if !bounded.is_empty() {
        sources.extend(load_datasets(service, &json!(bounded))?.into_iter().map(|(alias, table)| {
            (alias, DiskDataset::Table(table))
        }));
    }
    SqliteQuerySession::from_disk_datasets_with_disk_estimate(&sources, estimated_bytes).map_err(sqlite_error)
}

fn batch_session(service: &Service, query: &Value, arrow_disk: bool) -> Result<SqliteQuerySession, ToolError> {
    #[cfg(test)]
    boundary_tests::SESSION_PREPARATIONS.with(|opens| opens.set(opens.get() + 1));
    const MEMORY_FILE_BYTES: u64 = 8 * 1024 * 1024;
    let Some(specs) = query["datasets"].as_array() else {
        if arrow_disk {
            return Err(ToolError::invalid("Explicit Arrow disk staging requires saved Arrow IPC datasets"));
        }
        return memory_query_session(service, query);
    };
    if !(1..=8).contains(&specs.len()) {
        return Err(ToolError::invalid("datasets must have 1 to 8 entries"));
    }
    if arrow_disk {
        return disk_dataset_session(service, specs, true);
    }
    let streaming = specs.iter().filter(|spec| spec["source"].as_str()
        .is_some_and(|path| streamable_kind(source_kind(Path::new(path)))));
    let mut bytes = 0u64;
    let mut has_streaming = false;
    for spec in streaming {
        has_streaming = true;
        let path = paths::resolve(&service.allow_dirs, spec.str_arg("source")?, DATASET_EXTENSIONS, true)?;
        bytes = bytes.saturating_add(std::fs::metadata(path).map_err(sqlite_error)?.len());
    }
    if bytes > MEMORY_FILE_BYTES || specs.iter().any(|spec| spec["recover"] == true || spec.get("paths").is_some()) {
        return disk_dataset_session(service, specs, false);
    }
    match memory_query_session(service, query) {
        Err(error) if has_streaming && (csv_size_limit(&error.message) || [
            DATASET_MEMORY_LIMIT,
            "SQLite table exceeds the query editor's table size limit",
            "SQLite text exceeds the import limit",
            "Parquet source exceeds the query editor's size limit",
            "Parquet text exceeds the import limit",
            "Arrow IPC source exceeds the query editor's table size limit",
            "Arrow IPC decoded buffers exceed the 128 MiB limit",
            "Arrow IPC text exceeds the 128 MiB limit",
            "Arrow IPC list exceeds the text budget",
        ].iter().any(|limit| error.message.ends_with(limit))) => disk_dataset_session(service, specs, false),
        result => result,
    }
}

fn batch_query_results(
    service: &Service,
    args: &Value,
    reports: Vec<(String, Value)>,
) -> Result<Vec<QueryReportResult>, ToolError> {
    let bindings = batch_bindings(args)?;
    validate_parameter_values(args, reports.iter().map(|(_, definition)| definition))?;
    let preview = args.opt_bool("preview")?.unwrap_or(false);
    let arrow_disk = args.opt_bool("arrow_disk")?.unwrap_or(false);
    let mut groups: Vec<(Value, Vec<_>)> = Vec::new();
    for (index, (name, definition)) in reports.into_iter().enumerate() {
        let query = (|| {
            let sources = definition["sources"].as_array()
                .filter(|sources| (1..=8).contains(&sources.len()))
                .ok_or_else(|| ToolError::invalid("Invalid saved report sources"))?;
            let sources = sources.iter().map(|source| {
                let file = source.str_arg("file")?;
                bindings.get(file).filter(|path| path.is_string())
                    .ok_or_else(|| ToolError::invalid(format!("Missing file binding for {file}")))
            }).collect::<Result<Vec<_>, _>>()?;
            {
                let mut supplied = json!({"sources":sources});
                if let Some(values) = args.get("parameter_values") {
                    supplied["parameter_values"] = values.clone();
                }
                bound_query(&definition, &supplied)
            }
        })().map_err(|error| report_error(&name, error))?;
        let key = json!({"datasets":query["datasets"],"path":query["path"]});
        let entry = (index, name, definition, query);
        if let Some((_, reports)) = groups.iter_mut().find(|(existing, _)| *existing == key) {
            reports.push(entry);
        } else if arrow_disk && !groups.is_empty() {
            return Err(report_error(&entry.1, ToolError::invalid(
                "Explicit disk staging requires identical Arrow IPC dataset bindings for all reports",
            )));
        } else {
            groups.push((key, vec![entry]));
        }
    }
    let mut results = Vec::new();
    // Finish each source group and drop its session before loading the next.
    for (_, reports) in groups {
        let (_, name, _, query) = &reports[0];
        let session = {
            #[cfg(feature = "report-profiling")]
            let _timer = ibis_doc_model::query::report_profile::Timer::start(
                ibis_doc_model::query::report_profile::Phase::Stage,
            );
            batch_session(service, query, arrow_disk).map_err(|error| report_error(name, error))?
        };
        let input = report_input_schema(&session).map_err(|error| report_error(name, error))?;
        for (index, name, definition, query) in reports {
            let (rows, checks) = refresh_query_in_session(&session, &query, &definition, preview)
                .map_err(|error| report_error(&name, error))?;
            let changed_params = changed_report_params(&definition, &query)?;
            let review = RefreshReview { checks, input_schema: input.clone(), changed_params };
            results.push((index, QueryReportResult { name, definition, rows, review }));
        }
    }
    results.sort_unstable_by_key(|(index, _)| *index);
    Ok(results.into_iter().map(|(_, result)| result).collect())
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
    let open = crate::tools_xlsx::workbook_document(service, id)?;
    let (summary, commands) =
        plan_batch_refresh(open.document.presentation(), args, results, preview)?;
    if !preview {
        crate::edit::execute(open, commands)?;
    }
    Ok(crate::edit::edited(id, open, summary, Vec::new()))
}

fn plan_batch_refresh(
    workbook: &ibis_doc_model::Presentation,
    args: &Value,
    results: Vec<QueryReportResult>,
    preview: bool,
) -> Result<(Value, Vec<ibis_doc_model::Command>), ToolError> {
    let metrics = batch_metrics(args, &results)?;
    let mut scratch = ibis_doc_model::Document::new(workbook.clone());
    let mut reports = Vec::with_capacity(results.len());
    let mut commands = Vec::new();
    let mut blocked = false;
    for result in results {
        let check_result = enforce_checks(&result.review.checks);
        blocked |= check_result.is_err();
        if !preview {
            check_result.map_err(|error| report_error(&result.name, error))?;
        }
        let (mut summary, mut update) = crate::tools_pivot::plan_query_table_refresh(
            scratch.presentation(),
            &json!({"table":result.name}),
            result.rows,
            &result.definition,
            metrics.get(&result.name).map_or(&[], Vec::as_slice),
        )
        .map_err(|error| report_error(&result.name, error))?;
        result.review.summarize(&result.definition, &mut summary, &mut update);
        scratch.execute(update.clone()).map_err(|error| {
            ToolError::new("edit_rejected", format!("{}: {error:?}", result.name))
        })?;
        reports.push(summary);
        commands.extend(update);
    }
    let charts: Vec<_> = if preview && !blocked {
        workbook.slides.iter().flat_map(|slide| {
            let updated = scratch.presentation().slide(slide.id);
            slide.shapes.iter().filter_map(move |shape| {
                let ibis_doc_model::ShapeKind::Chart(before) = &shape.kind else { return None };
                let ibis_doc_model::ShapeKind::Chart(after) = &updated?.shape(shape.id)?.kind else { return None };
                Some(json!({"sheet":slide.name,"shape_id":shape.id.0,
                    "title":before.title.as_ref().map(|title| &title.text),
                    "changed":before != after}))
            })
        }).collect()
    } else {
        Vec::new()
    };
    Ok((
        json!({"preview":preview,"blocked":blocked,"reports":reports,"charts":charts}),
        commands,
    ))
}

pub(crate) struct ReviewedReportRefresh {
    pub(crate) document_id: String,
    snapshot_id: String,
    base: ibis_doc_model::Presentation,
    summary: Value,
    commands: Vec<ibis_doc_model::Command>,
}

fn refresh_query_reports(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    if let Some(snapshot_id) = args.opt_str("snapshot_id")? {
        crate::edit::only_known(args, &["document_id", "snapshot_id"])?;
        if !service.report_refresh.as_ref().is_some_and(|review| review.document_id == id && review.snapshot_id == snapshot_id) {
            return Err(ToolError::invalid("Unknown or expired report preview. Preview the refresh again."));
        }
        let review = service.report_refresh.take().unwrap();
        let open = crate::tools_xlsx::workbook_document(service, id)?;
        if open.document.presentation() != &review.base {
            return Err(ToolError::invalid("Workbook changed since preview. Preview the refresh again."));
        }
        crate::edit::execute(open, review.commands)?;
        let mut summary = review.summary;
        summary["preview"] = json!(false);
        return Ok(crate::edit::edited(id, open, summary, Vec::new()));
    }
    service.report_refresh = None;
    let open = crate::tools_xlsx::workbook_document(service, id)?;
    let reports = batch_query_report_definitions(open.document.presentation(), args)?;
    let results = batch_query_results(service, args, reports)?;
    if !args.opt_bool("preview")?.unwrap_or(false) {
        return apply_batch_refresh(service, args, results);
    }
    let snapshot_id = service.new_document_id();
    let open = crate::tools_xlsx::workbook_document(service, id)?;
    let base = open.document.presentation();
    let (mut summary, commands) = plan_batch_refresh(base, args, results, true)?;
    let reviewed = if summary["blocked"] != true {
        let reviewed = ReviewedReportRefresh {
            document_id: id.to_owned(), snapshot_id: snapshot_id.clone(),
            base: base.clone(), summary: summary.clone(), commands,
        };
        summary["snapshot_id"] = json!(snapshot_id);
        Some(reviewed)
    } else {
        None
    };
    let result = crate::edit::edited(id, open, summary, Vec::new());
    service.report_refresh = reviewed;
    Ok(result)
}

pub struct SqliteReportBatch {
    arguments: Value,
    results: Vec<QueryReportResult>,
}

impl SqliteReportBatch {
    /// Review the exact prepared changes. A blocked review never returns commands.
    pub fn review(
        self,
        workbook: &ibis_doc_model::Presentation,
    ) -> Result<(Value, Vec<ibis_doc_model::Command>), ToolError> {
        let (summary, commands) =
            plan_batch_refresh(workbook, &self.arguments, self.results, true)?;
        let commands = if summary["blocked"] == true {
            Vec::new()
        } else {
            commands
        };
        Ok((summary, commands))
    }

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
    description: "Save read-only count checks on a query report. Checks run with its source bindings on every refresh. Optional diagnostics return example rows in previews. An empty list removes its checks.",
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
                            ("diagnostic", object(&[
                                ("sql", string("Optional read-only SELECT for examples when the check finds issues during preview.")),
                                ("params", json!({"type":"array","maxItems":100,"items":{"type":["string","number","null"]}})),
                            ], &["sql"])),
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
        crate::edit::only_known(check, &["name", "sql", "params", "max", "diagnostic"])?;
        let name = check.str_arg("name")?;
        if name.trim().is_empty() || name.chars().count() > 80 || !names.insert(name) {
            return Err(ToolError::invalid(
                "Check names must be distinct, nonblank and at most 80 characters",
            ));
        }
        validate_sqlite_select(check.str_arg("sql")?)
            .map_err(|error| ToolError::invalid(format!("{name}: {error}")))?;
        sql_params(check)?;
        if let Some(diagnostic) = check.get("diagnostic") {
            crate::edit::only_known(diagnostic, &["sql", "params"])?;
            validate_sqlite_select(diagnostic.str_arg("sql")?)
                .map_err(|error| ToolError::invalid(format!("{name} diagnostic: {error}")))?;
            sql_params(diagnostic)?;
        }
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
    if definition.to_string().len() > REPORT_DEFINITION_BYTES {
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

fn changed_report_params(definition: &Value, query: &Value) -> Result<Option<Value>, ToolError> {
    if definition["params"] == query["params"] {
        return Ok(None);
    }
    let mut next = definition.clone();
    next["params"] = query["params"].clone();
    if next.to_string().len() > REPORT_DEFINITION_BYTES {
        return Err(ToolError::invalid("The updated report definition exceeds 32 KiB"));
    }
    Ok(Some(query["params"].clone()))
}

fn refresh_query_result(
    service: &Service,
    args: &Value,
    definition: &Value,
) -> Result<(SqliteRows, RefreshReview), ToolError> {
    let session = query_session(service, args)?;
    let (rows, checks) = refresh_query_in_session(&session, args, definition, false)?;
    let input_schema = report_input_schema(&session)?;
    let changed_params = changed_report_params(definition, args)?;
    Ok((rows, RefreshReview { checks, input_schema, changed_params }))
}

fn refresh_query_in_session(
    session: &SqliteQuerySession,
    args: &Value,
    definition: &Value,
    preview: bool,
) -> Result<(SqliteRows, Vec<Value>), ToolError> {
    let checks = evaluate_report_checks(session, definition, preview)?;
    let rows = require_complete(run_session_query(session, args, 10_000, 4 * 1024 * 1024)?)?;
    Ok((rows, checks))
}

/// Evaluate only the draft report's checks using the supplied files in saved source order.
pub fn trial_selected_report_checks(
    paths: &[PathBuf],
    definition: &Value,
) -> Result<Vec<Value>, ToolError> {
    let sources = paths.iter().map(|path| path.to_str()
        .ok_or_else(|| ToolError::invalid("path is not valid UTF-8")))
        .collect::<Result<Vec<_>, _>>()?;
    let query = bound_query(definition, &json!({"sources":sources}))?;
    let service = selected_file_service(paths);
    let session = query_session(&service, &query)?;
    evaluate_report_checks(&session, definition, true)
}

fn evaluate_report_checks(
    session: &SqliteQuerySession,
    definition: &Value,
    preview: bool,
) -> Result<Vec<Value>, ToolError> {
    let checks = definition
        .get("checks")
        .map(validate_report_checks)
        .transpose()?
        .unwrap_or_default();
    checks
        .iter()
        .map(|check| {
            let name = check.str_arg("name")?;
            let result = run_session_query(session, check, 1, 1024)
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
            let mut summary = json!({"name":name,"count":count,"max":max,"passed":passed});
            if preview && count > 0 && passed != Some(true)
                && let Some(diagnostic) = check.get("diagnostic")
            {
                summary["diagnostic"] = match run_session_query(session, diagnostic, 20, 2 * 1024) {
                    Ok(result) => json!({
                        "columns":result.columns, "rows":result.rows, "truncated":result.truncated,
                    }),
                    Err(error) => {
                        let message: String = error.message.chars().take(256).collect();
                        json!({"error":{"code":error.code,"message":message}})
                    }
                };
            }
            Ok(summary)
        })
        .collect()
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

#[cfg(test)]
#[path = "tools_sqlite_boundary_tests.rs"]
mod boundary_tests;
