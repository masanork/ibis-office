// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Structural review candidates for machine-readable spreadsheets.
//!
//! These are deliberately not Harunobu rule results: no table or header is
//! inferred, so each finding is a candidate for human review.
use crate::sheet::{CellRange, CellValue, Sheet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateKind { MergedCells, HiddenRow, HiddenColumn, Formula, MultipleData, PlatformDependentCharacters, PrivateUseCharacters }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub kind: CandidateKind,
    /// Inclusive, zero-based cell coordinates.
    pub range: CellRange,
}

/// Character-level candidates based on Harunobu 0.2.2 L1-08. These do not
/// implement its file-encoding policy, table detection, severity or scoring.
/// Copyright (c) 2026 Digital Agency of Japan; see ../HARUNOBU-MIT.txt.
fn platform_character_candidate(value: &str) -> bool {
    const UNITS: &str = "㎡㎥㎢㎠㎝㎞㎜㎏㎎㎍㎅㎄㎃㎖㎗㎘㏄㏊";
    value.chars().any(|c| {
        if matches!(c, '㋿' | '〜' | '～') { return true; }
        if UNITS.contains(c) { return false; }
        matches!(c as u32, 0xff10..=0xff19 | 0xff21..=0xff3a | 0xff41..=0xff5a
            | 0x2460..=0x2473 | 0x3251..=0x325f | 0x32b1..=0x32bf
            | 0x2160..=0x217f | 0x3220..=0x3243 | 0x3280..=0x32fe
            | 0x3300..=0x33ff | 0xfe30..=0xfe4f)
    })
}

/// Advisory for Unicode private-use scalars, independent of L1-08 scoring.
/// Adapted from gjs (commit 9173a4d); see ../GJS-MIT.txt. Unlike gjs, exclude
/// the supplementary planes' ending noncharacters (U+FFFFE, U+FFFFF,
/// U+10FFFE and U+10FFFF).
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
    let data = sheet.cells.iter().filter(|c| !matches!(c.value, CellValue::Empty) || c.formula.as_ref().is_some_and(|f| !f.is_empty()));
    let mut max_row = None::<u32>;
    let mut max_col = None::<u32>;
    for cell in data {
        max_row = Some(max_row.map_or(cell.row, |n| n.max(cell.row)));
        max_col = Some(max_col.map_or(cell.column, |n| n.max(cell.column)));
        if let CellValue::Text(value) = &cell.value {
            let range = CellRange { first_row: cell.row, last_row: cell.row, first_column: cell.column, last_column: cell.column };
            // No schema is known here. The numeric guard avoids making a
            // comma-separated numeric-looking string a confident candidate.
            if crate::machine_readability_one_data::multiple_data_reason(value, Some(crate::machine_readability_one_data::ColumnType::Numeric)).is_some() {
                out.push(Candidate { kind: CandidateKind::MultipleData, range });
            }
            if platform_character_candidate(value) {
                out.push(Candidate { kind: CandidateKind::PlatformDependentCharacters, range });
            }
            if private_use_candidate(value) {
                out.push(Candidate { kind: CandidateKind::PrivateUseCharacters, range });
            }
        }
        if cell.formula.as_ref().is_some_and(|f| !f.is_empty()) {
            out.push(Candidate { kind: CandidateKind::Formula, range: CellRange { first_row: cell.row, last_row: cell.row, first_column: cell.column, last_column: cell.column } });
        }
    }
    if let (Some(last_row), Some(last_col)) = (max_row, max_col) {
        // The sheet model also treats an explicit zero height/width as hidden,
        // even if the saved-height maps have no entry. Collect each index once.
        let mut rows: std::collections::BTreeSet<u32> = sheet.hidden_rows.keys().copied().filter(|&r| r <= last_row).collect();
        rows.extend(sheet.rows.iter().take((last_row as usize).saturating_add(1)).enumerate().filter(|(_, height)| **height == 0).map(|(row, _)| row as u32));
        for row in rows {
            out.push(Candidate { kind: CandidateKind::HiddenRow, range: CellRange { first_row: row, last_row: row, first_column: 0, last_column: last_col } });
        }
        let mut columns: std::collections::BTreeSet<u32> = sheet.hidden_columns.keys().copied().filter(|&c| c <= last_col).collect();
        columns.extend(sheet.columns.iter().take((last_col as usize).saturating_add(1)).enumerate().filter(|(_, width)| **width == 0).map(|(col, _)| col as u32));
        for col in columns {
            out.push(Candidate { kind: CandidateKind::HiddenColumn, range: CellRange { first_row: 0, last_row, first_column: col, last_column: col } });
        }
    }
    out
}
