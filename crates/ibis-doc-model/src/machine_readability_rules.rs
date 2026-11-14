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
        let mut nums = Vec::new(); let mut unknown = false; let mut nonnumeric = false;
        for value in values.values() {
            if !number.is_match(value) {
                let numeric_shape = value.chars().any(|c| !c.is_ascii() && c.is_numeric()) && value.chars().all(|c| c.is_numeric() || matches!(c, '+' | '-' | '.' | '_' | 'e' | 'E'));
                if numeric_shape || matches!(value.to_ascii_lowercase().as_str(), "nan" | "+nan" | "-nan" | "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity") { unknown = true; continue; }
                nonnumeric = true; break;
            }
            let n: f64 = value.replace('_', "").parse().ok()?;
            if !n.is_finite() || n.abs() >= 9_007_199_254_740_992.0 { unknown = true; continue; }
            nums.push(n);
        }
        if unknown && !nonnumeric { return None; }
        if !nonnumeric && nums.len() as u64 == width && nums.iter().all(|n| n.fract() == 0.0) && nums.windows(2).all(|pair| pair[1] - pair[0] == 1.0) {
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

/// L3-01 using a detected table and the layout's zero-based header/body rows.
pub fn header_start_position(table: CellRange, header_rows: &[u32], body_first: u32) -> Option<RuleEvaluation> {
    if table.first_row > table.last_row || table.first_column > table.last_column { return None; }
    let mut findings = Vec::new();
    let mut fatal = false;
    if table.first_row > 0 {
        findings.push(Finding { location: format!("A1:A{}", table.first_row), severity: FindingSeverity::Error });
        fatal = true;
    }
    if let Some(&last) = header_rows.iter().max() {
        if u64::from(body_first) > u64::from(last) + 1 {
            findings.push(Finding { location: format!("A{}:A{}", u64::from(last) + 2, body_first), severity: FindingSeverity::Error });
            fatal = true;
        }
        if header_rows.len() >= 3 {
            findings.push(Finding { location: format!("A{}:A{}", u64::from(*header_rows.iter().min()?) + 1, u64::from(last) + 1), severity: FindingSeverity::Info });
        }
    }
    let severity = if !findings.is_empty() && !fatal { Severity::Major } else { Severity::Fatal };
    Some(result("L3-01", findings.is_empty(), 0.95, severity, findings))
}

/// L3 rules skipped by the pinned reference for single-file assessment.
pub fn reference_skipped_level3() -> [RuleEvaluation; 4] {
    [result("L3-02", true, 0.0, Severity::Critical, Vec::new()),
     result("L3-03", true, 0.0, Severity::Major, Vec::new()),
     result("L3-07", true, 0.0, Severity::Critical, Vec::new()),
     result("L3-08", true, 0.0, Severity::Major, Vec::new())]
}

/// L2-02 for an unmerged CSV table, with AI unavailable and inferred stub columns supplied.
pub fn csv_item_abbreviations(sheet: &Sheet, stub_columns: &[u32], body_first: u32, body_last: u32) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    if !sheet.merges.is_empty() { return None; }
    if stub_columns.is_empty() { return Some(result("L2-02", true, 0.0, Severity::Fatal, Vec::new())); }
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut cells = BTreeMap::new();
    for cell in sheet.cells.iter().filter(|c| c.row >= body_first && c.row <= body_last && stub_columns.contains(&c.column)) {
        let value = match &cell.value { CellValue::Text(v) => v.trim_matches(space), CellValue::Empty => "", _ => return None };
        cells.insert((cell.row, cell.column), value);
    }
    let mut findings = Vec::new();
    for &col in stub_columns {
        let mut seen = false;
        for row in body_first..=body_last {
            let cell = cells.get(&(row, col));
            let value = cell.copied().unwrap_or("");
            let marker = matches!(value, "〃" | "同上" | "同左" | "同右" | "〇" | "仝" | "ヽ" | "々");
            if marker || (value.is_empty() && seen) {
                let location = if cell.is_some() { address(row, col) } else { format!("R{}C{}", u64::from(row) + 1, u64::from(col) + 1) };
                findings.push(Finding { location, severity: if marker { FindingSeverity::Error } else { FindingSeverity::Warning } });
            } else if !value.is_empty() { seen = true; }
        }
    }
    Some(result("L2-02", findings.is_empty(), 0.70, Severity::Fatal, findings))
}

/// L2-01 for unmerged CSV/TSV text cells with reader-inferred column schemas,
/// with AI unavailable. A missing schema map uses the reference fallback.
pub fn csv_numeric_purity(sheet: &Sheet, table: CellRange, body_first: u32, body_last: u32, schemas: &[(u32, crate::machine_readability_layout::InferredColumnType)]) -> Option<RuleEvaluation> {
    numeric_purity(sheet, table, body_first, body_last, schemas, &BTreeSet::new())
}

/// L2-01 on reference-stringified XLSX values, with typed numeric positions.
pub fn xlsx_numeric_purity(sheet: &Sheet, table: CellRange, body_first: u32, body_last: u32, schemas: &[(u32, crate::machine_readability_layout::InferredColumnType)], typed_numeric: &BTreeSet<(u32, u32)>) -> Option<RuleEvaluation> {
    numeric_purity(sheet, table, body_first, body_last, schemas, typed_numeric)
}

