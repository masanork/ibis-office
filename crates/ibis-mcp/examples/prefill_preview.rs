// SPDX-License-Identifier: AGPL-3.0-only

use serde_json::{Value, json};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input: Vec<_> = std::env::args_os().skip(1).collect();
    let [workbook, source, mapping] = input.as_slice() else {
        return Err("usage: prefill_preview <workbook.xlsx> <source> <mapping.json>".into());
    };
    let workbook = PathBuf::from(workbook).canonicalize()?;
    let source = PathBuf::from(source);
    let format = source
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or("source needs an extension")?
        .to_ascii_lowercase();
    let settings: Value = serde_json::from_slice(&std::fs::read(mapping)?)?;
    let mut service = ibis_doc_service::Service::new(
        &[workbook
            .parent()
            .ok_or("workbook has no folder")?
            .to_path_buf()],
        true,
        Default::default(),
    );
    let doc = service.call("app_open_document", &json!({"path":workbook}))?;
    let mut bytes = Vec::new();
    use std::io::Read;
    std::fs::File::open(source)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err("source exceeds 1 MiB".into());
    }
    let text = String::from_utf8(bytes)?;
    let report = service.call(
        "xlsx_preview_prefill",
        &json!({
            "document_id":doc["document_id"], "sheet":settings["sheet"],
            "format":format, "source":text, "mappings":settings["mappings"],
        }),
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
