// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::path::{Path, PathBuf};

use ibis_doc_model::query::{
    QColumn, QTable, QType, SqlitePredicate, SqliteRows, SqliteScalar, read_parquet_bounded,
    read_sqlite_table_bounded, sqlite_columns, sqlite_count, sqlite_query, sqlite_query_datasets,
    sqlite_tables,
};
use serde_json::{Value, json};

use crate::catalog::{Access, Tool, document_id, integer, object, string};
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
    let directories = paths
        .iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    Ok(Service::new(&directories, true, Limits::default()))
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
                ], &["source","alias"]),
            }),
        ),
        (
            "params",
            json!({"type":"array","maxItems":100,"items":{"type":["string","number","null"]}}),
        ),
    ]
}

fn run_query(
    service: &Service,
    args: &Value,
    max_rows: usize,
    max_bytes: usize,
) -> Result<SqliteRows, ToolError> {
    let sql = args.str_arg("sql")?;
    let params = match args.get("params") {
        None => Vec::new(),
        Some(Value::Array(values)) if values.len() <= 100 => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    Ok(None)
                } else {
                    parse_scalar(value).map(Some)
                }
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(ToolError::invalid(
                "params must be an array of at most 100 values",
            ));
        }
    };
    if let Some(specs) = args.get("datasets") {
        if args.opt_str(PATH)?.is_some() {
            return Err(ToolError::invalid(
                "use path for a SQLite database or datasets for registered tables",
            ));
        }
        let tables = load_datasets(service, specs)?;
        sqlite_query_datasets(&tables, sql, &params, max_rows, max_bytes).map_err(sqlite_error)
    } else {
        sqlite_query(&database(service, args)?, sql, &params, max_rows, max_bytes)
            .map_err(sqlite_error)
    }
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
    let rows = run_query(service, args, 10_000, 4 * 1024 * 1024)?;
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
                    crate::tools_pivot::refresh_query_table(
                        service,
                        args,
                        self.rows,
                        &self.definition,
                    )
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
    })
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
        crate::edit::only_known(spec, &["source", "alias", "table"])?;
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
        let table = if is_sqlite(&path) {
            let name = table_name.ok_or_else(|| ToolError::invalid("SQLite datasets require a source table"))?;
            read_sqlite_table_bounded(&path, name, cells, text).map_err(sqlite_error)?
        } else {
            if table_name.is_some() {
                return Err(ToolError::invalid("table is only used for SQLite datasets"));
            }
            if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("parquet")) {
                read_parquet_bounded(&path, cells, text).map_err(sqlite_error)?
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
        json!({
            "file": path.file_name().unwrap_or_default().to_string_lossy(),
            "kind": source_kind(path),
            "alias": spec["alias"], "table": spec["table"],
        })
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
            Ok(json!({"source":path,"alias":source["alias"],"table":source["table"]}))
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
    let rows = complete_query(service, &bound_query(&definition, args)?)?;
    crate::tools_pivot::refresh_query_table(service, args, rows, &definition)
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
        let path = entry
            .as_str()
            .and_then(|value| value.parse::<usize>().ok())
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| paths.get(index))
            .and_then(|path| path.to_str())
            .ok_or_else(|| {
                ToolError::invalid("Use only file numbers selected in the assistant panel")
            })?;
        *entry = json!(path);
    }
    let folders = paths
        .iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    let service = Service::new(&folders, true, Limits::default());
    let rows = complete_query(&service, &bound_query(&definition, &args)?)?;
    Ok(SqliteTableExport {
        arguments: args,
        rows,
        definition,
        refresh: true,
    })
}