fn numeric_purity(sheet: &Sheet, table: CellRange, body_first: u32, body_last: u32, schemas: &[(u32, crate::machine_readability_layout::InferredColumnType)], typed_numeric: &BTreeSet<(u32, u32)>) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    use crate::machine_readability_layout::InferredColumnType;
    struct Patterns { pure: regex::Regex, strict: regex::Regex, unit: regex::Regex, percent_unit: regex::Regex, note: regex::Regex, percent: regex::Regex, triangle: regex::Regex, has_digit: regex::Regex }
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    let p = PATTERNS.get_or_init(|| Patterns {
        pure: regex::Regex::new(r"^[\s\x1c-\x1f\-ー−▲△▼]*[\d,]+\.?\d*[\s\x1c-\x1f]*$").unwrap(),
        strict: regex::Regex::new(r"^[\s\x1c-\x1f\-ー−]*[\d,]+\.?\d*[\s\x1c-\x1f]*$").unwrap(),
        unit: regex::Regex::new(r"^[\d,.]+[\s\x1c-\x1f]*(人|件|円|千円|百万円|億円|万|千|kg|km|m|g|t|ha|㎡|個|台|回|本|枚|箇所|か所|世帯|戸|社|校|棟|隻|機|両|組|点|冊|食|泊|日|月|年|時間|分|秒|歳|才|度|号|丁目)$").unwrap(),
        percent_unit: regex::Regex::new(r"^[\d,.]+[\s\x1c-\x1f]*(%|％)$").unwrap(),
        note: regex::Regex::new(r"^[\d,.]+[\s\x1c-\x1f]*[（(].*[）)]$").unwrap(),
        percent: regex::Regex::new(r"^[\s\x1c-\x1f\-ー−▲△▼]*[\d,.]+[\s\x1c-\x1f]*(%|％)[\s\x1c-\x1f]*$").unwrap(),
        triangle: regex::Regex::new(r"^[▲△▼][\s\x1c-\x1f]*[\d,]+\.?\d*[\s\x1c-\x1f]*$").unwrap(),
        has_digit: regex::Regex::new(r"\d").unwrap(),
    });
    if table.first_row > table.last_row || table.first_column > table.last_column || !sheet.merges.is_empty() { return None; }
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut cells = BTreeMap::new();
    let mut columns: BTreeMap<u32, Vec<(&str, bool)>> = BTreeMap::new();
    for cell in sheet.cells.iter().filter(|c| c.row >= body_first && c.row <= body_last && c.column >= table.first_column && c.column <= table.last_column) {
        let value = match &cell.value { CellValue::Text(v) => v.trim_matches(space), CellValue::Empty => "", _ => return None };
        if !value.is_empty() { cells.insert((cell.row, cell.column), value); columns.entry(cell.column).or_default().push((value, typed_numeric.contains(&(cell.row, cell.column)))); }
    }
    let schema_map: BTreeMap<_, _> = schemas.iter().copied().collect();
    let mut numeric = BTreeSet::new();
    let mut percent = BTreeSet::new();
    for col in table.first_column..=table.last_column {
        let vals = columns.get(&col).map_or(&[][..], Vec::as_slice);
        let majority = || !vals.is_empty() && vals.iter().filter(|(v, typed)| *typed || p.pure.is_match(v) || p.unit.is_match(v) || p.percent_unit.is_match(v) || p.note.is_match(v)).count() * 2 > vals.len();
        let selected = if schema_map.is_empty() { majority() } else { match schema_map.get(&col) { Some(InferredColumnType::Numeric) => true, Some(InferredColumnType::Mixed) => majority(), _ => false } };
        if selected {
            numeric.insert(col);
            if !vals.is_empty() && vals.iter().filter(|(v, _)| p.percent.is_match(v)).count() * 2 >= vals.len() { percent.insert(col); }
        }
    }
    let mut findings = Vec::new();
    for (&(row, col), &value) in &cells {
        if !numeric.contains(&col) || typed_numeric.contains(&(row, col)) || p.strict.is_match(value) || !p.has_digit.is_match(value) || (percent.contains(&col) && p.percent.is_match(value)) { continue; }
        let severity = if !p.triangle.is_match(value) && !p.unit.is_match(value) && !p.percent_unit.is_match(value) && p.note.is_match(value) { FindingSeverity::Warning } else { FindingSeverity::Error };
        findings.push(Finding { location: address(row, col), severity });
    }
    Some(result("L2-01", findings.is_empty(), 0.85, Severity::Fatal, findings))
}

/// L2-05 for unmerged CSV/TSV, with AI unavailable.
pub fn csv_other_details(sheet: &Sheet, table: CellRange, body_first: u32, body_last: u32, schemas: &[(u32, crate::machine_readability_layout::InferredColumnType)]) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    use crate::machine_readability_layout::InferredColumnType;
    if table.first_row > table.last_row || table.first_column > table.last_column || !sheet.merges.is_empty() { return None; }
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let schema_map: BTreeMap<_, _> = schemas.iter().copied().collect();
    let mut columns: BTreeMap<u32, Vec<(u32, &str)>> = BTreeMap::new();
    for cell in sheet.cells.iter().filter(|c| c.row >= body_first && c.row <= body_last && c.column >= table.first_column && c.column <= table.last_column) {
        let value = match &cell.value { CellValue::Text(v) => v.trim_matches(space), CellValue::Empty => "", _ => return None };
        if !value.is_empty() { columns.entry(cell.column).or_default().push((cell.row, value)); }
    }
    let mut analyzed = false;
    let mut findings = Vec::new();
    for col in table.first_column..=table.last_column {
        if matches!(schema_map.get(&col), Some(InferredColumnType::Numeric | InferredColumnType::Date | InferredColumnType::Empty)) { continue; }
        let Some(cells) = columns.get_mut(&col) else { continue; };
        cells.sort_by_key(|v| v.0);
        if cells.len() < 20 { continue; }
        let mut values: Vec<(&str, Vec<u32>)> = Vec::new();
        for &(row, value) in cells.iter() {
            if let Some((_, rows)) = values.iter_mut().find(|(v, _)| *v == value) { rows.push(row); } else { values.push((value, vec![row])); }
            if values.len() > 20 { break; }
        }
        if values.len() > 20 { continue; }
        analyzed = true;
        if values.iter().filter(|(value, _)| value.contains("その他")).count() >= 2 {
            for (_, rows) in values.iter().filter(|(value, rows)| value.contains("その他") && rows.len() < 3) {
                findings.push(Finding { location: address(rows[0], col), severity: FindingSeverity::Warning });
            }
        }
    }
    Some(result("L2-05", findings.is_empty(), if analyzed { 0.8 } else { 0.0 }, Severity::Major, findings))
}

/// Normalize a category value the way Harunobu's L2-04 does before comparing.
fn normalized_choice(value: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let value: String = value.nfkc().collect();
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut seen = false;
    let mut cut = value.len();
    for (index, c) in value.char_indices() {
        if seen && matches!(c, '※' | '＊' | '＃' | '#' | '（' | '(' | '「' | '【') { cut = index; break; }
        if !space(c) { seen = true; }
    }
    value[..cut].trim_matches(space).to_string()
}

/// Matching-character count from Python difflib.SequenceMatcher (no junk),
/// including its popular-character heuristic for sequences of length >= 200.
fn sequence_matches(a: &[char], b: &[char]) -> usize {
    use std::collections::BTreeMap;
    let mut positions: BTreeMap<char, Vec<usize>> = BTreeMap::new();
    for (j, &c) in b.iter().enumerate() { positions.entry(c).or_default().push(j); }
    if b.len() >= 200 { let threshold = b.len() / 100 + 1; positions.retain(|_, v| v.len() <= threshold); }
    let mut ranges = vec![(0, a.len(), 0, b.len())];
    let mut matches = 0;
    while let Some((alo, ahi, blo, bhi)) = ranges.pop() {
        let (mut ai, mut bj, mut best) = (alo, blo, 0usize);
        let mut previous: BTreeMap<usize, usize> = BTreeMap::new();
        for i in alo..ahi {
            let mut current = BTreeMap::new();
            if let Some(js) = positions.get(&a[i]) {
                for &j in js.iter().filter(|&&j| j >= blo && j < bhi) {
                    let size = j.checked_sub(1).and_then(|j| previous.get(&j)).copied().unwrap_or(0) + 1;
                    current.insert(j, size);
                    if size > best { ai = i + 1 - size; bj = j + 1 - size; best = size; }
                }
            }
            previous = current;
        }
        while ai > alo && bj > blo && a[ai - 1] == b[bj - 1] { ai -= 1; bj -= 1; best += 1; }
        while ai + best < ahi && bj + best < bhi && a[ai + best] == b[bj + best] { best += 1; }
        if best > 0 {
            matches += best;
            if alo < ai && blo < bj { ranges.push((alo, ai, blo, bj)); }
            if ai + best < ahi && bj + best < bhi { ranges.push((ai + best, ahi, bj + best, bhi)); }
        }
    }
    matches
}

