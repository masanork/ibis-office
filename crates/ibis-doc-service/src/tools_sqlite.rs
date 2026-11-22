// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::path::{Path, PathBuf};

use ibis_doc_model::query::{
    SqlitePredicate, SqliteScalar, sqlite_columns, sqlite_count, sqlite_tables,
};
use serde_json::{Value, json};

use crate::catalog::{Access, Tool, integer, object, string};
use crate::service::{Args, Limits, Service};
use crate::{ToolError, paths};

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
pub fn run_selected_sqlite_tool(
    path: &Path,
    name: &str,
    mut args: Value,
) -> Result<Value, ToolError> {
    let parent = path
        .parent()
        .ok_or_else(|| ToolError::invalid("SQLite file has no parent directory"))?;
    let path = path
        .to_str()
        .ok_or_else(|| ToolError::invalid("SQLite path is not valid UTF-8"))?;
    let object = args
        .as_object_mut()
        .ok_or_else(|| ToolError::invalid("the arguments are not an object"))?;
    if object.contains_key(PATH) {
        return Err(ToolError::invalid(
            "the SQLite database is selected in the assistant panel",
        ));
    }
    object.insert(PATH.into(), json!(path));
    let tool = TOOLS.iter().find(|tool| tool.name == name).ok_or_else(|| {
        ToolError::new(
            "unsupported_operation",
            "no SQLite analysis tool by that name",
        )
    })?;
    let mut service = Service::new(&[parent.to_path_buf()], true, Limits::default());
    (tool.run)(&mut service, &args)
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
            let value = match value {
                Some(Value::String(value)) => SqliteScalar::Text(value),
                Some(Value::Number(value)) => {
                    if let Some(integer) = value.as_i64() {
                        SqliteScalar::Integer(integer)
                    } else if value.is_f64()
                        && let Some(real) = value.as_f64()
                    {
                        SqliteScalar::Real(real)
                    } else {
                        return Err(ToolError::invalid(
                            "filter integer is outside SQLite's signed 64-bit range",
                        ));
                    }
                }
                _ => {
                    return Err(ToolError::invalid(
                        "comparison filters require a string or number value",
                    ));
                }
            };
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
