// SPDX-License-Identifier: AGPL-3.0-only
//! Pure-Rust dBase / FoxPro (`.dbf`) reader, schema inspector, and Turso SQLite exporter.

use super::{QColumn, QType, sqlite_source::dataset_sql_type};
use crate::CellValue;
use encoding_rs::{Encoding, SHIFT_JIS, UTF_8, WINDOWS_1252};
use serde_json::json;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_COLUMNS: usize = 512;
const MAX_RECORDS: usize = 1_048_575;

pub fn is_dbf_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dbf"))
}

#[derive(Debug, Clone)]
struct DbfField {
    name: String,
    field_type: char,
    length: usize,
    decimals: u8,
    offset: usize,
}

#[derive(Debug, Clone)]
struct DbfHeader {
    version: u8,
    last_update: String,
    num_records: u32,
    header_len: u16,
    record_len: u16,
    ldid: u8,
    encoding: &'static Encoding,
    fields: Vec<DbfField>,
}

fn ldid_to_encoding(ldid: u8) -> &'static Encoding {
    match ldid {
        0x13 | 0x78 => SHIFT_JIS, // Japanese Shift-JIS (CP932)
        0x00 | 0x01 | 0x02 | 0x03 | 0x57 | 0x58 | 0xC8 => WINDOWS_1252,
        _ => UTF_8,
    }
}

fn decode_dbf_bytes(bytes: &[u8], enc: &'static Encoding) -> String {
    let trimmed = match bytes.iter().position(|&b| b == 0) {
        Some(pos) => &bytes[..pos],
        None => bytes,
    };
    if enc == UTF_8 {
        if let Ok(s) = std::str::from_utf8(trimmed) {
            return s.trim().to_owned();
        }
        let (cow, _, _) = SHIFT_JIS.decode(trimmed);
        return cow.trim().to_owned();
    }
    let (cow, _, _) = enc.decode(trimmed);
    cow.trim().to_owned()
}

fn dbf_field_to_qtype(field: &DbfField) -> QType {
    match field.field_type {
        'L' => QType::Logical,
        'I' => QType::Int32,
        'N' | 'F' => {
            if field.decimals == 0 && field.length <= 9 {
                QType::Whole
            } else {
                QType::Decimal
            }
        }
        'B' => QType::Decimal,
        _ => QType::Text,
    }
}

fn read_dbf_header(file: &mut File) -> Result<DbfHeader, String> {
    let mut hdr = [0u8; 32];
    file.read_exact(&mut hdr)
        .map_err(|_| "File is too small to be a dBase (.dbf) file".to_owned())?;

    let version = hdr[0];
    let year = 1900u16 + u16::from(hdr[1]);
    let month = hdr[2];
    let day = hdr[3];
    let last_update = format!("{year:04}-{month:02}-{day:02}");
    let num_records = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
    let header_len = u16::from_le_bytes([hdr[8], hdr[9]]);
    let record_len = u16::from_le_bytes([hdr[10], hdr[11]]);
    let ldid = hdr[29];
    let encoding = ldid_to_encoding(ldid);

    if header_len < 33 || record_len < 2 {
        return Err("Invalid dBase (.dbf) header length or record length".into());
    }
    if num_records as usize > MAX_RECORDS {
        return Err("dBase (.dbf) file exceeds the maximum supported record count".into());
    }

    let mut fields = Vec::new();
    let mut offset = 1usize; // byte 0 of each record is deletion flag
    loop {
        let pos = file.stream_position().map_err(|e| e.to_string())?;
        if pos >= u64::from(header_len) {
            break;
        }
        let mut first = [0u8; 1];
        file.read_exact(&mut first).map_err(|e| e.to_string())?;
        if first[0] == 0x0D {
            break;
        }
        let mut rest = [0u8; 31];
        file.read_exact(&mut rest).map_err(|e| e.to_string())?;
        let mut desc = [0u8; 32];
        desc[0] = first[0];
        desc[1..].copy_from_slice(&rest);

        let name = decode_dbf_bytes(&desc[0..11], encoding);
        let field_type = (desc[11] as char).to_ascii_uppercase();
        let length = usize::from(desc[16]);
        let decimals = desc[17];
        if fields.len() >= MAX_COLUMNS {
            return Err("dBase (.dbf) file exceeds maximum supported columns".into());
        }
        if offset + length > usize::from(record_len) {
            return Err("dBase (.dbf) field descriptor exceeds record length".into());
        }
        fields.push(DbfField {
            name,
            field_type,
            length,
            decimals,
            offset,
        });
        offset += length;
    }

    if fields.is_empty() {
        return Err("dBase (.dbf) file contains no field descriptors".into());
    }

    Ok(DbfHeader {
        version,
        last_update,
        num_records,
        header_len,
        record_len,
        ldid,
        encoding,
        fields,
    })
}