/// L2-04 for unmerged CSV/TSV with AI unavailable. Category values whose
/// comparison would exceed the bounded work limit return None, not a pass.
pub fn csv_choice_standardization(sheet: &Sheet, table: CellRange, body_first: u32, body_last: u32, schemas: &[(u32, crate::machine_readability_layout::InferredColumnType)]) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    use crate::machine_readability_layout::InferredColumnType;
    if table.first_row > table.last_row || table.first_column > table.last_column || !sheet.merges.is_empty() { return None; }
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let schema_map: BTreeMap<_, _> = schemas.iter().copied().collect();
    let mut columns: BTreeMap<u32, Vec<(u32, &str)>> = BTreeMap::new();
    for cell in sheet.cells.iter().filter(|c| c.row >= body_first && c.row <= body_last && c.column >= table.first_column && c.column <= table.last_column) {
        let value = match &cell.value { CellValue::Text(v) => v.trim_matches(space), CellValue::Empty => "", _ => return None };
        if !value.is_empty() { columns.entry(cell.column).or_default().push((cell.row, value)); }
    }
    let mut analyzed = false;
    let mut findings = Vec::new();
    for col in table.first_column..=table.last_column {
        if matches!(schema_map.get(&col), Some(InferredColumnType::Numeric | InferredColumnType::Date | InferredColumnType::Empty)) { continue; }
        let Some(cells) = columns.get_mut(&col) else { continue; };
        cells.sort_by_key(|v| v.0);
        if cells.len() < 20 { continue; }
        let mut values: Vec<(&str, Vec<u32>)> = Vec::new();
        for &(row, value) in cells.iter() {
            if let Some((_, rows)) = values.iter_mut().find(|(v, _)| *v == value) { rows.push(row); } else { values.push((value, vec![row])); }
            if values.len() > 20 { break; }
        }
        if values.len() > 20 { continue; }
        analyzed = true;
        let outlier = |rows: &[u32]| rows.len() <= 2 && rows.len() * 20 <= cells.len();
        let dominant: Vec<Vec<char>> = values.iter().filter(|(_, rows)| !outlier(rows)).map(|(v, _)| normalized_choice(v).chars().collect()).collect();
        for (value, rows) in values.iter().filter(|(_, rows)| outlier(rows)) {
            let candidate: Vec<_> = normalized_choice(value).chars().collect();
            if candidate.len() > 4096 || dominant.iter().any(|v| v.len() > 4096) { return None; }
            let variant = dominant.iter().any(|v| candidate == *v || (candidate.len().min(v.len()) >= 2 && 8 * sequence_matches(&candidate, v) >= 3 * (candidate.len() + v.len())));
            if variant { findings.push(Finding { location: address(rows[0], col), severity: FindingSeverity::Warning }); }
        }
    }
    Some(result("L2-04", findings.is_empty(), if analyzed { 0.7 } else { 0.0 }, Severity::Critical, findings))
}

/// L3-09 for unmerged CSV/TSV header text, with AI unavailable.
pub fn csv_vertical_format(sheet: &Sheet, table: CellRange, header_rows: &[u32]) -> Option<RuleEvaluation> {
    vertical_format(sheet, table, header_rows, &BTreeSet::new())
}

/// L3-09 on reference-stringified XLSX values with Python-falsy numeric cells.
pub fn xlsx_vertical_format(sheet: &Sheet, table: CellRange, header_rows: &[u32], falsy: &BTreeSet<(u32, u32)>) -> Option<RuleEvaluation> {
    vertical_format(sheet, table, header_rows, falsy)
}

fn vertical_format(sheet: &Sheet, table: CellRange, header_rows: &[u32], falsy: &BTreeSet<(u32, u32)>) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    struct Patterns { year: regex::Regex, month: regex::Regex, period: regex::Regex, era: regex::Regex, numbered: regex::Regex }
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    let p = PATTERNS.get_or_init(|| Patterns {
        year: regex::Regex::new(r"^(19|20)\d{2}[\s\x1c-\x1f]*(年|年度)?$").unwrap(),
        month: regex::Regex::new(r"^\d{1,2}[\s\x1c-\x1f]*月$").unwrap(),
        period: regex::Regex::new(r"^(第?\d+[期回]|(上|下|前|後)半期?|[1-4]Q|Q[1-4]|[上下]期)$").unwrap(),
        era: regex::Regex::new(r"^(平成|令和|昭和)(\d{1,2}|元)[\s\x1c-\x1f]*(年|年度)?$|^[RHS]\d{1,2}$").unwrap(),
        numbered: regex::Regex::new(r"^(.+?)[\s\x1c-\x1f_\-]?(\d+)$").unwrap(),
    });
    if table.first_row > table.last_row || table.first_column > table.last_column || !sheet.merges.is_empty() { return None; }
    let Some(&header) = header_rows.last() else { return Some(result("L3-09", true, 0.5, Severity::Major, Vec::new())); };
    let width = u64::from(table.last_column) - u64::from(table.first_column) + 1;
    if width < 4 { return Some(result("L3-09", true, 0.8, Severity::Major, Vec::new())); }
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut cells = BTreeMap::new();
    for cell in sheet.cells.iter().filter(|c| c.row == header && c.column >= table.first_column && c.column <= table.last_column) {
        let value = match &cell.value { CellValue::Text(v) => Some(v.as_str()), CellValue::Empty => None, _ => return None };
        cells.insert(cell.column, value);
    }
    let location = |column| if cells.contains_key(&column) { address(header, column) } else { format!("R{}C{}", u64::from(header) + 1, u64::from(column) + 1) };
    let range = |start, end| format!("{}:{}", location(start), location(end));
    let mut findings = Vec::new();
    if width > 100 { findings.push(Finding { location: range(table.first_column, table.last_column), severity: FindingSeverity::Error }); }
    let mut groups: [Vec<u32>; 4] = std::array::from_fn(|_| Vec::new());
    for (&col, value) in &cells {
        if let Some(value) = value {
            let value = value.trim_matches(space);
            if p.year.is_match(value) { groups[0].push(col); } else if p.month.is_match(value) { groups[1].push(col); } else if p.period.is_match(value) { groups[2].push(col); } else if p.era.is_match(value) { groups[3].push(col); }
        }
    }
    if let Some(wide) = groups.iter().find(|v| v.len() >= 3) {
        findings.push(Finding { location: range(wide[0], *wide.last()?), severity: FindingSeverity::Warning });
    } else {
        let nonempty: Vec<_> = cells.iter().filter_map(|(&col, value)| value.filter(|v| !v.is_empty() && !falsy.contains(&(header, col))).map(|v| (col, v.trim_matches(space)))).collect();
        if nonempty.len() >= 5 {
            let mut groups: Vec<(String, Vec<u32>)> = Vec::new();
            for (col, value) in nonempty {
                if let Some(c) = p.numbered.captures(value) {
                    let base = c.get(1)?.as_str().trim_matches(space);
                    if base.chars().all(|c| c.is_ascii_digit()) { continue; }
                    if base.chars().all(|c| c.is_numeric()) { return None; }
                    if let Some((_, cols)) = groups.iter_mut().find(|(v, _)| v == base) { cols.push(col); } else { groups.push((base.to_string(), vec![col])); }
                }
            }
            for (_, cols) in groups { if cols.len() >= 4 { findings.push(Finding { location: range(cols[0], *cols.last()?), severity: FindingSeverity::Info }); } }
        }
    }
    Some(result("L3-09", findings.is_empty(), 0.7, Severity::Major, findings))
}

