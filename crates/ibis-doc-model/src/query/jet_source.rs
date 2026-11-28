// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType, sqlite_source::dataset_sql_type};
use crate::CellValue;
use jetdb::{
    BlobValue, ColumnType, ComplexColumn, ComplexValues, MacroDef, MacroStatement, PageReader,
    TableRows, Value, calculated_column_types_in, column_flags, control_type_name, find_table,
    list_forms, list_macros, query_to_sql, read_catalog, read_complex_column,
    read_embedded_macros, read_form_properties, read_macro, read_queries, read_relationships,
    read_table_def, read_vba_project, table_names,
};
use serde_json::json;
use std::path::Path;

const MAX_TABLES: usize = 1_024;
const MAX_COLUMNS: usize = 256;
const MAX_ROWS: usize = 1_048_575;
const MAX_CELLS: usize = 5_000_000;
const MAX_TEXT_BYTES: usize = 128 * 1024 * 1024;

pub fn is_jet_extension(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        ["mdb", "accdb", "mde", "accde"]
            .iter()
            .any(|name| ext.eq_ignore_ascii_case(name))
    })
}

fn open_jet(path: &Path) -> Result<PageReader, String> {
    if !path.is_file() {
        return Err("Select a local Microsoft Access (.mdb/.accdb) database file".into());
    }
    PageReader::open(path).map_err(|error| match error {
        jetdb::FileError::PasswordRequired => {
            "Access database is password-protected (password_required)".into()
        }
        jetdb::FileError::InvalidPassword => {
            "Access database password is invalid (invalid_password)".into()
        }
        jetdb::FileError::UnsupportedEncryption { reason } => {
            format!("Access database uses unsupported encryption: {reason}")
        }
        other => format!("Failed to open Access database: {other}"),
    })
}

pub(crate) fn jet_type_to_qtype(col_type: ColumnType) -> QType {
    match col_type {
        ColumnType::Boolean => QType::Logical,
        ColumnType::Byte | ColumnType::Int => QType::Int32,
        ColumnType::Long | ColumnType::BigInt => QType::Whole,
        ColumnType::Float | ColumnType::Double => QType::Decimal,
        ColumnType::Timestamp => QType::DateTime,
        ColumnType::Money
        | ColumnType::Numeric
        | ColumnType::Text
        | ColumnType::Memo
        | ColumnType::Guid
        | ColumnType::DateTimeExtended
        | ColumnType::Binary
        | ColumnType::Ole
        | ColumnType::ComplexType
        | ColumnType::Unknown(_) => QType::Text,
        _ => QType::Text,
    }
}

pub(crate) fn jet_value_to_cell(value: Value) -> CellValue {
    match value {
        Value::Null => CellValue::Empty,
        Value::Bool(b) => CellValue::Bool(b),
        Value::Byte(b) => CellValue::Number(f64::from(b)),
        Value::Int(i) => CellValue::Number(f64::from(i)),
        Value::Long(l) => CellValue::Number(f64::from(l)),
        Value::BigInt(n) => {
            if n.unsigned_abs() < (1u64 << 53) {
                CellValue::Number(n as f64)
            } else {
                CellValue::Text(n.to_string())
            }
        }
        Value::Float(f) => {
            let v = f64::from(f);
            if v.is_finite() {
                CellValue::Number(v)
            } else {
                CellValue::Empty
            }
        }
        Value::Double(d) | Value::Timestamp(d) => {
            if d.is_finite() {
                CellValue::Number(d)
            } else {
                CellValue::Empty
            }
        }
        Value::Text(s)
        | Value::Money(s)
        | Value::Numeric(s)
        | Value::Guid(s)
        | Value::DateTimeExtended(s) => CellValue::Text(s),
        Value::Binary(bytes) => CellValue::Text(format!("[{} bytes]", bytes.len())),
        _ => CellValue::Empty,
    }
}

fn resolve_projection(
    table_def: &jetdb::TableDef,
    selected: &[String],
) -> Result<(Vec<QColumn>, Vec<usize>), String> {
    if table_def.columns.is_empty() {
        return Err(format!("Access table has no columns: {}", table_def.name));
    }
    if selected.is_empty() {
        if table_def.columns.len() > MAX_COLUMNS {
            return Err(format!(
                "Access table {} has more than {MAX_COLUMNS} columns",
                table_def.name
            ));
        }
        let columns = table_def
            .columns
            .iter()
            .map(|col| QColumn {
                name: col.name.clone(),
                ty: jet_type_to_qtype(col.col_type),
            })
            .collect();
        let indices = (0..table_def.columns.len()).collect();
        return Ok((columns, indices));
    }
    if selected.len() > MAX_COLUMNS {
        return Err(format!("Too many selected columns (maximum {MAX_COLUMNS})"));
    }
    let mut columns = Vec::with_capacity(selected.len());
    let mut indices = Vec::with_capacity(selected.len());
    for name in selected {
        let Some((idx, col)) = table_def
            .columns
            .iter()
            .enumerate()
            .find(|(_, col)| col.name == *name)
        else {
            return Err(format!(
                "Column '{name}' does not exist in Access table '{}'",
                table_def.name
            ));
        };
        columns.push(QColumn {
            name: col.name.clone(),
            ty: jet_type_to_qtype(col.col_type),
        });
        indices.push(idx);
    }
    Ok((columns, indices))
}

/// List user table names in a Microsoft Access (.mdb / .accdb) database.
pub fn jet_tables(path: &Path) -> Result<Vec<String>, String> {
    let mut reader = open_jet(path)?;
    let mut names = table_names(&mut reader).map_err(|error| error.to_string())?;
    if names.len() > MAX_TABLES {
        return Err("Access database has too many tables to list".into());
    }
    names.sort();
    Ok(names)
}

/// Return SQL-visible column definitions for one Access table without reading rows.
pub fn jet_dataset_columns(
    path: &Path,
    table: &str,
    selected: &[String],
) -> Result<Vec<QColumn>, String> {
    let mut reader = open_jet(path)?;
    let catalog = read_catalog(&mut reader).map_err(|error| error.to_string())?;
    let entry = find_table(&catalog, table).map_err(|error| error.to_string())?;
    let table_def =
        read_table_def(&mut reader, &entry.name, entry.table_page).map_err(|error| error.to_string())?;
    let (columns, _) = resolve_projection(&table_def, selected)?;
    Ok(columns)
}

/// Stream one Access table's schema and rows into a visitor.
pub(super) fn visit_jet_table(
    path: &Path,
    table: &str,
    selected: &[String],
    schema_only: bool,
    mut visit: impl FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
) -> Result<(), String> {
    let mut reader = open_jet(path)?;
    let catalog = read_catalog(&mut reader).map_err(|error| error.to_string())?;
    let entry = find_table(&catalog, table).map_err(|error| error.to_string())?;
    let table_def =
        read_table_def(&mut reader, &entry.name, entry.table_page).map_err(|error| error.to_string())?;
    let (columns, indices) = resolve_projection(&table_def, selected)?;
    visit(&columns, None)?;
    if schema_only {
        return Ok(());
    }
    let complex_cols: Vec<Option<ComplexColumn>> = if schema_only {
        Vec::new()
    } else {
        let mut resolved = Vec::with_capacity(indices.len());
        for &idx in &indices {
            let col = &table_def.columns[idx];
            if col.col_type == ColumnType::ComplexType {
                resolved.push(
                    read_complex_column(&mut reader, &catalog, &table_def, col)
                        .ok()
                        .flatten(),
                );
            } else {
                resolved.push(None);
            }
        }
        resolved
    };

    let calculated = calculated_column_types_in(&catalog, &table_def);
    let mut rows = if selected.is_empty() {
        TableRows::new(&table_def, &calculated)
    } else {
        TableRows::new(&table_def, &calculated).with_columns(&indices)
    };
    let mut count = 0usize;
    while let Some(row) = rows.next_row(&mut reader).map_err(|error| error.to_string())? {
        count += 1;
        if count > MAX_ROWS || columns.len().saturating_mul(count) > MAX_CELLS {
            return Err("Access table exceeds the query editor's row or cell limit".into());
        }
        let mut values = Vec::with_capacity(row.values.len());
        for (pos, val) in row.values.into_iter().enumerate() {
            if let (Some(Some(cc)), Value::Long(complex_id)) = (complex_cols.get(pos), &val) {
                if let Ok(cv) = cc.values(&mut reader, *complex_id) {
                    values.push(complex_values_to_cell(cv));
                    continue;
                }
            }
            values.push(jet_value_to_cell(val));
        }
        visit(&columns, Some(values))?;
    }
    rows.warn_skipped(&table_def.name);
    Ok(())
}

fn jet_value_to_json(value: Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => json!(b),
        Value::Byte(b) => json!(b),
        Value::Int(i) => json!(i),
        Value::Long(l) => json!(l),
        Value::BigInt(n) => json!(n),
        Value::Float(f) => json!(f),
        Value::Double(d) | Value::Timestamp(d) => json!(d),
        Value::Text(s)
        | Value::Money(s)
        | Value::Numeric(s)
        | Value::Guid(s)
        | Value::DateTimeExtended(s) => json!(s),
        Value::Binary(bytes) => json!(format!("[{} bytes]", bytes.len())),
        _ => serde_json::Value::Null,
    }
}

