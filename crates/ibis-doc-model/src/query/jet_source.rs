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
        .map(|qdef| {
            let sql = query_to_sql(&qdef);
            let sqlite_sql = jet_sql_to_sqlite(&sql);
            json!({
                "name": qdef.name,
                "query_type": format!("{:?}", qdef.query_type),
                "sql": sql,
                "sqlite_sql": sqlite_sql,
            })
        })
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

fn form_properties_to_json(fp: &jetdb::FormProperties) -> serde_json::Value {
    let (properties, named_properties) = encode_blob_properties(&fp.properties);
    let controls: Vec<_> = fp
        .controls
        .iter()
        .map(|c| {
            let (props, named_props) = encode_blob_properties(&c.properties);
            json!({
                "name": c.name,
                "type": control_type_name(c.type_code),
                "type_code": c.type_code,
                "named_properties": named_props,
                "properties": props,
            })
        })
        .collect();
    json!({
        "name": fp.form_name,
        "object_type": format!("{:?}", fp.object_type),
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
        .map(|qdef| {
            let sql = query_to_sql(&qdef);
            let sqlite_sql = jet_sql_to_sqlite(&sql);
            json!({
                "name": qdef.name,
                "query_type": format!("{:?}", qdef.query_type),
                "sql": sql,
                "sqlite_sql": sqlite_sql,
            })
        })
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

    rewrite_jet_functions(&pass1)
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