/// L3-05 for unmerged CSV/TSV text cells and known layout rows.
pub fn csv_time_axis(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    struct Patterns { wareki: regex::Regex, wareki_start: regex::Regex, seireki: regex::Regex, nendo: regex::Regex, estat: regex::Regex, iso: regex::Regex, jis: regex::Regex, nonstandard: regex::Regex, keyword: regex::Regex }
    static P: OnceLock<Patterns> = OnceLock::new();
    let p = P.get_or_init(|| Patterns {
        wareki: regex::Regex::new(r"(明治|大正|昭和|平成|令和|M|T|S|H|R)[\s\x1c-\x1f]*\d{1,2}[\s\x1c-\x1f]*(年|\.|-|/)").unwrap(),
        wareki_start: regex::Regex::new(r"^(明治|大正|昭和|平成|令和|M|T|S|H|R)[\s\x1c-\x1f]*\d{1,2}[\s\x1c-\x1f]*(年|\.|-|/)").unwrap(),
        seireki: regex::Regex::new(r"^(19|20)\d{2}[\s\x1c-\x1f]*(年|/|-|\.)").unwrap(),
        nendo: regex::Regex::new(r"^(19|20)\d{2}[\s\x1c-\x1f]*年度").unwrap(),
        estat: regex::Regex::new(r"^(19|20)\d{2}(年\d{1,2}月(\d{1,2}日)?|/\d{1,2}(/\d{1,2})?|-\d{1,2}(-\d{1,2})?)$").unwrap(),
        iso: regex::Regex::new(r"^\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})?)?$").unwrap(),
        jis: regex::Regex::new(r"^[MTSHR]\d{1,2}\.\d{1,2}\.\d{1,2}(T\d{2}:\d{2}(:\d{2}(\.\d+)?)?)?$").unwrap(),
        nonstandard: regex::Regex::new(r"^(\d{2}/\d{1,2}/\d{1,2}|(明治|大正|昭和|平成|令和)\d{1,2})$").unwrap(),
        keyword: regex::Regex::new(r"(?i)(年|月|日|年度|時期|期間|期日|日付|日時|時点|年月|調査時点|対象期間|date|year|month|period)").unwrap(),
    });
    if table.first_row > table.last_row || table.first_column > table.last_column || !sheet.merges.is_empty() { return None; }
    let Some(&header) = header_rows.last() else { return Some(result("L3-05", true, 0.5, Severity::Major, Vec::new())); };
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut cells = BTreeMap::new();
    for cell in sheet.cells.iter().filter(|c| c.column >= table.first_column && c.column <= table.last_column && (c.row == header || (c.row >= body_first && c.row <= body_last))) {
        let value = match &cell.value { CellValue::Text(v) => v.as_str(), CellValue::Empty => "", _ => return None };
        cells.insert((cell.row, cell.column), value);
    }
    let mut time_cols = Vec::new();
    let mut findings = Vec::new();
    for col in table.first_column..=table.last_column {
        if let Some(&value) = cells.get(&(header, col)) {
            if !value.is_empty() && p.keyword.is_match(value) { time_cols.push(col); }
            let value = value.trim_matches(space);
            if !p.iso.is_match(value) && !p.jis.is_match(value) && p.wareki_start.is_match(value) {
                findings.push(Finding { location: address(header, col), severity: FindingSeverity::Warning });
            }
        }
    }
    for col in time_cols {
        let mut formats = BTreeSet::new();
        for row in body_first..=body_last {
            let Some(value) = cells.get(&(row, col)).map(|v| v.trim_matches(space)) else { continue; };
            if value.is_empty() { continue; }
            let format = if p.jis.is_match(value) { Some("jis_wareki") }
                else if p.iso.is_match(value) { Some(if value.contains('T') { "iso8601_datetime" } else { "iso8601_date" }) }
                else if p.wareki.is_match(value) { Some("wareki") }
                else if p.nonstandard.is_match(value) { Some("nonstandard") }
                else if p.nendo.is_match(value) { Some("nendo") }
                else if p.estat.is_match(value) { Some(if value.contains('年') { "estat_kanji" } else if value.contains('/') { "estat_slash" } else { "estat_hyphen" }) }
                else if p.seireki.is_match(value) { Some("seireki_other") } else { None };
            if let Some(format) = format {
                if format == "wareki" || format == "nonstandard" { findings.push(Finding { location: address(row, col), severity: if format == "wareki" { FindingSeverity::Error } else { FindingSeverity::Warning } }); }
                else { formats.insert(format); }
            }
        }
        if formats.len() > 1 { findings.push(Finding { location: address(header, col), severity: FindingSeverity::Warning }); }
    }
    Some(result("L3-05", findings.is_empty(), 0.75, Severity::Major, findings))
}

/// L3-06 for unmerged CSV/TSV cells. Embedded region-name sets are derived
/// from the pinned Harunobu resource, not from a runtime Python installation.
pub fn csv_region_codes(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32) -> Option<RuleEvaluation> {
    region_codes(sheet, table, header_rows, body_first, body_last, &BTreeSet::new())
}

/// L3-06 on reference-stringified XLSX values with Python-falsy numeric cells.
pub fn xlsx_region_codes(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32, falsy: &BTreeSet<(u32, u32)>) -> Option<RuleEvaluation> {
    region_codes(sheet, table, header_rows, body_first, body_last, falsy)
}

