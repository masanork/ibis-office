// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Structural review candidates for machine-readable spreadsheets.
//!
//! These are deliberately not Harunobu rule results: no table or header is
//! inferred, so every merge and formula is a candidate for human review.
use crate::sheet::{CellRange, CellValue, Sheet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateKind { MergedCells, HiddenRow, HiddenColumn, Formula, PlatformDependentCharacters, PrivateUseCharacters }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub kind: CandidateKind,
    /// Inclusive, zero-based cell coordinates.
    pub range: CellRange,
}

/// Harunobu 0.2.2 L1-08 character categories, applied to live text. This is
/// only a review candidate: table inference, encoding and rule severity are
/// intentionally left to the reference engine.
/// Reference: digital-go-jp/machine_readability_rule, MIT license,
/// harunobu/rules/level1/L1_08_platform_dependent_chars.py (commit 88014ff9).
/// Copyright (c) 2026 Digital Agency of Japan; see ../HARUNOBU-MIT.txt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CharacterRisk { Info, Warning }

/// Character-level classification; table and encoding policy are separate.
pub(crate) fn character_risk(c: char) -> Option<CharacterRisk> {
    const UNITS: &str = "㎡㎥㎢㎠㎝㎞㎜㎏㎎㎍㎅㎄㎃㎖㎗㎘㏄㏊";
    if matches!(c, '㋿' | '〜' | '～') { return Some(CharacterRisk::Warning); }
    if UNITS.contains(c) { return None; }
    let cp = c as u32;
    if matches!(cp, 0xff10..=0xff19 | 0xff21..=0xff3a | 0xff41..=0xff5a) { return Some(CharacterRisk::Info); }
    if matches!(cp, 0x2460..=0x2473 | 0x3251..=0x325f | 0x32b1..=0x32bf
        | 0x2160..=0x217f | 0x3220..=0x3243 | 0x3280..=0x32fe | 0x3300..=0x33ff | 0xfe30..=0xfe4f) {
        return Some(CharacterRisk::Warning);
    }
    None
}

fn platform_character_candidate(value: &str) -> bool { value.chars().any(|c| character_risk(c).is_some()) }

/// Unicode private-use scalars may require a shared font or mapping. This is
/// advisory and is deliberately independent of Harunobu L1-08 scoring.
/// See gjs `src/shared/txtbin/char-analysis.ts` (commit 9173a4d) for the
/// corresponding character categories. Exclude plane-ending noncharacters.
fn private_use_candidate(value: &str) -> bool {
    value.chars().any(|c| matches!(c as u32, 0xe000..=0xf8ff | 0xf0000..=0xffffd | 0x100000..=0x10fffd))
}

/// Inspect the current sheet in memory. Formatting-only cells do not define
/// the data extent; candidates are not compliance decisions or scores.
pub fn candidates(sheet: &Sheet) -> Vec<Candidate> {
    let mut out = Vec::new();
    for &range in &sheet.merges {
        out.push(Candidate { kind: CandidateKind::MergedCells, range });
    }
    let data = sheet.cells.iter().filter(|c| !matches!(c.value, CellValue::Empty) || c.formula.is_some());
    let mut max_row = None::<u32>;
    let mut max_col = None::<u32>;
    for cell in data {
        max_row = Some(max_row.map_or(cell.row, |n| n.max(cell.row)));
        max_col = Some(max_col.map_or(cell.column, |n| n.max(cell.column)));
        if let CellValue::Text(value) = &cell.value {
            if platform_character_candidate(value) {
                out.push(Candidate { kind: CandidateKind::PlatformDependentCharacters, range: CellRange { first_row: cell.row, last_row: cell.row, first_column: cell.column, last_column: cell.column } });
            }
            if private_use_candidate(value) {
                out.push(Candidate { kind: CandidateKind::PrivateUseCharacters, range: CellRange { first_row: cell.row, last_row: cell.row, first_column: cell.column, last_column: cell.column } });
            }
        }
        if cell.formula.as_ref().is_some_and(|f| !f.is_empty()) {
            out.push(Candidate { kind: CandidateKind::Formula, range: CellRange { first_row: cell.row, last_row: cell.row, first_column: cell.column, last_column: cell.column } });
        }
    }
    if let (Some(last_row), Some(last_col)) = (max_row, max_col) {
        for &row in sheet.hidden_rows.keys().filter(|&&r| r <= last_row) {
            out.push(Candidate { kind: CandidateKind::HiddenRow, range: CellRange { first_row: row, last_row: row, first_column: 0, last_column: last_col } });
        }
        for &col in sheet.hidden_columns.keys().filter(|&&c| c <= last_col) {
            out.push(Candidate { kind: CandidateKind::HiddenColumn, range: CellRange { first_row: 0, last_row, first_column: col, last_column: col } });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::platform_character_candidate;

    #[test]
    fn platform_characters_follow_reference_categories() {
        for value in ["①", "㉑", "㊿", "Ⅰ-0", "Ａ", "９", "㈱", "㋿", "〜", "～", "㍻", "︰"] {
            assert!(platform_character_candidate(value), "{value}");
        }
        for value in ["abc日本語123", "㎡", "㎏㎥", "", "Ⓐ", "😀"] {
            assert!(!platform_character_candidate(value), "{value}");
        }
        assert!(platform_character_candidate("面積㎡／Ａ"));
    }
}
