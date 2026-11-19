// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType};
use crate::CellValue;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use std::{fs::File, path::Path};

fn number(value: f64, ty: QType) -> Result<(CellValue, QType), String> {
    if !value.is_finite() {
        return Err("Non-finite Parquet number".into());
    }
    Ok((CellValue::Number(value), ty))
}

fn integer(value: i128) -> Result<(CellValue, QType), String> {
    if value.unsigned_abs() >= 1 << 53 {
        return Err("Parquet integer exceeds spreadsheet precision".into());
    }
    number(value as f64, QType::Whole)
}

fn cell(field: Field) -> Result<(CellValue, QType), String> {
    match field {
        Field::Null => Ok((CellValue::Empty, QType::Any)),
        Field::Bool(v) => Ok((CellValue::Bool(v), QType::Logical)),
        Field::Str(v) => Ok((CellValue::Text(v), QType::Text)),
        Field::Byte(v) => integer(v.into()),
        Field::Short(v) => integer(v.into()),
        Field::Int(v) => integer(v.into()),
        Field::Long(v) => integer(v.into()),
        Field::UByte(v) => integer(v.into()),
        Field::UShort(v) => integer(v.into()),
        Field::UInt(v) => integer(v.into()),
        Field::ULong(v) => integer(v.into()),
        Field::Float16(v) => number(v.to_f64(), QType::Decimal),
        Field::Float(v) => number(v.into(), QType::Decimal),
        Field::Double(v) => number(v, QType::Decimal),
        Field::Date(v) => number(f64::from(v) + 25_569., QType::Date),
        Field::TimeMillis(v) => number(f64::from(v) / 86_400_000., QType::Time),
        Field::TimeMicros(v) => number(v as f64 / 86_400_000_000., QType::Time),
        Field::TimestampMillis(v) => number(v as f64 / 86_400_000. + 25_569., QType::DateTime),
        Field::TimestampMicros(v) => number(v as f64 / 86_400_000_000. + 25_569., QType::DateTime),
        _ => Err("Unsupported Parquet value type".into()),
    }
}

/// Materialize a user-selected local Parquet file in the query editor.
pub fn read_parquet(path: &Path) -> Result<QTable, String> {
    if !path.is_file() {
        return Err("Select a local Parquet file".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let reader = SerializedFileReader::new(file).map_err(|e| e.to_string())?;
    let meta = reader.metadata().file_metadata();
    let schema = meta.schema_descr();
    if schema
        .columns()
        .iter()
        .any(|c| c.path().parts().len() != 1 || c.max_rep_level() != 0)
    {
        return Err("Nested Parquet columns are unsupported".into());
    }
    let width = schema.num_columns();
    if width > 16_384
        || meta.num_rows() >= 1_048_576
        || (width as u64).saturating_mul(meta.num_rows() as u64) > 5_000_000
    {
        return Err("Parquet source exceeds the query editor's size limit".into());
    }
    let mut table = QTable {
        columns: schema
            .columns()
            .iter()
            .map(|c| QColumn {
                name: c.name().to_string(),
                ty: QType::Any,
            })
            .collect(),
        rows: Vec::new(),
    };
    for row in reader.get_row_iter(None).map_err(|e| e.to_string())? {
        let mut values = Vec::with_capacity(width);
        for ((name, field), col) in row
            .map_err(|e| e.to_string())?
            .into_columns()
            .into_iter()
            .zip(&mut table.columns)
        {
            let (value, ty) = cell(field).map_err(|e| format!("{name}: {e}"))?;
            if col.ty == QType::Any {
                col.ty = ty;
            }
            values.push(value);
        }
        table.rows.push(values);
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_snappy_table() {
        use parquet::basic::Compression;
        use parquet::data_type::{ByteArray, ByteArrayType, Int64Type};
        use parquet::file::{properties::WriterProperties, writer::SerializedFileWriter};
        use parquet::schema::parser::parse_message_type;
        use std::sync::Arc;

        let file =
            std::env::temp_dir().join(format!("ibis-parquet-{}.parquet", std::process::id()));
        let output = File::options()
            .write(true)
            .create_new(true)
            .open(&file)
            .unwrap();
        let schema = Arc::new(
            parse_message_type(
                "message example { REQUIRED BINARY code (UTF8); OPTIONAL INT64 amount; }",
            )
            .unwrap(),
        );
        let properties = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();
        let mut writer = SerializedFileWriter::new(output, schema, Arc::new(properties)).unwrap();
        let mut group = writer.next_row_group().unwrap();
        let mut code = group.next_column().unwrap().unwrap();
        code.typed::<ByteArrayType>()
            .write_batch(
                &[ByteArray::from("01202"), ByteArray::from("00001")],
                None,
                None,
            )
            .unwrap();
        code.close().unwrap();
        let mut amount = group.next_column().unwrap().unwrap();
        amount
            .typed::<Int64Type>()
            .write_batch(&[7], Some(&[1, 0]), None)
            .unwrap();
        amount.close().unwrap();
        group.close().unwrap();
        writer.close().unwrap();

        let result = read_parquet(&file);
        std::fs::remove_file(file).unwrap();
        let table = result.unwrap();
        assert_eq!(table.names(), ["code", "amount"]);
        assert_eq!(
            table.rows,
            [
                vec![CellValue::Text("01202".into()), CellValue::Number(7.)],
                vec![CellValue::Text("00001".into()), CellValue::Empty],
            ]
        );
    }

    #[test]
    fn preserves_codes_and_missing_values() {
        assert_eq!(
            cell(Field::Str("01202".into())).unwrap(),
            (CellValue::Text("01202".into()), QType::Text)
        );
        assert_eq!(cell(Field::Null).unwrap(), (CellValue::Empty, QType::Any));
        assert_eq!(
            cell(Field::Long(1202)).unwrap(),
            (CellValue::Number(1202.), QType::Whole)
        );
        assert_eq!(
            cell(Field::Date(0)).unwrap(),
            (CellValue::Number(25_569.), QType::Date)
        );
        assert!(cell(Field::Long(i64::MAX)).is_err());
    }
}