fn parse_dbf_cell(raw: &[u8], field: &DbfField, enc: &'static Encoding) -> CellValue {
    match field.field_type {
        'I' if raw.len() == 4 => {
            let v = i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            CellValue::Number(f64::from(v))
        }
        'B' if raw.len() == 8 => {
            let v = f64::from_le_bytes([
                raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
            ]);
            if v.is_finite() {
                CellValue::Number(v)
            } else {
                CellValue::Empty
            }
        }
        'L' => {
            let b = raw.first().copied().unwrap_or(b' ');
            match b {
                b'T' | b't' | b'Y' | b'y' => CellValue::Bool(true),
                b'F' | b'f' | b'N' | b'n' => CellValue::Bool(false),
                _ => CellValue::Empty,
            }
        }
        'D' => {
            let s = decode_dbf_bytes(raw, enc);
            if s.len() == 8 && s.bytes().all(|c| c.is_ascii_digit()) {
                CellValue::Text(format!("{}-{}-{}", &s[0..4], &s[4..6], &s[6..8]))
            } else if s.is_empty() {
                CellValue::Empty
            } else {
                CellValue::Text(s)
            }
        }
        'N' | 'F' => {
            let s = decode_dbf_bytes(raw, enc);
            if s.is_empty() {
                CellValue::Empty
            } else if let Ok(n) = s.parse::<f64>()
                && n.is_finite()
            {
                CellValue::Number(n)
            } else {
                CellValue::Text(s)
            }
        }
        _ => {
            let s = decode_dbf_bytes(raw, enc);
            if s.is_empty() {
                CellValue::Empty
            } else {
                CellValue::Text(s)
            }
        }
    }
}

pub fn visit_dbf<F>(
    path: &Path,
    selected: &[String],
    schema_only: bool,
    mut visit: F,
) -> Result<(), String>
where
    F: FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
{
    let mut file = File::open(path).map_err(|e| format!("Failed to open dBase file: {e}"))?;
    let header = read_dbf_header(&mut file)?;

    let indices: Vec<usize> = if selected.is_empty() {
        (0..header.fields.len()).collect()
    } else {
        let mut out = Vec::with_capacity(selected.len());
        for col_name in selected {
            let idx = header
                .fields
                .iter()
                .position(|f| f.name == *col_name)
                .ok_or_else(|| format!("Column '{col_name}' does not exist in dBase file"))?;
            out.push(idx);
        }
        out
    };

    let columns: Vec<QColumn> = indices
        .iter()
        .map(|&idx| {
            let f = &header.fields[idx];
            QColumn {
                name: f.name.clone(),
                ty: dbf_field_to_qtype(f),
            }
        })
        .collect();

    visit(&columns, None)?;
    if schema_only {
        return Ok(());
    }

    file.seek(SeekFrom::Start(u64::from(header.header_len)))
        .map_err(|e| e.to_string())?;

    let rec_len = usize::from(header.record_len);
    let mut buf = vec![0u8; rec_len];
    for _ in 0..header.num_records {
        match file.read_exact(&mut buf) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err.to_string()),
        }
        if buf[0] == 0x1A {
            break; // EOF marker
        }
        if buf[0] == b'*' {
            continue; // deleted record
        }
        let mut row = Vec::with_capacity(indices.len());
        for &idx in &indices {
            let f = &header.fields[idx];
            row.push(parse_dbf_cell(
                &buf[f.offset..f.offset + f.length],
                f,
                header.encoding,
            ));
        }
        visit(&columns, Some(row))?;
    }

    Ok(())
}

