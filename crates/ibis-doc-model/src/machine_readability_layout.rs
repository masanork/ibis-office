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
