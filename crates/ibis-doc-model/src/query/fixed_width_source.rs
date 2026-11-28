// SPDX-License-Identifier: AGPL-3.0-only
//! Pure-Rust fixed-width record reader (`.dat`/`.txt`, Shift-JIS / UTF-8 byte widths) and Turso SQLite exporter.

use super::{QColumn, QType, sqlite_source::dataset_sql_type};
use crate::CellValue;
use encoding_rs::{Encoding, SHIFT_JIS};
use serde_json::json;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

const MAX_COLUMNS: usize = 512;
const MAX_ROWS: usize = 1_048_575;

#[derive(Debug, Clone)]
pub struct FixedWidthField {
    pub name: String,
    pub width: usize,
    pub ty: QType,
}

impl FixedWidthField {
    pub fn parse_type(raw: Option<&str>) -> Result<QType, String> {
        match raw.unwrap_or("text").trim().to_ascii_lowercase().as_str() {
            "text" | "string" => Ok(QType::Text),
            "int" | "integer" | "whole" => Ok(QType::Whole),
            "real" | "float" | "double" | "decimal" | "number" => Ok(QType::Decimal),
            "bool" | "boolean" | "logical" => Ok(QType::Logical),
            "date" | "datetime" => Ok(QType::DateTime),
            other => Err(format!(
                "Unsupported fixed-width field type '{other}'; expected text, integer, decimal, boolean, or datetime"
            )),
        }
    }
}

fn resolve_encoding(name: Option<&str>) -> Result<&'static Encoding, String> {
    let label = name.unwrap_or("utf-8").trim();
    if label.eq_ignore_ascii_case("cp932") || label.eq_ignore_ascii_case("sjis") {
        return Ok(SHIFT_JIS);
    }
    Encoding::for_label(label.as_bytes())
        .ok_or_else(|| format!("Unsupported character encoding '{label}'"))
}

fn decode_slice(bytes: &[u8], enc: &'static Encoding) -> String {
    let (cow, _, _) = enc.decode(bytes);
    cow.trim().to_owned()
}

fn parse_cell(text: String, ty: QType) -> CellValue {
    if text.is_empty() {
        return CellValue::Empty;
    }
    match ty {
        QType::Whole | QType::Int32 => text
            .parse::<i64>()
            .map(|n| CellValue::Number(n as f64))
            .unwrap_or(CellValue::Text(text)),
        QType::Decimal => text
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(CellValue::Number)
            .unwrap_or(CellValue::Text(text)),
        QType::Logical => match text.to_ascii_lowercase().as_str() {
            "1" | "true" | "t" | "yes" | "y" => CellValue::Bool(true),
            "0" | "false" | "f" | "no" | "n" => CellValue::Bool(false),
            _ => CellValue::Text(text),
        },
        _ => CellValue::Text(text),
    }
}

pub fn visit_fixed_width<F>(
    path: &Path,
    fields: &[FixedWidthField],
    encoding_name: Option<&str>,
    skip_header_lines: usize,
    schema_only: bool,
    mut visit: F,
) -> Result<(), String>
where
    F: FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
{
    if fields.is_empty() {
        return Err("Fixed-width schema must define at least one field".into());
    }
    if fields.len() > MAX_COLUMNS {
        return Err(format!("Fixed-width schema exceeds {MAX_COLUMNS} fields"));
    }
    for f in fields {
        if f.name.trim().is_empty() {
            return Err("Fixed-width field name must not be empty".into());
        }
        if f.width == 0 {
            return Err(format!("Fixed-width field '{}' must have width > 0", f.name));
        }
    }

    let enc = resolve_encoding(encoding_name)?;
    let columns: Vec<QColumn> = fields
        .iter()
        .map(|f| QColumn {
            name: f.name.clone(),
            ty: f.ty,
        })
        .collect();

    visit(&columns, None)?;
    if schema_only {
        return Ok(());
    }

    let file = File::open(path).map_err(|e| format!("Failed to open fixed-width file: {e}"))?;
    let mut reader = BufReader::new(file);
    let mut raw_line = Vec::new();
    let mut line_no = 0usize;
    let mut row_count = 0usize;

    loop {
        raw_line.clear();
        let n = reader
            .read_until(b'\n', &mut raw_line)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if line_no < skip_header_lines {
            line_no += 1;
            continue;
        }
        line_no += 1;

        while matches!(raw_line.last(), Some(b'\n' | b'\r')) {
            raw_line.pop();
        }
        if raw_line.is_empty() {
            continue;
        }
        row_count += 1;
        if row_count > MAX_ROWS {
            return Err(format!("Fixed-width file exceeds {MAX_ROWS} rows"));
        }

        let mut offset = 0usize;
        let mut row = Vec::with_capacity(fields.len());
        for f in fields {
            let start = offset.min(raw_line.len());
            let end = offset.saturating_add(f.width).min(raw_line.len());
            let text = decode_slice(&raw_line[start..end], enc);
            row.push(parse_cell(text, f.ty));
            offset = offset.saturating_add(f.width);
        }
        visit(&columns, Some(row))?;
    }

    Ok(())
}