fn region_codes(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32, falsy: &BTreeSet<(u32, u32)>) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    static DICT: OnceLock<[BTreeSet<&'static str>; 4]> = OnceLock::new();
    static KEYWORDS: OnceLock<regex::Regex> = OnceLock::new();
    let dict = DICT.get_or_init(|| {
        let mut sets = std::array::from_fn(|_| BTreeSet::new());
        for line in include_str!("../resources/harunobu-region-names.tsv").lines() {
            if let Some((tag, value)) = line.split_once('\t') {
                let index = match tag { "P" => 0, "S" => 1, "M" => 2, "A" => 3, _ => continue };
                sets[index].insert(value);
            }
        }
        sets
    });
    let keywords = KEYWORDS.get_or_init(|| regex::Regex::new(r"(?i)(都道府県|地域|地区|市区町村|自治体|所在地|住所|県|府|市|町|村|region|prefecture|city|address|地域コード|団体コード)").unwrap());
    if table.first_row > table.last_row || table.first_column > table.last_column || !sheet.merges.is_empty() { return None; }
    let Some(&header) = header_rows.last() else { return Some(result("L3-06", true, 0.5, Severity::Critical, Vec::new())); };
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut cells = BTreeMap::new();
    for cell in sheet.cells.iter().filter(|c| c.column >= table.first_column && c.column <= table.last_column && (c.row == header || (c.row >= body_first && c.row <= body_last))) {
        let value = match &cell.value { CellValue::Text(v) => v.as_str(), CellValue::Empty => "", _ => return None };
        cells.insert((cell.row, cell.column), value);
    }
    let mut region_cols = Vec::new();
    for col in table.first_column..=table.last_column {
        if let Some(value) = cells.get(&(header, col)).filter(|v| !v.is_empty()) {
            let value = value.trim_matches(space);
            if keywords.is_match(value) { region_cols.push((col, value)); }
        }
    }
    if region_cols.is_empty() {
        for col in table.first_column..=table.last_column {
            let mut total = 0usize; let mut count = 0usize;
            for row in body_first..=body_last.min(body_first.saturating_add(29)) {
                if let Some(value) = cells.get(&(row, col)).filter(|v| !v.is_empty() && !falsy.contains(&(row, col))) {
                    total += 1;
                    if dict.iter().any(|set| set.contains(value.trim_matches(space))) { count += 1; }
                }
            }
            if total > 0 && count * 2 >= total { region_cols.push((col, cells.get(&(header, col)).copied().unwrap_or("").trim_matches(space))); }
        }
    }
    if region_cols.is_empty() { return Some(result("L3-06", true, 0.6, Severity::Critical, Vec::new())); }
    let has_code = region_cols.iter().any(|(_, h)| h.contains("コード") || h.to_lowercase().contains("code"));
    let mut info = false; let mut findings = Vec::new();
    for (col, _) in region_cols {
        for row in body_first..=body_last {
            let Some(value) = cells.get(&(row, col)).map(|v| v.trim_matches(space)).filter(|v| !v.is_empty()) else { continue; };
            let abbreviated = (dict[1].contains(value) && !dict[0].contains(value)) || (dict[3].contains(value) && !dict[2].contains(value));
            if abbreviated { findings.push(Finding { location: address(row, col), severity: FindingSeverity::Warning }); }
            else if !info && !has_code && (dict[0].contains(value) || dict[2].contains(value)) {
                findings.push(Finding { location: address(row, col), severity: FindingSeverity::Info }); info = true;
            }
        }
    }
    Some(result("L3-06", findings.is_empty(), 0.70, Severity::Critical, findings))
}

/// L3-04 for unmerged CSV/TSV text cells, with AI unavailable.
pub fn csv_data_units(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32) -> Option<RuleEvaluation> {
    data_units(sheet, table, header_rows, body_first, body_last, &BTreeSet::new())
}

/// L3-04 on reference-stringified XLSX values, with typed numeric positions.
pub fn xlsx_data_units(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32, typed_numeric: &BTreeSet<(u32, u32)>) -> Option<RuleEvaluation> {
    data_units(sheet, table, header_rows, body_first, body_last, typed_numeric)
}

fn data_units(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32, typed_numeric: &BTreeSet<(u32, u32)>) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    struct Patterns { numeric: regex::Regex, unit: regex::Regex }
    static P: OnceLock<Patterns> = OnceLock::new();
    let p = P.get_or_init(|| Patterns {
        numeric: regex::Regex::new(r"^[\s\x1c-\x1f\-ー−▲△]*[\d,]+\.?\d*[\s\x1c-\x1f]*$").unwrap(),
        unit: regex::Regex::new(r"[（(](人|件|円|千円|百万円|億円|万円|%|％|kg|km|m|cm|mm|g|t|ha|㎡|㎢|個|台|回|本|枚|箇所|か所|世帯|戸|社|校|棟|隻|機|両|組|点|冊|食|泊|日|月|年|時間|分|秒|歳|才|度|kW|kWh|MW|GW|L|mL|dL)[）)]").unwrap(),
    });
    if table.first_row > table.last_row || table.first_column > table.last_column || !sheet.merges.is_empty() { return None; }
    let Some(&header) = header_rows.last() else { return Some(result("L3-04", true, 0.5, Severity::Major, Vec::new())); };
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut cells = BTreeMap::new();
    for cell in sheet.cells.iter().filter(|c| c.column >= table.first_column && c.column <= table.last_column && (c.row == header || (c.row >= body_first && c.row <= body_last))) {
        match &cell.value { CellValue::Text(v) => { cells.insert((cell.row, cell.column), v.as_str()); }, CellValue::Empty => {}, _ => return None }
    }
    let implied = ["金額","価格","費用","コスト","売上","収入","支出","人口","人数","従業員数","利用者数","面積","距離","長さ","高さ","幅","重量","重さ","質量","割合","率","比率","構成比","年齢","年数","温度","気温"];
    let suffix = ["件数","枚数","回数","か所数","箇所数","人数","数"];
    let unitless = ["コード","code","番号","No","ID","指数","指標","順位","ランク","フラグ","区分"];
    let english = ["id","code","key","name","type","flag","status","index","sort","rank","order","level","class","year","month","date","fiscal_year","survey_year","number","count","cnt","num","no","row","col","seq","version","is","has","will","can","use"];
    let mut findings = Vec::new();
    for col in table.first_column..=table.last_column {
        let mut total = 0usize; let mut numeric = 0usize;
        for row in body_first..=body_last.min(body_first.saturating_add(29)) {
            if let Some(value) = cells.get(&(row, col)) { total += 1; if typed_numeric.contains(&(row, col)) || p.numeric.is_match(value.trim_matches(space)) { numeric += 1; } }
        }
        if total == 0 || numeric * 10 < total * 7 { continue; }
        let Some(header) = cells.get(&(header, col)).map(|v| v.trim_matches(space)) else { continue; };
        use unicode_normalization::UnicodeNormalization;
        let mut digits = !header.is_empty();
        let mut unknown_digit = false;
        for ch in header.chars() {
            if ch.is_ascii_digit() { continue; }
            if !ch.is_numeric() { digits = false; unknown_digit = false; break; }
            let text = ch.to_string();
            let normalized: Vec<_> = text.nfkc().collect();
            if normalized.len() != 1 || !normalized[0].is_ascii_digit() { unknown_digit = true; }
        }
        if digits && unknown_digit { return None; }
        let lowered = header.to_lowercase().replace('-', "_");
        if p.unit.is_match(header) || implied.iter().any(|v| header.contains(v)) || suffix.iter().any(|v| header.ends_with(v)) || unitless.iter().any(|v| header.contains(v)) || digits || lowered.split('_').any(|v| english.contains(&v)) { continue; }
        findings.push(Finding { location: address(*header_rows.last()?, col), severity: FindingSeverity::Warning });
    }
    Some(result("L3-04", findings.is_empty(), 0.65, Severity::Major, findings))
}

