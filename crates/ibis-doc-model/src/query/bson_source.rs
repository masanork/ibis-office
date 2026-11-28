// SPDX-License-Identifier: AGPL-3.0-only
//! Pure-Rust MongoDB BSON (`.bson`) dump reader, schema inspector, and Turso SQLite exporter.

use super::{QColumn, QType, sqlite_source::dataset_sql_type};
use crate::CellValue;
use serde_json::{Map, Value as JsonValue, json};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const MAX_COLUMNS: usize = 512;
const MAX_DOCS: usize = 1_048_575;
const MAX_DOC_BYTES: usize = 16 * 1024 * 1024;

pub fn is_bson_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bson"))
}

fn read_cstring<'a>(buf: &'a [u8], pos: &mut usize) -> Result<&'a str, String> {
    let rel = buf[*pos..]
        .iter()
        .position(|&b| b == 0)
        .ok_or("Unterminated BSON key")?;
    let s = std::str::from_utf8(&buf[*pos..*pos + rel])
        .map_err(|_| "BSON key is not valid UTF-8".to_owned())?;
    *pos += rel + 1;
    Ok(s)
}

fn read_u32_le(buf: &[u8], pos: &mut usize) -> Result<u32, String> {
    if *pos + 4 > buf.len() {
        return Err("Unexpected EOF in BSON integer".into());
    }
    let v = u32::from_le_bytes([buf[*pos], buf[*pos + 1], buf[*pos + 2], buf[*pos + 3]]);
    *pos += 4;
    Ok(v)
}

fn read_i32_le(buf: &[u8], pos: &mut usize) -> Result<i32, String> {
    Ok(read_u32_le(buf, pos)? as i32)
}

fn read_i64_le(buf: &[u8], pos: &mut usize) -> Result<i64, String> {
    if *pos + 8 > buf.len() {
        return Err("Unexpected EOF in BSON int64".into());
    }
    let v = i64::from_le_bytes(buf[*pos..*pos + 8].try_into().unwrap());
    *pos += 8;
    Ok(v)
}

fn read_f64_le(buf: &[u8], pos: &mut usize) -> Result<f64, String> {
    if *pos + 8 > buf.len() {
        return Err("Unexpected EOF in BSON double".into());
    }
    let v = f64::from_le_bytes(buf[*pos..*pos + 8].try_into().unwrap());
    *pos += 8;
    Ok(v)
}

fn parse_bson_value(vtype: u8, buf: &[u8], pos: &mut usize) -> Result<JsonValue, String> {
    match vtype {
        0x01 => {
            let f = read_f64_le(buf, pos)?;
            Ok(if f.is_finite() { json!(f) } else { JsonValue::Null })
        }
        0x02 | 0x0D | 0x0E => {
            let len = read_i32_le(buf, pos)?;
            if len < 1 || *pos + len as usize > buf.len() {
                return Err("Invalid BSON string length".into());
            }
            let slice = &buf[*pos..*pos + len as usize - 1];
            *pos += len as usize;
            let s = std::str::from_utf8(slice).map_err(|_| "Invalid UTF-8 in BSON string")?;
            Ok(json!(s))
        }
        0x03 => {
            let doc_len = read_i32_le(buf, pos)? as usize;
            if doc_len < 5 || *pos - 4 + doc_len > buf.len() {
                return Err("Invalid embedded BSON document length".into());
            }
            let doc_bytes = &buf[*pos - 4..*pos - 4 + doc_len];
            *pos = *pos - 4 + doc_len;
            Ok(JsonValue::Object(parse_bson_doc(doc_bytes)?))
        }
        0x04 => {
            let doc_len = read_i32_le(buf, pos)? as usize;
            if doc_len < 5 || *pos - 4 + doc_len > buf.len() {
                return Err("Invalid BSON array length".into());
            }
            let doc_bytes = &buf[*pos - 4..*pos - 4 + doc_len];
            *pos = *pos - 4 + doc_len;
            let obj = parse_bson_doc(doc_bytes)?;
            Ok(JsonValue::Array(obj.into_values().collect()))
        }
        0x05 => {
            let len = read_i32_le(buf, pos)?;
            if len < 0 || *pos + 1 + len as usize > buf.len() {
                return Err("Invalid BSON binary length".into());
            }
            *pos += 1 + len as usize;
            Ok(json!(format!("[{len} bytes]")))
        }
        0x06 | 0x0A | 0x7F | 0xFF => Ok(JsonValue::Null),
        0x07 => {
            if *pos + 12 > buf.len() {
                return Err("Truncated BSON ObjectId".into());
            }
            let mut hex = String::with_capacity(24);
            for b in &buf[*pos..*pos + 12] {
                hex.push_str(&format!("{b:02x}"));
            }
            *pos += 12;
            Ok(json!(hex))
        }
        0x08 => {
            if *pos >= buf.len() {
                return Err("Truncated BSON boolean".into());
            }
            let b = buf[*pos] != 0;
            *pos += 1;
            Ok(json!(b))
        }
        0x09 | 0x11 => {
            let v = read_i64_le(buf, pos)?;
            Ok(json!(v))
        }
        0x0B => {
            let pat = read_cstring(buf, pos)?;
            let opts = read_cstring(buf, pos)?;
            Ok(json!(format!("/{pat}/{opts}")))
        }
        0x10 => {
            let v = read_i32_le(buf, pos)?;
            Ok(json!(v))
        }
        0x12 => {
            let v = read_i64_le(buf, pos)?;
            Ok(json!(v))
        }
        0x13 => {
            if *pos + 16 > buf.len() {
                return Err("Truncated BSON decimal128".into());
            }
            *pos += 16;
            Ok(json!("[decimal128]"))
        }
        other => Err(format!("Unsupported BSON value type 0x{other:02x}")),
    }
}

