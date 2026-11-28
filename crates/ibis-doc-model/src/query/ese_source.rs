// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType, sqlite_source::dataset_sql_type};
use crate::CellValue;
use ese_core::{
    ColumnDef, EseDatabase, EseValue, coltyp, decode_ese_record,
};
use serde_json::json;
use std::path::Path;

const MAX_TABLES: usize = 1_024;
const MAX_COLUMNS: usize = 256;
const MAX_ROWS: usize = 1_048_575;
const MAX_CELLS: usize = 5_000_000;
const MAX_TEXT_BYTES: usize = 128 * 1024 * 1024;

pub fn is_ese_extension(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        ["edb", "sdb"]
            .iter()
            .any(|name| ext.eq_ignore_ascii_case(name))
    })
}

fn open_ese(path: &Path) -> Result<EseDatabase, String> {
    if !path.is_file() {
        return Err("Select a local ESE / JET Blue (.edb/.sdb) database file".into());
    }
    EseDatabase::open(path).map_err(|error| format!("Failed to open ESE database: {error}"))
}

pub(crate) fn ese_type_to_qtype(ty: u8) -> QType {
    match ty {
        coltyp::BIT => QType::Logical,
        coltyp::UNSIGNED_BYTE | coltyp::SHORT => QType::Int32,
        coltyp::LONG | coltyp::UNSIGNED_SHORT | coltyp::UNSIGNED_LONG => QType::Whole,
        coltyp::IEEE_SINGLE | coltyp::IEEE_DOUBLE => QType::Decimal,
        coltyp::DATE_TIME => QType::DateTime,
        _ => QType::Text,
    }
}

pub(crate) fn ese_value_to_cell(value: EseValue) -> CellValue {
    match value {
        EseValue::Null => CellValue::Empty,
        EseValue::Bool(b) => CellValue::Bool(b),
        EseValue::U8(v) => CellValue::Number(f64::from(v)),
        EseValue::I16(v) => CellValue::Number(f64::from(v)),
        EseValue::U16(v) => CellValue::Number(f64::from(v)),
        EseValue::I32(v) => CellValue::Number(f64::from(v)),
        EseValue::U32(v) => CellValue::Number(f64::from(v)),
        EseValue::I64(v) => {
            if v.unsigned_abs() < (1u64 << 53) {
                CellValue::Number(v as f64)
            } else {
                CellValue::Text(v.to_string())
            }
        }
        EseValue::U64(v) => {
            if v < (1u64 << 53) {
                CellValue::Number(v as f64)
            } else {
                CellValue::Text(v.to_string())
            }
        }
        EseValue::F32(v) => {
            let f = f64::from(v);
            if f.is_finite() {
                CellValue::Number(f)
            } else {
                CellValue::Empty
            }
        }
        EseValue::F64(v) | EseValue::DateTime(v) => {
            if v.is_finite() {
                CellValue::Number(v)
            } else {
                CellValue::Empty
            }
        }
        EseValue::Text(s) => CellValue::Text(s),
        EseValue::Binary(bytes) => CellValue::Text(format!("[{} bytes]", bytes.len())),
        EseValue::Guid(g) => CellValue::Text(format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            u32::from_le_bytes([g[0], g[1], g[2], g[3]]),
            u16::from_le_bytes([g[4], g[5]]),
            u16::from_le_bytes([g[6], g[7]]),
            g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]
        )),
    }
}

fn resolve_projection(
    col_defs: &[ColumnDef],
    table: &str,
    selected: &[String],
) -> Result<(Vec<QColumn>, Vec<usize>), String> {
    if col_defs.is_empty() {
        return Err(format!("ESE table has no columns: {table}"));
    }
    if selected.is_empty() {
        if col_defs.len() > MAX_COLUMNS {
            return Err(format!(
                "ESE table {table} has more than {MAX_COLUMNS} columns"
            ));
        }
        let columns = col_defs
            .iter()
            .map(|c| QColumn {
                name: c.name.clone(),
                ty: ese_type_to_qtype(c.coltyp),
            })
            .collect();
        let indices = (0..col_defs.len()).collect();
        return Ok((columns, indices));
    }
    if selected.len() > MAX_COLUMNS {
        return Err(format!("Too many selected columns (maximum {MAX_COLUMNS})"));
    }
    let mut columns = Vec::with_capacity(selected.len());
    let mut indices = Vec::with_capacity(selected.len());
    for name in selected {
        let Some((idx, col)) = col_defs
            .iter()
            .enumerate()
            .find(|(_, c)| c.name == *name)
        else {
            return Err(format!(
                "Column '{name}' does not exist in ESE table '{table}'"
            ));
        };
        columns.push(QColumn {
            name: col.name.clone(),
            ty: ese_type_to_qtype(col.coltyp),
        });
        indices.push(idx);
    }
    Ok((columns, indices))
}

/// List table names in an ESE / JET Blue (.edb / .sdb) database.
pub fn ese_tables(path: &Path) -> Result<Vec<String>, String> {
    let db = open_ese(path)?;
    let entries = db.catalog_entries().map_err(|e| e.to_string())?;
    let mut names: Vec<String> = entries
        .into_iter()
        .filter(|e| e.object_type == 1 && !e.object_name.starts_with("MSys"))
        .map(|e| e.object_name)
        .collect();
    if names.len() > MAX_TABLES {
        return Err("ESE database has too many tables to list".into());
    }
    names.sort();
    names.dedup();
    Ok(names)
}

/// Return SQL-visible column definitions for one ESE table without reading rows.
pub fn ese_dataset_columns(
    path: &Path,
    table: &str,
    selected: &[String],
) -> Result<Vec<QColumn>, String> {
    let db = open_ese(path)?;
    let col_defs = db.table_columns(table).map_err(|e| e.to_string())?;
    let (columns, _) = resolve_projection(&col_defs, table, selected)?;
    Ok(columns)
}

