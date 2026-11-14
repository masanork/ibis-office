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

/// The input encoding as established by the reader. XLSX has no text encoding
/// classification; UTF-8 includes UTF-8 with BOM. Unknown must not be guessed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEncoding { Xlsx, Utf8, Cp932, Unknown }

fn roman_classification_code(value: &str) -> bool {
    // Python's str.strip includes the four ASCII information separators.
    let trimmed = value.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'));
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else { return false; };
    if !(('\u{2160}'..='\u{217f}').contains(&first) && matches!(chars.next(), Some('ー' | '－' | '-'))) { return false; }
    let mut digits = 0usize;
    for c in chars {
        if !(c.is_ascii_digit() || ('０'..='９').contains(&c)) { return false; }
        digits += 1;
    }
    digits > 0
}

/// L1-08: inspect text values within a provided table range. A caller must
/// supply the reader's encoding; Unknown is skipped rather than assuming UTF-8.
pub fn platform_characters(sheet: &Sheet, table: CellRange, encoding: TextEncoding) -> RuleEvaluation {
    use crate::machine_readability::{character_risk, CharacterRisk};
    if encoding == TextEncoding::Unknown || table.first_row > table.last_row || table.first_column > table.last_column {
        return result("L1-08", true, 0.0, Severity::Critical, Vec::new());
    }
    let mut findings = Vec::new();
    let mut any_warning = false;
    for cell in sheet.cells.iter().filter(|c| c.row >= table.first_row && c.row <= table.last_row && c.column >= table.first_column && c.column <= table.last_column) {
        let CellValue::Text(value) = &cell.value else { continue; };
        let roman_code = roman_classification_code(value);
        let mut info = false;
        let mut warning = false;
        for c in value.chars() {
            match character_risk(c) {
                Some(CharacterRisk::Info) => info = true,
                Some(CharacterRisk::Warning) if roman_code && ('\u{2160}'..='\u{217f}').contains(&c) => info = true,
                Some(CharacterRisk::Warning) => warning = true,
                None => {},
            }
        }
        if warning || info {
            any_warning |= warning;
            findings.push(Finding { location: address(cell.row, cell.column), severity: if warning { FindingSeverity::Warning } else { FindingSeverity::Info } });
        }
    }
    result("L1-08", encoding != TextEncoding::Cp932 || !any_warning, 0.95, Severity::Critical, findings)
}

/// A layout detector's inferred column type for L1-07. `None` at the call
/// site means no schema is available, matching Harunobu's unguarded fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnType { Numeric, Other }

/// Classify one textual cell with Harunobu's L1-07 heuristics. The order of
/// checks matters because only the first issue is reported for each cell.
pub fn multiple_data_reason(value: &str, column: Option<ColumnType>) -> Option<&'static str> {
    use std::sync::OnceLock;
    use regex::Regex;
    static TWO: OnceLock<Regex> = OnceLock::new();
    static THOUSANDS: OnceLock<Regex> = OnceLock::new();
    static DIGIT: OnceLock<Regex> = OnceLock::new();
    static PAREN: OnceLock<Regex> = OnceLock::new();
    let two = TWO.get_or_init(|| Regex::new(r"^\d+（\d+）$").unwrap());
    let thousands = THOUSANDS.get_or_init(|| Regex::new(r"^\d{1,3}(,\d{3})*(\.\d+)?$").unwrap());
    let digit = DIGIT.get_or_init(|| Regex::new(r"^\d$").unwrap());
    let paren = PAREN.get_or_init(|| Regex::new(r"（[^）]*）").unwrap());
    let python_space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    if value.split('\n').filter(|line| !line.trim_matches(python_space).is_empty()).take(2).count() >= 2 { return Some("セル内改行による複数データ"); }
    if two.is_match(value.trim_matches(python_space)) { return Some("全角括弧内に別数値（複数指標の混在）"); }
    if value.matches('、').count() >= 2 { return Some("読点（、）による列挙"); }
    if value.matches('・').count() >= 2 { return Some("中黒（・）による列挙"); }
    let chars: Vec<char> = value.chars().collect();
    let is_digit = |c: char| digit.is_match(c.encode_utf8(&mut [0; 4]));
    let separator = chars.iter().enumerate().any(|(i, &c)| c == ',' && (i == 0 || !is_digit(chars[i-1])) && (i + 1 == chars.len() || !is_digit(chars[i+1])));
    if separator && !thousands.is_match(value.trim_matches(python_space)) { return Some("カンマ区切りによる複数データ"); }
    if column != Some(ColumnType::Numeric) {
        let compact: String = value.chars().filter(|&c| !python_space(c)).collect();
        let compact = paren.replace_all(&compact, "");
        if compact.contains(',') && !thousands.is_match(&compact) { return Some("カンマ区切りによる複数データ"); }
    }
    None
}