/// Inspect schema and metadata of a dBase / FoxPro (`.dbf`) file.
pub fn dbf_inspect_schema(path: &Path) -> Result<serde_json::Value, String> {
    let mut file = File::open(path).map_err(|e| format!("Failed to open dBase file: {e}"))?;
    let header = read_dbf_header(&mut file)?;
    let fields: Vec<_> = header
        .fields
        .iter()
        .map(|f| {
            json!({
                "name": f.name,
                "dbf_type": f.field_type.to_string(),
                "sql_type": dataset_sql_type(dbf_field_to_qtype(f)),
                "length": f.length,
                "decimals": f.decimals,
            })
        })
        .collect();
    Ok(json!({
        "format": format!("dBase-0x{:02x}", header.version),
        "last_update": header.last_update,
        "record_count": header.num_records,
        "ldid": format!("0x{:02x}", header.ldid),
        "encoding": header.encoding.name(),
        "columns": fields,
    }))
}

/// Materialize a dBase (`.dbf`) table into a standalone SQLite file via Turso.
pub fn dbf_export_sqlite(
    input_path: &Path,
    output_path: &Path,
    table_name: Option<&str>,
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
        .unwrap_or("dbf_table");
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
    let res = visit_dbf(input_path, &[], false, |columns, row| {
        if let Some(row) = row {
            let stmt = stmt_insert
                .as_mut()
                .ok_or("Missing dBase table schema")?;
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

    fn build_sample_dbf(ldid: u8, name_bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        // 32-byte header: 2 records, header_len = 32 + 3*32 + 1 = 129, record_len = 1 + 16 + 6 + 8 = 31
        out.push(0x03);
        out.extend_from_slice(&[126, 11, 27]); // 2026-11-27
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&129u16.to_le_bytes());
        out.extend_from_slice(&31u16.to_le_bytes());
        out.extend_from_slice(&[0u8; 17]);
        out.push(ldid);
        out.extend_from_slice(&[0u8; 2]);

        let mut push_field = |name: &[u8], ftype: u8, len: u8, dec: u8| {
            let mut f = [0u8; 32];
            f[..name.len().min(11)].copy_from_slice(&name[..name.len().min(11)]);
            f[11] = ftype;
            f[16] = len;
            f[17] = dec;
            out.extend_from_slice(&f);
        };
        push_field(b"NAME", b'C', 16, 0);
        push_field(b"AMOUNT", b'N', 6, 0);
        push_field(b"ORDER_DATE", b'D', 8, 0);
        out.push(0x0D);

        // Active record
        out.push(b' ');
        let mut col1 = [b' '; 16];
        col1[..name_bytes.len()].copy_from_slice(name_bytes);
        out.extend_from_slice(&col1);
        out.extend_from_slice(b"  1500");
        out.extend_from_slice(b"20261127");

        // Deleted record (should be skipped)
        out.push(b'*');
        out.extend_from_slice(&[b' '; 16]);
        out.extend_from_slice(b"  9999");
        out.extend_from_slice(b"20260101");

        out.push(0x1A);
        out
    }

    #[test]
    fn inspects_queries_and_exports_dbf_with_shift_jis() {
        let dir = tempfile::tempdir().unwrap();
        let dbf_path = dir.path().join("orders.dbf");
        let (sjis, _, _) = SHIFT_JIS.encode("東京商店");
        std::fs::write(&dbf_path, build_sample_dbf(0x13, &sjis)).unwrap();

        let schema = dbf_inspect_schema(&dbf_path).unwrap();
        assert_eq!(schema["encoding"], "Shift_JIS");
        assert_eq!(schema["columns"][0]["name"], "NAME");

        let session = SqliteQuerySession::from_disk_datasets(&[(
            "orders".into(),
            DiskDataset::Dbf(dbf_path.clone(), vec![]),
        )])
        .unwrap();
        let res = session
            .query("SELECT NAME, AMOUNT, ORDER_DATE FROM orders", &[], 10, 64 * 1024)
            .unwrap();
        assert_eq!(res.rows, vec![vec![json!("東京商店"), json!(1500), json!("2026-11-27")]]);

        let sqlite_path = dir.path().join("orders.sqlite");
        let exported = dbf_export_sqlite(&dbf_path, &sqlite_path, None, false).unwrap();
        assert_eq!(exported["rows"], 1);
    }
}
