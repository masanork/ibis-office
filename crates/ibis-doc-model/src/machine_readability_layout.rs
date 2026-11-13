// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Building blocks for the Harunobu island detector. These do not by
//! themselves detect tables, headers, or cells outside tables.
//!
//! Based on Harunobu 0.2.2 `harunobu/core/layout/island.py` (commit
//! 88014ff9); copyright (c) 2026 Digital Agency of Japan. See ../HARUNOBU-MIT.txt.
use std::collections::{BTreeMap, BTreeSet};
use crate::sheet::CellRange;

/// Split sorted occupied indices at gaps. Returns `None` if input is not
/// strictly increasing; callers must not silently treat invalid input as empty.
/// Indices are zero based; `gap_threshold` is the maximum allowed distance
/// between successive occupied indices, exactly as in the reference detector.
pub fn occupied_bands(indices: &[u32], gap_threshold: u32) -> Option<Vec<(u32, u32)>> {
    if indices.windows(2).any(|w| w[0] >= w[1]) { return None; }
    let Some(&first) = indices.first() else { return Some(Vec::new()); };
    let mut bands = Vec::new();
    let mut start = first;
    let mut previous = first;
    for &index in &indices[1..] {
        if index - previous > gap_threshold { bands.push((start, previous)); start = index; }
        previous = index;
    }
    bands.push((start, previous));
    Some(bands)
}

/// Confidence for a *previously trimmed* region, before layout estimation.
/// The supplied coordinates must be the complete set of nonempty cell positions
/// in the region. Missing cells, duplicates or invalid ranges are not inferred.
/// Callers must not use this as a complete table detector.
pub fn trimmed_region_confidence(region: CellRange, occupied: &[(u32, u32)]) -> Option<f64> {
    if region.first_row > region.last_row || region.first_column > region.last_column { return None; }
    let mut distinct = BTreeSet::new();
    let mut rows = BTreeMap::<u32, u64>::new();
    for &(row, column) in occupied {
        if row < region.first_row || row > region.last_row || column < region.first_column || column > region.last_column || !distinct.insert((row, column)) { return None; }
        *rows.entry(row).or_default() += 1;
    }
    let height = u64::from(region.last_row) - u64::from(region.first_row) + 1;
    let width = u64::from(region.last_column) - u64::from(region.first_column) + 1;
    let minimum = ((width as f64 * 0.3).min(2.0)).max(1.0);
    let filled_rows = rows.values().filter(|&&n| n as f64 >= minimum).count();
    let fill_rate = (distinct.len() as f64) / ((height as f64) * (width as f64));
    let row_rate = (filled_rows as f64) / height as f64;
    let score = (fill_rate * 0.6 + row_rate * 0.6).min(1.0);
    // Python round(x, 3) rounds the actual binary float; multiplying by
    // 1000 first can introduce a second rounding (e.g. 0.0375).
    format!("{score:.3}").parse().ok()
}

/// Value category required by layout inference. Numbers and booleans are
/// Numeric; Other includes dates and errors. `None` means no cell value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutValue { Text(String), Numeric, Other }

/// Reader-provided cell data; formula includes the leading `=` as in Harunobu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutCell {
    pub row: u32, pub column: u32, pub value: Option<LayoutValue>,
    pub bold: bool, pub merged: bool, pub formula: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtherRegion { Title, Total, Note, Source }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrimmedOther { pub row: u32, pub column: u32, pub kind: OtherRegion }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrimmedRegion { pub range: Option<CellRange>, pub others: Vec<TrimmedOther> }

fn python_space(c: char) -> bool { c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}') }
fn is_total_formula(formula: &str) -> bool {
    let Some(formula) = formula.strip_prefix('=') else { return false; };
    let formula = formula.trim_start_matches(python_space);
    ["SUM", "SUBTOTAL"].iter().any(|word| {
        formula.get(..word.len()).is_some_and(|s| s.eq_ignore_ascii_case(word)) &&
            formula.get(word.len()..).is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_'))
    })
}
fn is_total_row(cells: &[&LayoutCell]) -> bool {
    cells.iter().filter_map(|c| match &c.value { Some(LayoutValue::Text(t)) => Some(t.trim_matches(python_space)), _ => None }).take(3)
        .any(|t| matches!(t, "合計" | "小計" | "総計" | "計" | "総数" | "総合計" | "3カ年平均"))
}
fn is_note(value: &str) -> bool {
    let value = value.trim_start_matches(python_space);
    ["※", "（注）", "注：", "注:", "注）", "注)", "備考"].iter().any(|prefix| value.starts_with(prefix))
}
fn is_source(value: &str) -> bool {
    let value = value.trim_matches(python_space);
    ["出典", "Source", "source", "参考", "出所", "資料"].iter().any(|prefix| value.starts_with(prefix))
}