/// L1-07 over an explicitly supplied table and column schema. Invalid ranges
/// are skipped; no table or schema is inferred here.
pub fn one_data_per_cell(sheet: &Sheet, table: CellRange, columns: Option<&std::collections::BTreeMap<u32, ColumnType>>) -> RuleEvaluation {
    if table.first_row > table.last_row || table.first_column > table.last_column {
        return result("L1-07", true, 0.0, Severity::Critical, Vec::new());
    }
    let mut findings = Vec::new();
    for cell in sheet.cells.iter().filter(|c| c.row >= table.first_row && c.row <= table.last_row && c.column >= table.first_column && c.column <= table.last_column) {
        if let CellValue::Text(value) = &cell.value {
            if !value.trim().is_empty() && multiple_data_reason(value, columns.and_then(|c| c.get(&cell.column).copied())).is_some() {
                findings.push(Finding { location: address(cell.row, cell.column), severity: FindingSeverity::Warning });
            }
        }
    }
    let passed = findings.is_empty();
    result("L1-07", passed, if passed { 0.82 } else { 0.78 }, Severity::Critical, findings)
}

/// A cell separated from every detected table by the layout engine.
/// Coordinates use the sheet model's zero-based convention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutsideTableCell { pub row: u32, pub column: u32 }

/// L1-04: inspect the layout engine's complete `others` list. `None` means
/// layout detection did not complete and is not an indication of a clean sheet.
pub fn irrelevant_information(others: Option<&[OutsideTableCell]>) -> RuleEvaluation {
    let Some(others) = others else { return result("L1-04", true, 0.0, Severity::Major, Vec::new()); };
    let findings = others.iter().map(|cell| Finding { location: address(cell.row, cell.column), severity: FindingSeverity::Warning }).collect::<Vec<_>>();
    result("L1-04", findings.is_empty(), 0.9, Severity::Major, findings)
}

/// Harunobu L1-06 issue descriptions for one cell. Address columns and
/// several semantic uses of full-width spaces are treated as exceptions.
pub fn whitespace_issues(value: &str, is_address_column: bool) -> Vec<&'static str> {
    use std::sync::OnceLock;
    static SEMANTIC: OnceLock<regex::Regex> = OnceLock::new();
    static CONSECUTIVE: OnceLock<regex::Regex> = OnceLock::new();
    let semantic = SEMANTIC.get_or_init(|| regex::Regex::new(concat!(
        r"^(?:[ア-ンa-zａ-ｚ①-⑳（(][）)]?\u{3000}|※\d*\u{3000}|第[０-９0-9一二三四五六七八九十]+\u{3000}",
        r"|\u{3000}+〃(?:\n)?$|\u{3000}+[○●・◯]\u{3000}?|\[.+?\]\u{3000}?[0-9０-９]",
        r"|[\u{2160}-\u{217f}][．.]\u{3000}?|[0-9０-９]+[．.]\u{3000}|\u{3000}{2,}[0-9０-９]+[．.])",
        r"|\([A-Za-z]\)/\([A-Za-z]\)\u{3000}"
    )).unwrap());
    let consecutive = CONSECUTIVE.get_or_init(|| regex::Regex::new(r"[ \u{3000}]{2,}").unwrap());
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let semantic = semantic.is_match(value);
    let mut issues = Vec::new();
    let check = value.trim_end_matches('\n');
    if check != check.trim_matches(space) && !semantic { issues.push("先頭または末尾に空白があります"); }
    if !is_address_column && value.contains('\u{3000}') && !semantic { issues.push("全角スペースが含まれています"); }
    if !is_address_column && consecutive.is_match(value.trim_matches(space)) && !semantic { issues.push("連続するスペースが含まれています"); }
    let content = value.trim_end_matches(['\r', '\n']);
    if (content.contains('\n') || content.contains('\r')) && value.chars().filter(|&c| space(c)).count() as f64 / value.chars().count().max(1) as f64 > 0.10 {
        issues.push("セル内に体裁目的の改行が含まれています");
    }
    issues
}

