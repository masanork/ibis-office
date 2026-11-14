// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Local in-process machine-readability analysis.
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct Report { pub(crate) json: serde_json::Value, pub(crate) csv: String, pub(crate) bulk: bool, pub(crate) snapshot: bool, pub(crate) sources: Vec<PathBuf> }

/// Detect content independently from the filename; an ambiguous text stream can
/// be valid CSV and TSV, so the extension still selects its delimiter.
fn validate_content(name: &str, bytes: &[u8]) -> Result<(), String> {
    use ibis_pptx::machine_readability_detect::{detect, Content};
    let extension=Path::new(name).extension().and_then(|v|v.to_str()).unwrap_or("").to_ascii_lowercase();
    let valid=matches!((extension.as_str(), detect(bytes)),
        ("xlsx" | "xlsm", Content::Workbook | Content::Zip)
        | ("csv" | "tsv", Content::Text));
    if valid { Ok(()) } else { Err("Unsupported input format".into()) }
}

fn analyze_bytes(source: &Path, bytes: &[u8], mode: &str) -> Result<serde_json::Value, String> {
    let name=source.file_name().and_then(|v|v.to_str()).ok_or("Input file name is missing")?;
    validate_content(name,bytes)?;
    match source.extension().and_then(|v|v.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "csv" | "tsv" => ibis_pptx::machine_readability_native::analyze_csv_report_experimental(bytes,name,mode).map_err(|e| { log::warn!("CSV analysis failed: {e:?}"); "Native analysis unavailable".to_string() }),
        "xlsx" | "xlsm" => ibis_pptx::machine_readability_xlsx::analyze_xlsx_report_experimental(bytes,name,mode).map_err(|e| { log::warn!("Excel analysis failed: {e:?}"); "Native analysis unavailable".to_string() }),
        _ => Err("Unsupported input format".into()),
    }
}

pub(crate) fn analyze(source: &Path, mode: &str) -> Result<Report, String> {
    let bytes=std::fs::read(source).map_err(|e|e.to_string())?;
    analyze_snapshot_inner(source,&bytes,mode,false)
}

pub(crate) fn analyze_snapshot(source: &Path, bytes: &[u8], mode: &str) -> Result<Report, String> {
    analyze_snapshot_inner(source,bytes,mode,true)
}

fn analyze_snapshot_inner(source: &Path, bytes: &[u8], mode: &str, snapshot: bool) -> Result<Report, String> {
    let json=analyze_bytes(source,bytes,mode)?;
    let csv=ibis_pptx::machine_readability_report::file_report_csv(&json).ok_or("Invalid native report")?;
    Ok(Report { json,csv,bulk:false,snapshot,sources:vec![source.to_path_buf()] })
}

pub(crate) fn analyze_bulk(sources: &[PathBuf], mode: &str) -> Result<Report, String> {
    if sources.len()<2 { return Err("Select at least two files".into()); }
    let mut files=Vec::with_capacity(sources.len());
    for source in sources {
        let bytes=std::fs::read(source).map_err(|e|e.to_string())?;
        files.push(analyze_bytes(source,&bytes,mode)?);
    }
    let json=ibis_pptx::machine_readability_report::combine_native_reports(&files).ok_or("Invalid native bulk report")?;
    let csv=ibis_pptx::machine_readability_report::bulk_report_csv(&json).ok_or("Invalid native bulk CSV")?;
    Ok(Report { json,csv,bulk:true,snapshot:false,sources:sources.to_vec() })
}

/// Interpret only unambiguous one-based locations emitted by Harunobu.
pub(crate) fn cell_location(value: &str) -> Option<(u32, u32)> {
    let value = value.trim();
    if let Some(row) = value.strip_prefix('行') {
        let n = row.split('-').next()?.parse::<u32>().ok()?;
        return (1..=1_048_576).contains(&n).then_some((n - 1, 0));
    }
    if let Some(col) = value.strip_prefix('列') {
        let n = col.split('-').next()?.parse::<u32>().ok()?;
        return (1..=16_384).contains(&n).then_some((0, n - 1));
    }
    let first = value.rsplit('!').next()?.split(':').next()?.replace('$', "");
    let split = first.find(|c: char| !c.is_ascii_alphabetic())?;
    let (letters, digits) = first.split_at(split);
    if letters.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) { return None; }
    let mut col = 0u32;
    for c in letters.bytes() {
        col = col.checked_mul(26)?.checked_add((c.to_ascii_uppercase() - b'A' + 1) as u32)?;
    }
    let row = digits.parse::<u32>().ok()?;
    if !(1..=1_048_576).contains(&row) || !(1..=16_384).contains(&col) { return None; }
    Some((row - 1, col - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_validation_rejects_binary_but_preserves_ambiguous_text() {
        assert!(validate_content("table.csv", b"\"alpha\tbeta\"\n\"gamma\tdelta\"\n").is_ok());
        assert!(validate_content("table.tsv", b"[1,2,3]\n").is_ok());
        assert!(validate_content("image.csv", b"GIF89a\0\0").is_err());
        assert!(validate_content("book.xlsx", b"not a workbook").is_err());
    }
    #[test]
    fn analysis_uses_native_reports_without_runtime_lookup() {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
        let csv=root.join("data.csv");let xlsx=root.join("structure.xlsx");
        let report=analyze(&csv,"thorough").unwrap();assert!(report.json["sheets"].is_array());assert!(report.csv.contains("ルールID"));
        let report=analyze(&xlsx,"thorough").unwrap();assert!(report.json["sheets"].is_array());
        let report=analyze_bulk(&[csv,xlsx],"standard").unwrap();assert_eq!(report.json["ファイル数"],2);assert!(report.json["ファイル横断チェック"]["L3-07"].is_object());
        assert!(validate_content("fake.csv",b"PK\x03\x04\0\0\0\0").is_err());
        let docx=std::fs::read(root.join("../../benchmark/securities-registration-statement.docx")).unwrap();
        assert!(analyze_bytes(Path::new("not-a-workbook.xlsx"),&docx,"standard").is_err());
    }
}