/// Trim titles, totals, notes, and sources from a candidate island. Input must
/// include *all* cells in the region, including formatted empty cells. `None`
/// means duplicate cells or an invalid range; it is not an empty detection.
/// This is only boundary trimming; band merging and header inference are separate.
pub fn trim_boundaries(region: CellRange, cells: &[LayoutCell]) -> Option<TrimmedRegion> {
    if region.first_row > region.last_row || region.first_column > region.last_column { return None; }
    let mut positions = BTreeSet::new();
    let mut rows: BTreeMap<u32, Vec<&LayoutCell>> = BTreeMap::new();
    for cell in cells {
        if !positions.insert((cell.row, cell.column)) { return None; }
        if cell.row >= region.first_row && cell.row <= region.last_row && cell.column >= region.first_column && cell.column <= region.last_column {
            rows.entry(cell.row).or_default().push(cell);
        }
    }
    for row in rows.values_mut() { row.sort_by_key(|cell| cell.column); }
    let width = u64::from(region.last_column) - u64::from(region.first_column) + 1;
    let mut first = region.first_row;
    let mut last = region.last_row;
    let mut others = Vec::new();
    let append = |others: &mut Vec<TrimmedOther>, row: &[&LayoutCell], kind| {
        others.extend(row.iter().filter(|cell| cell.value.is_some()).map(|cell| TrimmedOther { row: cell.row, column: cell.column, kind }));
    };
    while width >= 3 && first < last {
        let row = rows.get(&first).map(Vec::as_slice).unwrap_or(&[]);
        let count = row.iter().filter(|c| c.value.is_some()).count();
        let signal = row.iter().any(|c| c.bold || c.merged);
        let fill = count as f64 / width as f64;
        let title = (count == 1 && signal && width >= 4) || (fill <= 0.15 && !signal) || ((1..=2).contains(&count) && !signal);
        if !title { break; }
        append(&mut others, row, OtherRegion::Title);
        // Empty rows are all titles here; skip directly to the next stored row.
        first = rows.range((std::ops::Bound::Excluded(first), std::ops::Bound::Included(last))).next().map_or(last, |(&r, _)| r);
    }
    while last > first {
        let row = rows.get(&last).map(Vec::as_slice).unwrap_or(&[]);
        let occupied: Vec<_> = row.iter().copied().filter(|c| c.value.is_some()).collect();
        if occupied.is_empty() {
            last = rows.range((std::ops::Bound::Included(first), std::ops::Bound::Excluded(last)))
                .rev().find(|(_, r)| r.iter().any(|c| c.value.is_some())).map_or(first, |(&r, _)| r);
            continue;
        }
        let first_value = match &occupied[0].value { Some(LayoutValue::Text(s)) => Some(s.as_str()), _ => None };
        let has_formula = occupied.iter().any(|c| c.formula.as_deref().is_some_and(is_total_formula));
        let kind = if has_formula || is_total_row(&occupied) { Some(OtherRegion::Total) }
            else if first_value.is_some_and(is_note) { Some(OtherRegion::Note) }
            else if (occupied.len() as f64 / width as f64) < 0.3 {
                first_value.and_then(|s| if is_source(s) { Some(OtherRegion::Source) } else if (occupied.len() as f64 / width as f64) <= 0.2 { Some(OtherRegion::Note) } else { None })
            } else { None };
        let Some(kind) = kind else { break; };
        append(&mut others, &occupied, kind);
        last -= 1;
    }
    let row = rows.get(&first).map(Vec::as_slice).unwrap_or(&[]);
    let occupied: Vec<_> = row.iter().copied().filter(|c| c.value.is_some()).collect();
    let remove_last = first == last && width >= 3 && !occupied.is_empty() && occupied.len() as f64 / width as f64 <= 0.2 && occupied.iter().all(|c| matches!(&c.value, Some(LayoutValue::Text(_))));
    if remove_last { append(&mut others, &occupied, OtherRegion::Note); }
    let range = (!remove_last).then_some(CellRange { first_row: first, last_row: last, ..region });
    Some(TrimmedRegion { range, others })
}

/// A candidate island after boundary trimming. This does not yet include
/// Harunobu's header/column inference or its post-detection merge passes.
#[derive(Clone, Debug, PartialEq)]
pub struct IslandCandidate { pub range: CellRange, pub confidence: f64 }
#[derive(Clone, Debug, PartialEq)]
pub struct IslandEvaluation { pub candidate: Option<IslandCandidate>, pub others: Vec<TrimmedOther> }

/// Evaluate one pair of previously determined row/column bands. Complete
/// cell metadata is required; `None` denotes invalid or duplicate input.
pub fn evaluate_island(region: CellRange, cells: &[LayoutCell]) -> Option<IslandEvaluation> {
    if region.first_row > region.last_row || region.first_column > region.last_column { return None; }
    let trimmed = trim_boundaries(region, cells)?;
    let has_value = cells.iter().any(|c| c.row >= region.first_row && c.row <= region.last_row && c.column >= region.first_column && c.column <= region.last_column && c.value.is_some());
    if !has_value { return Some(IslandEvaluation { candidate: None, others: Vec::new() }); }
    let candidate = if let Some(range) = trimmed.range {
        let positions: Vec<_> = cells.iter().filter(|c| c.row >= range.first_row && c.row <= range.last_row && c.column >= range.first_column && c.column <= range.last_column && c.value.is_some()).map(|c| (c.row, c.column)).collect();
        Some(IslandCandidate { range, confidence: trimmed_region_confidence(range, &positions)? })
    } else { None };
    Some(IslandEvaluation { candidate, others: trimmed.others })
}