/// L1-06 over the supplied table and header rows. `None` means header metadata
/// is unavailable; an empty list uses the first table row as in Harunobu.
pub fn whitespace_formatting(sheet: &Sheet, table: CellRange, header_rows: Option<&[u32]>) -> RuleEvaluation {
    use std::sync::OnceLock;
    static ADDRESS: OnceLock<regex::Regex> = OnceLock::new();
    if table.first_row > table.last_row || table.first_column > table.last_column || header_rows.is_none() {
        return result("L1-06", true, 0.0, Severity::Critical, Vec::new());
    }
    let address_re = ADDRESS.get_or_init(|| regex::Regex::new(r"(?i)(住所|所在地|address|addr|勤務地|居所|連絡先住所|本籍|現住所|送付先)").unwrap());
    let headers = header_rows.unwrap();
    let header_rows: BTreeSet<_> = if headers.is_empty() { BTreeSet::from([table.first_row]) } else { headers.iter().copied().collect() };
    let address_columns: BTreeSet<_> = sheet.cells.iter().filter_map(|c| {
        if !header_rows.contains(&c.row) || c.column < table.first_column || c.column > table.last_column { return None; }
        match &c.value { CellValue::Text(value) if address_re.is_match(value) => Some(c.column), _ => None }
    }).collect();
    let findings: Vec<_> = sheet.cells.iter().filter_map(|cell| {
        if cell.row < table.first_row || cell.row > table.last_row || cell.column < table.first_column || cell.column > table.last_column { return None; }
        match &cell.value {
            CellValue::Text(value) if !value.is_empty() && !whitespace_issues(value, address_columns.contains(&cell.column)).is_empty() => Some(Finding { location: address(cell.row, cell.column), severity: FindingSeverity::Warning }),
            _ => None,
        }
    }).collect();
    result("L1-06", findings.is_empty(), 0.95, Severity::Critical, findings)
}

/// L1-03 for a single, unmerged CSV/TSV table containing only text cells.
/// The caller must establish that there are no other tables on the sheet.
/// Unsupported input returns None instead of treating it as a clean result.
pub fn csv_data_fragmentation(sheet: &Sheet, table: CellRange) -> Option<RuleEvaluation> {
    if table.first_row > table.last_row || table.first_column > table.last_column || !sheet.merges.is_empty() {
        return None;
    }
    let mut rows = BTreeSet::new();
    let mut columns = BTreeSet::new();
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    for cell in sheet.cells.iter().filter(|c| c.row >= table.first_row && c.row <= table.last_row && c.column >= table.first_column && c.column <= table.last_column) {
        match &cell.value {
            CellValue::Text(value) => if !value.trim_matches(space).is_empty() { rows.insert(cell.row); columns.insert(cell.column); },
            CellValue::Empty => {},
            _ => return None,
        }
    }
    let mut findings = Vec::new();
    if u64::from(table.last_row) - u64::from(table.first_row) + 1 > 2 {
        for row in table.first_row..=table.last_row {
            if !rows.contains(&row) { findings.push(Finding { location: format!("行{}", u64::from(row) + 1), severity: FindingSeverity::Error }); }
        }
        for col in table.first_column..=table.last_column {
            if !columns.contains(&col) { findings.push(Finding { location: format!("列{}", u64::from(col) + 1), severity: FindingSeverity::Error }); }
        }
    }
    Some(result("L1-03", findings.is_empty(), if findings.is_empty() { 0.85 } else { 0.9 }, Severity::Critical, findings))
}

