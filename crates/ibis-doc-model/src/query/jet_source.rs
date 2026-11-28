// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType, sqlite_source::dataset_sql_type};
use crate::CellValue;
use jetdb::{
    ColumnType, PageReader, TableRows, Value, calculated_column_types_in, column_flags,
    find_table, list_forms, query_to_sql, read_catalog, read_queries, read_relationships,
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
        let values = row.values.into_iter().map(jet_value_to_cell).collect();
        visit(&columns, Some(values))?;
    }
    rows.warn_skipped(&table_def.name);
    Ok(())
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
}
