// SPDX-License-Identifier: AGPL-3.0-only
//! Read-only metadata and head-row inspection for Parquet and Arrow IPC / Feather files.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnarSchemaField {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnarCellKind {
    String,
    Number,
    Boolean,
    Null,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnarCell {
    pub text: String,
    pub kind: ColumnarCellKind,
}

#[derive(Clone, Debug)]
pub struct ColumnarPreviewData {
    pub format_label: &'static str,
    pub total_rows: usize,
    pub row_groups: Option<usize>,
    pub created_by: Option<String>,
    pub schema: Vec<ColumnarSchemaField>,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<ColumnarCell>>,
    pub truncated: bool,
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn inspect_parquet(bytes: &[u8], max_rows: usize) -> Result<ColumnarPreviewData, String> {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    use std::io::{Seek, SeekFrom, Write};

    let mut tmp = tempfile::tempfile().map_err(|e| format!("Parquet temp file error: {e}"))?;
    tmp.write_all(bytes)
        .map_err(|e| format!("Parquet write error: {e}"))?;
    tmp.seek(SeekFrom::Start(0))
        .map_err(|e| format!("Parquet seek error: {e}"))?;

    let reader = SerializedFileReader::new(tmp).map_err(|e| format!("Invalid Parquet: {e}"))?;
    let meta = reader.metadata();
    let file_meta = meta.file_metadata();
    let total_rows = usize::try_from(file_meta.num_rows()).unwrap_or(0);
    let row_groups = Some(meta.num_row_groups());
    let created_by = file_meta.created_by().map(str::to_string);

    let schema_descr = file_meta.schema_descr();
    let mut schema = Vec::new();
    let mut headers = Vec::new();
    for col in schema_descr.columns() {
        let name = col.path().string();
        let nullable = col.self_type().get_basic_info().repetition()
            != parquet::basic::Repetition::REQUIRED;
        let data_type = if let Some(lt) = col.logical_type_ref() {
            format!("{:?} ({:?})", col.physical_type(), lt)
        } else {
            format!("{:?}", col.physical_type())
        };
        headers.push(name.clone());
        schema.push(ColumnarSchemaField {
            name,
            data_type,
            nullable,
        });
    }

    let mut rows = Vec::new();
    let mut truncated = false;
    let iter = reader
        .get_row_iter(None)
        .map_err(|e| format!("Parquet row reader error: {e}"))?;
    for row_res in iter {
        if rows.len() >= max_rows {
            truncated = true;
            break;
        }
        let row = row_res.map_err(|e| format!("Parquet row decode error: {e}"))?;
        let mut cells = Vec::with_capacity(row.len());
        for (_, field) in row.get_column_iter() {
            cells.push(format_parquet_field(field));
        }
        rows.push(cells);
    }
    if total_rows > rows.len() {
        truncated = true;
    }

    Ok(ColumnarPreviewData {
        format_label: "Apache Parquet",
        total_rows,
        row_groups,
        created_by,
        schema,
        headers,
        rows,
        truncated,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn inspect_parquet(_bytes: &[u8], _max_rows: usize) -> Result<ColumnarPreviewData, String> {
    Err("Parquet preview is not supported on this target".into())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn format_parquet_field(field: &parquet::record::Field) -> ColumnarCell {
    use parquet::record::Field as PqField;
    match field {
        PqField::Null => ColumnarCell {
            text: "null".into(),
            kind: ColumnarCellKind::Null,
        },
        PqField::Bool(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Boolean,
        },
        PqField::Byte(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Number,
        },
        PqField::Short(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Number,
        },
        PqField::Int(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Number,
        },
        PqField::Long(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Number,
        },
        PqField::UByte(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Number,
        },
        PqField::UShort(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Number,
        },
        PqField::UInt(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Number,
        },
        PqField::ULong(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Number,
        },
        PqField::Float16(v) => ColumnarCell {
            text: format!("{}", v.to_f64()),
            kind: ColumnarCellKind::Number,
        },
        PqField::Float(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Number,
        },
        PqField::Double(v) => ColumnarCell {
            text: v.to_string(),
            kind: ColumnarCellKind::Number,
        },
        PqField::Decimal(d) => ColumnarCell {
            text: format!("{d:?}"),
            kind: ColumnarCellKind::Number,
        },
        PqField::Str(s) => ColumnarCell {
            text: s.clone(),
            kind: ColumnarCellKind::String,
        },
        PqField::Bytes(b) => ColumnarCell {
            text: format!("<{} bytes>", b.len()),
            kind: ColumnarCellKind::String,
        },
        PqField::Date(v) => ColumnarCell {
            text: format!("date({v})"),
            kind: ColumnarCellKind::String,
        },
        PqField::TimestampMillis(v) => ColumnarCell {
            text: format!("{v} ms"),
            kind: ColumnarCellKind::Number,
        },
        PqField::TimestampMicros(v) => ColumnarCell {
            text: format!("{v} us"),
            kind: ColumnarCellKind::Number,
        },
        other => ColumnarCell {
            text: format!("{other}"),
            kind: ColumnarCellKind::String,
        },
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn inspect_arrow_ipc(bytes: &[u8], max_rows: usize) -> Result<ColumnarPreviewData, String> {
    use arrow_ipc::reader::{FileReader, StreamReader};
    use std::io::Cursor;

    let (arrow_schema, batches) = if let Ok(reader) = FileReader::try_new(Cursor::new(bytes), None)
    {
        let schema = reader.schema();
        let mut out = Vec::new();
        for batch in reader {
            out.push(batch.map_err(|e| format!("Arrow IPC batch error: {e}"))?);
        }
        (schema, out)
    } else if let Ok(reader) = StreamReader::try_new(Cursor::new(bytes), None) {
        let schema = reader.schema();
        let mut out = Vec::new();
        for batch in reader {
            out.push(batch.map_err(|e| format!("Arrow stream batch error: {e}"))?);
        }
        (schema, out)
    } else {
        return Err("Invalid Arrow IPC / Feather file".into());
    };

    let schema: Vec<ColumnarSchemaField> = arrow_schema
        .fields()
        .iter()
        .map(|f| ColumnarSchemaField {
            name: f.name().clone(),
            data_type: format!("{}", f.data_type()),
            nullable: f.is_nullable(),
        })
        .collect();
    let headers: Vec<String> = schema.iter().map(|f| f.name.clone()).collect();

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    let mut rows = Vec::new();
    let mut truncated = false;

    'outer: for batch in &batches {
        for r in 0..batch.num_rows() {
            if rows.len() >= max_rows {
                truncated = true;
                break 'outer;
            }
            let mut row_cells = Vec::with_capacity(batch.num_columns());
            for c in 0..batch.num_columns() {
                let col = batch.column(c);
                row_cells.push(format_arrow_cell(col.as_ref(), r));
            }
            rows.push(row_cells);
        }
    }
    if total_rows > rows.len() {
        truncated = true;
    }

    Ok(ColumnarPreviewData {
        format_label: "Apache Arrow IPC",
        total_rows,
        row_groups: Some(batches.len()),
        created_by: None,
        schema,
        headers,
        rows,
        truncated,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn inspect_arrow_ipc(_bytes: &[u8], _max_rows: usize) -> Result<ColumnarPreviewData, String> {
    Err("Arrow IPC preview is not supported on this target".into())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn format_arrow_cell(array: &dyn arrow_array::Array, row: usize) -> ColumnarCell {
    use arrow_array::cast::AsArray;
    use arrow_array::types::*;
    use arrow_schema::DataType;

    if array.is_null(row) {
        return ColumnarCell {
            text: "null".into(),
            kind: ColumnarCellKind::Null,
        };
    }

    match array.data_type() {
        DataType::Boolean => ColumnarCell {
            text: array.as_boolean().value(row).to_string(),
            kind: ColumnarCellKind::Boolean,
        },
        DataType::Int8 => ColumnarCell {
            text: array.as_primitive::<Int8Type>().value(row).to_string(),
            kind: ColumnarCellKind::Number,
        },
        DataType::Int16 => ColumnarCell {
            text: array.as_primitive::<Int16Type>().value(row).to_string(),
            kind: ColumnarCellKind::Number,
        },
        DataType::Int32 => ColumnarCell {
            text: array.as_primitive::<Int32Type>().value(row).to_string(),
            kind: ColumnarCellKind::Number,
        },
        DataType::Int64 => ColumnarCell {
            text: array.as_primitive::<Int64Type>().value(row).to_string(),
            kind: ColumnarCellKind::Number,
        },
        DataType::UInt8 => ColumnarCell {
            text: array.as_primitive::<UInt8Type>().value(row).to_string(),
            kind: ColumnarCellKind::Number,
        },
        DataType::UInt16 => ColumnarCell {
            text: array.as_primitive::<UInt16Type>().value(row).to_string(),
            kind: ColumnarCellKind::Number,
        },
        DataType::UInt32 => ColumnarCell {
            text: array.as_primitive::<UInt32Type>().value(row).to_string(),
            kind: ColumnarCellKind::Number,
        },
        DataType::UInt64 => ColumnarCell {
            text: array.as_primitive::<UInt64Type>().value(row).to_string(),
            kind: ColumnarCellKind::Number,
        },
        DataType::Float32 => ColumnarCell {
            text: array.as_primitive::<Float32Type>().value(row).to_string(),
            kind: ColumnarCellKind::Number,
        },
        DataType::Float64 => ColumnarCell {
            text: array.as_primitive::<Float64Type>().value(row).to_string(),
            kind: ColumnarCellKind::Number,
        },
        DataType::Utf8 => ColumnarCell {
            text: array.as_string::<i32>().value(row).to_string(),
            kind: ColumnarCellKind::String,
        },
        DataType::LargeUtf8 => ColumnarCell {
            text: array.as_string::<i64>().value(row).to_string(),
            kind: ColumnarCellKind::String,
        },
        dt => ColumnarCell {
            text: format!("<{dt}>"),
            kind: ColumnarCellKind::String,
        },
    }
}

#[cfg(all(
    test,
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
mod tests {
    use super::*;

    #[test]
    fn inspects_parquet_and_arrow_ipc_roundtrip() {
        use arrow_array::{Int64Array, RecordBatch, StringArray};
        use arrow_schema::{DataType, Field, Schema};
        use std::sync::Arc;

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec![Some("alpha"), None, Some("gamma")])),
            ],
        )
        .unwrap();

        let mut arrow_buf = Vec::new();
        {
            let mut writer =
                arrow_ipc::writer::FileWriter::try_new(&mut arrow_buf, &schema).unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();
        }
        let arrow_prev = inspect_arrow_ipc(&arrow_buf, 50).unwrap();
        assert_eq!(arrow_prev.total_rows, 3);
        assert_eq!(arrow_prev.schema.len(), 2);
        assert_eq!(arrow_prev.rows[0][0].text, "1");
        assert_eq!(arrow_prev.rows[1][1].kind, ColumnarCellKind::Null);

        use parquet::data_type::{ByteArray, ByteArrayType, Int64Type};
        use parquet::file::{properties::WriterProperties, writer::SerializedFileWriter};
        use parquet::schema::parser::parse_message_type;

        let pq_schema = Arc::new(
            parse_message_type(
                "message row { REQUIRED INT64 id; OPTIONAL BYTE_ARRAY label (UTF8); }",
            )
            .unwrap(),
        );
        let props = Arc::new(WriterProperties::builder().build());
        let mut pq_buf = Vec::new();
        {
            let mut writer = SerializedFileWriter::new(&mut pq_buf, pq_schema, props).unwrap();
            let mut rg = writer.next_row_group().unwrap();
            {
                let mut col = rg.next_column().unwrap().unwrap();
                col.typed::<Int64Type>().write_batch(&[10, 20], None, None).unwrap();
                col.close().unwrap();
            }
            {
                let mut col = rg.next_column().unwrap().unwrap();
                col.typed::<ByteArrayType>()
                    .write_batch(
                        &[ByteArray::from("ok"), ByteArray::from("done")],
                        Some(&[1, 1]),
                        None,
                    )
                    .unwrap();
                col.close().unwrap();
            }
            rg.close().unwrap();
            writer.close().unwrap();
        }
        let pq_prev = inspect_parquet(&pq_buf, 50).unwrap();
        assert_eq!(pq_prev.total_rows, 2);
        assert_eq!(pq_prev.schema.len(), 2);
        assert_eq!(pq_prev.rows[0][0].text, "10");
        assert_eq!(pq_prev.rows[1][1].text, "done");
    }
}
