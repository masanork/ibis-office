// SPDX-License-Identifier: AGPL-3.0-only

use super::{QColumn, QType};
use crate::CellValue;
use encoding_rs::{DecoderResult, Encoding, SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE};
use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::Path,
};

const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;
const MAX_COLUMNS: usize = 256;

fn decode(
    input: &mut BufReader<File>,
    encoding: &'static Encoding,
    offset: u64,
    length: u64,
    mut visit: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<bool, String> {
    input
        .seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let mut buffer = [0; 16 * 1024];
    let mut remaining = length.saturating_sub(offset);
    loop {
        let bytes = input.fill_buf().map_err(|e| e.to_string())?;
        let bytes = &bytes[..bytes.len().min(remaining.try_into().unwrap_or(usize::MAX))];
        let end = bytes.is_empty();
        let (status, read, written) =
            decoder.decode_to_utf8_without_replacement(bytes, &mut buffer, end);
        if matches!(status, DecoderResult::Malformed(..)) {
            return Ok(false);
        }
        visit(&buffer[..written])?;
        input.consume(read);
        remaining -= read as u64;
        if end && status == DecoderResult::InputEmpty {
            return Ok(true);
        }
    }
}

#[derive(Default)]
struct Records {
    field: Vec<u8>,
    fields: Vec<String>,
    quoted: bool,
    after_quote: bool,
    after_cr: bool,
    record_bytes: usize,
    finished: bool,
}

impl Records {
    fn finish_field(&mut self) -> Result<(), String> {
        if self.fields.len() == MAX_COLUMNS {
            return Err("CSV needs at most 256 fields per record".into());
        }
        self.fields
            .push(String::from_utf8(std::mem::take(&mut self.field)).map_err(|e| e.to_string())?);
        Ok(())
    }

    fn push(
        &mut self,
        bytes: &[u8],
        header_only: bool,
        mut visit: impl FnMut(Vec<String>) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        for &byte in bytes {
            if std::mem::take(&mut self.after_cr) && byte == b'\n' {
                continue;
            }
            self.record_bytes += 1;
            if self.record_bytes > MAX_RECORD_BYTES {
                return Err("CSV record exceeds the size limit".into());
            }
            if std::mem::take(&mut self.after_quote) {
                if byte == b'"' {
                    self.field.push(byte);
                    continue;
                }
                self.quoted = false;
            }
            if self.quoted {
                if byte == b'"' {
                    self.after_quote = true;
                } else {
                    self.field.push(byte);
                }
                continue;
            }
            match byte {
                b'"' if self.field.is_empty() => self.quoted = true,
                b',' => self.finish_field()?,
                b'\n' | b'\r' => {
                    self.finish_field()?;
                    visit(std::mem::take(&mut self.fields))?;
                    self.record_bytes = 0;
                    if header_only {
                        self.finished = true;
                        return Ok(());
                    }
                    self.after_cr = byte == b'\r';
                }
                _ => self.field.push(byte),
            }
        }
        Ok(())
    }
}

pub(super) fn visit_csv(
    path: &Path,
    schema_only: bool,
    mut visit: impl FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
) -> Result<(), String> {
    let mut input = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    let metadata = input.get_ref().metadata().map_err(|e| e.to_string())?;
    let prefix = input.fill_buf().map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    if prefix.starts_with(b"\xef\xbb\xbf") {
        candidates.push((UTF_8, 3));
    } else {
        if prefix.starts_with(b"\xff\xfe") {
            candidates.push((UTF_16LE, 2));
        } else if prefix.starts_with(b"\xfe\xff") {
            candidates.push((UTF_16BE, 2));
        }
        candidates.extend([(UTF_8, 0), (SHIFT_JIS, 0)]);
    }
    // Prefer valid UTF-8 over Shift_JIS, even when the first non-ASCII byte is late.
    let mut selected = None;
    for (encoding, offset) in candidates {
        if decode(&mut input, encoding, offset, metadata.len(), |_| Ok(()))? {
            selected = Some((encoding, offset));
            break;
        }
    }
    let (encoding, offset) = selected.ok_or("CSV encoding is invalid")?;
    let mut columns = Vec::new();
    let mut record = |fields: Vec<String>| {
        if columns.is_empty() {
            if fields.iter().any(|name| name.trim().is_empty()) {
                return Err("CSV needs 1 to 256 nonblank headers".into());
            }
            columns = fields
                .into_iter()
                .map(|name| QColumn {
                    name,
                    ty: QType::Text,
                })
                .collect();
            visit(&columns, None)
        } else if schema_only {
            Ok(())
        } else if fields.len() != columns.len() {
            Err("CSV rows must match their header width".into())
        } else {
            visit(
                &columns,
                Some(fields.into_iter().map(CellValue::Text).collect()),
            )
        }
    };
    let mut records = Records::default();
    let valid = decode(&mut input, encoding, offset, metadata.len(), |bytes| {
        records.push(bytes, schema_only, &mut record)
    })?;
    if !valid {
        return Err("CSV encoding is invalid".into());
    }
    if !records.field.is_empty() || !records.fields.is_empty() {
        records.finish_field()?;
        record(records.fields)?;
    }
    if columns.is_empty() {
        return Err("CSV must have headers".into());
    }
    let after = input.get_ref().metadata().map_err(|e| e.to_string())?;
    if metadata.len() != after.len() || metadata.modified().ok() != after.modified().ok() {
        return Err("CSV file changed during import".into());
    }
    Ok(())
}