/// L1-03 for an unmerged CSV sheet with all detected table ranges supplied.
/// Inter-table blank rows are attributed only to the first table in row order.
pub fn csv_fragmentation_with_tables(sheet: &Sheet, current: CellRange, tables: &[DetectedTable]) -> Option<RuleEvaluation> {
    if tables.is_empty() || tables.iter().any(|t| t.range.first_row > t.range.last_row || t.range.first_column > t.range.last_column) { return None; }
    if !tables.iter().any(|t| t.range == current) { return None; }
    let mut checks = csv_data_fragmentation(sheet, current)?;
    if tables.len() <= 1 { return Some(checks); }
    let mut sorted = tables.to_vec();
    sorted.sort_by_key(|t| (t.range.first_row, t.range.first_column));
    if (current.first_row, current.first_column) != (sorted[0].range.first_row, sorted[0].range.first_column) { return Some(checks); }
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut occupied = BTreeSet::new();
    for cell in sheet.cells.iter() {
        match &cell.value {
            CellValue::Text(value) => if !value.trim_matches(space).is_empty() { occupied.insert(cell.row); },
            CellValue::Empty => {},
            _ => return None,
        }
    }
    for pair in sorted.windows(2) {
        let prev = pair[0].range; let next = pair[1].range;
        if prev.last_row >= next.first_row { continue; }
        for row in (u64::from(prev.last_row) + 1)..u64::from(next.first_row) {
            if !occupied.contains(&(row as u32)) { checks.findings.push(Finding { location: format!("行{}", row + 1), severity: FindingSeverity::Error }); }
        }
    }
    checks.result.passed = checks.findings.is_empty();
    checks.result.confidence = if checks.result.passed { 0.85 } else { 0.9 };
    Some(checks)
}

/// A formula or cached #REF! reference found in the original OOXML package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormulaArtifact { pub row: u32, pub column: u32, pub cell_ref: String }

/// L2-06 over authoritative package-wide inspection results. `None` means the
/// inspection failed or was incomplete and is reported as a skipped rule.
pub fn formula_artifacts(table: CellRange, references: Option<(&[FormulaArtifact], &[FormulaArtifact])>) -> RuleEvaluation {
    let Some((formulas, errors)) = references else { return result("L2-06", true, 0.0, Severity::Major, Vec::new()); };
    if table.first_row > table.last_row || table.first_column > table.last_column { return result("L2-06", true, 0.0, Severity::Major, Vec::new()); }
    let within = |v: &&FormulaArtifact| v.row >= table.first_row && v.row <= table.last_row && v.column >= table.first_column && v.column <= table.last_column;
    let mut seen = BTreeSet::new();
    let mut findings = Vec::new();
    for artifact in errors.iter().filter(within) {
        seen.insert((artifact.row, artifact.column));
        findings.push(Finding { location: artifact.cell_ref.clone(), severity: FindingSeverity::Error });
    }
    for artifact in formulas.iter().filter(within) {
        if seen.insert((artifact.row, artifact.column)) { findings.push(Finding { location: artifact.cell_ref.clone(), severity: FindingSeverity::Error }); }
    }
    result("L2-06", findings.is_empty(), 1.0, Severity::Major, findings)
}

/// L1-03 for XLSX values and merge metadata from the reference reader window.
/// `read_only` is true for the reference's large-file two-pass reader, which
/// can retain cached values in non-master cells of merged regions.
pub fn xlsx_data_fragmentation(sheet: &Sheet, current: CellRange, tables: &[DetectedTable], read_only: bool) -> Option<RuleEvaluation> {
    if tables.is_empty() || !tables.iter().any(|t| t.range == current) || tables.iter().any(|t| t.range.first_row > t.range.last_row || t.range.first_column > t.range.last_column) { return None; }
    if sheet.merges.iter().any(|m| m.first_row > m.last_row || m.first_column > m.last_column) { return None; }
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let is_nonmaster = |row: u32, col: u32| sheet.merges.iter().any(|m| row >= m.first_row && row <= m.last_row && col >= m.first_column && col <= m.last_column && col != m.first_column);
    let is_merged_dependent = |row: u32, col: u32| sheet.merges.iter().any(|m| row >= m.first_row && row <= m.last_row && col >= m.first_column && col <= m.last_column && (row != m.first_row || col != m.first_column));
    let mut occupied_rows = BTreeSet::new(); let mut occupied_cols = BTreeSet::new(); let mut all_occupied_rows = BTreeSet::new();
    for cell in sheet.cells.iter() {
        if !read_only && is_merged_dependent(cell.row, cell.column) { continue; }
        let nonempty = match &cell.value {
            CellValue::Empty => false,
            CellValue::Text(value) => !value.starts_with('=') && !value.trim_matches(space).is_empty(),
            CellValue::Number(_) | CellValue::Bool(_) | CellValue::Error(_) => true,
        };
        if !nonempty { continue; }
        all_occupied_rows.insert(cell.row);
        if cell.row >= current.first_row && cell.row <= current.last_row && cell.column >= current.first_column && cell.column <= current.last_column {
            occupied_rows.insert(cell.row); occupied_cols.insert(cell.column);
        }
    }
    let mut findings = Vec::new();
    if u64::from(current.last_row) - u64::from(current.first_row) + 1 > 2 {
        for row in current.first_row..=current.last_row { if !occupied_rows.contains(&row) { findings.push(Finding { location: format!("行{}", u64::from(row) + 1), severity: FindingSeverity::Error }); } }
        for col in current.first_column..=current.last_column {
            if !occupied_cols.contains(&col) && !(current.first_row..=current.last_row).all(|row| is_nonmaster(row, col)) {
                findings.push(Finding { location: format!("列{}", u64::from(col) + 1), severity: FindingSeverity::Error });
            }
        }
    }
    if tables.len() > 1 {
        let mut sorted = tables.to_vec(); sorted.sort_by_key(|t| (t.range.first_row, t.range.first_column));
        if (current.first_row, current.first_column) == (sorted[0].range.first_row, sorted[0].range.first_column) {
            for pair in sorted.windows(2) {
                let prev = pair[0].range; let next = pair[1].range;
                if prev.last_row >= next.first_row { continue; }
                for row in (u64::from(prev.last_row) + 1)..u64::from(next.first_row) {
                    if !all_occupied_rows.contains(&(row as u32)) { findings.push(Finding { location: format!("行{}", row + 1), severity: FindingSeverity::Error }); }
                }
            }
        }
    }
    Some(result("L1-03", findings.is_empty(), if findings.is_empty() { 0.85 } else { 0.9 }, Severity::Critical, findings))
}

