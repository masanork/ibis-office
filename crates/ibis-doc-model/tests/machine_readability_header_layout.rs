// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Pinned Harunobu cases. Repeated cell/merge inputs are stored once in the fixture.
use ibis_doc_model::machine_readability_layout::{band_is_header_like, infer_header_rows, merge_header_body_bands, LayoutCell, LayoutValue};
use ibis_doc_model::sheet::CellRange;
use serde_json::Value;

fn data() -> Value {
    let value: Value = serde_json::from_str(include_str!("../../../test/fixtures/machine-readability/harunobu-header-layout-reference.json")).unwrap();
    assert_eq!(value["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    value
}
fn n(value: &Value) -> u32 { value.as_u64().unwrap().try_into().unwrap() }
fn pairs(value: &Value) -> Vec<(u32, u32)> {
    value.as_array().unwrap().iter().map(|v| (n(&v[0]), n(&v[1]))).collect()
}
fn input(data: &Value, index: usize) -> (Vec<LayoutCell>, Vec<CellRange>) {
    let input = &data["inputs"][index];
    let cells = input[0].as_array().unwrap().iter().map(|c| LayoutCell {
        row: n(&c[0]), column: n(&c[1]),
        value: if c[2].is_null() { None } else if let Some(s) = c[2].as_str() { Some(LayoutValue::Text(s.into())) }
            else if c[2].is_number() || c[2].is_boolean() { Some(LayoutValue::Numeric) } else { Some(LayoutValue::Other) },
        bold: c[3].as_bool().unwrap(), merged: c[4].as_bool().unwrap(), formula: None,
    }).collect();
    let merges = input[1].as_array().unwrap().iter().map(|v| CellRange {
        first_row: n(&v[0]), first_column: n(&v[1]), last_row: n(&v[2]), last_column: n(&v[3]),
    }).collect();
    (cells, merges)
}

#[test]
fn header_bands_and_rows_match_reference() {
    let data = data();
    let cases = data["bands"].as_array().unwrap();
    assert_eq!(cases.len(), 450);
    for (index, case) in cases.iter().enumerate() {
        let (cells, merges) = input(&data, n(&case[2]) as usize);
        let band = CellRange { first_row: 0, last_row: n(&case[0]) - 1, first_column: 0, last_column: n(&case[1]) - 1 };
        assert_eq!(band_is_header_like(band, &cells, &merges), case[3].as_bool(), "case {index}");
        let result = infer_header_rows(band, &cells, &merges).unwrap();
        let wanted: Vec<u32> = case[4][0].as_array().unwrap().iter().map(n).collect();
        assert_eq!(result.rows, wanted, "case {index}");
        assert_eq!(result.body_first, n(&case[4][1]), "case {index}");
        assert_eq!(result.body_last, n(&case[4][2]), "case {index}");
        let shifted_cells: Vec<_> = cells.iter().cloned().map(|mut c| { c.row += 7; c.column += 5; c }).collect();
        let shifted_merges: Vec<_> = merges.iter().map(|m| CellRange { first_row: m.first_row + 7, last_row: m.last_row + 7, first_column: m.first_column + 5, last_column: m.last_column + 5 }).collect();
        let shifted = CellRange { first_row: 7, last_row: band.last_row + 7, first_column: 5, last_column: band.last_column + 5 };
        let inferred = infer_header_rows(shifted, &shifted_cells, &shifted_merges).unwrap();
        assert_eq!(inferred.rows, result.rows.iter().map(|r| r + 7).collect::<Vec<_>>(), "offset case {index}");
        assert_eq!(inferred.body_first, result.body_first + 7, "offset body case {index}");
    }
}

#[test]
fn header_body_band_merging_matches_reference() {
    let data = data();
    let cases = data["band_merges"].as_array().unwrap();
    assert_eq!(cases.len(), 454);
    for (index, case) in cases.iter().enumerate() {
        let (cells, merges) = input(&data, n(&case[0]) as usize);
        assert_eq!(merge_header_body_bands(&pairs(&case[1]), &pairs(&case[2]), n(&case[3]), &cells, &merges), Some(pairs(&case[4])), "case {index}");
    }
    assert_eq!(merge_header_body_bands(&[(0, 1), (1, 2)], &[], 1, &[], &[]), None);
    assert_eq!(merge_header_body_bands(&[(2, 1)], &[], 1, &[], &[]), None);
}
