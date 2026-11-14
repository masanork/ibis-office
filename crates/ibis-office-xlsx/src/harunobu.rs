// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Local in-process machine-readability analysis.
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Clone)]
pub(crate) struct Report { pub(crate) json: serde_json::Value, pub(crate) csv: String, pub(crate) bulk: bool, pub(crate) snapshot: bool, pub(crate) sources: Vec<PathBuf> }

/// A failed Magika initialization or identification follows Harunobu's
/// extension-only fallback; no network service or Python interpreter is used.
fn content_label(bytes: &[u8]) -> Option<String> {
    static MAGIKA: OnceLock<Option<Mutex<magika::Session>>> = OnceLock::new();
    MAGIKA.get_or_init(||magika::Session::new().ok().map(Mutex::new)).as_ref()
        .and_then(|session|session.lock().ok())
        .and_then(|mut session|session.identify_content_sync(bytes).ok())
        .map(|result|result.info().label.to_string())
}

fn validate_content(name: &str, bytes: &[u8]) -> Result<(), String> {
    let label = content_label(bytes);
    let extension=Path::new(name).extension().and_then(|v|v.to_str()).unwrap_or("").to_ascii_lowercase();
    let valid=match (extension.as_str(),label.as_deref()) {
        ("xlsx" | "xlsm",Some("xlsx" | "zip")) | ("csv",Some("csv" | "txt")) | ("tsv",Some("csv" | "tsv" | "txt")) => true,
        ("xlsx" | "xlsm" | "csv" | "tsv",None) => true,
        _ => false,
    };
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
    fn native_magika_matches_pinned_reference_labels() {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
        let reference:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("harunobu-magika-reference.json")).unwrap()).unwrap();
        assert_eq!(reference["harunobu_commit"],"88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
        let mut fixtures=std::collections::BTreeMap::new();
        for (i,case) in reference["cases"].as_array().unwrap().iter().enumerate() {
            let raw=if let Some(name)=case["fixture"].as_str() {
                let data=fixtures.entry(name.to_string()).or_insert_with(||serde_json::from_slice::<serde_json::Value>(&std::fs::read(root.join(name)).unwrap()).unwrap());
                data["cases"][case["index"].as_u64().unwrap() as usize]["bytes"].as_str().unwrap()
            } else { case["bytes"].as_str().unwrap() };
            let bytes:Vec<_>=raw.as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
            assert_eq!(content_label(&bytes).as_deref(),case["label"].as_str(),"case {i}");
        }
    }
    #[test]
    fn analysis_uses_native_reports_without_runtime_lookup() {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
        let csv=root.join("data.csv");let xlsx=root.join("structure.xlsx");
        let report=analyze(&csv,"thorough").unwrap();assert!(report.json["sheets"].is_array());assert!(report.csv.contains("ルールID"));
        let report=analyze(&xlsx,"thorough").unwrap();assert!(report.json["sheets"].is_array());
        let report=analyze_bulk(&[csv,xlsx],"standard").unwrap();assert_eq!(report.json["ファイル数"],2);assert!(report.json["ファイル横断チェック"]["L3-07"].is_object());
        assert!(validate_content("fake.csv",b"PK\x03\x04\0\0\0\0").is_err());
    }
}