/// Preview rows from a fixed-width file (`.dat`/`.txt`) using byte-width field definitions.
pub fn fixed_width_preview(
    path: &Path,
    fields: &[FixedWidthField],
    encoding_name: Option<&str>,
    skip_header_lines: usize,
    limit: usize,
) -> Result<serde_json::Value, String> {
    let max_rows = limit.clamp(1, 200);
    let mut cols_json = Vec::new();
    let mut rows_json = Vec::new();
    let mut total_rows = 0usize;
    let mut truncated = false;

    visit_fixed_width(path, fields, encoding_name, skip_header_lines, false, |columns, row| {
        if cols_json.is_empty() {
            cols_json = columns
                .iter()
                .map(|c| {
                    json!({
                        "name": c.name,
                        "sql_type": dataset_sql_type(c.ty),
                    })
                })
                .collect();
        }
        if let Some(row) = row {
            total_rows += 1;
            if rows_json.len() < max_rows {
                let vals: Vec<serde_json::Value> = row
                    .into_iter()
                    .map(|cell| match cell {
                        CellValue::Empty => serde_json::Value::Null,
                        CellValue::Bool(b) => json!(b),
                        CellValue::Number(n) => {
                            if n.is_finite() && n.fract() == 0.0 && n.abs() < (1i64 << 53) as f64 {
                                json!(n as i64)
                            } else {
                                json!(n)
                            }
                        }
                        CellValue::Text(s) => json!(s),
                        other => json!(format!("{other:?}")),
                    })
                    .collect();
                rows_json.push(serde_json::Value::Array(vals));
            } else {
                truncated = true;
            }
        }
        Ok(())
    })?;

    Ok(json!({
        "encoding": resolve_encoding(encoding_name)?.name(),
        "columns": cols_json,
        "rows": rows_json,
        "total_rows": total_rows,
        "truncated": truncated,
    }))
}

/// Materialize a fixed-width record file into a standalone SQLite file via Turso.
pub fn fixed_width_export_sqlite(
    input_path: &Path,
    output_path: &Path,
    table_name: Option<&str>,
    fields: &[FixedWidthField],
    encoding_name: Option<&str>,
    skip_header_lines: usize,
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

    let default_name = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("fixed_width_table");
    let target_table = table_name.unwrap_or(default_name);
    let identifier = super::sqlite_source::sql_identifier(target_table);

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

    let mut stmt_insert: Option<turso_core::Statement> = None;
    let mut row_count = 0usize;
    let mut batch_count = 0usize;

    execute("BEGIN")?;
    let res = visit_fixed_width(
        input_path,
        fields,
        encoding_name,
        skip_header_lines,
        false,
        |columns, row| {
            if let Some(row) = row {
                let stmt = stmt_insert
                    .as_mut()
                    .ok_or("Missing fixed-width table schema")?;
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
        },
    );
    if let Err(err) = res {
        let _ = execute("ROLLBACK");
        return Err(err);
    }
    execute("COMMIT")?;
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
        "table": target_table,
        "rows": row_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{DiskDataset, SqliteQuerySession};

    #[test]
    fn previews_queries_and_exports_shift_jis_fixed_width() {
        let dir = tempfile::tempdir().unwrap();
        let dat_path = dir.path().join("zengin.dat");
        // Header line + 2 fixed-width SJIS rows: code(4) + name(10 bytes) + amount(6)
        let mut raw = Vec::new();
        raw.extend_from_slice(b"HEADER_SKIP_ME\r\n");
        let (sjis1, _, _) = SHIFT_JIS.encode("東京商事  "); // 4*2 + 2 = 10 bytes
        raw.extend_from_slice(b"0001");
        raw.extend_from_slice(&sjis1);
        raw.extend_from_slice(b"012500\r\n");
        let (sjis2, _, _) = SHIFT_JIS.encode("大阪工業  ");
        raw.extend_from_slice(b"0002");
        raw.extend_from_slice(&sjis2);
        raw.extend_from_slice(b"003400\n");
        std::fs::write(&dat_path, raw).unwrap();

        let fields = vec![
            FixedWidthField { name: "code".into(), width: 4, ty: QType::Text },
            FixedWidthField { name: "name".into(), width: 10, ty: QType::Text },
            FixedWidthField { name: "amount".into(), width: 6, ty: QType::Whole },
        ];

        let preview = fixed_width_preview(&dat_path, &fields, Some("cp932"), 1, 10).unwrap();
        assert_eq!(preview["encoding"], "Shift_JIS");
        assert_eq!(preview["total_rows"], 2);
        assert_eq!(preview["rows"][0], json!(["0001", "東京商事", 12500]));

        let session = SqliteQuerySession::from_disk_datasets(&[(
            "zengin".into(),
            DiskDataset::FixedWidth(dat_path.clone(), fields.clone(), Some("cp932".into()), 1),
        )])
        .unwrap();
        let res = session
            .query("SELECT code, name, amount FROM zengin WHERE amount > 5000", &[], 10, 64 * 1024)
            .unwrap();
        assert_eq!(res.rows, vec![vec![json!("0001"), json!("東京商事"), json!(12500)]]);

        let sqlite_path = dir.path().join("zengin.sqlite");
        let exported = fixed_width_export_sqlite(&dat_path, &sqlite_path, Some("zengin"), &fields, Some("cp932"), 1, false).unwrap();
        assert_eq!(exported["rows"], 2);
    }
}
