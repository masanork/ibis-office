// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Conservative adapter from a reader-provided XLSX sheet to native layout
//! cells. This is not a complete XLSX reader or a full Harunobu analysis.
use std::collections::{BTreeMap, BTreeSet};
use crate::machine_readability_layout::{LayoutCell, LayoutValue};
use crate::sheet::{CellRange, CellValue, Sheet};

/// Exact zero-based bounds applied by the source reader. The caller must
/// derive these from the XLSX reader, including its row/column limits; they
/// must not be guessed from the displayed or edited sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XlsxReadWindow { Empty, Through { last_row: u32, last_column: u32 } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XlsxLayoutError {
    MissingStyle, DuplicateCell, InvalidMerge, OverlappingMerge, MergeTooLarge,
    NumberFormatRequiresMetadata, EmptyTextRequiresMetadata, NonFiniteNumber, FormulaCacheUnknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XlsxLayoutInput { pub cells: Vec<LayoutCell>, pub merges: Vec<CellRange> }

/// Adapt a sheet only when the caller can guarantee that values are the saved
/// non-formula values, and that cells/styles/merges cover the reference
/// reader's window. Formula values must be supplied from the original XLSX
/// XML, because the editor may have recalculated them. Out-of-range date and
/// duration serials are rejected until authoritative typing is available.
pub fn prepare_xlsx_layout(sheet: &Sheet, window: XlsxReadWindow, formula_cache: &BTreeMap<(u32, u32), CellValue>) -> Result<XlsxLayoutInput, XlsxLayoutError> {
    const MAX_MERGED_CELLS: u64 = 1_000_000;
    let mut cells = BTreeMap::<(u32, u32), LayoutCell>::new();
    let mut seen = BTreeSet::new();
    for cell in sheet.cells.iter() {
        let within = match window { XlsxReadWindow::Empty => false, XlsxReadWindow::Through { last_row, last_column } => cell.row <= last_row && cell.column <= last_column };
        if !within { continue; }
        if !seen.insert((cell.row, cell.column)) { return Err(XlsxLayoutError::DuplicateCell); }
        let style = sheet.styles.get(cell.style as usize).ok_or(XlsxLayoutError::MissingStyle)?;
        let value = if cell.formula.is_some() { formula_cache.get(&(cell.row, cell.column)).ok_or(XlsxLayoutError::FormulaCacheUnknown)? } else { &cell.value };
        let (value, formula) = match value {
            CellValue::Empty => (None, None),
            CellValue::Text(text) if text.is_empty() => return Err(XlsxLayoutError::EmptyTextRequiresMetadata),
            CellValue::Text(text) if text.starts_with('=') => (None, Some(text.clone())),
            CellValue::Text(text) => (Some(LayoutValue::Text(text.clone())), None),
            CellValue::Bool(_) => (Some(LayoutValue::Numeric), None),
            CellValue::Error(error) => (Some(LayoutValue::Text(error.to_string())), None),
            CellValue::Number(value) => {
                if !value.is_finite() { return Err(XlsxLayoutError::NonFiniteNumber); }
                let kind = classify_xlsx_number_format(&style.number_format);
                let layout = match kind {
                    XlsxNumberFormat::Numeric => LayoutValue::Numeric,
                    XlsxNumberFormat::DateTime | XlsxNumberFormat::Duration if !(0.0..=2_900_000.0).contains(value) => return Err(XlsxLayoutError::NumberFormatRequiresMetadata),
                    XlsxNumberFormat::DateTime => LayoutValue::Date,
                    XlsxNumberFormat::Duration => LayoutValue::Other,
                };
                (Some(layout), None)
            }
        };
        if value.is_none() && !style.bold && formula.is_none() { continue; }
        let layout = LayoutCell { row: cell.row, column: cell.column, value, bold: style.bold, merged: false, formula };
        if cells.insert((cell.row, cell.column), layout).is_some() { return Err(XlsxLayoutError::DuplicateCell); }
    }
    let mut expanded = 0u64;
    for merge in &sheet.merges {
        if merge.first_row > merge.last_row || merge.first_column > merge.last_column { return Err(XlsxLayoutError::InvalidMerge); }
        let height = u64::from(merge.last_row) - u64::from(merge.first_row) + 1;
        let width = u64::from(merge.last_column) - u64::from(merge.first_column) + 1;
        expanded = expanded.checked_add(height.checked_mul(width).ok_or(XlsxLayoutError::MergeTooLarge)?).ok_or(XlsxLayoutError::MergeTooLarge)?;
        if expanded > MAX_MERGED_CELLS { return Err(XlsxLayoutError::MergeTooLarge); }
        for row in merge.first_row..=merge.last_row {
            for column in merge.first_column..=merge.last_column {
                let cell = cells.entry((row, column)).or_insert_with(|| LayoutCell { row, column, value: None, bold: false, merged: false, formula: None });
                if cell.merged { return Err(XlsxLayoutError::OverlappingMerge); }
                // openpyxl represents non-master positions as empty merged cells.
                if row != merge.first_row || column != merge.first_column { cell.value = None; cell.bold = false; cell.formula = None; }
                cell.merged = true;
            }
        }
    }
    Ok(XlsxLayoutInput { cells: cells.into_values().collect(), merges: sheet.merges.clone() })
}

/// Numeric interpretation selected by the pinned openpyxl date-format test.
/// This is a format classification, not a date parser or an Excel guarantee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XlsxNumberFormat { Numeric, DateTime, Duration }

/// Match openpyxl's first-section date and elapsed-time format tests, including
/// its treatment of quoted text, brackets and escaped date letters.
pub fn classify_xlsx_number_format(format: &str) -> XlsxNumberFormat {
    let section = format.split(';').next().unwrap_or("");
    let chars: Vec<char> = section.chars().collect();
    let mut stripped = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            if let Some(offset) = chars[i + 1..].iter().position(|&c| c == '"' || c == '\n') {
                let end = i + 1 + offset;
                if chars[end] == '"' { i = end + 1; continue; }
            }
        } else if chars[i] == '[' {
            if let Some(offset) = chars[i + 1..].iter().position(|&c| c == ']') {
                let end = i + 1 + offset;
                let inner: String = chars[i + 1..end].iter().collect();
                if !matches!(inner.as_str(), "h" | "hh" | "m" | "mm" | "s" | "ss") { i = end + 1; continue; }
            }
        }
        stripped.push(chars[i]); i += 1;
    }
    let is_date = stripped.iter().enumerate().any(|(i, &c)| "dmhysDMHYS".contains(c) && (i == 0 || !matches!(stripped[i - 1], '_' | '\\')));
    if !is_date { return XlsxNumberFormat::Numeric; }
    let mut duration = false;
    for (i, c) in chars.iter().enumerate() {
        if *c != '[' { continue; }
        if let Some(offset) = chars[i + 1..].iter().position(|&c| c == ']') {
            let end = i + 1 + offset;
            let inner: String = chars[i + 1..end].iter().collect();
            if ["h", "hh", "m", "mm", "s", "ss"].iter().any(|t| inner.eq_ignore_ascii_case(t)) { duration = true; break; }
        }
    }
    if duration { XlsxNumberFormat::Duration } else { XlsxNumberFormat::DateTime }
}
