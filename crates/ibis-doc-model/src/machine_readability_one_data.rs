// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Cell-text building block for Harunobu 0.2.2 L1-07.
//!
//! This does not infer tables, column types, rule results or scores.
//! Based on Digital Agency of Japan code under MIT; see ../HARUNOBU-MIT.txt.

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
    let is_digit = |c: char| digit.is_match(c.encode_utf8(&mut [0; 4]));
    let mut chars = value.chars().peekable();
    let mut previous = None;
    let mut separator = false;
    while let Some(c) = chars.next() {
        if c == ',' && previous.is_none_or(|c| !is_digit(c)) && chars.peek().is_none_or(|&c| !is_digit(c)) {
            separator = true;
            break;
        }
        previous = Some(c);
    }
    if separator && !thousands.is_match(value.trim_matches(python_space)) { return Some("カンマ区切りによる複数データ"); }
    if column != Some(ColumnType::Numeric) && value.contains(',') {
        let compact: String = value.chars().filter(|&c| !python_space(c)).collect();
        let compact = paren.replace_all(&compact, "");
        if compact.contains(',') && !thousands.is_match(&compact) { return Some("カンマ区切りによる複数データ"); }
    }
    None
}
