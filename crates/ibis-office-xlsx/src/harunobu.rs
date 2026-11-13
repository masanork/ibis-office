// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Local Harunobu reference backend. The subprocess gets a private byte-for-byte
//! copy of the saved input or an in-memory workbook snapshot; no shell or
//! network-backed AI is used.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use wait_timeout::ChildExt;

#[derive(Clone)]
pub(crate) struct Report { pub(crate) json: serde_json::Value, pub(crate) csv: String, pub(crate) bulk: bool, pub(crate) snapshot: bool, pub(crate) sources: Vec<PathBuf> }

const PIN: &str = "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f";

pub(crate) fn runtime() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME").map(PathBuf::from).map(|p| p.join("Library/Application Support/Sorane Docs/harunobu"));
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).map(|p| p.join("Sorane Docs/harunobu"));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let root = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share"))).map(|p| p.join("sorane-docs/harunobu"));
    let root = root.ok_or_else(|| "Harunobu runtime location is unavailable".to_string())?;
    let marker = std::fs::read_to_string(root.join(".ibis-harunobu-commit")).map_err(|_| "Harunobu runtime is not installed".to_string())?;
    if marker.trim() != PIN { return Err("Harunobu runtime version does not match".to_string()); }
    let git = |args: &[&str]| -> Result<String, String> {
        let output = Command::new("git").arg("-C").arg(&root).args(args).output().map_err(|_| "Cannot verify Harunobu runtime".to_string())?;
        if !output.status.success() { return Err("Cannot verify Harunobu runtime".to_string()); }
        String::from_utf8(output.stdout).map_err(|_| "Cannot verify Harunobu runtime".to_string())
    };
    if git(&["rev-parse", "HEAD"])?.trim() != PIN || !git(&["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty() {
        return Err("Harunobu runtime source is modified".to_string());
    }
    #[cfg(target_os = "windows")]
    let bin = root.join(".venv/Scripts/python.exe");
    #[cfg(not(target_os = "windows"))]
    let bin = root.join(".venv/bin/python");
    if !bin.is_file() { return Err("Harunobu executable is missing".to_string()); }
    Ok(bin)
}

/// Run on a saved file snapshot. `mode` is restricted to Harunobu's three modes.
pub(crate) fn analyze(bin: &Path, source: &Path, mode: &str) -> Result<Report, String> {
    run(bin, &[source.to_path_buf()], mode, false, None)
}

pub(crate) fn analyze_bulk(bin: &Path, sources: &[PathBuf], mode: &str) -> Result<Report, String> {
    if sources.len() < 2 { return Err("Select at least two files".into()); }
    run(bin, sources, mode, true, None)
}

/// Analyze an in-memory serialization while preserving the original name for reports.
pub(crate) fn analyze_snapshot(bin: &Path, source: &Path, bytes: &[u8], mode: &str) -> Result<Report, String> {
    run(bin, &[source.to_path_buf()], mode, false, Some(bytes))
}

fn run(bin: &Path, sources: &[PathBuf], mode: &str, bulk: bool, snapshot: Option<&[u8]>) -> Result<Report, String> {
    if !matches!(mode, "lite" | "standard" | "thorough") { return Err("Unsupported analysis mode".into()); }
    let dir = tempfile::Builder::new().prefix("sorane-harunobu-").tempdir().map_err(|e| e.to_string())?;
    let mut inputs = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let ext = source.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "xlsx" | "xlsm" | "csv" | "tsv") { return Err("Unsupported input format".into()); }
        let name = source.file_name().ok_or_else(|| "Input file name is missing".to_string())?;
        let folder = dir.path().join(index.to_string());
        std::fs::create_dir(&folder).map_err(|e| e.to_string())?;
        let input = folder.join(name);
        if let Some(bytes) = snapshot { std::fs::write(&input, bytes).map_err(|e| e.to_string())?; }
        else { std::fs::copy(source, &input).map_err(|e| e.to_string())?; }
        inputs.push(input);
    }
    const PYTHON: &str = r#"import json, sys
from pathlib import Path
from importlib.metadata import version
from harunobu import analyze, Config
from harunobu.output.json_writer import to_dict, to_bulk_dict
from harunobu.output.csv_writer import to_csv_string, to_bulk_csv_string
assert version('harunobu') == '0.2.2', 'unexpected Harunobu version'
mode, bulk, *paths = sys.argv[1:]
if bulk == '1':
    from io import BytesIO
    from harunobu.core.analyzer import Analyzer
    from harunobu.core.reader import read_workbook
    workbooks = [read_workbook(Path(p).name, BytesIO(Path(p).read_bytes())) for p in paths]
    result = Analyzer(mode=mode).analyze_bulk(workbooks)
    report, csv = to_bulk_dict(result), to_bulk_csv_string(result)
else:
    result = analyze(Path(paths[0]), Config(mode=mode))
    report, csv = to_dict(result), to_csv_string(result)
print(json.dumps({'json': report, 'csv': csv}, ensure_ascii=False))"#;
    let mut command = Command::new(bin);
    command.arg("-I").arg("-X").arg("utf8").arg("-c").arg(PYTHON).arg(mode).arg(if bulk { "1" } else { "0" }).args(&inputs);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("HARUNOBU_") { command.env_remove(key); }
    }
    for key in ["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "GOOGLE_API_KEY", "GOOGLE_APPLICATION_CREDENTIALS", "AZURE_OPENAI_API_KEY"] {
        command.env_remove(key);
    }
    // File-backed output avoids a full pipe blocking the child while we wait.
    // The directory is private and is removed together with the input snapshot.
    let output_path = dir.path().join("report.json");
    let output_file = std::fs::File::create(&output_path).map_err(|e| e.to_string())?;
    let mut child = command.env("HARUNOBU_AI_DISABLED", "1").stdin(Stdio::null()).stdout(Stdio::from(output_file)).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let status = match child.wait_timeout(std::time::Duration::from_secs(600)) {
        Ok(Some(status)) => status,
        Ok(None) => { let _ = child.kill(); let _ = child.wait(); return Err("Harunobu timed out".into()); }
        Err(error) => { let _ = child.kill(); let _ = child.wait(); return Err(error.to_string()); }
    };
    if !status.success() { return Err(format!("Harunobu exited with {status}")); }
    let bytes = std::fs::read(output_path).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("Invalid Harunobu output: {e}"))?;
    let report = value.get("json").cloned().ok_or("Missing Harunobu report")?;
    let csv = value.get("csv").and_then(|v| v.as_str()).ok_or("Missing Harunobu CSV")?.to_string();
    let valid = if bulk { report.get("ファイル別結果").is_some_and(|v| v.is_array()) && report.get("ファイル横断チェック").is_some_and(|v| v.is_object()) }
        else { report.get("sheets").is_some_and(|v| v.is_array()) && report.get("レベル別スコア").is_some_and(|v| v.is_object()) };
    if !valid { return Err("Invalid Harunobu report".into()); }
    Ok(Report { json: report, csv, bulk, snapshot: snapshot.is_some(), sources: sources.to_vec() })
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