fn parse_bson_doc(doc_bytes: &[u8]) -> Result<Map<String, JsonValue>, String> {
    if doc_bytes.len() < 5 {
        return Err("BSON document too short".into());
    }
    let mut pos = 4usize;
    let mut map = Map::new();
    while pos < doc_bytes.len() - 1 {
        let vtype = doc_bytes[pos];
        pos += 1;
        if vtype == 0 {
            break;
        }
        let key = read_cstring(doc_bytes, &mut pos)?;
        let val = parse_bson_value(vtype, doc_bytes, &mut pos)?;
        map.insert(key.to_owned(), val);
    }
    if doc_bytes[doc_bytes.len() - 1] != 0 {
        return Err("BSON document missing trailing NUL".into());
    }
    Ok(map)
}

fn read_bson_docs(path: &Path) -> Result<Vec<Map<String, JsonValue>>, String> {
    let mut file = File::open(path).map_err(|e| format!("Failed to open BSON file: {e}"))?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).map_err(|e| e.to_string())?;
    let mut docs = Vec::new();
    let mut offset = 0usize;
    while offset < data.len() {
        if offset + 4 > data.len() {
            return Err("Truncated BSON document header".into());
        }
        let len = i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        if len < 5 || len > MAX_DOC_BYTES || offset + len > data.len() {
            return Err("Invalid BSON document length".into());
        }
        docs.push(parse_bson_doc(&data[offset..offset + len])?);
        if docs.len() > MAX_DOCS {
            return Err(format!("BSON file exceeds {MAX_DOCS} documents"));
        }
        offset += len;
    }
    Ok(docs)
}

fn infer_qtype(cur: Option<QType>, val: &JsonValue) -> Option<QType> {
    let next = match val {
        JsonValue::Null => return cur,
        JsonValue::Bool(_) => QType::Logical,
        JsonValue::Number(n) => {
            if n.is_i64() || n.is_u64() {
                QType::Whole
            } else {
                QType::Decimal
            }
        }
        JsonValue::String(_) | JsonValue::Array(_) | JsonValue::Object(_) => QType::Text,
    };
    match (cur, next) {
        (None, t) => Some(t),
        (Some(a), b) if a == b => Some(a),
        (Some(QType::Whole), QType::Decimal) | (Some(QType::Decimal), QType::Whole) => {
            Some(QType::Decimal)
        }
        _ => Some(QType::Text),
    }
}

fn json_to_cell(val: &JsonValue) -> CellValue {
    match val {
        JsonValue::Null => CellValue::Empty,
        JsonValue::Bool(b) => CellValue::Bool(*b),
        JsonValue::Number(n) => n
            .as_f64()
            .filter(|f| f.is_finite())
            .map(CellValue::Number)
            .unwrap_or_else(|| CellValue::Text(n.to_string())),
        JsonValue::String(s) => CellValue::Text(s.clone()),
        other => CellValue::Text(other.to_string()),
    }
}

