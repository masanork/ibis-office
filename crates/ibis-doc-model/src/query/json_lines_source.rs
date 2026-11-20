// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType};
use crate::CellValue;
use serde_json::{Value, value::RawValue};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_COLUMNS: usize = 16_384;
const MAX_DATA_ROWS: usize = 1_048_575;
const MAX_CELLS: usize = 5_000_000;
const MAX_TEXT_UNITS: usize = 32_767;

/// Read one JSON object per line. Absent fields and nulls become empty cells.
pub fn read_json_lines(path: &Path) -> Result<QTable, String> {
    if !path.is_file() {
        return Err("Select a local JSON Lines file".into());
    }
    let file = File::open(path).map_err(|error| error.to_string())?;
    read_records(BufReader::new(file)).map_err(|error| {
        format!(
            "{}: {error}",
            path.file_name().unwrap_or_default().to_string_lossy()
        )
    })
}

fn read_records(mut reader: impl BufRead) -> Result<QTable, String> {
    let mut table = QTable::default();
    let mut indices = HashMap::<String, usize>::new();
    let mut types = Vec::<Option<QType>>::new();
    let mut line = Vec::new();
    let mut bytes = 0;
    for line_number in 1.. {
        line.clear();
        let read = reader
            .by_ref()
            .take((MAX_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("Line {line_number}: {error}"))?;
        if read == 0 {
            break;
        }
        bytes += read;
        if read > MAX_LINE_BYTES || bytes > MAX_SOURCE_BYTES {
            return Err(format!(
                "Line {line_number}: JSON Lines source exceeds the byte limit"
            ));
        }
        let fields = serde_json::from_slice::<BTreeMap<String, &RawValue>>(&line)
            .map_err(|error| format!("Line {line_number}: expected one JSON object ({error})"))?;
        let width = indices.len()
            + fields
                .keys()
                .filter(|key| !indices.contains_key(*key))
                .count();
        let height = table.rows.len() + 1;
        if width > MAX_COLUMNS || height > MAX_DATA_ROWS || width.saturating_mul(height) > MAX_CELLS
        {
            return Err(format!(
                "Line {line_number}: JSON Lines source exceeds the table size limit"
            ));
        }
        let mut row = vec![CellValue::Empty; width];
        for (name, value) in fields {
            if exceeds_text_limit(&name) {
                return Err(format!(
                    "Line {line_number}: field name exceeds the spreadsheet cell limit"
                ));
            }
            let index = *indices.entry(name.clone()).or_insert_with(|| {
                let index = table.columns.len();
                table.columns.push(QColumn {
                    name: name.clone(),
                    ty: QType::Any,
                });
                types.push(None);
                index
            });
            let (cell, ty) =
                cell(value).map_err(|error| format!("Line {line_number}, {name}: {error}"))?;
            if let Some(ty) = ty {
                types[index] = Some(match (types[index], ty) {
                    (None, ty) => ty,
                    (Some(previous), ty) if previous == ty => previous,
                    (Some(QType::Whole), QType::Decimal) | (Some(QType::Decimal), QType::Whole) => {
                        QType::Decimal
                    }
                    _ => QType::Any,
                });
            }
            row[index] = cell;
        }
        table.rows.push(row);
    }
    if table.columns.is_empty() {
        return Err("JSON Lines source contains no fields".into());
    }
    for (column, ty) in table.columns.iter_mut().zip(types) {
        column.ty = ty.unwrap_or(QType::Any);
    }
    for row in &mut table.rows {
        row.resize(table.columns.len(), CellValue::Empty);
    }
    Ok(table)
}

fn cell(raw: &RawValue) -> Result<(CellValue, Option<QType>), String> {
    let text = raw.get();
    if text.starts_with(['[', '{']) {
        if exceeds_text_limit(text) {
            return Err("Text exceeds the spreadsheet cell limit".into());
        }
        return Ok((CellValue::Text(text.into()), Some(QType::Text)));
    }
    let value = serde_json::from_str::<Value>(text).map_err(|error| error.to_string())?;
    let (cell, ty) = match value {
        Value::Null => return Ok((CellValue::Empty, None)),
        Value::Bool(value) => (CellValue::Bool(value), QType::Logical),
        Value::String(value) => (CellValue::Text(value), QType::Text),
        Value::Number(value) => {
            let number = value
                .as_f64()
                .ok_or("Number exceeds spreadsheet precision")?;
            if number.abs() >= (1u64 << 53) as f64 && number.fract() == 0. {
                return Err("Number exceeds spreadsheet precision".into());
            }
            let ty = if value.is_i64() || value.is_u64() {
                QType::Whole
            } else {
                QType::Decimal
            };
            (CellValue::Number(number), ty)
        }
        Value::Array(_) | Value::Object(_) => return Err("Expected a scalar JSON value".into()),
    };
    if let CellValue::Text(text) = &cell {
        if exceeds_text_limit(text) {
            return Err("Text exceeds the spreadsheet cell limit".into());
        }
    }
    Ok((cell, Some(ty)))
}

fn exceeds_text_limit(text: &str) -> bool {
    text.encode_utf16().take(MAX_TEXT_UNITS + 1).count() > MAX_TEXT_UNITS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_sparse_records_with_new_fields_and_mixed_types() {
        let table = read_records(&b"{\"amount\":2,\"code\":\"001\",\"tags\":[true]}\r\n{\"amount\":2.5,\"extra\":false,\"tags\":null}\n{\"code\":9,\"tags\":{\"a\":18446744073709551617}}"[..]).unwrap();
        assert_eq!(table.names(), ["amount", "code", "tags", "extra"]);
        assert_eq!(
            table.columns.iter().map(|col| col.ty).collect::<Vec<_>>(),
            [QType::Decimal, QType::Any, QType::Text, QType::Logical]
        );
        assert_eq!(
            table.rows[0],
            [
                CellValue::Number(2.),
                CellValue::Text("001".into()),
                CellValue::Text("[true]".into()),
                CellValue::Empty
            ]
        );
        assert_eq!(
            table.rows[1],
            [
                CellValue::Number(2.5),
                CellValue::Empty,
                CellValue::Empty,
                CellValue::Bool(false)
            ]
        );
        assert_eq!(
            table.rows[2][2],
            CellValue::Text("{\"a\":18446744073709551617}".into())
        );
    }

    #[test]
    fn rejects_invalid_records_and_precision_loss_with_line_numbers() {
        for record in [
            "[]",
            "{bad}",
            "",
            "{\"n\":9007199254740993}",
            "{\"n\":18446744073709551617}",
        ] {
            let data = format!("{{\"n\":1}}\n{record}\n");
            assert!(
                read_records(data.as_bytes())
                    .unwrap_err()
                    .starts_with("Line 2")
            );
        }
        assert!(
            read_records(vec![b' '; MAX_LINE_BYTES + 1].as_slice())
                .unwrap_err()
                .contains("byte limit")
        );
        assert!(read_records(&b"{}"[..]).is_err());
    }
}
