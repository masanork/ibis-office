// SPDX-License-Identifier: AGPL-3.0-only
//! Local qualification tool. Reports counts, never document contents.
use ibis_pptx::jtd::{self, RecordKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: jtd_import INPUT.jtd [OUTPUT.docx]")?;
    let data = std::fs::read(path)?;
    let decoded = jtd::inspect(&data)?;
    let headers: Vec<_> = decoded
        .records
        .iter()
        .filter_map(|record| match &record.kind {
            RecordKind::Record {
                class: 0x10,
                payload,
            } => decoded.tlv(payload.clone()).ok(),
            _ => None,
        })
        .flatten()
        .filter(|(tag, _)| *tag == 0x8f)
        .collect();
    let model = jtd::read(&data)?;
    if let Some(path) = args.next() {
        std::fs::write(path, jtd::export_docx(&model)?)?;
    }
    println!(
        "{{\"units\":{},\"pieces\":{},\"records\":{},\"ruled_headers\":{},\"style_spans\":{},\"undecoded_style_pieces\":{},\"diagnostics\":{}}}",
        decoded.units.len(),
        decoded.pieces.len(),
        decoded.records.len(),
        headers.len(),
        decoded.styles.len(),
        decoded.undecoded_styles.len(),
        decoded.diagnostics.len()
    );
    Ok(())
}