/// L1-05 for XLSX, respecting header merges and saved typed cell values.
pub fn xlsx_column_headers(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32, read_only: bool) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    use crate::machine_readability_xlsx::{classify_xlsx_number_format, XlsxNumberFormat};
    static NUMBER: OnceLock<regex::Regex> = OnceLock::new();
    let number = NUMBER.get_or_init(|| regex::Regex::new(r"^[+-]?(?:(?:[0-9](?:_?[0-9])*)(?:\.(?:[0-9](?:_?[0-9])*)?)?|\.[0-9](?:_?[0-9])*)(?:[eE][+-]?[0-9](?:_?[0-9])*)?$").unwrap());
    if table.first_row > table.last_row || table.first_column > table.last_column || sheet.merges.iter().any(|m| m.first_row > m.last_row || m.first_column > m.last_column) { return None; }
    let fallback = [table.first_row];
    let headers = if header_rows.is_empty() { &fallback[..] } else { header_rows };
    let last_header = *headers.last()?;
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut cells = BTreeMap::new();
    let mut body_columns = BTreeSet::new(); let mut header_columns = BTreeSet::new();
    for cell in sheet.cells.iter() {
        let dependent = sheet.merges.iter().any(|m| cell.row >= m.first_row && cell.row <= m.last_row && cell.column >= m.first_column && cell.column <= m.last_column && (cell.row != m.first_row || cell.column != m.first_column));
        if !read_only && dependent { continue; }
        let nonempty = match &cell.value { CellValue::Empty => false, CellValue::Text(v) => !v.starts_with('=') && !v.trim_matches(space).is_empty(), _ => true };
        if nonempty {
            cells.insert((cell.row, cell.column), cell);
            if cell.row >= body_first && cell.row <= body_last { body_columns.insert(cell.column); }
            if headers.contains(&cell.row) { header_columns.insert(cell.column); }
        }
    }
    let mut covered = BTreeSet::new();
    for merge in &sheet.merges {
        let overlaps = headers.iter().any(|&r| r >= merge.first_row && r <= merge.last_row) || (merge.first_row < headers[0] && merge.last_row >= headers[0]);
        if overlaps && cells.contains_key(&(merge.first_row, merge.first_column)) {
            for col in merge.first_column.max(table.first_column)..=merge.last_column.min(table.last_column) { covered.insert(col); }
        }
    }
    let mut findings = Vec::new();
    for col in table.first_column..=table.last_column {
        if covered.contains(&col) { continue; }
        if body_columns.contains(&col) && !header_columns.contains(&col) { findings.push(Finding { location: address(headers[0], col), severity: FindingSeverity::Error }); }
    }
    let width = u64::from(table.last_column) - u64::from(table.first_column) + 1;
    if width >= 2 {
        let mut nums = Vec::new(); let mut unknown = false; let mut nonnumeric = false;
        for col in table.first_column..=table.last_column {
            let Some(cell) = cells.get(&(last_header, col)) else { nonnumeric = true; break; };
            let n = match &cell.value {
                CellValue::Number(n) => {
                    let style = sheet.styles.get(cell.style as usize)?;
                    if classify_xlsx_number_format(&style.number_format) != XlsxNumberFormat::Numeric { nonnumeric = true; break; }
                    *n
                }
                CellValue::Text(value) => {
                    let value = value.trim_matches(space);
                    if !number.is_match(value) {
                        let numeric_shape = value.chars().any(|c| !c.is_ascii() && c.is_numeric()) && value.chars().all(|c| c.is_numeric() || matches!(c, '+' | '-' | '.' | '_' | 'e' | 'E'));
                        if numeric_shape || matches!(value.to_ascii_lowercase().as_str(), "nan" | "+nan" | "-nan" | "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity") { unknown = true; continue; }
                        nonnumeric = true; break;
                    }
                    value.replace('_', "").parse::<f64>().ok()?
                }
                _ => { nonnumeric = true; break; }
            };
            if !n.is_finite() || n.abs() >= 9_007_199_254_740_992.0 { unknown = true; continue; }
            nums.push(n);
        }
        if unknown && !nonnumeric { return None; }
        if !nonnumeric && nums.len() as u64 == width && nums.iter().all(|n| n.fract() == 0.0) && nums.windows(2).all(|v| v[1] - v[0] == 1.0) {
            for col in table.first_column..=table.last_column { findings.push(Finding { location: address(last_header, col), severity: FindingSeverity::Error }); }
        }
    }
    Some(result("L1-05", findings.is_empty(), if findings.is_empty() { 0.8 } else { 0.85 }, Severity::Critical, findings))
}

/// A drawing anchor observed in the original XLSX package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectArtifact { pub row: u32, pub column: u32, pub kind: String, pub name: Option<String>, pub drawing_part: String }

/// L1-10; absent inspection data produces the same skip as Harunobu.
pub fn object_artifacts(table: CellRange, abstain: bool, objects: Option<&[ObjectArtifact]>) -> RuleEvaluation {
    let Some(objects) = objects else { return result("L1-10", true, 0.0, Severity::Critical, Vec::new()); };
    if table.first_row > table.last_row || table.first_column > table.last_column { return result("L1-10", true, 0.0, Severity::Critical, Vec::new()); }
    let findings: Vec<_> = objects.iter().filter(|v| abstain || (v.row >= table.first_row && v.row <= table.last_row && v.column >= table.first_column && v.column <= table.last_column)).map(|v| Finding { location: address(v.row, v.column), severity: FindingSeverity::Error }).collect();
    result("L1-10", findings.is_empty(), 1.0, Severity::Critical, findings)
}

/// Reader-normalized direct formatting used by the reference L1-11 rule.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferenceCellFormat { pub font_color: Option<String>, pub background: Option<String>, pub stub_background_nonwhite: bool, pub bold: bool, pub italic: bool, pub border_left: bool, pub border_right: bool }

