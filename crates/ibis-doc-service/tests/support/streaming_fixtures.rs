// SPDX-License-Identifier: AGPL-3.0-only

use apache_avro::{Schema as AvroSchema, Writer, types::Value as AvroValue};
use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_ipc::writer::StreamWriter;
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

pub fn write_sources(dir: &Path, rows: usize) -> [PathBuf; 3] {
    let arrow = dir.join("facts.ipc");
    let jsonl = dir.join("facts.jsonl");
    let avro = dir.join("facts.avro");
    let amounts = (0..rows).map(|i| (i % 7) as i64);
    let batch = RecordBatch::try_from_iter([
        (
            "code",
            Arc::new(StringArray::from_iter_values(std::iter::repeat_n(
                "001", rows,
            ))) as _,
        ),
        (
            "amount",
            Arc::new(Int64Array::from_iter_values(amounts.clone())) as _,
        ),
    ])
    .unwrap();
    let mut writer = StreamWriter::try_new(File::create(&arrow).unwrap(), &batch.schema()).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let schema = AvroSchema::parse_str(r#"{"type":"record","name":"Fact","fields":[{"name":"code","type":"string"},{"name":"amount","type":"long"}]}"#).unwrap();
    let mut avro_writer = Writer::new(&schema, File::create(&avro).unwrap()).unwrap();
    let mut json_writer = BufWriter::new(File::create(&jsonl).unwrap());
    for amount in amounts {
        writeln!(json_writer, r#"{{"code":"001","amount":{amount}}}"#).unwrap();
        avro_writer
            .append_value(AvroValue::Record(vec![
                ("code".into(), AvroValue::String("001".into())),
                ("amount".into(), AvroValue::Long(amount)),
            ]))
            .unwrap();
    }
    json_writer.flush().unwrap();
    avro_writer.flush().unwrap();
    [arrow, jsonl, avro]
}