pub fn visit_bson<F>(
    path: &Path,
    selected: &[String],
    schema_only: bool,
    mut visit: F,
) -> Result<(), String>
where
    F: FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
{
    let docs = read_bson_docs(path)?;
    if docs.is_empty() {
        return Err("BSON file contains no documents".into());
    }

    let mut inferred: BTreeMap<String, Option<QType>> = BTreeMap::new();
    for doc in &docs {
        for (k, v) in doc {
            if !inferred.contains_key(k) && inferred.len() >= MAX_COLUMNS {
                return Err(format!("BSON file exceeds {MAX_COLUMNS} top-level fields"));
            }
            let entry = inferred.entry(k.clone()).or_insert(None);
            *entry = infer_qtype(*entry, v);
        }
    }

    let mut columns: Vec<QColumn> = if selected.is_empty() {
        inferred
            .iter()
            .map(|(name, ty)| QColumn {
                name: name.clone(),
                ty: ty.unwrap_or(QType::Text),
            })
            .collect()
    } else {
        selected
            .iter()
            .map(|name| {
                let ty = inferred
                    .get(name)
                    .ok_or_else(|| format!("Column '{name}' does not exist in BSON file"))?;
                Ok(QColumn {
                    name: name.clone(),
                    ty: ty.unwrap_or(QType::Text),
                })
            })
            .collect::<Result<Vec<_>, String>>()?
    };

    // Put `_id` first if present and unselected
    if selected.is_empty()
        && let Some(pos) = columns.iter().position(|c| c.name == "_id")
        && pos > 0
    {
        let id_col = columns.remove(pos);
        columns.insert(0, id_col);
    }

    visit(&columns, None)?;
    if schema_only {
        return Ok(());
    }

    let col_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    for doc in &docs {
        let mut row = Vec::with_capacity(col_names.len());
        for name in &col_names {
            let cell = doc.get(name).map(json_to_cell).unwrap_or(CellValue::Empty);
            row.push(cell);
        }
        visit(&columns, Some(row))?;
    }

    Ok(())
}

/// Inspect schema and document count of a MongoDB BSON (`.bson`) file.
pub fn bson_inspect_schema(path: &Path) -> Result<JsonValue, String> {
    let mut cols_json = Vec::new();
    let docs = read_bson_docs(path)?;
    let doc_count = docs.len();
    let _ = docs;
    visit_bson(path, &[], true, |columns, _| {
        cols_json = columns
            .iter()
            .map(|c| {
                json!({
                    "name": c.name,
                    "sql_type": dataset_sql_type(c.ty),
                })
            })
            .collect();
        Ok(())
    })?;
    Ok(json!({
        "format": "bson",
        "document_count": doc_count,
        "columns": cols_json,
    }))
}

/// Materialize a MongoDB BSON (`.bson`) file into a standalone SQLite file via Turso.
pub fn bson_export_sqlite(
    input_path: &Path,
    output_path: &Path,
    table_name: Option<&str>,
    overwrite: bool,
) -> Result<JsonValue, String> {
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
        .unwrap_or("bson_collection");
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
    let mut pk_index: Option<usize> = None;

    execute("BEGIN")?;
    let res = visit_bson(input_path, &[], false, |columns, row| {
        if let Some(row) = row {
            let stmt = stmt_insert
                .as_mut()
                .ok_or("Missing BSON table schema")?;
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
                .enumerate()
                .map(|(idx, col)| {
                    let ident = super::sqlite_source::sql_identifier(&col.name);
                    if col.name == "_id" && pk_index.is_none() {
                        pk_index = Some(idx);
                        format!("{ident} {} PRIMARY KEY", dataset_sql_type(col.ty))
                    } else {
                        format!("{ident} {}", dataset_sql_type(col.ty))
                    }
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

    let _ = HashMap::<String, ()>::new();
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

    fn make_bson_doc(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (k, t, val) in entries {
            body.push(*t);
            body.extend_from_slice(k.as_bytes());
            body.push(0);
            body.extend_from_slice(val);
        }
        body.push(0);
        let total = (4 + body.len()) as i32;
        let mut out = total.to_le_bytes().to_vec();
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn inspects_queries_and_exports_bson() {
        let dir = tempfile::tempdir().unwrap();
        let bson_path = dir.path().join("users.bson");
        let oid1 = [0x65, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xaa, 0xbb];
        let mut name_val = 6i32.to_le_bytes().to_vec();
        name_val.extend_from_slice(b"Alice\0");
        let doc1 = make_bson_doc(&[
            ("_id", 0x07, &oid1),
            ("name", 0x02, &name_val),
            ("score", 0x10, &95i32.to_le_bytes()),
            ("active", 0x08, &[1]),
        ]);
        std::fs::write(&bson_path, doc1).unwrap();

        let schema = bson_inspect_schema(&bson_path).unwrap();
        assert_eq!(schema["document_count"], 1);
        assert_eq!(schema["columns"][0]["name"], "_id");

        let session = SqliteQuerySession::from_disk_datasets(&[(
            "users".into(),
            DiskDataset::Bson(bson_path.clone(), vec![]),
        )])
        .unwrap();
        let res = session
            .query("SELECT _id, name, score, active FROM users", &[], 10, 64 * 1024)
            .unwrap();
        assert_eq!(
            res.rows,
            vec![vec![
                json!("65102030405060708090aabb"),
                json!("Alice"),
                json!(95),
                json!(1)
            ]]
        );

        let sqlite_path = dir.path().join("users.sqlite");
        let exported = bson_export_sqlite(&bson_path, &sqlite_path, None, false).unwrap();
        assert_eq!(exported["rows"], 1);
    }
}
