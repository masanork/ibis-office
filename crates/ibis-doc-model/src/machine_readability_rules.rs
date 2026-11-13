// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! In-process rule building blocks. Callers provide the detected table and
//! authoritative reader metadata; these functions do not infer either.
//!
//! Based on Harunobu 0.2.2 L1-12, L1-13 and L2-06; copyright (c) 2026
//! Digital Agency of Japan. See ../HARUNOBU-MIT.txt.
use std::collections::BTreeSet;
use crate::machine_readability_scoring::{RuleResult, Severity};
use crate::sheet::{CellError, CellRange, CellValue, Sheet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindingSeverity { Error, Warning, Info }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding { pub location: String, pub severity: FindingSeverity }
#[derive(Clone, Debug, PartialEq)]
pub struct RuleEvaluation { pub result: RuleResult, pub findings: Vec<Finding> }

fn result(id: &str, passed: bool, confidence: f64, severity: Severity, findings: Vec<Finding>) -> RuleEvaluation {
    RuleEvaluation { result: RuleResult { rule_id: id.to_string(), passed, confidence, severity: Some(severity) }, findings }
}
fn intersects(a: CellRange, b: CellRange) -> bool {
    a.first_row <= b.last_row && a.last_row >= b.first_row && a.first_column <= b.last_column && a.last_column >= b.first_column
}
fn column_letters(column: u32) -> String {
    let mut n = u64::from(column) + 1;
    let mut chars = Vec::new();
    while n > 0 { n -= 1; chars.push((b'A' + (n % 26) as u8) as char); n /= 26; }
    chars.into_iter().rev().collect()
}
fn address(row: u32, column: u32) -> String { format!("{}{}", column_letters(column), u64::from(row) + 1) }
fn range_address(r: CellRange) -> String { format!("{}:{}", address(r.first_row, r.first_column), address(r.last_row, r.last_column)) }

/// Excel-only L1-12: identify merges intersecting an explicitly supplied table range.
/// Header rows use zero-based indices. None means unknown (skip); an empty set
/// means that the table is known to have no header rows.
pub fn merged_cells(sheet: &Sheet, table: CellRange, header_rows: Option<&BTreeSet<u32>>) -> RuleEvaluation {
    let Some(header_rows) = header_rows else { return result("L1-12", true, 0.0, Severity::Critical, Vec::new()); };
    if table.first_row > table.last_row || table.first_column > table.last_column || sheet.merges.iter().any(|m| m.first_row > m.last_row || m.first_column > m.last_column) {
        return result("L1-12", true, 0.0, Severity::Critical, Vec::new());
    }
    let mut findings = Vec::new();
    let mut has_data_merge = false;
    for &merge in sheet.merges.iter().filter(|&&m| intersects(m, table)) {
        let header_count = header_rows.range(merge.first_row..=merge.last_row).count() as u64;
        let header = header_count == u64::from(merge.last_row) - u64::from(merge.first_row) + 1;
        if !header { has_data_merge = true; }
        findings.push(Finding { location: range_address(merge), severity: if header { FindingSeverity::Info } else { FindingSeverity::Error } });
    }
    let severity = if has_data_merge { Severity::Fatal } else if findings.is_empty() { Severity::Critical } else { Severity::Major };
    result("L1-12", !has_data_merge, 1.0, severity, findings)
}

/// A reader's knowledge of the used extent, with inclusive zero-based coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsedExtent { Unknown, Empty, Range(CellRange) }

/// Excel-only L1-13: inspect hidden rows and columns within a reader-provided used extent.
pub fn hidden_rows_columns(sheet: &Sheet, used: UsedExtent) -> RuleEvaluation {
    if used == UsedExtent::Unknown { return result("L1-13", true, 0.0, Severity::Critical, Vec::new()); }
    let mut findings = Vec::new();
    if let UsedExtent::Range(used) = used {
        if used.first_row > used.last_row || used.first_column > used.last_column {
            return result("L1-13", true, 0.0, Severity::Critical, Vec::new());
        }
        let rows: Vec<u32> = sheet.hidden_rows.keys().copied().filter(|&r| r <= used.last_row).collect();
        let cols: Vec<u32> = sheet.hidden_columns.keys().copied().filter(|&c| c <= used.last_column).collect();
        for (prefix, indices) in [("行", rows), ("列", cols)] {
            let mut groups: Vec<(u32, u32)> = Vec::new();
            for n in indices {
                if let Some(last) = groups.last_mut() { if last.1.checked_add(1) == Some(n) { last.1 = n; continue; } }
                groups.push((n, n));
            }
            for (a, b) in groups {
                let location = if a == b { format!("{prefix}{}", u64::from(a) + 1) } else { format!("{prefix}{}-{}", u64::from(a) + 1, u64::from(b) + 1) };
                findings.push(Finding { location, severity: FindingSeverity::Error });
            }
        }
    }
    result("L1-13", findings.is_empty(), 1.0, Severity::Critical, findings)
}

/// Excel-only L2-06 fallback over the parsed sheet. `complete` must be false if the reader
/// cannot guarantee that formula and error cells in the table were all loaded;
/// incomplete input is skipped, never treated as a pass.
pub fn formula_cells(sheet: &Sheet, table: CellRange, complete: bool) -> RuleEvaluation {
    if !complete || table.first_row > table.last_row || table.first_column > table.last_column {
        return result("L2-06", true, 0.0, Severity::Major, Vec::new());
    }
    let mut findings = Vec::new();
    for cell in sheet.cells.iter().filter(|c| c.row >= table.first_row && c.row <= table.last_row && c.column >= table.first_column && c.column <= table.last_column) {
        if matches!(&cell.value, CellValue::Error(CellError::Ref)) || matches!(&cell.value, CellValue::Text(value) if value == "#REF!") || cell.formula.as_ref().is_some_and(|f| !f.is_empty()) {
            findings.push(Finding { location: address(cell.row, cell.column), severity: FindingSeverity::Error });
        }
    }
    result("L2-06", findings.is_empty(), 1.0, Severity::Major, findings)
}

/// A reader-confirmed normalized format. XLSM is normalized to XLSX by the
/// reference reader; callers must make that normalization themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkbookFormat { Xlsx, Csv, Tsv, Other, Unknown }

/// L1-01: a format check after content/extension validation by the reader.
/// This does not validate a file signature or inspect its contents.
pub fn file_format(format: WorkbookFormat) -> RuleEvaluation {
    match format {
        WorkbookFormat::Unknown => result("L1-01", true, 0.0, Severity::Fatal, Vec::new()),
        WorkbookFormat::Other => result("L1-01", false, 1.0, Severity::Fatal,
            vec![Finding { location: "ファイル全体".to_string(), severity: FindingSeverity::Error }]),
        WorkbookFormat::Xlsx | WorkbookFormat::Csv | WorkbookFormat::Tsv => result("L1-01", true, 1.0, Severity::Fatal, Vec::new()),
    }
}

/// A table detected by a separate layout engine. Confidence is the reference
/// detector's table confidence, rather than rule confidence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectedTable { pub range: CellRange, pub confidence: f64 }

/// L1-02: evaluate an explicitly supplied, complete set of detected tables.
/// None or an empty list means detection is unavailable for this current
/// table; both are skipped rather than guessing the detector's fallback.
pub fn one_table_per_sheet(current: CellRange, tables: Option<&[DetectedTable]>) -> RuleEvaluation {
    let Some(tables) = tables else { return result("L1-02", true, 0.0, Severity::Fatal, Vec::new()); };
    if tables.is_empty() { return result("L1-02", true, 0.0, Severity::Fatal, Vec::new()); }
    let invalid = |r: CellRange| r.first_row > r.last_row || r.first_column > r.last_column;
    if invalid(current) || tables.iter().any(|t| invalid(t.range) || !t.confidence.is_finite() || !(0.0..=1.0).contains(&t.confidence)) {
        return result("L1-02", true, 0.0, Severity::Fatal, Vec::new());
    }
    let confident: Vec<_> = tables.iter().filter(|t| t.confidence >= 0.5).collect();
    let considered: Vec<_> = if confident.is_empty() { tables.iter().collect() } else { confident };
    if considered.len() <= 1 || considered.first().is_some_and(|first| first.range.first_row != current.first_row || first.range.first_column != current.first_column) {
        return result("L1-02", true, 0.85, Severity::Fatal, Vec::new());
    }
    let findings = considered.iter().map(|t| Finding { location: range_address(t.range), severity: FindingSeverity::Warning }).collect();
    result("L1-02", false, 0.85, Severity::Fatal, findings)
}