/// L1-05 for an unmerged CSV/TSV table with reader-supplied header and body
/// rows. Ambiguous numeric header formats return None rather than a pass.
pub fn csv_column_headers(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    if table.first_row > table.last_row || table.first_column > table.last_column || !sheet.merges.is_empty() { return None; }
    let fallback = [table.first_row];
    let headers = if header_rows.is_empty() { &fallback[..] } else { header_rows };
    let last_header = *headers.last()?;
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut values = BTreeMap::new();
    let mut body_columns = BTreeSet::new();
    let mut header_columns = BTreeSet::new();
    for cell in sheet.cells.iter().filter(|c| c.column >= table.first_column && c.column <= table.last_column) {
        let value = match &cell.value { CellValue::Text(v) => v.trim_matches(space), CellValue::Empty => "", _ => return None };
        if value.is_empty() { continue; }
        if headers.contains(&cell.row) { header_columns.insert(cell.column); }
        if cell.row == last_header { values.insert(cell.column, value); }
        if cell.row >= body_first && cell.row <= body_last { body_columns.insert(cell.column); }
    }
    let mut findings = Vec::new();
    for column in table.first_column..=table.last_column {
        if body_columns.contains(&column) && !header_columns.contains(&column) {
            findings.push(Finding { location: address(headers[0], column), severity: FindingSeverity::Error });
        }
    }
    let width = u64::from(table.last_column) - u64::from(table.first_column) + 1;
    if width >= 2 && values.len() as u64 == width {
        static NUMBER: OnceLock<regex::Regex> = OnceLock::new();
        let number = NUMBER.get_or_init(|| regex::Regex::new(r"^[+-]?(?:(?:[0-9](?:_?[0-9])*)(?:\.(?:[0-9](?:_?[0-9])*)?)?|\.[0-9](?:_?[0-9])*)(?:[eE][+-]?[0-9](?:_?[0-9])*)?$").unwrap());
        let mut nums = Vec::new();
        for value in values.values() {
            if !number.is_match(value) {
                let numeric_shape = value.chars().any(|c| !c.is_ascii() && c.is_numeric()) && value.chars().all(|c| c.is_numeric() || matches!(c, '+' | '-' | '.' | '_' | 'e' | 'E'));
                if numeric_shape || matches!(value.to_ascii_lowercase().as_str(), "nan" | "+nan" | "-nan" | "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity") { return None; }
                nums.clear(); break;
            }
            let n: f64 = value.replace('_', "").parse().ok()?;
            if !n.is_finite() || n.abs() >= 9_007_199_254_740_992.0 { return None; }
            nums.push(n);
        }
        if nums.len() as u64 == width && nums.iter().all(|n| n.fract() == 0.0) && nums.windows(2).all(|pair| pair[1] - pair[0] == 1.0) {
            for column in table.first_column..=table.last_column { findings.push(Finding { location: address(last_header, column), severity: FindingSeverity::Error }); }
        }
    }
    Some(result("L1-05", findings.is_empty(), if findings.is_empty() { 0.8 } else { 0.85 }, Severity::Critical, findings))
}

/// L1-09 explicitly abstains when the reference AI checker is unavailable.
/// This does not implement the optional AI-assisted judgment.
pub fn blank_vs_zero_ai_disabled() -> RuleEvaluation {
    result("L1-09", true, 0.0, Severity::Minor, Vec::new())
}

/// L2-03 for an unmerged CSV/TSV table with known header and body rows.
/// Unsupported reader data returns None; no detected header is a low-confidence skip.
pub fn csv_unique_headers(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    if table.first_row > table.last_row || table.first_column > table.last_column || !sheet.merges.is_empty() { return None; }
    let Some(&last_header) = header_rows.last() else { return Some(result("L2-03", true, 0.4, Severity::Fatal, Vec::new())); };
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut cells = BTreeMap::new();
    let mut body_columns = BTreeSet::new();
    for cell in sheet.cells.iter().filter(|c| c.column >= table.first_column && c.column <= table.last_column) {
        let value = match &cell.value { CellValue::Text(v) => v.trim_matches(space), CellValue::Empty => "", _ => return None };
        if cell.row >= body_first && cell.row <= body_last && !value.is_empty() { body_columns.insert(cell.column); }
        cells.insert((cell.row, cell.column), value);
    }
    let location = |col| if cells.contains_key(&(last_header, col)) { address(last_header, col) } else { format!("R{}C{}", u64::from(last_header) + 1, u64::from(col) + 1) };
    let mut groups: Vec<(String, Vec<u32>)> = Vec::new();
    let mut findings = Vec::new();
    for col in table.first_column..=table.last_column {
        let mut parts = Vec::new();
        for &row in header_rows {
            if let Some(&value) = cells.get(&(row, col)) { if !value.is_empty() && !parts.contains(&value) { parts.push(value); } }
        }
        if parts.is_empty() {
            if body_columns.contains(&col) { findings.push(Finding { location: location(col), severity: FindingSeverity::Error }); }
        } else {
            let name = parts.join("_");
            if let Some((_, columns)) = groups.iter_mut().find(|(existing, _)| existing == &name) { columns.push(col); } else { groups.push((name, vec![col])); }
        }
    }
    for (_, columns) in groups { if columns.len() > 1 { for col in columns { findings.push(Finding { location: location(col), severity: FindingSeverity::Error }); } } }
    Some(result("L2-03", findings.is_empty(), 0.95, Severity::Fatal, findings))
}
