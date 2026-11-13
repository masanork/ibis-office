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