/// Stream one ESE table's schema and rows into a visitor.
pub(super) fn visit_ese_table(
    path: &Path,
    table: &str,
    selected: &[String],
    schema_only: bool,
    mut visit: impl FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
) -> Result<(), String> {
    let db = open_ese(path)?;
    let col_defs = db.table_columns(table).map_err(|e| e.to_string())?;
    let (columns, indices) = resolve_projection(&col_defs, table, selected)?;
    visit(&columns, None)?;
    if schema_only {
        return Ok(());
    }
    let cursor = db.table_records(table).map_err(|e| e.to_string())?;
    let mut count = 0usize;
    for item in cursor {
        let (_page, _tag, raw) = item.map_err(|e| e.to_string())?;
        let decoded = decode_ese_record(&raw, &col_defs, db.is_extended_format())
            .map_err(|e| e.to_string())?;
        count += 1;
        if count > MAX_ROWS || columns.len().saturating_mul(count) > MAX_CELLS {
            return Err("ESE table exceeds the query editor's row or cell limit".into());
        }
        let row = indices
            .iter()
            .map(|&idx| {
                decoded
                    .get(idx)
                    .map(|(_, val)| ese_value_to_cell(val.clone()))
                    .unwrap_or(CellValue::Empty)
            })
            .collect();
        visit(&columns, Some(row))?;
    }
    Ok(())
}

/// Read one ESE table into memory within explicit cell and text budgets.
pub fn read_ese_table_bounded(
    path: &Path,
    table: &str,
    selected: &[String],
    max_cells: usize,
    max_text: usize,
) -> Result<QTable, String> {
    let mut table_out = QTable::default();
    let mut text_budget = max_text.min(MAX_TEXT_BYTES);
    let mut cell_budget = max_cells.min(MAX_CELLS);
    visit_ese_table(path, table, selected, false, |columns, row| {
        if let Some(row) = row {
            cell_budget = cell_budget
                .checked_sub(columns.len())
                .ok_or("ESE table exceeds the in-memory cell limit")?;
            super::sqlite_source::charge_dataset_text(&row, &mut text_budget)?;
            table_out.rows.push(row);
        } else {
            table_out.columns = columns.to_vec();
        }
        Ok(())
    })?;
    Ok(table_out)
}

/// Inspect header metadata, tables, and column schemas in an ESE / JET Blue database.
pub fn ese_inspect_schema(path: &Path) -> Result<serde_json::Value, String> {
    let db = open_ese(path)?;
    let entries = db.catalog_entries().map_err(|e| e.to_string())?;
    let mut names: Vec<String> = entries
        .into_iter()
        .filter(|e| e.object_type == 1 && !e.object_name.starts_with("MSys"))
        .map(|e| e.object_name)
        .collect();
    if names.len() > MAX_TABLES {
        return Err("ESE database has too many tables to inspect".into());
    }
    names.sort();
    names.dedup();

    let mut tables = Vec::with_capacity(names.len());
    for name in &names {
        let col_defs = db.table_columns(name).map_err(|e| e.to_string())?;
        let columns: Vec<_> = col_defs
            .iter()
            .map(|c| {
                let qtype = ese_type_to_qtype(c.coltyp);
                json!({
                    "name": c.name,
                    "coltyp": c.coltyp,
                    "sql_type": dataset_sql_type(qtype),
                })
            })
            .collect();
        tables.push(json!({
            "name": name,
            "columns": columns,
        }));
    }

    Ok(json!({
        "format": "ese",
        "page_size": db.header.page_size,
        "page_count": db.page_count(),
        "db_state": db.header.db_state,
        "tables": tables,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_ese_types_and_values() {
        assert_eq!(ese_type_to_qtype(coltyp::BIT), QType::Logical);
        assert_eq!(ese_type_to_qtype(coltyp::SHORT), QType::Int32);
        assert_eq!(ese_type_to_qtype(coltyp::LONG), QType::Whole);
        assert_eq!(ese_type_to_qtype(coltyp::IEEE_DOUBLE), QType::Decimal);
        assert_eq!(ese_type_to_qtype(coltyp::DATE_TIME), QType::DateTime);
        assert_eq!(ese_type_to_qtype(coltyp::LONG_TEXT), QType::Text);

        assert_eq!(ese_value_to_cell(EseValue::Null), CellValue::Empty);
        assert_eq!(ese_value_to_cell(EseValue::Bool(true)), CellValue::Bool(true));
        assert_eq!(ese_value_to_cell(EseValue::I32(99)), CellValue::Number(99.0));
        assert_eq!(
            ese_value_to_cell(EseValue::I64(i64::MAX)),
            CellValue::Text(i64::MAX.to_string())
        );
        assert_eq!(
            ese_value_to_cell(EseValue::Binary(vec![0xAA, 0xBB])),
            CellValue::Text("[2 bytes]".into())
        );
    }

    #[test]
    fn rejects_invalid_and_reads_minimal_ese_header() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.edb");
        std::fs::write(&bad, b"not an ese file").unwrap();
        assert!(ese_tables(&bad).is_err());

        let mut buf = vec![0u8; 4096];
        buf[4..8].copy_from_slice(&0x89AB_CDEF_u32.to_le_bytes());
        buf[236..240].copy_from_slice(&4096u32.to_le_bytes());
        let minimal = dir.path().join("minimal.edb");
        std::fs::write(&minimal, &buf).unwrap();
        let db = open_ese(&minimal).unwrap();
        assert_eq!(db.header.page_size, 4096);
    }
}