/// L1-11; incomplete formatting metadata abstains instead of producing a pass.
pub fn format_based_semantics(sheet: &Sheet, table: CellRange, body_first: u32, formats: Option<&std::collections::BTreeMap<(u32, u32), ReferenceCellFormat>>) -> RuleEvaluation {
    use std::collections::BTreeMap;
    let Some(formats) = formats else { return result("L1-11", true, 0.0, Severity::Critical, Vec::new()); };
    if table.first_row > table.last_row || table.first_column > table.last_column { return result("L1-11", true, 0.0, Severity::Critical, Vec::new()); }
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut columns: BTreeMap<u32, Vec<&ReferenceCellFormat>> = BTreeMap::new();
    for cell in sheet.cells.iter().filter(|c| c.row >= body_first && c.row <= table.last_row && c.column >= table.first_column && c.column <= table.last_column) {
        let nonempty = match &cell.value { CellValue::Empty => false, CellValue::Text(v) => !v.trim_matches(space).is_empty(), _ => true };
        if !nonempty { continue; }
        let Some(format) = formats.get(&(cell.row, cell.column)) else { return result("L1-11", true, 0.0, Severity::Critical, Vec::new()); };
        columns.entry(cell.column).or_default().push(format);
    }
    let mut findings = Vec::new();
    for (col, cells) in columns {
        if cells.len() < 2 { continue; }
        let font: BTreeSet<_> = cells.iter().map(|v| v.font_color.as_deref()).collect();
        let background: BTreeSet<_> = cells.iter().map(|v| v.background.as_deref()).collect();
        let bold = cells.iter().filter(|v| v.bold).count();
        let italic = cells.iter().filter(|v| v.italic).count();
        let location = format!("列{}", u64::from(col) + 1);
        if font.len() >= 2 && font.iter().any(|v| !matches!(v, None | Some("000000") | Some("FF000000"))) { findings.push(Finding { location: location.clone(), severity: FindingSeverity::Warning }); }
        if background.len() >= 2 { findings.push(Finding { location: location.clone(), severity: FindingSeverity::Warning }); }
        if bold > 0 && bold < cells.len() { findings.push(Finding { location: location.clone(), severity: FindingSeverity::Info }); }
        if italic > 0 && italic < cells.len() { findings.push(Finding { location, severity: FindingSeverity::Info }); }
    }
    result("L1-11", findings.is_empty(), 0.75, Severity::Critical, findings)
}

/// L2-03 over reference-stringified XLSX cells and source merge metadata.
pub fn xlsx_unique_headers(sheet: &Sheet, table: CellRange, header_rows: &[u32], body_first: u32, body_last: u32, merges: &[CellRange]) -> Option<RuleEvaluation> {
    use std::collections::BTreeMap;
    if table.first_row > table.last_row || table.first_column > table.last_column || merges.iter().any(|m| m.first_row > m.last_row || m.first_column > m.last_column) { return None; }
    let Some(&last_header) = header_rows.last() else { return Some(result("L2-03", true, 0.4, Severity::Fatal, Vec::new())); };
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    let mut cells = BTreeMap::new(); let mut body_columns = BTreeSet::new();
    for cell in sheet.cells.iter().filter(|c| c.column >= table.first_column && c.column <= table.last_column) {
        let value = match &cell.value { CellValue::Text(v) => v.trim_matches(space), CellValue::Empty => "", _ => return None };
        if cell.row >= body_first && cell.row <= body_last && !value.is_empty() { body_columns.insert(cell.column); }
        cells.insert((cell.row, cell.column), value);
    }
    let location = |col| if cells.contains_key(&(last_header, col)) { address(last_header, col) } else { format!("R{}C{}", u64::from(last_header) + 1, u64::from(col) + 1) };
    let mut names: Vec<(String, Vec<u32>)> = Vec::new(); let mut seen_merges = BTreeSet::new(); let mut findings = Vec::new();
    for col in table.first_column..=table.last_column {
        let merge_at = |row| merges.iter().find(|m| row >= m.first_row && row <= m.last_row && col >= m.first_column && col <= m.last_column);
        let dependent = header_rows.iter().any(|&row| merge_at(row).is_some_and(|m| row != m.first_row || col != m.first_column));
        if dependent {
            let merged = header_rows.iter().find_map(|&row| merge_at(row))?;
            let label = if merged.first_row == merged.last_row && merged.first_column == merged.last_column { address(merged.first_row, merged.first_column) } else { range_address(*merged) };
            if seen_merges.insert(label.clone()) { findings.push(Finding { location: label, severity: FindingSeverity::Error }); }
            continue;
        }
        let mut parts = Vec::new();
        for &row in header_rows { if let Some(&value) = cells.get(&(row, col)) { if !value.is_empty() && !parts.contains(&value) { parts.push(value); } } }
        if parts.is_empty() {
            if body_columns.contains(&col) { findings.push(Finding { location: location(col), severity: FindingSeverity::Error }); }
        } else {
            let name = parts.join("_");
            if let Some((_, cols)) = names.iter_mut().find(|(n, _)| n == &name) { cols.push(col); } else { names.push((name, vec![col])); }
        }
    }
    for (_, cols) in names { if cols.len() > 1 { for col in cols { findings.push(Finding { location: location(col), severity: FindingSeverity::Error }); } } }
    Some(result("L2-03", findings.is_empty(), 0.95, Severity::Fatal, findings))
}

/// L3-07 across files, using each file's first available table header labels.
/// Each inner list must already reflect the reference's nonempty-label filter
/// followed by trimming; an empty list denotes a missing extractable header.
pub fn bulk_format_consistency(file_headers: &[Vec<String>]) -> RuleEvaluation {
    if file_headers.len() < 2 { return result("L3-07", true, 0.5, Severity::Critical, Vec::new()); }
    let mut findings = Vec::new();
    if file_headers.iter().any(Vec::is_empty) { findings.push(Finding { location: String::new(), severity: FindingSeverity::Warning }); }
    let files: Vec<_> = file_headers.iter().filter(|v| !v.is_empty()).collect();
    if files.len() < 2 { return result("L3-07", findings.is_empty(), 0.5, Severity::Critical, findings); }
    let sets: Vec<BTreeSet<&str>> = files.iter().map(|v| v.iter().map(String::as_str).collect()).collect();
    let mut min_jaccard = 1.0f64;
    for i in 0..sets.len() { for j in i+1..sets.len() {
        let union = sets[i].union(&sets[j]).count();
        if union > 0 { min_jaccard = min_jaccard.min(sets[i].intersection(&sets[j]).count() as f64 / union as f64); }
    }}
    if min_jaccard < 0.8 { findings.push(Finding { location: String::new(), severity: FindingSeverity::Warning }); }
    let mean = files.iter().map(|v| v.len() as f64).sum::<f64>() / files.len() as f64;
    let deviation = (files.iter().map(|v| (v.len() as f64 - mean).powi(2)).sum::<f64>() / files.len() as f64).sqrt();
    if deviation > 1.5 { findings.push(Finding { location: String::new(), severity: FindingSeverity::Warning }); }
    let common = sets.iter().skip(1).fold(sets[0].clone(), |acc, set| acc.intersection(set).copied().collect());
    if common.len() >= 2 {
        let reference: Vec<_> = files[0].iter().filter(|v| common.contains(v.as_str())).collect();
        if files.iter().skip(1).any(|headers| headers.iter().filter(|v| common.contains(v.as_str())).collect::<Vec<_>>() != reference) {
            findings.push(Finding { location: String::new(), severity: FindingSeverity::Info });
        }
    }
    result("L3-07", findings.is_empty(), 0.8, Severity::Critical, findings)
}