/// Whether a text cell matches Harunobu's CSV numeric patterns and represents
/// a data-like value rather than a short column number.
fn is_data_like_numeric(value: &str) -> bool {
    use std::sync::OnceLock;
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| regex::Regex::new(r"^[+-]?(?:\d+|\d+\.\d+|\d{1,3}(?:,\d{3})*(?:\.\d+)?)$").unwrap());
    pattern.is_match(value) && (value.contains(',') || value.contains('.') || value.trim_start_matches(['+', '-']).chars().count() >= 3)
}

/// Header signal for one row, before any header-row selection. Returns
/// 0 for no signal, 1 for textual signal, or 2 for bold/merge signal.
/// The caller must provide all cells and merged ranges for the sheet; an
/// invalid range or duplicate cell makes the result unknown (`None`).
pub fn header_row_strength(row: u32, first_column: u32, last_column: u32, cells: &[LayoutCell], merges: &[CellRange]) -> Option<u8> {
    if first_column > last_column || merges.iter().any(|m| m.first_row > m.last_row || m.first_column > m.last_column) { return None; }
    let mut positions = BTreeSet::new();
    let mut nonempty = Vec::new();
    for cell in cells {
        if !positions.insert((cell.row, cell.column)) { return None; }
        if cell.row == row && cell.column >= first_column && cell.column <= last_column && cell.value.is_some() { nonempty.push(cell); }
    }
    Some(row_strength_for_cells(row, first_column, last_column, &nonempty, merges))
}

fn row_strength_for_cells(row: u32, first_column: u32, last_column: u32, nonempty: &[&LayoutCell], merges: &[CellRange]) -> u8 {
    if nonempty.is_empty() { return 0; }
    if nonempty.iter().any(|cell| cell.bold) { return 2; }
    if nonempty.iter().any(|cell| cell.merged) {
        let mut vertical_count = 0;
        for m in merges.iter().filter(|m| m.first_row <= row && m.last_row >= row && m.first_column >= first_column && m.last_column <= last_column) {
            if m.last_column > m.first_column { return 2; }
            if m.last_row > m.first_row { vertical_count += 1; }
        }
        if vertical_count >= 3 { return 2; }
    }
    if nonempty.iter().all(|cell| matches!(&cell.value, Some(LayoutValue::Text(_)))) {
        let mut data_like = 0;
        let mut meaningful = 0;
        for cell in nonempty {
            let Some(LayoutValue::Text(value)) = &cell.value else { unreachable!() };
            let value = value.trim_matches(python_space);
            if value.is_empty() || matches!(value, "-" | "－" | "–" | "―" | "…" | "‥" | "*" | "×" | "x" | "X" | "N/A" | "n/a" | "NA" | "-\u{3000}") { continue; }
            meaningful += 1;
            if is_data_like_numeric(value) { data_like += 1; }
        }
        if data_like as f64 <= (meaningful.max(1) as f64) * 0.3 { return 1; }
    }
    0
}

/// Whether a band is a header-only region suitable for possible merging with
/// a following body band. This does not merge bands or select header rows.
/// All sheet cells and merge metadata must be provided by the reader.
pub fn band_is_header_like(band: CellRange, cells: &[LayoutCell], merges: &[CellRange]) -> Option<bool> {
    if band.first_row > band.last_row || band.first_column > band.last_column || merges.iter().any(|m| m.first_row > m.last_row || m.first_column > m.last_column) { return None; }
    let mut positions = BTreeSet::new();
    let mut rows: BTreeMap<u32, Vec<&LayoutCell>> = BTreeMap::new();
    for cell in cells {
        if !positions.insert((cell.row, cell.column)) { return None; }
        if cell.row >= band.first_row && cell.row <= band.last_row && cell.column >= band.first_column && cell.column <= band.last_column && cell.value.is_some() {
            rows.entry(cell.row).or_default().push(cell);
        }
    }
    let width = u64::from(band.last_column) - u64::from(band.first_column) + 1;
    let mut strong = false;
    let mut data = false;
    for (&row, row_cells) in &rows {
        let strength = row_strength_for_cells(row, band.first_column, band.last_column, row_cells, merges);
        if strength >= 2 { strong = true; continue; }
        if strength == 1 || (row_cells.len() <= 2 && width >= 3) { continue; }
        let numeric = row_cells.iter().any(|cell| match &cell.value {
            Some(LayoutValue::Numeric) => true,
            Some(LayoutValue::Text(value)) => is_data_like_numeric(value.trim_matches(python_space)),
            _ => false,
        });
        if numeric && row_cells.len() as f64 / width as f64 > 0.3 { data = true; }
    }
    Some(strong && !data)
}
