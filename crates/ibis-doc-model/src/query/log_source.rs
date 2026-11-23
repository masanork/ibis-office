// SPDX-License-Identifier: AGPL-3.0-only

use super::{QColumn, QType};
use crate::CellValue;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

const MAX_LINE_BYTES: usize = 1024 * 1024;

pub(super) fn visit_log(
    path: &Path,
    schema_only: bool,
    mut visit: impl FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
) -> Result<(), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    let columns = [
        QColumn {
            name: "line_number".into(),
            ty: QType::Whole,
        },
        QColumn {
            name: "message".into(),
            ty: QType::Text,
        },
    ];
    visit(&columns, None)?;
    if schema_only {
        return Ok(());
    }
    let mut reader = BufReader::new((&file).take(metadata.len()));
    let mut bytes = Vec::new();
    for number in 1.. {
        bytes.clear();
        let count = reader
            .by_ref()
            .take((MAX_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if count > MAX_LINE_BYTES {
            return Err(format!("Log line {number} exceeds 1 MiB"));
        }
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
        }
        let message = std::str::from_utf8(&bytes)
            .map_err(|_| format!("Log line {number} is not valid UTF-8"))?;
        visit(
            &columns,
            Some(vec![
                CellValue::Number(number as f64),
                CellValue::Text(message.into()),
            ]),
        )?;
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    if metadata.len() != after.len() || metadata.modified().ok() != after.modified().ok() {
        return Err("Log file changed during import".into());
    }
    Ok(())
}
