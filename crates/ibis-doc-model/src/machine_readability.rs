// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Structural review candidates for machine-readable spreadsheets.
//!
//! These are deliberately not Harunobu rule results: no table or header is
//! inferred, so every merge and formula is a candidate for human review.
use crate::sheet::{CellRange, CellValue, Sheet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateKind { MergedCells, HiddenRow, HiddenColumn, Formula }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub kind: CandidateKind,
    /// Inclusive, zero-based cell coordinates.
    pub range: CellRange,
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
