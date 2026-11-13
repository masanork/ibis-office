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
    weighted_region_confidence(region, occupied, 0.6, 0.6)
}

fn weighted_region_confidence(region: CellRange, occupied: &[(u32, u32)], fill_weight: f64, row_weight: f64) -> Option<f64> {
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
    let score = (fill_rate * fill_weight + row_rate * row_weight).min(1.0);
    // Python round(x, 3) rounds the actual binary float; multiplying by
    // 1000 first can introduce a second rounding (e.g. 0.0375).
    format!("{score:.3}").parse().ok()
}

/// Value category required by layout inference. Numbers and booleans are
/// Numeric; Date represents date/time values and Other covers errors.
/// `None` means no cell value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutValue { Text(String), Numeric, Date, Other }

/// Reader-provided cell data; formula includes the leading `=` as in Harunobu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutCell {
    pub row: u32, pub column: u32, pub value: Option<LayoutValue>,
    pub bold: bool, pub merged: bool, pub formula: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtherRegion { Title, Total, Note, Source, Isolated }
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
fn is_numeric_string(value: &str) -> bool {
    use std::sync::OnceLock;
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| regex::Regex::new(r"^[+-]?(?:\d+|\d+\.\d+|\d{1,3}(?:,\d{3})*(?:\.\d+)?)$").unwrap()).is_match(value)
}
fn is_data_like_numeric(value: &str) -> bool {
    is_numeric_string(value) && (value.contains(',') || value.contains('.') || value.trim_start_matches(['+', '-']).chars().count() >= 3)
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
    Some(header_like_from_rows(band, &rows, merges))
}