fn complex_values_to_cell(cv: ComplexValues) -> CellValue {
    let arr: Vec<serde_json::Value> = match cv {
        ComplexValues::MultiValue(vals) => vals.into_iter().map(jet_value_to_json).collect(),
        ComplexValues::Attachment(atts) => atts
            .into_iter()
            .map(|a| {
                let size = match &a.data {
                    Value::Binary(b) => Some(b.len()),
                    _ => None,
                };
                json!({
                    "name": jet_value_to_json(a.name),
                    "file_type": jet_value_to_json(a.file_type),
                    "size": size,
                })
            })
            .collect(),
        ComplexValues::VersionHistory(vers) => vers
            .into_iter()
            .map(|v| {
                json!({
                    "value": jet_value_to_json(v.value),
                    "modified": jet_value_to_json(v.modified),
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    CellValue::Text(serde_json::Value::Array(arr).to_string())
}

/// Read one Access table into memory within explicit cell and text budgets.
pub fn read_jet_table_bounded(
    path: &Path,
    table: &str,
    selected: &[String],
    max_cells: usize,
    max_text: usize,
) -> Result<QTable, String> {
    let mut table_out = QTable::default();
    let mut text_budget = max_text.min(MAX_TEXT_BYTES);
    let mut cell_budget = max_cells.min(MAX_CELLS);
    visit_jet_table(path, table, selected, false, |columns, row| {
        if let Some(row) = row {
            cell_budget = cell_budget
                .checked_sub(columns.len())
                .ok_or("Access table exceeds the in-memory cell limit")?;
            super::sqlite_source::charge_dataset_text(&row, &mut text_budget)?;
            table_out.rows.push(row);
        } else {
            table_out.columns = columns.to_vec();
        }
        Ok(())
    })?;
    Ok(table_out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JetQueryParam {
    name: String,
    jet_type: String,
}

fn parse_jet_parameters(sql: &str) -> (Vec<JetQueryParam>, &str) {
    let trimmed = sql.trim_start();
    if trimmed.len() < 11 || !trimmed[..10].eq_ignore_ascii_case("PARAMETERS") {
        return (Vec::new(), sql);
    }
    if !trimmed[10..].chars().next().is_some_and(char::is_whitespace) {
        return (Vec::new(), sql);
    }
    let Some(semi) = trimmed.find(';') else {
        return (Vec::new(), sql);
    };
    let decl = trimmed[10..semi].trim();
    let body = trimmed[semi + 1..].trim_start();
    let mut params = Vec::new();
    for part in split_top_level_args(decl) {
        let item = part.trim();
        if item.is_empty() {
            continue;
        }
        let (name, jet_type) = if let Some(rest) = item.strip_prefix('[') {
            if let Some(end) = rest.find(']') {
                (rest[..end].trim().to_owned(), rest[end + 1..].trim().to_owned())
            } else {
                continue;
            }
        } else {
            let mut ws = item.splitn(2, char::is_whitespace);
            let n = ws.next().unwrap_or("").trim().to_owned();
            let t = ws.next().unwrap_or("").trim().to_owned();
            (n, t)
        };
        if !name.is_empty() {
            params.push(JetQueryParam { name, jet_type });
        }
    }
    (params, body)
}

fn replace_param_placeholders(sqlite_sql: &str, params: &[JetQueryParam]) -> (String, Vec<String>) {
    if params.is_empty() {
        return (sqlite_sql.to_owned(), Vec::new());
    }
    let mut out = String::with_capacity(sqlite_sql.len());
    let mut positional = Vec::new();
    let bytes = sqlite_sql.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'\'' {
            out.push('\'');
            i += 1;
            while i < bytes.len() {
                out.push(bytes[i] as char);
                if bytes[i] == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        out.push('\'');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'"' {
            let mut end = i + 1;
            let mut ident = String::new();
            while end < bytes.len() {
                if bytes[end] == b'"' {
                    if end + 1 < bytes.len() && bytes[end + 1] == b'"' {
                        ident.push('"');
                        end += 2;
                        continue;
                    }
                    break;
                }
                let ch = sqlite_sql[end..].chars().next().unwrap();
                ident.push(ch);
                end += ch.len_utf8();
            }
            if end < bytes.len()
                && bytes[end] == b'"'
                && let Some(matched) = params.iter().find(|p| p.name.eq_ignore_ascii_case(&ident))
            {
                out.push('?');
                positional.push(matched.name.clone());
                i = end + 1;
                continue;
            }
        }
        let ch = sqlite_sql[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    (out, positional)
}

fn query_def_to_json(qdef: &jetdb::QueryDef) -> serde_json::Value {
    let sql = query_to_sql(qdef);
    let (params, body) = parse_jet_parameters(&sql);
    let sqlite_sql = jet_sql_to_sqlite(body);
    let (sqlite_parameterized_sql, positional_params) =
        replace_param_placeholders(&sqlite_sql, &params);
    let params_json: Vec<_> = params
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "jet_type": p.jet_type,
            })
        })
        .collect();
    json!({
        "name": qdef.name,
        "query_type": format!("{:?}", qdef.query_type),
        "is_action_query": !matches!(qdef.query_type, jetdb::QueryType::Select),
        "sql": sql,
        "sqlite_sql": jet_sql_to_sqlite(&sql),
        "parameters": params_json,
        "sqlite_parameterized_sql": sqlite_parameterized_sql,
        "positional_params": positional_params,
    })
}

/// Inspect schema, relationships, saved queries, forms/reports, and VBA modules in an Access database.
pub fn jet_inspect_schema(path: &Path) -> Result<serde_json::Value, String> {
    let mut reader = open_jet(path)?;
    let version = format!("{:?}", reader.header().version);
    let catalog = read_catalog(&mut reader).map_err(|error| error.to_string())?;
    let mut names = table_names(&mut reader).map_err(|error| error.to_string())?;
    if names.len() > MAX_TABLES {
        return Err("Access database has too many tables to inspect".into());
    }
    names.sort();

    let mut tables = Vec::with_capacity(names.len());
    for name in &names {
        let entry = find_table(&catalog, name).map_err(|error| error.to_string())?;
        let def = read_table_def(&mut reader, &entry.name, entry.table_page)
            .map_err(|error| error.to_string())?;
        let columns: Vec<_> = def
            .columns
            .iter()
            .map(|col| {
                let qtype = jet_type_to_qtype(col.col_type);
                json!({
                    "name": col.name,
                    "jet_type": format!("{:?}", col.col_type),
                    "sql_type": dataset_sql_type(qtype),
                    "size": col.col_size,
                    "auto_number": (col.flags & column_flags::AUTO_LONG) != 0,
                })
            })
            .collect();
        let indexes: Vec<_> = def
            .indexes
            .iter()
            .map(|idx| {
                let cols: Vec<_> = idx
                    .columns
                    .iter()
                    .filter_map(|c| {
                        def.columns
                            .iter()
                            .find(|col| col.col_num == c.col_num)
                            .map(|col| col.name.as_str())
                    })
                    .collect();
                json!({
                    "name": idx.name,
                    "columns": cols,
                })
            })
            .collect();
        tables.push(json!({
            "name": def.name,
            "row_count": def.num_rows,
            "columns": columns,
            "indexes": indexes,
        }));
    }

    let relationships = read_relationships(&mut reader)
        .unwrap_or_default()
        .into_iter()
        .map(|rel| {
            json!({
                "name": rel.name,
                "from_table": rel.from_table,
                "to_table": rel.to_table,
                "columns": rel.columns.iter().map(|c| json!({
                    "from": c.from_column,
                    "to": c.to_column,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    let queries = read_queries(&mut reader)
        .unwrap_or_default()
        .into_iter()
        .map(|qdef| query_def_to_json(&qdef))
        .collect::<Vec<_>>();

    let forms = list_forms(&mut reader)
        .unwrap_or_default()
        .into_iter()
        .map(|form| {
            json!({
                "name": form.name,
                "object_type": format!("{:?}", form.object_type),
            })
        })
        .collect::<Vec<_>>();

    let vba_modules = read_vba_project(&mut reader)
        .ok()
        .map(|proj| {
            proj.modules
                .into_iter()
                .map(|m| {
                    json!({
                        "name": m.name,
                        "module_type": format!("{:?}", m.module_type),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let macros: Vec<_> = list_macros(&mut reader)
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.name)
        .collect();

    Ok(json!({
        "format": version,
        "tables": tables,
        "relationships": relationships,
        "queries": queries,
        "forms": forms,
        "macros": macros,
        "vba_modules": vba_modules,
    }))
}

fn mermaid_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "Table".to_owned()
    } else {
        out
    }
}

/// Generate a Markdown migration specification with a Mermaid `erDiagram` for an Access database.
pub fn jet_migration_markdown(path: &Path) -> Result<String, String> {
    use std::fmt::Write as _;

    let schema = jet_inspect_schema(path)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("database.mdb");
    let format = schema["format"].as_str().unwrap_or("Jet");

    let mut md = String::new();
    let _ = writeln!(md, "# Access Migration Specification: `{file_name}`\n");
    let _ = writeln!(md, "- Engine Format: `{format}`");

    let tables = schema["tables"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let rels = schema["relationships"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let queries = schema["queries"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let forms = schema["forms"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let vba = schema["vba_modules"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let _ = writeln!(
        md,
        "- Inventory: {} tables, {} relationships, {} saved queries, {} forms/reports, {} VBA modules\n",
        tables.len(),
        rels.len(),
        queries.len(),
        forms.len(),
        vba.len()
    );

    if !tables.is_empty() {
        let _ = writeln!(md, "## Entity Relationship Diagram\n");
        let _ = writeln!(md, "```mermaid");
        let _ = writeln!(md, "erDiagram");
        for table in tables {
            let tname = table["name"].as_str().unwrap_or("Table");
            let ident = mermaid_ident(tname);
            let _ = writeln!(md, "    {ident} {{");
            if let Some(cols) = table["columns"].as_array() {
                for col in cols {
                    let cname = mermaid_ident(col["name"].as_str().unwrap_or("col"));
                    let sql_type = col["sql_type"].as_str().unwrap_or("TEXT");
                    let pk = if col["auto_number"].as_bool() == Some(true) {
                        " PK"
                    } else {
                        ""
                    };
                    let _ = writeln!(md, "        {sql_type} {cname}{pk}");
                }
            }
            let _ = writeln!(md, "    }}");
        }
        for rel in rels {
            let from_t = mermaid_ident(rel["from_table"].as_str().unwrap_or("From"));
            let to_t = mermaid_ident(rel["to_table"].as_str().unwrap_or("To"));
            let label = rel["name"].as_str().unwrap_or("rel").replace('"', "'");
            let _ = writeln!(md, "    {from_t} }}o--|| {to_t} : \"{label}\"");
        }
        let _ = writeln!(md, "```\n");
    }

    if !queries.is_empty() {
        let _ = writeln!(md, "## Saved Queries\n");
        for q in queries {
            let name = q["name"].as_str().unwrap_or("Query");
            let qtype = q["query_type"].as_str().unwrap_or("Select");
            let sql = q["sql"].as_str().unwrap_or("");
            let sqlite_sql = q["sqlite_sql"].as_str().unwrap_or("");
            let _ = writeln!(md, "### `{name}` ({qtype})\n");
            let _ = writeln!(md, "```sql\n{sql}\n```\n");
            if !sqlite_sql.is_empty() && sqlite_sql != sql {
                let _ = writeln!(md, "Turso / SQLite equivalent:\n\n```sql\n{sqlite_sql}\n```\n");
            }
        }
    }

    if !forms.is_empty() || !vba.is_empty() {
        let _ = writeln!(md, "## Forms, Reports & VBA Modules\n");
        for f in forms {
            let name = f["name"].as_str().unwrap_or("");
            let kind = f["object_type"].as_str().unwrap_or("Form");
            let _ = writeln!(md, "- [{kind}] `{name}`");
        }
        for m in vba {
            let name = m["name"].as_str().unwrap_or("");
            let kind = m["module_type"].as_str().unwrap_or("Module");
            let _ = writeln!(md, "- [VBA {kind}] `{name}`");
        }
        let _ = writeln!(md);
    }

    let _ = writeln!(md, "## Google Workspace Migration Notes\n");
    let _ = writeln!(md, "| Access / JET Type | Turso / SQLite | Google Sheets / AppSheet | BigQuery |");
    let _ = writeln!(md, "| --- | --- | --- | --- |");
    let _ = writeln!(md, "| Text / Memo / GUID | `TEXT` | Text | `STRING` |");
    let _ = writeln!(md, "| Byte / Integer / Long / AutoNumber | `INTEGER` | Number | `INT64` |");
    let _ = writeln!(md, "| Single / Double | `REAL` | Number | `FLOAT64` |");
    let _ = writeln!(md, "| Currency / Numeric | `TEXT` (exact) | Currency / Number | `NUMERIC` |");
    let _ = writeln!(md, "| Date/Time | `REAL` (OLE serial) | Date / DateTime | `DATETIME` |");
    let _ = writeln!(md, "| Yes/No | `INTEGER` (0/1) | Checkbox (Boolean) | `BOOL` |");
    let _ = writeln!(md, "| MultiValue / Attachment | `TEXT` (JSON array) | Text / Drive link | `ARRAY<STRING>` |\n");
    let _ = writeln!(md, "- Export tables & views to SQLite: `jet_export_sqlite`");
    let _ = writeln!(md, "- Export query results to `.xlsx` (Google Sheets): `sqlite_query_to_table`");
    let _ = writeln!(md, "- Extract VBA for Apps Script translation: `jet_inspect_vba`");

    Ok(md)
}

fn blob_value_to_json(val: &BlobValue) -> serde_json::Value {
    match val {
        BlobValue::Bool(b) => json!(b),
        BlobValue::Long(l) => json!(l),
        BlobValue::Double(d) => json!(d),
        BlobValue::Guid(s) | BlobValue::Text(s) => json!(s),
        BlobValue::Binary(bytes) => json!(format!("[{} bytes]", bytes.len())),
        _ => serde_json::Value::Null,
    }
}

/// Extract VBA module source code from an Access database.
pub fn jet_inspect_vba(path: &Path, module: Option<&str>) -> Result<serde_json::Value, String> {
    let mut reader = open_jet(path)?;
    let project = match read_vba_project(&mut reader) {
        Ok(proj) => proj,
        Err(jetdb::FileError::ModuleNotFound { .. }) => {
            return Ok(json!({ "modules": [] }));
        }
        Err(error) => return Err(format!("Failed to read VBA project: {error}")),
    };
    let mut modules = Vec::new();
    for m in project.modules {
        if let Some(target) = module
            && m.name != target
        {
            continue;
        }
        modules.push(json!({
            "name": m.name,
            "module_type": format!("{:?}", m.module_type),
            "source": m.source,
        }));
    }
    if let Some(target) = module
        && modules.is_empty()
    {
        return Err(format!("VBA module not found: {target}"));
    }
    Ok(json!({ "modules": modules }))
}

fn encode_blob_properties(
    props: &[jetdb::BlobProperty],
) -> (Vec<serde_json::Value>, serde_json::Map<String, serde_json::Value>) {
    let mut list = Vec::with_capacity(props.len());
    let mut named = serde_json::Map::new();
    for p in props {
        let val = blob_value_to_json(&p.value);
        if let Some(name) = p.name() {
            named.insert(name.to_owned(), val.clone());
        }
        list.push(json!({
            "prop_id": format!("0x{:04x}", p.prop_id),
            "name": p.display_name(),
            "value": val,
        }));
    }
    (list, named)
}

fn twips_to_px(twips: f64) -> f64 {
    (twips / 15.0 * 10.0).round() / 10.0
}

fn json_as_f64(val: Option<&serde_json::Value>) -> Option<f64> {
    val.and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
}

fn extract_control_layout(named: &serde_json::Map<String, serde_json::Value>) -> serde_json::Value {
    let left_twips = json_as_f64(named.get("Left"));
    let top_twips = json_as_f64(named.get("Top"));
    let width_twips = json_as_f64(named.get("Width"));
    let height_twips = json_as_f64(named.get("Height"));
    let control_source = named.get("ControlSource").and_then(|v| v.as_str());
    let expression = control_source.filter(|s| s.trim_start().starts_with('='));

    json!({
        "left_px": left_twips.map(twips_to_px),
        "top_px": top_twips.map(twips_to_px),
        "width_px": width_twips.map(twips_to_px),
        "height_px": height_twips.map(twips_to_px),
        "tab_index": named.get("TabIndex").cloned().unwrap_or(serde_json::Value::Null),
        "tab_stop": named.get("TabStop").cloned().unwrap_or(serde_json::Value::Null),
        "visible": named.get("Visible").cloned().unwrap_or(serde_json::Value::Null),
        "caption": named.get("Caption").cloned().unwrap_or(serde_json::Value::Null),
        "control_source": control_source,
        "expression": expression,
    })
}

fn form_properties_to_json(fp: &jetdb::FormProperties) -> serde_json::Value {
    let (properties, named_properties) = encode_blob_properties(&fp.properties);
    let mut tab_entries: Vec<(i64, f64, f64, String)> = Vec::new();
    let controls: Vec<_> = fp
        .controls
        .iter()
        .map(|c| {
            let (props, named_props) = encode_blob_properties(&c.properties);
            let layout = extract_control_layout(&named_props);
            let tab_idx = named_props
                .get("TabIndex")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            let top = json_as_f64(named_props.get("Top")).unwrap_or(f64::MAX);
            let left = json_as_f64(named_props.get("Left")).unwrap_or(f64::MAX);
            tab_entries.push((tab_idx, top, left, c.name.clone()));
            json!({
                "name": c.name,
                "type": control_type_name(c.type_code),
                "type_code": c.type_code,
                "layout": layout,
                "named_properties": named_props,
                "properties": props,
            })
        })
        .collect();
    tab_entries.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.total_cmp(&b.1))
            .then_with(|| a.2.total_cmp(&b.2))
    });
    let tab_order: Vec<String> = tab_entries.into_iter().map(|e| e.3).collect();
    let form_layout = json!({
        "width_px": json_as_f64(named_properties.get("Width")).map(twips_to_px),
        "caption": named_properties.get("Caption").cloned().unwrap_or(serde_json::Value::Null),
        "record_source": named_properties.get("RecordSource").cloned().unwrap_or(serde_json::Value::Null),
        "default_view": named_properties.get("DefaultView").cloned().unwrap_or(serde_json::Value::Null),
        "tab_order": tab_order,
    });
    json!({
        "name": fp.form_name,
        "object_type": format!("{:?}", fp.object_type),
        "layout": form_layout,
        "named_properties": named_properties,
        "properties": properties,
        "controls": controls,
    })
}

/// Extract form or report properties and bound controls from an Access database.
pub fn jet_inspect_form(path: &Path, form_name: &str) -> Result<serde_json::Value, String> {
    let mut reader = open_jet(path)?;
    let fp = read_form_properties(&mut reader, form_name)
        .map_err(|error| format!("Failed to read form/report '{form_name}': {error}"))?;
    Ok(form_properties_to_json(&fp))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JetBundleMode {
    Metadata,
    Code,
    Full,
}

impl JetBundleMode {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "metadata" => Ok(Self::Metadata),
            "code" => Ok(Self::Code),
            "full" | "all" | "data" => Ok(Self::Full),
            other => Err(format!(
                "Invalid bundle mode '{other}': expected 'metadata', 'code', or 'full'"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Code => "code",
            Self::Full => "full",
        }
    }
}

fn cell_to_json(cell: &CellValue) -> serde_json::Value {
    match cell {
        CellValue::Empty => serde_json::Value::Null,
        CellValue::Bool(b) => json!(b),
        CellValue::Number(n) => {
            if n.is_finite() && n.fract() == 0.0 && n.abs() < (1i64 << 53) as f64 {
                json!(*n as i64)
            } else {
                json!(n)
            }
        }
        CellValue::Text(s) => json!(s),
        other => json!(format!("{other:?}")),
    }
}

fn text_mentions_ident(haystack_lower: &str, needle: &str) -> bool {
    let needle_lower = needle.to_ascii_lowercase();
    if needle_lower.is_empty() {
        return false;
    }
    let mut start = 0usize;
    while let Some(pos) = haystack_lower[start..].find(&needle_lower) {
        let abs = start + pos;
        let before_ok = abs == 0
            || !haystack_lower[..abs]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let after = abs + needle_lower.len();
        let after_ok = after >= haystack_lower.len()
            || !haystack_lower[after..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if before_ok && after_ok {
            return true;
        }
        start = abs + needle_lower.len();
    }
    false
}

fn collect_macro_targets(stmts: &[serde_json::Value], out: &mut std::collections::BTreeMap<&'static str, Vec<String>>) {
    for stmt in stmts {
        match stmt.get("type").and_then(serde_json::Value::as_str) {
            Some("action") => {
                let action = stmt.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
                let args = stmt.get("arguments").and_then(serde_json::Value::as_object);
                let push_arg = |key: &'static str, arg_name: &str, map: &mut std::collections::BTreeMap<&'static str, Vec<String>>| {
                    if let Some(val) = args.and_then(|a| a.get(arg_name)).and_then(serde_json::Value::as_str)
                        && !val.is_empty()
                    {
                        let list = map.entry(key).or_default();
                        if !list.iter().any(|x| x == val) {
                            list.push(val.to_owned());
                        }
                    }
                };
                match action.to_ascii_lowercase().as_str() {
                    "openform" => push_arg("opens_forms", "FormName", out),
                    "openreport" => push_arg("opens_reports", "ReportName", out),
                    "openquery" => push_arg("opens_queries", "QueryName", out),
                    "opentable" => push_arg("opens_tables", "TableName", out),
                    "runmacro" => push_arg("runs_macros", "MacroName", out),
                    "runcode" => push_arg("runs_vba", "FunctionName", out),
                    _ => {}
                }
            }
            Some("conditional") => {
                if let Some(branches) = stmt.get("branches").and_then(serde_json::Value::as_array) {
                    for b in branches {
                        if let Some(sub) = b.get("statements").and_then(serde_json::Value::as_array) {
                            collect_macro_targets(sub, out);
                        }
                    }
                }
            }
            Some("group" | "sub_macro" | "data_block") => {
                if let Some(sub) = stmt.get("statements").and_then(serde_json::Value::as_array) {
                    collect_macro_targets(sub, out);
                }
            }
            _ => {}
        }
    }
}

fn build_jet_cross_references(
    tables: &[serde_json::Value],
    queries: &[serde_json::Value],
    forms: &[serde_json::Value],
    macros: &[serde_json::Value],
    vba_modules: &[serde_json::Value],
) -> serde_json::Value {
    let table_names: Vec<String> = tables
        .iter()
        .filter_map(|t| t.get("name").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();
    let query_names: Vec<String> = queries
        .iter()
        .filter_map(|q| q.get("name").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();
    let form_names: Vec<String> = forms
        .iter()
        .filter_map(|f| f.get("name").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();

    let query_refs: Vec<serde_json::Value> = queries
        .iter()
        .filter_map(|q| {
            let name = q.get("name").and_then(serde_json::Value::as_str)?;
            let sql_lower = q
                .get("sql")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let tables_used: Vec<String> = table_names
                .iter()
                .filter(|t| text_mentions_ident(&sql_lower, t))
                .cloned()
                .collect();
            let queries_used: Vec<String> = query_names
                .iter()
                .filter(|qn| qn.as_str() != name && text_mentions_ident(&sql_lower, qn))
                .cloned()
                .collect();
            Some(json!({
                "name": name,
                "tables": tables_used,
                "queries": queries_used,
            }))
        })
        .collect();

    let form_refs: Vec<serde_json::Value> = forms
        .iter()
        .filter_map(|f| {
            let name = f.get("name").and_then(serde_json::Value::as_str)?;
            let record_source = f
                .get("named_properties")
                .and_then(|np| np.get("RecordSource"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let mut events = serde_json::Map::new();
            if let Some(np) = f.get("named_properties").and_then(serde_json::Value::as_object) {
                for (k, v) in np {
                    if k.starts_with("On") || k.ends_with("Update") {
                        events.insert(format!("Form.{k}"), v.clone());
                    }
                }
            }
            let mut bound_controls = Vec::new();
            if let Some(ctrls) = f.get("controls").and_then(serde_json::Value::as_array) {
                for c in ctrls {
                    let cname = c.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
                    if let Some(np) = c.get("named_properties").and_then(serde_json::Value::as_object) {
                        let cs = np.get("ControlSource").cloned();
                        let rs = np.get("RowSource").cloned();
                        if cs.is_some() || rs.is_some() {
                            bound_controls.push(json!({
                                "control": cname,
                                "control_source": cs,
                                "row_source": rs,
                            }));
                        }
                        for (k, v) in np {
                            if k.starts_with("On") || k.ends_with("Update") {
                                events.insert(format!("{cname}.{k}"), v.clone());
                            }
                        }
                    }
                }
            }
            let mut macro_targets = std::collections::BTreeMap::new();
            if let Some(ems) = f.get("embedded_macros").and_then(serde_json::Value::as_array) {
                for em in ems {
                    if let Some(stmts) = em.get("statements").and_then(serde_json::Value::as_array) {
                        collect_macro_targets(stmts, &mut macro_targets);
                    }
                }
            }
            Some(json!({
                "name": name,
                "record_source": record_source,
                "bound_controls": bound_controls,
                "events": events,
                "embedded_macro_targets": macro_targets,
            }))
        })
        .collect();

    let macro_refs: Vec<serde_json::Value> = macros
        .iter()
        .filter_map(|m| {
            let name = m.get("name").and_then(serde_json::Value::as_str)?;
            let mut targets = std::collections::BTreeMap::new();
            if let Some(stmts) = m.get("statements").and_then(serde_json::Value::as_array) {
                collect_macro_targets(stmts, &mut targets);
            }
            Some(json!({
                "name": name,
                "targets": targets,
            }))
        })
        .collect();

    let vba_refs: Vec<serde_json::Value> = vba_modules
        .iter()
        .filter_map(|m| {
            let name = m.get("name").and_then(serde_json::Value::as_str)?;
            let src_lower = m
                .get("source")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let tables_used: Vec<String> = table_names
                .iter()
                .filter(|t| text_mentions_ident(&src_lower, t))
                .cloned()
                .collect();
            let queries_used: Vec<String> = query_names
                .iter()
                .filter(|q| text_mentions_ident(&src_lower, q))
                .cloned()
                .collect();
            let forms_used: Vec<String> = form_names
                .iter()
                .filter(|f| text_mentions_ident(&src_lower, f))
                .cloned()
                .collect();
            Some(json!({
                "name": name,
                "tables": tables_used,
                "queries": queries_used,
                "forms": forms_used,
            }))
        })
        .collect();

    json!({
        "forms": form_refs,
        "queries": query_refs,
        "macros": macro_refs,
        "vba_modules": vba_refs,
    })
}

/// Export a machine-readable reverse-engineering bundle from an Access database.
pub fn jet_export_bundle(
    path: &Path,
    mode: JetBundleMode,
    max_rows_per_table: usize,
) -> Result<serde_json::Value, String> {
    let mut reader = open_jet(path)?;
    let version = format!("{:?}", reader.header().version);
    let catalog = read_catalog(&mut reader).map_err(|error| error.to_string())?;
    let mut names = table_names(&mut reader).map_err(|error| error.to_string())?;
    if names.len() > MAX_TABLES {
        return Err("Access database has too many tables to inspect".into());
    }
    names.sort();

    let mut tables = Vec::with_capacity(names.len());
    for name in &names {
        let entry = find_table(&catalog, name).map_err(|error| error.to_string())?;
        let def = read_table_def(&mut reader, &entry.name, entry.table_page)
            .map_err(|error| error.to_string())?;
        let columns: Vec<_> = def
            .columns
            .iter()
            .map(|col| {
                let qtype = jet_type_to_qtype(col.col_type);
                json!({
                    "name": col.name,
                    "jet_type": format!("{:?}", col.col_type),
                    "sql_type": dataset_sql_type(qtype),
                    "size": col.col_size,
                    "auto_number": (col.flags & column_flags::AUTO_LONG) != 0,
                })
            })
            .collect();
        let indexes: Vec<_> = def
            .indexes
            .iter()
            .map(|idx| {
                let cols: Vec<_> = idx
                    .columns
                    .iter()
                    .filter_map(|c| {
                        def.columns
                            .iter()
                            .find(|col| col.col_num == c.col_num)
                            .map(|col| col.name.as_str())
                    })
                    .collect();
                json!({
                    "name": idx.name,
                    "columns": cols,
                })
            })
            .collect();
        tables.push(json!({
            "name": def.name,
            "row_count": def.num_rows,
            "columns": columns,
            "indexes": indexes,
        }));
    }

    let relationships = read_relationships(&mut reader)
        .unwrap_or_default()
        .into_iter()
        .map(|rel| {
            json!({
                "name": rel.name,
                "from_table": rel.from_table,
                "to_table": rel.to_table,
                "columns": rel.columns.iter().map(|c| json!({
                    "from": c.from_column,
                    "to": c.to_column,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    let queries = read_queries(&mut reader)
        .unwrap_or_default()
        .into_iter()
        .map(|qdef| query_def_to_json(&qdef))
        .collect::<Vec<_>>();

    fn macro_statements_to_json(stmts: &[MacroStatement]) -> Vec<serde_json::Value> {
        stmts
            .iter()
            .map(|stmt| match stmt {
                MacroStatement::Action { name, arguments } => {
                    let args: serde_json::Map<String, serde_json::Value> = arguments
                        .iter()
                        .map(|a| (a.name.clone(), json!(a.value)))
                        .collect();
                    json!({
                        "type": "action",
                        "name": name,
                        "arguments": args,
                    })
                }
                MacroStatement::Comment(text) => json!({
                    "type": "comment",
                    "text": text,
                }),
                MacroStatement::Conditional { branches } => json!({
                    "type": "conditional",
                    "branches": branches.iter().map(|b| json!({
                        "condition": b.condition,
                        "statements": macro_statements_to_json(&b.statements),
                    })).collect::<Vec<_>>(),
                }),
                MacroStatement::Group { description, statements } => json!({
                    "type": "group",
                    "description": description,
                    "statements": macro_statements_to_json(statements),
                }),
                MacroStatement::SubMacro { name, statements } => json!({
                    "type": "sub_macro",
                    "name": name,
                    "statements": macro_statements_to_json(statements),
                }),
                MacroStatement::DataBlock { kind, data, statements } => {
                    let data_map: serde_json::Map<String, serde_json::Value> = data
                        .iter()
                        .map(|(k, v)| (k.clone(), json!(v)))
                        .collect();
                    json!({
                        "type": "data_block",
                        "kind": kind,
                        "data": data_map,
                        "statements": macro_statements_to_json(statements),
                    })
                }
                MacroStatement::Unknown(elem) => json!({
                    "type": "unknown",
                    "tag": elem.name,
                }),
                _ => json!({ "type": "unknown" }),
            })
            .collect()
    }

    fn macro_def_to_json(name: &str, def: &MacroDef) -> serde_json::Value {
        json!({
            "name": name,
            "statements": macro_statements_to_json(&def.statements),
        })
    }

    let form_entries = list_forms(&mut reader).unwrap_or_default();
    let mut forms = Vec::with_capacity(form_entries.len());
    for entry in &form_entries {
        match read_form_properties(&mut reader, &entry.name) {
            Ok(fp) => {
                let mut form_json = form_properties_to_json(&fp);
                if mode != JetBundleMode::Metadata
                    && let Ok(embedded) = read_embedded_macros(&mut reader, &entry.name)
                    && !embedded.is_empty()
                {
                    let em_json: Vec<_> = embedded
                        .iter()
                        .map(|def| {
                            let label = match &def.source {
                                jetdb::MacroSource::Embedded { control, event, .. } => {
                                    match control.as_deref() {
                                        Some(ctrl) if !ctrl.is_empty() => {
                                            format!("{ctrl}.{event}")
                                        }
                                        _ => event.clone(),
                                    }
                                }
                                _ => "embedded".to_owned(),
                            };
                            macro_def_to_json(&label, def)
                        })
                        .collect();
                    form_json["embedded_macros"] = serde_json::Value::Array(em_json);
                }
                forms.push(form_json);
            }
            Err(error) => forms.push(json!({
                "name": entry.name,
                "object_type": format!("{:?}", entry.object_type),
                "error": error.to_string(),
            })),
        }
    }

    let macro_entries = list_macros(&mut reader).unwrap_or_default();
    let mut macros = Vec::with_capacity(macro_entries.len());
    for entry in macro_entries {
        if mode == JetBundleMode::Metadata {
            macros.push(json!({ "name": entry.name }));
        } else {
            match read_macro(&mut reader, &entry.name) {
                Ok(def) => macros.push(macro_def_to_json(&entry.name, &def)),
                Err(error) => macros.push(json!({
                    "name": entry.name,
                    "error": error.to_string(),
                })),
            }
        }
    }

    let vba_modules = read_vba_project(&mut reader)
        .ok()
        .map(|proj| {
            proj.modules
                .into_iter()
                .map(|m| {
                    if mode == JetBundleMode::Metadata {
                        json!({
                            "name": m.name,
                            "module_type": format!("{:?}", m.module_type),
                        })
                    } else {
                        json!({
                            "name": m.name,
                            "module_type": format!("{:?}", m.module_type),
                            "source": m.source,
                        })
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let cross_references = build_jet_cross_references(&tables, &queries, &forms, &macros, &vba_modules);

    let mut bundle = json!({
        "format": version,
        "mode": mode.as_str(),
        "summary": {
            "tables": tables.len(),
            "relationships": relationships.len(),
            "queries": queries.len(),
            "forms": forms.len(),
            "macros": macros.len(),
            "vba_modules": vba_modules.len(),
        },
        "tables": tables,
        "relationships": relationships,
        "queries": queries,
        "forms": forms,
        "macros": macros,
        "vba_modules": vba_modules,
        "cross_references": cross_references,
    });

    if mode == JetBundleMode::Full {
        let mut table_data = serde_json::Map::new();
        let row_limit = max_rows_per_table.clamp(1, 1_000);
        let mut remaining_cells = 100_000usize;
        let mut remaining_text = 8 * 1024 * 1024usize;
        for name in &names {
            let mut col_names = Vec::new();
            let mut rows_out = Vec::new();
            let mut total_rows = 0usize;
            let mut truncated = false;
            let visit_res = visit_jet_table(path, name, &[], false, |columns, row| {
                if col_names.is_empty() {
                    col_names = columns.iter().map(|c| c.name.clone()).collect();
                }
                if let Some(row) = row {
                    total_rows += 1;
                    if rows_out.len() >= row_limit
                        || remaining_cells < row.len()
                    {
                        truncated = true;
                        return Ok(());
                    }
                    if super::sqlite_source::charge_dataset_text(&row, &mut remaining_text).is_err()
                    {
                        truncated = true;
                        return Ok(());
                    }
                    remaining_cells -= row.len();
                    rows_out.push(serde_json::Value::Array(
                        row.iter().map(cell_to_json).collect(),
                    ));
                }
                Ok(())
            });
            match visit_res {
                Ok(()) => {
                    table_data.insert(
                        name.clone(),
                        json!({
                            "columns": col_names,
                            "rows": rows_out,
                            "total_rows": total_rows,
                            "truncated": truncated,
                        }),
                    );
                }
                Err(error) => {
                    table_data.insert(
                        name.clone(),
                        json!({
                            "error": error,
                        }),
                    );
                }
            }
        }
        bundle["table_data"] = serde_json::Value::Object(table_data);
    }

    Ok(bundle)
}

/// Materialize user tables from an Access (.mdb / .accdb) database into a standalone SQLite file via Turso.
pub fn jet_export_sqlite(
    input_path: &Path,
    output_path: &Path,
    tables_filter: Option<&[String]>,
    overwrite: bool,
) -> Result<serde_json::Value, String> {
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use turso_core::{Database, PlatformIO, SqliteDialect};

    if output_path.exists() && !overwrite {
        return Err(format!(
            "Output SQLite file already exists: {}",
            output_path.display()
        ));
    }
    let available = jet_tables(input_path)?;
    let target_tables: Vec<String> = match tables_filter {
        Some(filter) => {
            if filter.is_empty() {
                return Err("tables filter must not be empty".into());
            }
            for name in filter {
                if !available.iter().any(|t| t == name) {
                    return Err(format!("Access table does not exist: {name}"));
                }
            }
            filter.to_vec()
        }
        None => available,
    };

    let parent = output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("Invalid output SQLite path")?;
    let temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let temp_str = temp
        .path()
        .to_str()
        .ok_or("Temporary SQLite path is not valid UTF-8")?
        .to_owned();

    let io = Arc::new(PlatformIO::new().map_err(|e| e.to_string())?);
    let database =
        Database::open_file(io, &temp_str, Arc::new(SqliteDialect)).map_err(|e| e.to_string())?;
    let connection = database.connect().map_err(|e| e.to_string())?;
    let execute = |sql: &str| {
        connection
            .prepare(sql)
            .and_then(|mut stmt| stmt.run_ignore_rows())
            .map_err(|e| e.to_string())
    };

    let (catalog_opt, relationships) = match open_jet(input_path) {
        Ok(mut reader) => (
            read_catalog(&mut reader).ok(),
            read_relationships(&mut reader).unwrap_or_default(),
        ),
        Err(_) => (None, Vec::new()),
    };

    let mut exported = Vec::with_capacity(target_tables.len());
    let mut total_rows = 0usize;
    let mut indexes_created = Vec::new();
    let mut skipped_indexes = Vec::new();

    for table_name in &target_tables {
        let identifier = super::sqlite_source::sql_identifier(table_name);
        let table_def_opt = if let Some(catalog) = &catalog_opt {
            open_jet(input_path).ok().and_then(|mut reader| {
                let entry = find_table(catalog, table_name).ok()?;
                read_table_def(&mut reader, &entry.name, entry.table_page).ok()
            })
        } else {
            None
        };

        let mut stmt_insert: Option<turso_core::Statement> = None;
        let mut row_count = 0usize;
        let mut batch_count = 0usize;

        execute("BEGIN")?;
        let res = visit_jet_table(input_path, table_name, &[], false, |columns, row| {
            if let Some(row) = row {
                let stmt = stmt_insert
                    .as_mut()
                    .ok_or("Missing Access table schema")?;
                for (index, value) in row.into_iter().enumerate() {
                    let sql_val = super::sqlite_source::dataset_value(value, columns[index].ty)?;
                    stmt.bind_at(NonZeroUsize::new(index + 1).unwrap(), sql_val)
                        .map_err(|e| e.to_string())?;
                }
                stmt.run_ignore_rows()
                    .and_then(|_| stmt.reset())
                    .map_err(|e| e.to_string())?;
                row_count += 1;
                batch_count += 1;
                if batch_count >= 4096 {
                    execute("COMMIT")?;
                    execute("BEGIN")?;
                    batch_count = 0;
                }
            } else {
                let pk_cols: Vec<String> = table_def_opt
                    .as_ref()
                    .and_then(|def| def.indexes.iter().find(|idx| idx.index_type == 0x01))
                    .map(|idx| {
                        idx.columns
                            .iter()
                            .filter_map(|c| {
                                table_def_opt.as_ref().and_then(|def| {
                                    def.columns
                                        .iter()
                                        .find(|col| col.col_num == c.col_num)
                                        .map(|col| col.name.clone())
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let auto_col_name: Option<String> = table_def_opt.as_ref().and_then(|def| {
                    if pk_cols.len() == 1 {
                        let pk_name = &pk_cols[0];
                        def.columns
                            .iter()
                            .find(|c| {
                                &c.name == pk_name && (c.flags & column_flags::AUTO_LONG) != 0
                            })
                            .map(|c| c.name.clone())
                    } else if pk_cols.is_empty() {
                        def.columns
                            .iter()
                            .find(|c| (c.flags & column_flags::AUTO_LONG) != 0)
                            .map(|c| c.name.clone())
                    } else {
                        None
                    }
                });

                let mut col_defs: Vec<String> = columns
                    .iter()
                    .map(|col| {
                        let col_ident = super::sqlite_source::sql_identifier(&col.name);
                        if auto_col_name.as_deref() == Some(col.name.as_str()) {
                            format!("{col_ident} INTEGER PRIMARY KEY")
                        } else {
                            format!("{col_ident} {}", dataset_sql_type(col.ty))
                        }
                    })
                    .collect();

                if auto_col_name.is_none() && !pk_cols.is_empty() {
                    let pk_list: Vec<String> = pk_cols
                        .iter()
                        .map(|c| super::sqlite_source::sql_identifier(c))
                        .collect();
                    col_defs.push(format!("PRIMARY KEY ({})", pk_list.join(", ")));
                }

                for rel in &relationships {
                    if &rel.from_table == table_name
                        && target_tables.iter().any(|t| t == &rel.to_table)
                        && !rel.columns.is_empty()
                    {
                        let from_cols: Vec<String> = rel
                            .columns
                            .iter()
                            .map(|c| super::sqlite_source::sql_identifier(&c.from_column))
                            .collect();
                        let to_cols: Vec<String> = rel
                            .columns
                            .iter()
                            .map(|c| super::sqlite_source::sql_identifier(&c.to_column))
                            .collect();
                        let to_ident = super::sqlite_source::sql_identifier(&rel.to_table);
                        col_defs.push(format!(
                            "FOREIGN KEY ({}) REFERENCES {to_ident} ({})",
                            from_cols.join(", "),
                            to_cols.join(", ")
                        ));
                    }
                }

                execute(&format!("CREATE TABLE {identifier} ({})", col_defs.join(", ")))?;
                let placeholders = vec!["?"; columns.len()].join(", ");
                let stmt = connection
                    .prepare(format!("INSERT INTO {identifier} VALUES ({placeholders})"))
                    .map_err(|e| e.to_string())?;
                stmt_insert = Some(stmt);
            }
            Ok(())
        });
        if let Err(err) = res {
            let _ = execute("ROLLBACK");
            return Err(err);
        }
        execute("COMMIT")?;

        if let Some(def) = &table_def_opt {
            for idx in &def.indexes {
                if idx.index_type != 0x00 || idx.columns.is_empty() {
                    continue;
                }
                let cols: Vec<String> = idx
                    .columns
                    .iter()
                    .filter_map(|c| {
                        def.columns
                            .iter()
                            .find(|col| col.col_num == c.col_num)
                            .map(|col| {
                                let col_ident = super::sqlite_source::sql_identifier(&col.name);
                                match c.order {
                                    jetdb::IndexColumnOrder::Descending => {
                                        format!("{col_ident} DESC")
                                    }
                                    jetdb::IndexColumnOrder::Ascending => col_ident,
                                }
                            })
                    })
                    .collect();
                if cols.is_empty() {
                    continue;
                }
                let unique = if (idx.flags & 0x01) != 0 {
                    "UNIQUE "
                } else {
                    ""
                };
                let idx_name = format!("{}_{}", table_name, idx.name);
                let idx_ident = super::sqlite_source::sql_identifier(&idx_name);
                let create_idx = format!(
                    "CREATE {unique}INDEX IF NOT EXISTS {idx_ident} ON {identifier} ({})",
                    cols.join(", ")
                );
                match execute(&create_idx) {
                    Ok(()) => indexes_created.push(idx_name),
                    Err(error) => skipped_indexes.push(json!({
                        "table": table_name,
                        "name": idx.name,
                        "error": error,
                    })),
                }
            }
        }

        total_rows += row_count;
        exported.push(json!({
            "name": table_name,
            "rows": row_count,
        }));
    }

    let mut views = Vec::new();
    let mut skipped_views = Vec::new();
    if let Ok(mut reader) = open_jet(input_path)
        && let Ok(queries) = read_queries(&mut reader)
    {
        let mut pending: Vec<(String, String)> = Vec::new();
        for qdef in queries {
            if !matches!(qdef.query_type, jetdb::QueryType::Select) {
                continue;
            }
            let raw_sql = query_to_sql(&qdef);
            let (params, _) = parse_jet_parameters(&raw_sql);
            if !params.is_empty() {
                skipped_views.push(json!({
                    "name": qdef.name,
                    "error": "Parameterized query cannot be materialized as a SQLite VIEW; use sqlite_parameterized_sql from jet_inspect_schema / jet_export_bundle",
                }));
                continue;
            }
            let sqlite_sql = jet_sql_to_sqlite(&raw_sql);
            let trimmed = sqlite_sql.trim().trim_end_matches(';').trim();
            if trimmed.is_empty() {
                continue;
            }
            let view_ident = super::sqlite_source::sql_identifier(&qdef.name);
            let create_sql = format!("CREATE VIEW IF NOT EXISTS {view_ident} AS {trimmed}");
            pending.push((qdef.name, create_sql));
        }

        while !pending.is_empty() {
            let mut next_pending = Vec::new();
            let mut last_errors = Vec::new();
            let prev_len = pending.len();
            for (name, create_sql) in pending {
                match execute(&create_sql) {
                    Ok(()) => views.push(name),
                    Err(error) => {
                        next_pending.push((name.clone(), create_sql));
                        last_errors.push((name, error));
                    }
                }
            }
            if next_pending.len() == prev_len {
                for (name, error) in last_errors {
                    skipped_views.push(json!({
                        "name": name,
                        "error": error,
                    }));
                }
                break;
            }
            pending = next_pending;
        }
    }

    execute("PRAGMA wal_checkpoint(TRUNCATE)")?;
    drop(connection);
    drop(database);

    let wal_sidecar = format!("{temp_str}-wal");
    let _ = std::fs::remove_file(&wal_sidecar);
    if overwrite && output_path.exists() {
        std::fs::remove_file(output_path).map_err(|e| e.to_string())?;
    }
    temp.persist(output_path).map_err(|e| e.error.to_string())?;

    Ok(json!({
        "output": output_path.display().to_string(),
        "tables": exported,
        "indexes": indexes_created,
        "skipped_indexes": skipped_indexes,
        "views": views,
        "skipped_views": skipped_views,
        "total_rows": total_rows,
    }))
}

/// Verify an exported SQLite database against its source Access (`.mdb`/`.accdb`) database:
/// checks table presence, exact row counts, primary-key uniqueness, foreign-key integrity, and view execution.
pub fn jet_verify_sqlite(
    jet_path: &Path,
    sqlite_path: &Path,
) -> Result<serde_json::Value, String> {
    let jet_table_list = jet_tables(jet_path)?;
    let sqlite_table_list = super::sqlite_source::sqlite_tables(sqlite_path)?;

    let mut missing_tables = Vec::new();
    let mut table_checks = Vec::with_capacity(jet_table_list.len());
    let mut pk_violations = Vec::new();
    let mut all_rows_match = true;

    let mut jet_reader = open_jet(jet_path)?;
    let catalog = read_catalog(&mut jet_reader).ok();

    for table_name in &jet_table_list {
        let mut jet_rows = 0usize;
        visit_jet_table(jet_path, table_name, &[], false, |_, row| {
            if row.is_some() {
                jet_rows += 1;
            }
            Ok(())
        })?;

        let Some(sqlite_table_name) = sqlite_table_list
            .iter()
            .find(|t| t.eq_ignore_ascii_case(table_name))
        else {
            all_rows_match = false;
            missing_tables.push(table_name.clone());
            table_checks.push(json!({
                "table": table_name,
                "jet_rows": jet_rows,
                "sqlite_rows": serde_json::Value::Null,
                "match": false,
            }));
            continue;
        };

        let ident = super::sqlite_source::sql_identifier(sqlite_table_name);
        let count_res = super::sqlite_source::sqlite_query(
            sqlite_path,
            &format!("SELECT COUNT(*) FROM {ident}"),
            &[],
            1,
            4096,
        )?;
        let sqlite_rows = count_res
            .rows
            .first()
            .and_then(|r| r.first())
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        let matched = jet_rows == sqlite_rows;
        if !matched {
            all_rows_match = false;
        }
        table_checks.push(json!({
            "table": table_name,
            "jet_rows": jet_rows,
            "sqlite_rows": sqlite_rows,
            "match": matched,
        }));

        if let Some(cat) = &catalog
            && let Ok(entry) = find_table(cat, table_name)
            && let Ok(def) = read_table_def(&mut jet_reader, &entry.name, entry.table_page)
            && let Some(pk_idx) = def.indexes.iter().find(|idx| idx.index_type == 0x01)
        {
            let pk_cols: Vec<String> = pk_idx
                .columns
                .iter()
                .filter_map(|c| {
                    def.columns
                        .iter()
                        .find(|col| col.col_num == c.col_num)
                        .map(|col| super::sqlite_source::sql_identifier(&col.name))
                })
                .collect();
            if !pk_cols.is_empty() {
                let group_list = pk_cols.join(", ");
                let dup_sql = format!(
                    "SELECT COUNT(*) FROM (SELECT 1 FROM {ident} GROUP BY {group_list} HAVING COUNT(*) > 1)"
                );
                if let Ok(dup_res) =
                    super::sqlite_source::sqlite_query(sqlite_path, &dup_sql, &[], 1, 4096)
                {
                    let dup_count = dup_res
                        .rows
                        .first()
                        .and_then(|r| r.first())
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if dup_count > 0 {
                        pk_violations.push(json!({
                            "table": table_name,
                            "duplicate_keys": dup_count,
                        }));
                    }
                }
            }
        }
    }

    let fk_res = super::sqlite_source::sqlite_query(
        sqlite_path,
        "PRAGMA foreign_key_check",
        &[],
        100,
        64 * 1024,
    );
    let fk_violations: Vec<serde_json::Value> = match fk_res {
        Ok(res) => res
            .rows
            .into_iter()
            .map(|r| {
                json!({
                    "table": r.first().cloned().unwrap_or(serde_json::Value::Null),
                    "rowid": r.get(1).cloned().unwrap_or(serde_json::Value::Null),
                    "parent": r.get(2).cloned().unwrap_or(serde_json::Value::Null),
                    "fkid": r.get(3).cloned().unwrap_or(serde_json::Value::Null),
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    let mut view_checks = Vec::new();
    let mut views_ok = true;
    if let Ok(views_res) = super::sqlite_source::sqlite_query(
        sqlite_path,
        "SELECT name FROM sqlite_master WHERE type = 'view' ORDER BY name",
        &[],
        256,
        64 * 1024,
    ) {
        for row in views_res.rows {
            if let Some(name) = row.first().and_then(|v| v.as_str()) {
                let v_ident = super::sqlite_source::sql_identifier(name);
                match super::sqlite_source::sqlite_query(
                    sqlite_path,
                    &format!("SELECT * FROM {v_ident} LIMIT 1"),
                    &[],
                    1,
                    16 * 1024,
                ) {
                    Ok(_) => view_checks.push(json!({ "name": name, "ok": true })),
                    Err(err) => {
                        views_ok = false;
                        view_checks.push(json!({ "name": name, "ok": false, "error": err }));
                    }
                }
            }
        }
    }

    let ok = all_rows_match
        && missing_tables.is_empty()
        && pk_violations.is_empty()
        && fk_violations.is_empty()
        && views_ok;

    Ok(json!({
        "ok": ok,
        "tables": table_checks,
        "missing_tables": missing_tables,
        "pk_violations": pk_violations,
        "fk_violations": fk_violations,
        "views": view_checks,
    }))
}

fn sanitize_fs_component(raw: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
            out.push(ch);
        } else if !ch.is_control() && !matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches(|c| c == '.' || c == ' ');
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn sniff_binary_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"%PDF-") {
        "pdf"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "jpg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "gif"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else if bytes.starts_with(b"PK\x03\x04") {
        "zip"
    } else {
        "bin"
    }
}

/// Export Access Attachment (`ComplexType`) and `Binary`/`Ole` column payloads to `output_dir`
/// along with a `manifest.json` mapping table/row/column to exported file paths.
pub fn jet_export_attachments(
    jet_path: &Path,
    output_dir: &Path,
    tables_filter: Option<&[String]>,
    overwrite: bool,
) -> Result<serde_json::Value, String> {
    let available = jet_tables(jet_path)?;
    let target_tables: Vec<String> = match tables_filter {
        Some(filter) => {
            if filter.is_empty() {
                return Err("tables filter must not be empty".into());
            }
            for name in filter {
                if !available.iter().any(|t| t == name) {
                    return Err(format!("Access table does not exist: {name}"));
                }
            }
            filter.to_vec()
        }
        None => available,
    };

    std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    let mut reader = open_jet(jet_path)?;
    let catalog = read_catalog(&mut reader).map_err(|e| e.to_string())?;

    let mut exported_files = Vec::new();
    let mut total_bytes = 0usize;

    for table_name in &target_tables {
        let entry = find_table(&catalog, table_name).map_err(|e| e.to_string())?;
        let table_def =
            read_table_def(&mut reader, &entry.name, entry.table_page).map_err(|e| e.to_string())?;

        let blob_indices: Vec<usize> = table_def
            .columns
            .iter()
            .enumerate()
            .filter_map(|(idx, col)| {
                matches!(
                    col.col_type,
                    ColumnType::ComplexType | ColumnType::Binary | ColumnType::Ole
                )
                .then_some(idx)
            })
            .collect();
        if blob_indices.is_empty() {
            continue;
        }

        let complex_cols: Vec<(usize, Option<ComplexColumn>)> = blob_indices
            .iter()
            .map(|&idx| {
                let col = &table_def.columns[idx];
                let cc = if col.col_type == ColumnType::ComplexType {
                    read_complex_column(&mut reader, &catalog, &table_def, col)
                        .ok()
                        .flatten()
                } else {
                    None
                };
                (idx, cc)
            })
            .collect();

        let table_dir_name = sanitize_fs_component(table_name, "table");
        let table_dir = output_dir.join(&table_dir_name);
        let calculated = calculated_column_types_in(&catalog, &table_def);
        let mut rows = TableRows::new(&table_def, &calculated);
        let mut row_num = 0usize;

        while let Some(row) = rows.next_row(&mut reader).map_err(|e| e.to_string())? {
            row_num += 1;
            if row_num > MAX_ROWS {
                break;
            }
            for (col_idx, cc_opt) in &complex_cols {
                let col = &table_def.columns[*col_idx];
                let col_part = sanitize_fs_component(&col.name, "col");
                let Some(val) = row.values.get(*col_idx) else {
                    continue;
                };
                if let (Some(cc), Value::Long(complex_id)) = (cc_opt, val) {
                    if let Ok(ComplexValues::Attachment(atts)) = cc.values(&mut reader, *complex_id) {
                        for (att_idx, att) in atts.into_iter().enumerate() {
                            let Value::Binary(bytes) = att.data else {
                                continue;
                            };
                            let raw_name = match att.name {
                                Value::Text(s) => s,
                                _ => format!("attachment_{}", att_idx + 1),
                            };
                            let safe_name = sanitize_fs_component(&raw_name, "attachment.bin");
                            let file_name = format!("row{row_num}_{col_part}_{safe_name}");
                            std::fs::create_dir_all(&table_dir).map_err(|e| e.to_string())?;
                            let dest = table_dir.join(&file_name);
                            if dest.exists() && !overwrite {
                                return Err(format!(
                                    "Attachment output file already exists: {}",
                                    dest.display()
                                ));
                            }
                            std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
                            total_bytes = total_bytes.saturating_add(bytes.len());
                            exported_files.push(json!({
                                "table": table_name,
                                "row": row_num,
                                "column": col.name,
                                "kind": "attachment",
                                "filename": raw_name,
                                "bytes": bytes.len(),
                                "relative_path": format!("{table_dir_name}/{file_name}"),
                            }));
                        }
                    }
                } else if let Value::Binary(bytes) = val
                    && !bytes.is_empty()
                {
                    let ext = sniff_binary_ext(bytes);
                    let file_name = format!("row{row_num}_{col_part}.{ext}");
                    std::fs::create_dir_all(&table_dir).map_err(|e| e.to_string())?;
                    let dest = table_dir.join(&file_name);
                    if dest.exists() && !overwrite {
                        return Err(format!(
                            "Binary output file already exists: {}",
                            dest.display()
                        ));
                    }
                    std::fs::write(&dest, bytes).map_err(|e| e.to_string())?;
                    total_bytes = total_bytes.saturating_add(bytes.len());
                    exported_files.push(json!({
                        "table": table_name,
                        "row": row_num,
                        "column": col.name,
                        "kind": "binary",
                        "bytes": bytes.len(),
                        "relative_path": format!("{table_dir_name}/{file_name}"),
                    }));
                }
            }
        }
    }

    let manifest = json!({
        "output_dir": output_dir.display().to_string(),
        "exported_count": exported_files.len(),
        "total_bytes": total_bytes,
        "files": exported_files,
    });
    let manifest_path = output_dir.join("manifest.json");
    if manifest_path.exists() && !overwrite {
        return Err(format!(
            "Manifest already exists: {}",
            manifest_path.display()
        ));
    }
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_jet_column_types_and_values() {
        assert_eq!(jet_type_to_qtype(ColumnType::Boolean), QType::Logical);
        assert_eq!(jet_type_to_qtype(ColumnType::Int), QType::Int32);
        assert_eq!(jet_type_to_qtype(ColumnType::Long), QType::Whole);
        assert_eq!(jet_type_to_qtype(ColumnType::Double), QType::Decimal);
        assert_eq!(jet_type_to_qtype(ColumnType::Timestamp), QType::DateTime);
        assert_eq!(jet_type_to_qtype(ColumnType::Money), QType::Text);
        assert_eq!(jet_type_to_qtype(ColumnType::Text), QType::Text);

        assert_eq!(jet_value_to_cell(Value::Null), CellValue::Empty);
        assert_eq!(jet_value_to_cell(Value::Bool(true)), CellValue::Bool(true));
        assert_eq!(jet_value_to_cell(Value::Long(42)), CellValue::Number(42.0));
        assert_eq!(
            jet_value_to_cell(Value::BigInt(i64::MAX)),
            CellValue::Text(i64::MAX.to_string())
        );
        assert_eq!(
            jet_value_to_cell(Value::Money("1234.5600".into())),
            CellValue::Text("1234.5600".into())
        );
        assert_eq!(
            jet_value_to_cell(Value::Binary(vec![1, 2, 3])),
            CellValue::Text("[3 bytes]".into())
        );
    }

    #[test]
    fn rejects_missing_or_invalid_access_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.accdb");
        assert!(jet_tables(&missing).is_err());

        let invalid = dir.path().join("not_access.mdb");
        std::fs::write(&invalid, b"not a jet database").unwrap();
        assert!(jet_tables(&invalid).is_err());
        assert!(jet_inspect_schema(&invalid).is_err());
    }

    #[test]
    fn synthetic_jet4_mdb_end_to_end_with_turso_and_export() {
        use super::super::sqlite_source::{DiskDataset, SqliteQuerySession, sqlite_query, sqlite_tables};
        use datacraft_jet::jet::{
            create::create_jet4_file,
            tdef::{JetColumn, JetType, flags as tdef_flags},
            writer::Jet4Writer,
        };
        use datacraft_jet::StorageValue;

        let dir = tempfile::tempdir().unwrap();
        let mdb_path = dir.path().join("orders.mdb");
        create_jet4_file(&mdb_path, 0.0).unwrap();

        {
            let bytes = std::fs::read(&mdb_path).unwrap();
            let mut writer = Jet4Writer::new(bytes).unwrap();
            let cols = vec![
                JetColumn {
                    name: "id".into(),
                    data_type: JetType::LongInt,
                    flags: tdef_flags::FIXED | tdef_flags::AUTONUMBER,
                    column_id: 0,
                    var_index: None,
                    fixed_offset: 0,
                    len: 4,
                    compressed_text: false,
                    indexed: Default::default(),
                    misc: 0,
                },
                JetColumn {
                    name: "code".into(),
                    data_type: JetType::Text,
                    flags: 0,
                    column_id: 1,
                    var_index: Some(0),
                    fixed_offset: 0,
                    len: 100,
                    compressed_text: false,
                    indexed: Default::default(),
                    misc: 0,
                },
                JetColumn {
                    name: "amount".into(),
                    data_type: JetType::LongInt,
                    flags: tdef_flags::FIXED,
                    column_id: 2,
                    var_index: None,
                    fixed_offset: 4,
                    len: 4,
                    compressed_text: false,
                    indexed: Default::default(),
                    misc: 0,
                },
                JetColumn {
                    name: "paid".into(),
                    data_type: JetType::Bool,
                    flags: tdef_flags::FIXED,
                    column_id: 3,
                    var_index: None,
                    fixed_offset: 8,
                    len: 0,
                    compressed_text: false,
                    indexed: Default::default(),
                    misc: 0,
                },
            ];
            let tdef_page = writer.create_table("Orders", &cols).unwrap();
            writer
                .insert_row(
                    tdef_page,
                    &[
                        StorageValue::Null,
                        StorageValue::Text("A01".into()),
                        StorageValue::Int(1500),
                        StorageValue::Bit(true),
                    ],
                )
                .unwrap();
            writer
                .insert_row(
                    tdef_page,
                    &[
                        StorageValue::Null,
                        StorageValue::Text("B02".into()),
                        StorageValue::Int(2500),
                        StorageValue::Bit(false),
                    ],
                )
                .unwrap();
            std::fs::write(&mdb_path, writer.into_file()).unwrap();
        }

        // 1. Catalog & schema inspection
        assert_eq!(jet_tables(&mdb_path).unwrap(), vec!["Orders"]);
        let schema = jet_inspect_schema(&mdb_path).unwrap();
        assert_eq!(schema["tables"][0]["name"], "Orders");
        assert_eq!(schema["tables"][0]["columns"].as_array().unwrap().len(), 4);

        // 2. Turso cross-format JOIN (Access table + CSV QTable)
        let labels = QTable {
            columns: vec![
                QColumn { name: "code".into(), ty: QType::Text },
                QColumn { name: "label".into(), ty: QType::Text },
            ],
            rows: vec![
                vec![CellValue::Text("A01".into()), CellValue::Text("東京".into())],
                vec![CellValue::Text("B02".into()), CellValue::Text("大阪".into())],
            ],
        };
        let session = SqliteQuerySession::from_disk_datasets(&[
            ("orders".into(), DiskDataset::Jet(mdb_path.clone(), "Orders".into(), vec![])),
            ("labels".into(), DiskDataset::Table(labels)),
        ])
        .unwrap();
        let res = session
            .query(
                "SELECT o.id, l.label, o.amount, o.paid FROM orders o JOIN labels l USING (code) ORDER BY o.id",
                &[],
                50,
                64 * 1024,
            )
            .unwrap();
        assert_eq!(
            res.rows,
            vec![
                vec![json!(1), json!("東京"), json!(1500), json!(1)],
                vec![json!(2), json!("大阪"), json!(2500), json!(0)],
            ]
        );

        // 3. Materialize to standalone .sqlite via jet_export_sqlite and query it
        let sqlite_out = dir.path().join("migrated.sqlite");
        let export_res = jet_export_sqlite(&mdb_path, &sqlite_out, None, false).unwrap();
        assert_eq!(export_res["total_rows"], 2);
        assert_eq!(sqlite_tables(&sqlite_out).unwrap(), vec!["orders"]);
        let q = sqlite_query(
            &sqlite_out,
            "SELECT code, amount FROM \"orders\" WHERE paid = 1",
            &[],
            50,
            64 * 1024,
        )
        .unwrap();
        assert_eq!(q.rows, vec![vec![json!("A01"), json!(1500)]]);

        let verified = jet_verify_sqlite(&mdb_path, &sqlite_out).unwrap();
        assert_eq!(verified["ok"], true, "verified={verified:#}");
        assert_eq!(verified["tables"][0]["match"], true);
        assert_eq!(verified["missing_tables"], json!([]));
        assert_eq!(verified["pk_violations"], json!([]));
        assert_eq!(verified["fk_violations"], json!([]));

        let att_dir = dir.path().join("attachments");
        let att_res = jet_export_attachments(&mdb_path, &att_dir, None, false).unwrap();
        assert_eq!(att_res["exported_count"], 0);
        assert!(att_dir.join("manifest.json").is_file());

        // 4. Generate Markdown + Mermaid ER migration spec
        let md = jet_migration_markdown(&mdb_path).unwrap();
        assert!(md.contains("```mermaid\nerDiagram"));
        assert!(md.contains("INTEGER id PK"));
        assert!(md.contains("TEXT code"));
    }

    #[test]
    fn synthetic_jet4_japanese_identifiers() {
        use super::super::sqlite_source::{DiskDataset, SqliteQuerySession};
        use datacraft_jet::jet::{
            create::create_jet4_file,
            tdef::{JetColumn, JetType, flags as tdef_flags},
            writer::Jet4Writer,
        };
        use datacraft_jet::StorageValue;

        let dir = tempfile::tempdir().unwrap();
        let mdb_path = dir.path().join("legacy_ja.mdb");
        create_jet4_file(&mdb_path, 0.0).unwrap();

        let table_name = "Orders 2026 (JA)";
        {
            let bytes = std::fs::read(&mdb_path).unwrap();
            let mut writer = Jet4Writer::new(bytes).unwrap();
            let cols = vec![
                JetColumn {
                    name: "顧客名".into(),
                    data_type: JetType::Text,
                    flags: 0,
                    column_id: 0,
                    var_index: Some(0),
                    fixed_offset: 0,
                    len: 100,
                    compressed_text: false,
                    indexed: Default::default(),
                    misc: 0,
                },
                JetColumn {
                    name: "金額（税込）".into(),
                    data_type: JetType::LongInt,
                    flags: tdef_flags::FIXED,
                    column_id: 1,
                    var_index: None,
                    fixed_offset: 0,
                    len: 4,
                    compressed_text: false,
                    indexed: Default::default(),
                    misc: 0,
                },
            ];
            let tdef_page = writer.create_table(table_name, &cols).unwrap();
            writer
                .insert_row(
                    tdef_page,
                    &[
                        StorageValue::Text("株式会社テスト".into()),
                        StorageValue::Int(11000),
                    ],
                )
                .unwrap();
            std::fs::write(&mdb_path, writer.into_file()).unwrap();
        }

        assert_eq!(jet_tables(&mdb_path).unwrap(), vec![table_name]);
        let session = SqliteQuerySession::from_disk_datasets(&[(
            "ja_orders".into(),
            DiskDataset::Jet(mdb_path.clone(), table_name.into(), vec![]),
        )])
        .unwrap();
        let res = session
            .query(
                "SELECT \"顧客名\", \"金額（税込）\" FROM ja_orders",
                &[],
                50,
                64 * 1024,
            )
            .unwrap();
        assert_eq!(res.rows, vec![vec![json!("株式会社テスト"), json!(11000)]]);

        let md = jet_migration_markdown(&mdb_path).unwrap();
        assert!(md.contains("```mermaid\nerDiagram"));
        // Non-ASCII characters are sanitized to '_' for Mermaid identifier safety
        assert!(md.contains("TEXT ___"));

        // VBA/Form inspection on a database without VBA/forms returns empty/FormNotFound cleanly
        let vba = jet_inspect_vba(&mdb_path, None).unwrap();
        assert_eq!(vba["modules"], json!([]));
        assert!(jet_inspect_form(&mdb_path, "MissingForm").is_err());

        // Machine-readable bundle export in metadata / code / full modes
        let meta_bundle = jet_export_bundle(&mdb_path, JetBundleMode::Metadata, 100).unwrap();
        assert_eq!(meta_bundle["mode"], "metadata");
        assert_eq!(meta_bundle["summary"]["tables"], 1);
        assert!(meta_bundle.get("table_data").is_none());

        let code_bundle = jet_export_bundle(&mdb_path, JetBundleMode::Code, 100).unwrap();
        assert_eq!(code_bundle["mode"], "code");
        assert!(code_bundle.get("table_data").is_none());

        let full_bundle = jet_export_bundle(&mdb_path, JetBundleMode::Full, 10).unwrap();
        assert_eq!(full_bundle["mode"], "full");
        assert_eq!(
            full_bundle["table_data"][table_name]["rows"],
            json!([["株式会社テスト", 11000]])
        );
        assert_eq!(full_bundle["table_data"][table_name]["truncated"], false);

        assert_eq!(JetBundleMode::parse("metadata").unwrap(), JetBundleMode::Metadata);
        assert_eq!(JetBundleMode::parse("code").unwrap(), JetBundleMode::Code);
        assert_eq!(JetBundleMode::parse("all").unwrap(), JetBundleMode::Full);
        assert!(JetBundleMode::parse("unknown").is_err());
    }

    #[test]
    fn transpiles_jet_sql_idioms_to_sqlite() {
        assert_eq!(
            jet_sql_to_sqlite("SELECT [Orders].[Order ID], Nz([Amount], 0) FROM [Orders] WHERE [OrderDate] >= #2026/01/15#"),
            "SELECT \"Orders\".\"Order ID\", COALESCE(\"Amount\", 0) FROM \"Orders\" WHERE \"OrderDate\" >= '2026-01-15'"
        );
        assert_eq!(
            jet_sql_to_sqlite("SELECT IIf([Paid] = True, 'Yes', IIf(IsNull([Note]), 'Pending', [Note])) FROM [Orders]"),
            "SELECT CASE WHEN \"Paid\" = True THEN 'Yes' ELSE CASE WHEN (\"Note\" IS NULL) THEN 'Pending' ELSE \"Note\" END END FROM \"Orders\""
        );
        assert_eq!(
            jet_sql_to_sqlite("SELECT UCase(Trim([Name])), Len([Name]), Left([Code], 3), Mid([Code], 2, 4), Year([OrderDate]) FROM [Orders]"),
            "SELECT UPPER(TRIM(\"Name\")), LENGTH(\"Name\"), SUBSTR(\"Code\", 1, 3), SUBSTR(\"Code\", 2, 4), CAST(strftime('%Y', \"OrderDate\") AS INTEGER) FROM \"Orders\""
        );

        let raw_param = "PARAMETERS [StartDate] DateTime, [MinAmt] Currency; SELECT * FROM [Orders] WHERE [OrderDate] >= [StartDate] AND [Amount] >= [MinAmt]";
        let (params, body) = parse_jet_parameters(raw_param);
        assert_eq!(params.len(), 2);
        let sqlite_body = jet_sql_to_sqlite(body);
        let (param_sql, pos) = replace_param_placeholders(&sqlite_body, &params);
        assert_eq!(
            param_sql,
            "SELECT * FROM \"Orders\" WHERE \"OrderDate\" >= ? AND \"Amount\" >= ?"
        );
        assert_eq!(pos, vec!["StartDate", "MinAmt"]);

        let raw_del = "PARAMETERS [TargetID] Long; DELETE * FROM [Orders] WHERE [ID] = [TargetID]";
        let (dparams, dbody) = parse_jet_parameters(raw_del);
        let (dsql, dpos) = replace_param_placeholders(&jet_sql_to_sqlite(dbody), &dparams);
        assert_eq!(dsql, "DELETE FROM \"Orders\" WHERE \"ID\" = ?");
        assert_eq!(dpos, vec!["TargetID"]);
    }

    #[test]
    fn builds_cross_references_from_forms_queries_macros_and_vba() {
        let tables = vec![json!({"name": "Orders"}), json!({"name": "Customers"})];
        let queries = vec![json!({"name": "qOpenOrders", "sql": "SELECT * FROM Orders JOIN Customers ON Orders.CID = Customers.ID"})];
        let forms = vec![json!({
            "name": "frmOrders",
            "named_properties": {"RecordSource": "qOpenOrders", "OnOpen": "[Event Procedure]"},
            "controls": [{"name": "btnSave", "named_properties": {"ControlSource": "Amount", "OnClick": "mcrSave"}}],
            "embedded_macros": [{"name": "btnPrint.OnClick", "statements": [{"type": "action", "name": "OpenReport", "arguments": {"ReportName": "rptInvoice"}}]}],
        })];
        let macros = vec![json!({
            "name": "mcrSave",
            "statements": [{"type": "action", "name": "OpenForm", "arguments": {"FormName": "frmOrders"}}]
        })];
        let vba = vec![json!({
            "name": "Form_frmOrders",
            "source": "Private Sub Form_Open()\nDoCmd.OpenQuery \"qOpenOrders\"\nEnd Sub"
        })];

        let xref = build_jet_cross_references(&tables, &queries, &forms, &macros, &vba);
        assert_eq!(xref["queries"][0]["tables"], json!(["Orders", "Customers"]));
        assert_eq!(xref["forms"][0]["record_source"], json!("qOpenOrders"));
        assert_eq!(xref["forms"][0]["events"]["btnSave.OnClick"], json!("mcrSave"));
        assert_eq!(
            xref["forms"][0]["embedded_macro_targets"]["opens_reports"],
            json!(["rptInvoice"])
        );
        assert_eq!(xref["macros"][0]["targets"]["opens_forms"], json!(["frmOrders"]));
        assert_eq!(xref["vba_modules"][0]["queries"], json!(["qOpenOrders"]));
    }

    #[test]
    fn normalizes_form_control_layout_and_expressions() {
        let mut named = serde_json::Map::new();
        named.insert("Left".into(), json!(1500));
        named.insert("Top".into(), json!(300));
        named.insert("Width".into(), json!(2250));
        named.insert("Height".into(), json!(360));
        named.insert("TabIndex".into(), json!(2));
        named.insert("ControlSource".into(), json!("=Sum([Amount])"));
        let layout = extract_control_layout(&named);
        assert_eq!(layout["left_px"], json!(100.0));
        assert_eq!(layout["top_px"], json!(20.0));
        assert_eq!(layout["width_px"], json!(150.0));
        assert_eq!(layout["height_px"], json!(24.0));
        assert_eq!(layout["tab_index"], json!(2));
        assert_eq!(layout["expression"], json!("=Sum([Amount])"));
    }
}

/// Best-effort translation of Access JET SQL idioms into Turso / SQLite-compatible SQL.
pub fn jet_sql_to_sqlite(sql: &str) -> String {
    let mut pass1 = String::with_capacity(sql.len());
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '\'' {
            pass1.push('\'');
            i += 1;
            while i < chars.len() {
                pass1.push(chars[i]);
                if chars[i] == '\'' {
                    if i + 1 < chars.len() && chars[i + 1] == '\'' {
                        pass1.push('\'');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if ch == '[' {
            i += 1;
            pass1.push('"');
            while i < chars.len() && chars[i] != ']' {
                if chars[i] == '"' {
                    pass1.push('"');
                }
                pass1.push(chars[i]);
                i += 1;
            }
            pass1.push('"');
            if i < chars.len() && chars[i] == ']' {
                i += 1;
            }
            continue;
        }
        if ch == '#' {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && chars[end] != '#' && !chars[end].is_whitespace() {
                end += 1;
            }
            if end < chars.len() && chars[end] == '#' && end > start {
                let inner: String = chars[start..end].iter().collect();
                let normalized = inner.replace('/', "-");
                pass1.push('\'');
                pass1.push_str(&normalized);
                pass1.push('\'');
                i = end + 1;
                continue;
            }
        }
        pass1.push(ch);
        i += 1;
    }

    let rewritten = rewrite_jet_functions(&pass1);
    let trimmed = rewritten.trim_start();
    if trimmed.len() >= 13 && trimmed[..6].eq_ignore_ascii_case("DELETE") {
        let after_del = trimmed[6..].trim_start();
        if let Some(after_star) = after_del.strip_prefix('*') {
            let after_ws = after_star.trim_start();
            if after_ws.len() >= 4 && after_ws[..4].eq_ignore_ascii_case("FROM") {
                return format!("DELETE {after_ws}");
            }
        }
    }
    rewritten
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn split_top_level_args(inner: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut start = 0usize;
    let bytes = inner.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
        } else if in_double {
            if b == b'"' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                    i += 2;
                    continue;
                }
                in_double = false;
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                b',' if depth == 0 => {
                    args.push(inner[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    args.push(inner[start..].trim());
    args
}

fn find_matching_paren(s: &str, open_idx: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut i = open_idx;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
        } else if in_double {
            if b == b'"' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                    i += 2;
                    continue;
                }
                in_double = false;
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'(' => depth += 1,
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn rewrite_jet_functions(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let bytes = input.as_bytes();
    let mut in_single = false;
    let mut in_double = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            out.push(b as char);
            if b == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    out.push('\'');
                    i += 2;
                    continue;
                }
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            out.push(b as char);
            if b == b'"' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                    out.push('"');
                    i += 2;
                    continue;
                }
                in_double = false;
            }
            i += 1;
            continue;
        }
        if b == b'\'' {
            in_single = true;
            out.push('\'');
            i += 1;
            continue;
        }
        if b == b'"' {
            in_double = true;
            out.push('"');
            i += 1;
            continue;
        }

        let prev_is_ident = i > 0 && is_ident_char(bytes[i - 1] as char);
        if !prev_is_ident {
            let rest = &input[i..];
            if rest.len() >= 3 && rest[..3].eq_ignore_ascii_case("nz(") {
                if let Some(close) = find_matching_paren(input, i + 2) {
                    let inner = rewrite_jet_functions(&input[i + 3..close]);
                    out.push_str("COALESCE(");
                    out.push_str(&inner);
                    out.push(')');
                    i = close + 1;
                    continue;
                }
            } else if rest.len() >= 4 && rest[..4].eq_ignore_ascii_case("iif(") {
                if let Some(close) = find_matching_paren(input, i + 3) {
                    let inner = &input[i + 4..close];
                    let args = split_top_level_args(inner);
                    if args.len() == 3 {
                        let cond = rewrite_jet_functions(args[0]);
                        let then_v = rewrite_jet_functions(args[1]);
                        let else_v = rewrite_jet_functions(args[2]);
                        out.push_str("CASE WHEN ");
                        out.push_str(&cond);
                        out.push_str(" THEN ");
                        out.push_str(&then_v);
                        out.push_str(" ELSE ");
                        out.push_str(&else_v);
                        out.push_str(" END");
                        i = close + 1;
                        continue;
                    }
                }
            } else if rest.len() >= 7 && rest[..7].eq_ignore_ascii_case("isnull(") {
                if let Some(close) = find_matching_paren(input, i + 6) {
                    let inner = rewrite_jet_functions(input[i + 7..close].trim());
                    out.push('(');
                    out.push_str(&inner);
                    out.push_str(" IS NULL)");
                    i = close + 1;
                    continue;
                }
            } else if rest.len() >= 5 && rest[..5].eq_ignore_ascii_case("left(") {
                if let Some(close) = find_matching_paren(input, i + 4) {
                    let args = split_top_level_args(&input[i + 5..close]);
                    if args.len() == 2 {
                        let s = rewrite_jet_functions(args[0]);
                        let n = rewrite_jet_functions(args[1]);
                        out.push_str("SUBSTR(");
                        out.push_str(&s);
                        out.push_str(", 1, ");
                        out.push_str(&n);
                        out.push(')');
                        i = close + 1;
                        continue;
                    }
                }
            } else if let Some((prefix_len, sql_fn)) = [
                ("len(", "LENGTH("),
                ("ucase(", "UPPER("),
                ("lcase(", "LOWER("),
                ("trim(", "TRIM("),
                ("ltrim(", "LTRIM("),
                ("rtrim(", "RTRIM("),
                ("mid(", "SUBSTR("),
            ]
            .into_iter()
            .find(|(p, _)| rest.len() >= p.len() && rest[..p.len()].eq_ignore_ascii_case(p))
            {
                if let Some(close) = find_matching_paren(input, i + prefix_len.len() - 1) {
                    let inner = rewrite_jet_functions(&input[i + prefix_len.len()..close]);
                    out.push_str(sql_fn);
                    out.push_str(&inner);
                    out.push(')');
                    i = close + 1;
                    continue;
                }
            } else if let Some((prefix_len, fmt)) = [
                ("year(", "%Y"),
                ("month(", "%m"),
                ("day(", "%d"),
            ]
            .into_iter()
            .find(|(p, _)| rest.len() >= p.len() && rest[..p.len()].eq_ignore_ascii_case(p))
            {
                if let Some(close) = find_matching_paren(input, i + prefix_len.len() - 1) {
                    let inner = rewrite_jet_functions(input[i + prefix_len.len()..close].trim());
                    out.push_str("CAST(strftime('");
                    out.push_str(fmt);
                    out.push_str("', ");
                    out.push_str(&inner);
                    out.push_str(") AS INTEGER)");
                    i = close + 1;
                    continue;
                }
            } else if rest.len() >= 5 && rest[..5].eq_ignore_ascii_case("now()") {
                out.push_str("CURRENT_TIMESTAMP");
                i += 5;
                continue;
            } else if rest.len() >= 6 && rest[..6].eq_ignore_ascii_case("date()") {
                out.push_str("CURRENT_DATE");
                i += 6;
                continue;
            }
        }

        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}
