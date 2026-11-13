// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Compare layout building blocks with the pinned Harunobu island detector.
use ibis_doc_model::machine_readability_layout::{occupied_bands, trimmed_region_confidence};
use ibis_doc_model::sheet::CellRange;

#[test]
fn island_primitives_match_reference() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-layout-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in data["bands"].as_array().unwrap() {
        let input: Vec<_> = case["indices"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
        let wanted: Vec<_> = case["expected"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32)).collect();
        assert_eq!(occupied_bands(&input, case["gap"].as_u64().unwrap() as u32), Some(wanted));
    }
    for case in data["regions"].as_array().unwrap() {
        let region = CellRange { first_row: 0, first_column: 0, last_row: case["height"].as_u64().unwrap() as u32 - 1, last_column: case["width"].as_u64().unwrap() as u32 - 1 };
        let cells: Vec<_> = case["occupied"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32)).collect();
        assert_eq!(trimmed_region_confidence(region, &cells), case["confidence"].as_f64(), "{case}");
    }
    assert_eq!(occupied_bands(&[3, 3], 1), None);
    assert_eq!(occupied_bands(&[4, 3], 1), None);
    assert_eq!(occupied_bands(&[0, u32::MAX], u32::MAX), Some(vec![(0, u32::MAX)]));
    let region = CellRange { first_row: 0, last_row: 1, first_column: 0, last_column: 1 };
    let rounding = CellRange { first_row: 0, last_row: 1, first_column: 0, last_column: 7 };
    assert_eq!(trimmed_region_confidence(rounding, &[(0, 0)]), Some(0.037));
    assert_eq!(trimmed_region_confidence(region, &[(0, 0), (0, 0)]), None);
    assert_eq!(trimmed_region_confidence(region, &[(2, 0)]), None);
    assert_eq!(trimmed_region_confidence(CellRange { first_row: 2, ..region }, &[]), None);
}

#[test]
fn island_boundary_trimming_matches_reference() {
    use ibis_doc_model::machine_readability_layout::{trim_boundaries, LayoutCell, LayoutValue, OtherRegion};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-trim-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (index, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let region = CellRange { first_row: 0, first_column: 0, last_row: case["height"].as_u64().unwrap() as u32 - 1, last_column: case["width"].as_u64().unwrap() as u32 - 1 };
        let cells: Vec<_> = case["cells"].as_array().unwrap().iter().map(|v| LayoutCell {
            row: v["row"].as_u64().unwrap() as u32, column: v["column"].as_u64().unwrap() as u32,
            value: if v["value"].is_null() { None } else if let Some(s) = v["value"].as_str() { Some(LayoutValue::Text(s.into())) } else { Some(LayoutValue::Other) },
            bold: v["bold"].as_bool().unwrap(), merged: v["merged"].as_bool().unwrap(), formula: v["formula"].as_str().map(str::to_owned),
        }).collect();
        let result = trim_boundaries(region, &cells).unwrap();
        let expected = case["trimmed"].as_array().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32));
        assert_eq!(result.range.map(|r| (r.first_row, r.last_row)), expected, "case {index}: {case}");
        let actual: Vec<_> = result.others.iter().map(|v| (v.row, v.column, match v.kind { OtherRegion::Title => "title", OtherRegion::Total => "total", OtherRegion::Note => "note", OtherRegion::Source => "source" })).collect();
        let expected: Vec<_> = case["others"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_str().unwrap())).collect();
        assert_eq!(actual, expected, "case {index}: {case}");
    }
    let region = CellRange { first_row: 0, last_row: u32::MAX, first_column: 0, last_column: 2 };
    assert_eq!(trim_boundaries(region, &[]).unwrap().range.unwrap().first_row, u32::MAX);
    let cell = LayoutCell { row: 1, column: 1, value: None, bold: false, merged: false, formula: None };
    assert!(trim_boundaries(region, &[cell.clone(), cell]).is_none());
}