fn header_like_from_rows(band: CellRange, rows: &BTreeMap<u32, Vec<&LayoutCell>>, merges: &[CellRange]) -> bool {
    let width = u64::from(band.last_column) - u64::from(band.first_column) + 1;
    let mut strong = false;
    let mut data = false;
    for (&row, row_cells) in rows.range(band.first_row..=band.last_row) {
        let selected: Vec<_> = row_cells.iter().copied().filter(|c| c.column >= band.first_column && c.column <= band.last_column).collect();
        let row_cells = &selected;
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
    strong && !data
}

/// Combine adjacent row bands when the preceding band is header-like. The
/// row/column bands must be sorted, non-overlapping, inclusive zero-based
/// intervals, and cell and merge metadata must be complete. This is the
/// reference detector's header/body merge pass, not full table inference.
pub fn merge_header_body_bands(row_bands: &[(u32, u32)], column_bands: &[(u32, u32)], gap_tolerance: u32, cells: &[LayoutCell], merges: &[CellRange]) -> Option<Vec<(u32, u32)>> {
    let valid_bands = |bands: &[(u32, u32)]| bands.iter().all(|&(a, b)| a <= b) && bands.windows(2).all(|w| w[0].1 < w[1].0);
    if !valid_bands(row_bands) || !valid_bands(column_bands) || merges.iter().any(|m| m.first_row > m.last_row || m.first_column > m.last_column) { return None; }
    let mut positions = BTreeSet::new();
    let mut rows: BTreeMap<u32, Vec<&LayoutCell>> = BTreeMap::new();
    for cell in cells {
        if !positions.insert((cell.row, cell.column)) { return None; }
        if cell.value.is_some() { rows.entry(cell.row).or_default().push(cell); }
    }
    let Some(&(first_col, last_col)) = column_bands.iter().max_by_key(|&&(a, b)| (u64::from(b) - u64::from(a), std::cmp::Reverse(a))) else { return Some(row_bands.to_vec()); };
    let mut result = Vec::new();
    let mut i = 0;
    while i < row_bands.len() {
        let mut current = row_bands[i];
        i += 1;
        while i < row_bands.len() {
            let next = row_bands[i];
            let gap = u64::from(next.0) - u64::from(current.1) - 1;
            if gap > u64::from(gap_tolerance) { break; }
            let height = u64::from(current.1) - u64::from(current.0) + 1;
            let extending_merge = merges.iter().any(|m| m.first_row >= current.0 && m.first_row <= current.1 && m.first_column >= first_col && m.last_column <= last_col && m.last_row > m.first_row);
            let eligible_size = height <= 10 || extending_merge;
            let band = CellRange { first_row: current.0, last_row: current.1, first_column: first_col, last_column: last_col };
            if !eligible_size || !header_like_from_rows(band, &rows, merges) { break; }
            current.1 = next.1;
            i += 1;
        }
        result.push(current);
    }
    Some(result)
}

/// Header and body positions inferred inside one trimmed candidate region.
/// Column labels, column types, and stub columns are separate stages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderRows { pub rows: Vec<u32>, pub body_first: u32, pub body_last: u32 }

/// Infer header rows with the reference island detector's default maximum.
/// Cell/merge metadata must be complete; invalid or duplicated input is unknown.
pub fn infer_header_rows(region: CellRange, cells: &[LayoutCell], merges: &[CellRange]) -> Option<HeaderRows> {
    if region.first_row > region.last_row || region.first_column > region.last_column || merges.iter().any(|m| m.first_row > m.last_row || m.first_column > m.last_column) { return None; }
    let mut positions = BTreeSet::new();
    let mut rows: BTreeMap<u32, Vec<&LayoutCell>> = BTreeMap::new();
    for cell in cells {
        if !positions.insert((cell.row, cell.column)) { return None; }
        if cell.row >= region.first_row && cell.row <= region.last_row && cell.column >= region.first_column && cell.column <= region.last_column && cell.value.is_some() {
            rows.entry(cell.row).or_default().push(cell);
        }
    }
    for row in rows.values_mut() { row.sort_by_key(|c| c.column); }
    if region.first_row == region.last_row { return Some(HeaderRows { rows: Vec::new(), body_first: region.first_row, body_last: region.last_row }); }
    let first = u64::from(region.first_row);
    let last = u64::from(region.last_row);
    const MAX: u64 = 5;
    let mut effective = MAX;
    for m in merges {
        if u64::from(m.first_row) >= first && u64::from(m.first_row) < first + MAX && u64::from(m.last_row) > first + MAX - 1 && m.first_column >= region.first_column && m.last_column <= region.last_column {
            effective = effective.max(u64::from(m.last_row) - first + 1);
        }
    }
    effective = effective.min(((last - first + 1) / 2).max(MAX));
    let max_check = (first + effective).min(last + 1);
    let strength = |row: u32| row_strength_for_cells(row, region.first_column, region.last_column, rows.get(&row).map(Vec::as_slice).unwrap_or(&[]), merges);
    let mut headers = Vec::<u32>::new();
    let mut strengths = Vec::new();
    let mut r = first;
    while r < max_check {
        let value = strength(r as u32);
        if value > 0 { headers.push(r as u32); strengths.push(value); }
        else if !rows.contains_key(&(r as u32)) && r + 1 < max_check && strength((r + 1) as u32) > 0 { r += 1; continue; }
        else { break; }
        r += 1;
    }
    // Sparse signatures are equivalent to the reference's full-width tuple:
    // omitted columns are `none`, and typed occupied columns are sorted.
    let signature = |row: u32| -> Vec<(u32, u8)> {
        rows.get(&row).into_iter().flatten().map(|c| (c.column, match &c.value { Some(LayoutValue::Numeric) => 1, Some(LayoutValue::Text(_)) => 2, _ => 3 })).collect()
    };
    if headers.is_empty() {
        let a = signature(region.first_row);
        if a.iter().any(|(_, t)| *t == 1) && a == signature(region.first_row + 1) {
            return Some(HeaderRows { rows: Vec::new(), body_first: region.first_row, body_last: region.last_row });
        }
        headers.push(region.first_row); strengths.push(0);
        for row in (first + 1)..max_check {
            let value = strength(row as u32);
            if value == 0 { break; }
            headers.push(row as u32); strengths.push(value);
        }
    }
    let strong = strengths.iter().any(|&v| v >= 2);
    if strong && headers.len() > 1 {
        let mut defining_end = headers[0];
        let mut defining = false;
        for m in merges {
            if m.first_column >= region.first_column && m.last_column <= region.last_column && m.last_row > m.first_row && headers.contains(&m.first_row) {
                defining = true; defining_end = defining_end.max(m.last_row);
            }
        }
        if defining {
            headers.retain(|&r| r <= defining_end);
        }
    }
    if !strong && headers.len() > 1 {
        let body = u64::from(*headers.last().unwrap()) + 1;
        let numeric = body <= last && rows.get(&(body as u32)).is_some_and(|cells| cells.iter().any(|cell| match &cell.value {
            Some(LayoutValue::Numeric) => true,
            Some(LayoutValue::Text(t)) => is_data_like_numeric(t.trim_matches(python_space)),
            _ => false,
        }));
        if !numeric || (body <= last && signature(headers[0]) == signature(body as u32)) { headers.truncate(1); }
    }
    if headers.len() > 1 {
        let mut fills = Vec::new();
        for &row in &headers {
            let mut intervals: Vec<(u32, u32)> = rows.get(&row).into_iter().flatten().map(|c| (c.column, c.column)).collect();
            for m in merges.iter().filter(|m| m.first_row <= row && m.last_row >= row && m.first_column <= region.last_column && m.last_column >= region.first_column) {
                intervals.push((m.first_column.max(region.first_column), m.last_column.min(region.last_column)));
            }
            intervals.sort_unstable();
            let mut covered = 0u64;
            let mut end = None::<u32>;
            for (a, b) in intervals {
                let start = end.map_or(u64::from(a), |e| u64::from(a).max(u64::from(e) + 1));
                if start <= u64::from(b) { covered += u64::from(b) - start + 1; end = Some(b); }
            }
            fills.push(covered);
        }
        let max_fill = *fills.iter().max().unwrap();
        let width = u64::from(region.last_column) - u64::from(region.first_column) + 1;
        if max_fill > 0 {
            let retained: Vec<_> = headers.iter().copied().zip(fills).filter_map(|(r, f)| ((f as f64 >= max_fill as f64 * 0.2) || (f as f64 >= width as f64 * 0.3)).then_some(r)).collect();
            if !retained.is_empty() { headers = retained; }
        }
    }
    let mut last_header = *headers.last().unwrap();
    for m in merges {
        if m.first_row >= region.first_row && m.first_row <= last_header && m.last_row > last_header && u64::from(m.last_row) <= (first + MAX - 1).min(last - 1) && m.first_column >= region.first_column && m.last_column <= region.last_column {
            for extra in (u64::from(last_header) + 1)..=u64::from(m.last_row) {
                let extra = extra as u32;
                if !headers.contains(&extra) { headers.push(extra); last_header = last_header.max(extra); }
            }
        }
    }
    let body_start = u64::from(*headers.last().unwrap()) + 1;
    headers.sort_unstable();
    let mut body_first = if body_start <= last { body_start as u32 } else { region.last_row };
    let body_count = u64::from(region.last_row) - u64::from(body_first) + 1;
    if headers.len() >= 5 && body_count <= 2 { headers.truncate(1); body_first = headers[0] + 1; }
    else if headers.len() > MAX as usize { headers.truncate(MAX as usize); body_first = headers[headers.len()-1] + 1; }
    Some(HeaderRows { rows: headers, body_first, body_last: region.last_row })
}

/// Harunobu's inferred type for the nonempty values of one body column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InferredColumnType { Numeric, Text, Date, Mixed, Empty }

fn is_missing_placeholder(value: &str) -> bool {
    matches!(value, "-" | "－" | "–" | "―" | "…" | "‥" | "*" | "×" | "x" | "X" | "N/A" | "n/a" | "NA" | "-\u{3000}")
}
fn is_date_string(value: &str) -> bool {
    use std::sync::OnceLock;
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| regex::Regex::new(r"^(?:\d{4}[-/]\d{1,2}[-/]\d{1,2}|\d{1,2}[-/]\d{1,2}[-/]\d{4})$").unwrap()).is_match(value)
}

