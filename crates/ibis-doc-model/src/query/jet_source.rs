// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType, sqlite_source::dataset_sql_type};
use crate::CellValue;
use jetdb::{
    ColumnType, ComplexColumn, ComplexValues, PageReader, TableRows, Value,
    calculated_column_types_in, column_flags, find_table, list_forms, query_to_sql, read_catalog,
    read_complex_column, read_queries, read_relationships, read_table_def, read_vba_project,
    table_names,
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
    PageReader::open(path).map_err(|error| format!("Failed to open Access database: {error}"))
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
            json!({
                "name": qdef.name,
                "query_type": format!("{:?}", qdef.query_type),
                "sql": sql,
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

    Ok(json!({
        "format": version,
        "tables": tables,
        "relationships": relationships,
        "queries": queries,
        "forms": forms,
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
            let _ = writeln!(md, "### `{name}` ({qtype})\n");
            let _ = writeln!(md, "```sql\n{sql}\n```\n");
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
    }

    Ok(md)
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

    let mut exported = Vec::with_capacity(target_tables.len());
    let mut total_rows = 0usize;

    for table_name in &target_tables {
        let identifier = super::sqlite_source::sql_identifier(table_name);
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
                let col_defs: Vec<String> = columns
                    .iter()
                    .map(|col| {
                        format!(
                            "{} {}",
                            super::sqlite_source::sql_identifier(&col.name),
                            dataset_sql_type(col.ty)
                        )
                    })
                    .collect();
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
        total_rows += row_count;
        exported.push(json!({
            "name": table_name,
            "rows": row_count,
        }));
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
}