/// Infer one column from complete reader-provided cells in the inclusive body
/// range. An empty/reversed body is Empty, as in Harunobu. Missing and
/// whitespace-only values do not influence the 80% threshold. Duplicate cell
/// coordinates make the result unknown instead of silently choosing a value.
pub fn infer_column_type(column: u32, body_first: u32, body_last: u32, cells: &[LayoutCell]) -> Option<InferredColumnType> {
    Some(infer_column_schemas(&[(column, None)], body_first, body_last, cells)?.first()?.kind)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InferredColumnSchema { pub column: u32, pub kind: InferredColumnType, pub has_header: bool }

/// Construct a schema for a column using a previously generated header label.
/// An absent or empty label means no header; a nonempty label is retained as-is.
pub fn infer_column_schema(column: u32, body_first: u32, body_last: u32, cells: &[LayoutCell], header_label: Option<&str>) -> Option<InferredColumnSchema> {
    infer_column_schemas(&[(column, header_label)], body_first, body_last, cells)?.into_iter().next()
}

/// Infer several column schemas in one pass over cells. The supplied column
/// identifiers must be unique; output preserves the caller's column order.
pub fn infer_column_schemas(columns: &[(u32, Option<&str>)], body_first: u32, body_last: u32, cells: &[LayoutCell]) -> Option<Vec<InferredColumnSchema>> {
    let mut counts = BTreeMap::<u32, (u64, u64, u64)>::new();
    for &(column, _) in columns { if counts.insert(column, (0, 0, 0)).is_some() { return None; } }
    let mut positions = BTreeSet::new();
    for cell in cells {
        if !positions.insert((cell.row, cell.column)) { return None; }
        if cell.row < body_first || cell.row > body_last { continue; }
        let Some((numeric, text, date)) = counts.get_mut(&cell.column) else { continue; };
        match &cell.value {
            None => (),
            Some(LayoutValue::Numeric) => *numeric += 1,
            Some(LayoutValue::Date) => *date += 1,
            Some(LayoutValue::Other) => *text += 1,
            Some(LayoutValue::Text(value)) => {
                let value = value.trim_matches(python_space);
                if value.is_empty() || is_missing_placeholder(value) { continue; }
                if is_date_string(value) { *date += 1; }
                else if is_numeric_string(value) { *numeric += 1; }
                else { *text += 1; }
            }
        }
    }
    Some(columns.iter().map(|&(column, label)| {
        let &(numeric, text, date) = counts.get(&column).unwrap();
        let total = numeric + text + date;
        let threshold = total as f64 * 0.8;
        let kind = if total == 0 { InferredColumnType::Empty }
            else if numeric as f64 >= threshold { InferredColumnType::Numeric }
            else if date as f64 >= threshold { InferredColumnType::Date }
            else if text as f64 >= threshold { InferredColumnType::Text }
            else { InferredColumnType::Mixed };
        InferredColumnSchema { column, kind, has_header: label.is_some_and(|s| !s.is_empty()) }
    }).collect())
}

/// Reader-normalized header value and merge metadata. `value` is the same
/// textual representation Harunobu's reader would provide for its cell value;
/// `None` denotes a blank cell. Master coordinates are zero-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderCell { pub row: u32, pub column: u32, pub value: Option<String>, pub merged: bool, pub master: Option<(u32, u32)> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InferredColumnHeader { pub column: u32, pub rows: Vec<u32>, pub label: String, pub merged: bool }

/// Construct column labels from selected header rows. Complete cells, including
/// merge masters, must be supplied; missing required reader data cannot be
/// inferred here. Columns must be strictly increasing, and duplicate cell
/// coordinates are rejected. Only adjacent repeated labels are collapsed.
pub fn generate_column_headers(header_rows: &[u32], columns: &[u32], cells: &[HeaderCell]) -> Option<Vec<InferredColumnHeader>> {
    if columns.windows(2).any(|w| w[0] >= w[1]) { return None; }
    let mut by_position = BTreeMap::new();
    for cell in cells { if by_position.insert((cell.row, cell.column), cell).is_some() { return None; } }
    if header_rows.is_empty() { return Some(Vec::new()); }
    let mut out = Vec::with_capacity(columns.len());
    for &column in columns {
        let mut labels: Vec<&str> = Vec::new();
        let mut merged = false;
        for &row in header_rows {
            let Some(cell) = by_position.get(&(row, column)) else { continue; };
            merged |= cell.merged;
            let value = cell.value.as_deref().or_else(|| cell.master.and_then(|pos| by_position.get(&pos)).and_then(|master| master.value.as_deref()));
            if let Some(value) = value {
                let value = value.trim_matches(python_space);
                if !value.is_empty() && labels.last().copied() != Some(value) { labels.push(value); }
            }
        }
        out.push(InferredColumnHeader { column, rows: header_rows.to_vec(), label: labels.join(" / "), merged });
    }
    Some(out)
}

/// Initial island candidates and separated cells, before the reference
/// detector's column/form merging and header inheritance passes.
#[derive(Clone, Debug, PartialEq)]
pub struct PreliminaryIslands { pub candidates: Vec<IslandCandidate>, pub others: Vec<TrimmedOther> }

/// Combine occupancy bands, header/body merging, trimming and trivial-island
/// exclusion. This deliberately returns preliminary results: callers must not
/// treat these candidates as the final Harunobu table list. The caller supplies
/// complete cells, including formatted blanks, and complete merge metadata.
pub fn preliminary_islands(cells: &[LayoutCell], merges: &[CellRange]) -> Option<PreliminaryIslands> {
    if merges.iter().any(|m| m.first_row > m.last_row || m.first_column > m.last_column) { return None; }
    let mut positions = BTreeSet::new();
    let mut occupied_rows = BTreeSet::new();
    let mut occupied_columns = BTreeSet::new();
    for cell in cells {
        if !positions.insert((cell.row, cell.column)) { return None; }
        if cell.value.is_some() { occupied_rows.insert(cell.row); occupied_columns.insert(cell.column); }
    }
    if occupied_rows.is_empty() { return Some(PreliminaryIslands { candidates: Vec::new(), others: Vec::new() }); }
    let row_indices: Vec<_> = occupied_rows.into_iter().collect();
    let column_indices: Vec<_> = occupied_columns.into_iter().collect();
    let rows = occupied_bands(&row_indices, 1)?;
    let columns = occupied_bands(&column_indices, 1)?;
    let rows = merge_header_body_bands(&rows, &columns, 1, cells, merges)?;
    // Sparse grouping avoids walking the Cartesian product of unrelated bands.
    // BTreeMap preserves the detector's row-band, then column-band order.
    let mut groups = BTreeMap::<(usize, usize), Vec<LayoutCell>>::new();
    for cell in cells {
        let r = rows.partition_point(|band| band.1 < cell.row);
        let c = columns.partition_point(|band| band.1 < cell.column);
        if r < rows.len() && c < columns.len() && rows[r].0 <= cell.row && columns[c].0 <= cell.column {
            groups.entry((r, c)).or_default().push(cell.clone());
        }
    }
    let mut candidates = Vec::new();
    let mut others = Vec::new();
    let mut evaluated = Vec::new();
    for ((r, c), group) in groups {
        let region = CellRange { first_row: rows[r].0, last_row: rows[r].1, first_column: columns[c].0, last_column: columns[c].1 };
        let result = evaluate_island(region, &group)?;
        others.extend(result.others);
        if let Some(candidate) = result.candidate { evaluated.push((candidate, group)); }
    }
    for (candidate, mut group) in evaluated {
        // Reference excludes one-cell and one-column islands after evaluating
        // every band pair; separated cells are appended in sheet order.
        if candidate.range.first_column == candidate.range.last_column {
            group.sort_by_key(|c| (c.row, c.column));
            others.extend(group.iter().filter(|cell| cell.value.is_some() && cell.row >= candidate.range.first_row && cell.row <= candidate.range.last_row && cell.column >= candidate.range.first_column && cell.column <= candidate.range.last_column)
                .map(|cell| TrimmedOther { row: cell.row, column: cell.column, kind: OtherRegion::Isolated }));
        } else { candidates.push(candidate); }
    }
    Some(PreliminaryIslands { candidates, others })
}

/// Island plus its currently inferred header rows. Labels, types, and stub
/// columns are still separate, and this is not a final detection result.
#[derive(Clone, Debug, PartialEq)]
pub struct IslandWithHeaders { pub candidate: IslandCandidate, pub headers: HeaderRows }

/// Merge horizontally adjacent candidates with compatible row and header
/// ranges. Re-infer headers and confidence after each merge, as the reference
/// detector does. The form-fragment and header-inheritance passes remain.
pub fn merge_column_adjacent_regions(candidates: &[IslandCandidate], cells: &[LayoutCell], merges: &[CellRange]) -> Option<Vec<IslandWithHeaders>> {
    let mut regions: Vec<_> = candidates.iter().map(|candidate| Some(IslandWithHeaders { candidate: candidate.clone(), headers: infer_header_rows(candidate.range, cells, merges)? })).collect::<Option<_>>()?;
    regions.sort_by_key(|r| (r.candidate.range.first_row, r.candidate.range.first_column));
    let mut used = vec![false; regions.len()];
    let mut result = Vec::new();
    for i in 0..regions.len() {
        if used[i] { continue; }
        let mut current = regions[i].clone();
        for j in (i + 1)..regions.len() {
            if used[j] { continue; }
            let right = &regions[j];
            let left_range = current.candidate.range;
            let right_range = right.candidate.range;
            // Candidates are sorted by first row; later regions cannot match.
            if u64::from(right_range.first_row) > u64::from(left_range.first_row) + 1 { break; }
            if left_range.first_row.abs_diff(right_range.first_row) > 1 || left_range.last_row.abs_diff(right_range.last_row) > 1 { continue; }
            let gap = i64::from(right_range.first_column) - i64::from(left_range.last_column) - 1;
            if gap != 1 { continue; }
            let left_headers: BTreeSet<_> = current.headers.rows.iter().copied().collect();
            let right_headers: BTreeSet<_> = right.headers.rows.iter().copied().collect();
            if !left_headers.is_subset(&right_headers) && !right_headers.is_subset(&left_headers) { continue; }
            let range = CellRange { first_row: left_range.first_row.min(right_range.first_row), last_row: left_range.last_row.max(right_range.last_row), first_column: left_range.first_column, last_column: right_range.last_column };
            let occupied: Vec<_> = cells.iter().filter(|c| c.row >= range.first_row && c.row <= range.last_row && c.column >= range.first_column && c.column <= range.last_column && c.value.is_some()).map(|c| (c.row, c.column)).collect();
            current = IslandWithHeaders { candidate: IslandCandidate { range, confidence: trimmed_region_confidence(range, &occupied)? }, headers: infer_header_rows(range, cells, merges)? };
            used[j] = true;
        }
        result.push(current);
    }
    Some(result)
}

/// Merge groups of at least three narrow form fragments whose row ranges
/// overlap by at least 80%. Recalculate header positions and the reference
/// form-specific confidence. Header inheritance is a later pass.
pub fn merge_form_fragments(candidates: &[IslandWithHeaders], cells: &[LayoutCell], merges: &[CellRange]) -> Option<Vec<IslandWithHeaders>> {
    let mut assigned = vec![false; candidates.len()];
    let mut groups = Vec::<Vec<usize>>::new();
    for i in 0..candidates.len() {
        if assigned[i] { continue; }
        let a = candidates[i].candidate.range;
        if a.first_row > a.last_row || a.first_column > a.last_column { return None; }
        let mut group = vec![i];
        assigned[i] = true;
        for j in (i + 1)..candidates.len() {
            if assigned[j] { continue; }
            let b = candidates[j].candidate.range;
            if b.first_row > b.last_row || b.first_column > b.last_column { return None; }
            let start = a.first_row.max(b.first_row);
            let end = a.last_row.min(b.last_row);
            if start > end { continue; }
            let overlap = u64::from(end) - u64::from(start) + 1;
            let length_a = u64::from(a.last_row) - u64::from(a.first_row) + 1;
            let length_b = u64::from(b.last_row) - u64::from(b.first_row) + 1;
            if overlap as f64 / length_a.min(length_b) as f64 >= 0.8 { group.push(j); assigned[j] = true; }
        }
        groups.push(group);
    }
    let mut merged_indices = BTreeSet::new();
    let mut new_regions = Vec::new();
    for group in groups {
        let narrow = group.iter().filter(|&&i| { let r = candidates[i].candidate.range; u64::from(r.last_column) - u64::from(r.first_column) + 1 <= 3 }).count();
        if group.len() < 3 || narrow < 3 { continue; }
        let mut range = candidates[group[0]].candidate.range;
        for &i in &group[1..] {
            let r = candidates[i].candidate.range;
            range.first_row = range.first_row.min(r.first_row); range.last_row = range.last_row.max(r.last_row);
            range.first_column = range.first_column.min(r.first_column); range.last_column = range.last_column.max(r.last_column);
        }
        let occupied: Vec<_> = cells.iter().filter(|c| c.row >= range.first_row && c.row <= range.last_row && c.column >= range.first_column && c.column <= range.last_column && c.value.is_some()).map(|c| (c.row, c.column)).collect();
        new_regions.push(IslandWithHeaders { candidate: IslandCandidate { range, confidence: weighted_region_confidence(range, &occupied, 0.2, 0.85)? }, headers: infer_header_rows(range, cells, merges)? });
        merged_indices.extend(group);
    }
    let mut result: Vec<_> = candidates.iter().enumerate().filter(|(i, _)| !merged_indices.contains(i)).map(|(_, c)| c.clone()).collect();
    result.extend(new_regions);
    Some(result)
}
