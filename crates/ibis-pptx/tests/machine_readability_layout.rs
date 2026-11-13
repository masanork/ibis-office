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
    use ibis_doc_model::machine_readability_layout::{evaluate_island, trim_boundaries, LayoutCell, LayoutValue, OtherRegion};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-trim-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (index, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let region = CellRange { first_row: 0, first_column: 0, last_row: case["height"].as_u64().unwrap() as u32 - 1, last_column: case["width"].as_u64().unwrap() as u32 - 1 };
        let cells: Vec<_> = case["cells"].as_array().unwrap().iter().map(|v| LayoutCell {
            row: v["row"].as_u64().unwrap() as u32, column: v["column"].as_u64().unwrap() as u32,
            value: if v["value"].is_null() { None } else if let Some(s) = v["value"].as_str() { Some(LayoutValue::Text(s.into())) } else if v["value"].is_number() || v["value"].is_boolean() { Some(LayoutValue::Numeric) } else { Some(LayoutValue::Other) },
            bold: v["bold"].as_bool().unwrap(), merged: v["merged"].as_bool().unwrap(), formula: v["formula"].as_str().map(str::to_owned),
        }).collect();
        let result = trim_boundaries(region, &cells).unwrap();
        let expected = case["trimmed"].as_array().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32));
        assert_eq!(result.range.map(|r| (r.first_row, r.last_row)), expected, "case {index}: {case}");
        let actual: Vec<_> = result.others.iter().map(|v| (v.row, v.column, match v.kind { OtherRegion::Title => "title", OtherRegion::Total => "total", OtherRegion::Note => "note", OtherRegion::Source => "source", OtherRegion::Isolated => "isolated" })).collect();
        let expected: Vec<_> = case["others"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_str().unwrap())).collect();
        assert_eq!(actual, expected, "case {index}: {case}");
        let evaluated = evaluate_island(region, &cells).unwrap();
        let candidate = evaluated.candidate.map(|v| (v.range.first_row, v.range.last_row, v.confidence));
        let wanted = case["candidate"].as_array().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_f64().unwrap()));
        assert_eq!(candidate, wanted, "candidate case {index}: {case}");
        let actual: Vec<_> = evaluated.others.iter().map(|v| (v.row, v.column, match v.kind { OtherRegion::Title => "title", OtherRegion::Total => "total", OtherRegion::Note => "note", OtherRegion::Source => "source", OtherRegion::Isolated => "isolated" })).collect();
        let wanted: Vec<_> = case["detected_others"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_str().unwrap())).collect();
        assert_eq!(actual, wanted, "candidate others case {index}: {case}");
    }
    let region = CellRange { first_row: 0, last_row: u32::MAX, first_column: 0, last_column: 2 };
    assert_eq!(trim_boundaries(region, &[]).unwrap().range.unwrap().first_row, u32::MAX);
    let cell = LayoutCell { row: 1, column: 1, value: None, bold: false, merged: false, formula: None };
    assert!(trim_boundaries(region, &[cell.clone(), cell]).is_none());
}

#[test]
fn header_row_strength_matches_reference() {
    use ibis_doc_model::machine_readability_layout::{header_row_strength, LayoutCell, LayoutValue};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-header-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (index, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let cells: Vec<_> = case["cells"].as_array().unwrap().iter().map(|v| LayoutCell {
            row: v["row"].as_u64().unwrap() as u32, column: v["column"].as_u64().unwrap() as u32,
            value: if v["value"].is_null() { None } else if let Some(s) = v["value"].as_str() { Some(LayoutValue::Text(s.into())) } else if v["value"].is_number() || v["value"].is_boolean() { Some(LayoutValue::Numeric) } else { Some(LayoutValue::Other) },
            bold: v["bold"].as_bool().unwrap(), merged: v["merged"].as_bool().unwrap(), formula: None,
        }).collect();
        let merges: Vec<_> = case["merges"].as_array().unwrap().iter().map(|v| CellRange {
            first_row: v[0].as_u64().unwrap() as u32, first_column: v[1].as_u64().unwrap() as u32,
            last_row: v[2].as_u64().unwrap() as u32, last_column: v[3].as_u64().unwrap() as u32,
        }).collect();
        assert_eq!(header_row_strength(0, 0, case["width"].as_u64().unwrap() as u32 - 1, &cells, &merges), Some(case["strength"].as_u64().unwrap() as u8), "case {index}: {case}");
    }
    assert_eq!(header_row_strength(0, 2, 1, &[], &[]), None);
    assert_eq!(header_row_strength(0, 0, 1, &[], &[CellRange { first_row: 1, last_row: 0, first_column: 0, last_column: 0 }]), None);
}

#[test]
fn header_band_matches_reference() {
    use ibis_doc_model::machine_readability_layout::{band_is_header_like, infer_header_rows, LayoutCell, LayoutValue};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-header-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (index, case) in data["bands"].as_array().unwrap().iter().enumerate() {
        let cells: Vec<_> = case["cells"].as_array().unwrap().iter().map(|v| LayoutCell {
            row: v["row"].as_u64().unwrap() as u32, column: v["column"].as_u64().unwrap() as u32,
            value: if v["value"].is_null() { None } else if let Some(s) = v["value"].as_str() { Some(LayoutValue::Text(s.into())) } else if v["value"].is_number() || v["value"].is_boolean() { Some(LayoutValue::Numeric) } else { Some(LayoutValue::Other) },
            bold: v["bold"].as_bool().unwrap(), merged: v["merged"].as_bool().unwrap(), formula: None,
        }).collect();
        let merges: Vec<_> = case["merges"].as_array().unwrap().iter().map(|v| CellRange {
            first_row: v[0].as_u64().unwrap() as u32, first_column: v[1].as_u64().unwrap() as u32,
            last_row: v[2].as_u64().unwrap() as u32, last_column: v[3].as_u64().unwrap() as u32,
        }).collect();
        let band = CellRange { first_row: 0, last_row: case["height"].as_u64().unwrap() as u32 - 1, first_column: 0, last_column: case["width"].as_u64().unwrap() as u32 - 1 };
        assert_eq!(band_is_header_like(band, &cells, &merges), case["header_like"].as_bool(), "case {index}: {case}");
        let layout = infer_header_rows(band, &cells, &merges).unwrap();
        let wanted: Vec<u32> = case["layout"][0].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
        assert_eq!(layout.rows, wanted, "header rows case {index}: {case}");
        assert_eq!(layout.body_first, case["layout"][1].as_u64().unwrap() as u32, "body start case {index}: {case}");
        assert_eq!(layout.body_last, case["layout"][2].as_u64().unwrap() as u32, "body end case {index}: {case}");
        let shifted_cells: Vec<_> = cells.iter().cloned().map(|mut c| { c.row += 7; c.column += 5; c }).collect();
        let shifted_merges: Vec<_> = merges.iter().map(|m| CellRange { first_row: m.first_row + 7, last_row: m.last_row + 7, first_column: m.first_column + 5, last_column: m.last_column + 5 }).collect();
        let shifted = CellRange { first_row: band.first_row + 7, last_row: band.last_row + 7, first_column: band.first_column + 5, last_column: band.last_column + 5 };
        let inferred = infer_header_rows(shifted, &shifted_cells, &shifted_merges).unwrap();
        assert_eq!(inferred.rows, layout.rows.iter().map(|r| r + 7).collect::<Vec<_>>(), "offset case {index}");
        assert_eq!(inferred.body_first, layout.body_first + 7, "offset body case {index}");
    }
}

#[test]
fn header_body_band_merging_matches_reference() {
    use ibis_doc_model::machine_readability_layout::{merge_header_body_bands, LayoutCell, LayoutValue};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-header-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (index, case) in data["band_merges"].as_array().unwrap().iter().enumerate() {
        let cells: Vec<_> = case["cells"].as_array().unwrap().iter().map(|v| LayoutCell {
            row: v["row"].as_u64().unwrap() as u32, column: v["column"].as_u64().unwrap() as u32,
            value: if v["value"].is_null() { None } else if let Some(s) = v["value"].as_str() { Some(LayoutValue::Text(s.into())) } else if v["value"].is_number() || v["value"].is_boolean() { Some(LayoutValue::Numeric) } else { Some(LayoutValue::Other) },
            bold: v["bold"].as_bool().unwrap(), merged: v["merged"].as_bool().unwrap(), formula: None,
        }).collect();
        let merges: Vec<_> = case["merges"].as_array().unwrap().iter().map(|v| CellRange { first_row: v[0].as_u64().unwrap() as u32, first_column: v[1].as_u64().unwrap() as u32, last_row: v[2].as_u64().unwrap() as u32, last_column: v[3].as_u64().unwrap() as u32 }).collect();
        let pairs = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|a| (a[0].as_u64().unwrap() as u32, a[1].as_u64().unwrap() as u32)).collect::<Vec<_>>();
        assert_eq!(merge_header_body_bands(&pairs(&case["row_bands"]), &pairs(&case["column_bands"]), case["gap"].as_u64().unwrap() as u32, &cells, &merges), Some(pairs(&case["result"])), "case {index}: {case}");
    }
    assert_eq!(merge_header_body_bands(&[(0, 1), (1, 2)], &[], 1, &[], &[]), None);
    assert_eq!(merge_header_body_bands(&[(2, 1)], &[], 1, &[], &[]), None);
}

#[test]
fn column_schemas_match_reference() {
    use ibis_doc_model::machine_readability_layout::{infer_column_schema, infer_column_schemas, infer_column_type, InferredColumnType, LayoutCell, LayoutValue};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-column-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (index, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let cells: Vec<_> = case["cells"].as_array().unwrap().iter().map(|v| LayoutCell {
            row: v["row"].as_u64().unwrap() as u32, column: 4,
            value: if v["value"].is_null() { None } else if let Some(s) = v["value"].as_str() { Some(LayoutValue::Text(s.into())) } else if v["value"].is_number() || v["value"].is_boolean() { Some(LayoutValue::Numeric) } else { Some(LayoutValue::Date) },
            bold: false, merged: false, formula: None,
        }).collect();
        let kind = match case["kind"].as_str().unwrap() { "numeric" => InferredColumnType::Numeric, "text" => InferredColumnType::Text, "date" => InferredColumnType::Date, "mixed" => InferredColumnType::Mixed, "empty" => InferredColumnType::Empty, other => panic!("unexpected {other}") };
        let schema = infer_column_schema(4, 0, case["height"].as_u64().unwrap() as u32 - 1, &cells, case["label"].as_str()).unwrap();
        assert_eq!(schema.kind, kind, "case {index}: {case}");
        assert_eq!(schema.has_header, case["has_header"].as_bool().unwrap(), "case {index}: {case}");
        assert_eq!(schema.column, 4);
    }
    assert_eq!(infer_column_type(4, 2, 1, &[]), Some(InferredColumnType::Empty));
    let cell = LayoutCell { row: 0, column: 4, value: Some(LayoutValue::Numeric), bold: false, merged: false, formula: None };
    assert_eq!(infer_column_type(4, 0, 1, &[cell.clone(), cell.clone()]), None);
    let other = LayoutCell { column: 7, value: Some(LayoutValue::Text("label".into())), ..cell.clone() };
    let schemas = infer_column_schemas(&[(7, None), (4, Some("value")), (9, Some(""))], 0, 1, &[cell, other]).unwrap();
    assert_eq!(schemas.iter().map(|s| (s.column, s.kind, s.has_header)).collect::<Vec<_>>(), [(7, InferredColumnType::Text, false), (4, InferredColumnType::Numeric, true), (9, InferredColumnType::Empty, false)]);
    assert!(infer_column_schemas(&[(4, None), (4, None)], 0, 1, &[]).is_none());
}

#[test]
fn generated_column_headers_match_reference() {
    use ibis_doc_model::machine_readability_layout::{generate_column_headers, HeaderCell};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-label-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (index, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let cells: Vec<_> = case["cells"].as_array().unwrap().iter().map(|v| HeaderCell {
            row: v["row"].as_u64().unwrap() as u32, column: v["column"].as_u64().unwrap() as u32,
            value: v["value"].as_str().map(str::to_owned), merged: v["merged"].as_bool().unwrap(),
            master: v["master"].as_array().map(|a| (a[0].as_u64().unwrap() as u32, a[1].as_u64().unwrap() as u32)),
        }).collect();
        let rows: Vec<_> = case["rows"].as_array().unwrap().iter().map(|r| r.as_u64().unwrap() as u32).collect();
        let columns: Vec<_> = (0..case["width"].as_u64().unwrap() as u32).collect();
        let actual = generate_column_headers(&rows, &columns, &cells).unwrap();
        let expected = case["headers"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len(), "case {index}");
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.column, expected["column"].as_u64().unwrap() as u32, "case {index}");
            assert_eq!(actual.label, expected["label"].as_str().unwrap(), "case {index}");
            assert_eq!(actual.merged, expected["merged"].as_bool().unwrap(), "case {index}");
            assert_eq!(actual.rows, rows, "case {index}");
        }
    }
    let cell = |row, value: &str| HeaderCell { row, column: 0, value: Some(value.into()), merged: false, master: None };
    let cells = [cell(0, " A "), cell(1, "A"), cell(2, "B"), cell(3, "A")];
    assert_eq!(generate_column_headers(&[0, 1, 2, 3], &[0], &cells).unwrap()[0].label, "A / B / A");
    assert!(generate_column_headers(&[0], &[1,1], &[]).is_none());
    assert_eq!(generate_column_headers(&[], &[0], &[]).unwrap().len(), 0);
}

#[test]
fn preliminary_islands_match_reference() {
    use ibis_doc_model::machine_readability_layout::{merge_column_adjacent_regions, merge_form_fragments, preliminary_islands, LayoutCell, LayoutValue, OtherRegion};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-islands-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (index, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let cells: Vec<_> = case["cells"].as_array().unwrap().iter().map(|v| LayoutCell {
            row: v["row"].as_u64().unwrap() as u32, column: v["column"].as_u64().unwrap() as u32,
            value: if v["value"].is_null() { None } else if let Some(s) = v["value"].as_str() { Some(LayoutValue::Text(s.into())) } else { Some(LayoutValue::Numeric) },
            bold: v["bold"].as_bool().unwrap(), merged: v["merged"].as_bool().unwrap(), formula: v["formula"].as_str().map(str::to_owned),
        }).collect();
        let merges: Vec<_> = case["merges"].as_array().unwrap().iter().map(|v| CellRange { first_row: v[0].as_u64().unwrap() as u32, first_column: v[1].as_u64().unwrap() as u32, last_row: v[2].as_u64().unwrap() as u32, last_column: v[3].as_u64().unwrap() as u32 }).collect();
        let actual = preliminary_islands(&cells, &merges).unwrap();
        let regions: Vec<_> = actual.candidates.iter().map(|c| (c.range.first_row, c.range.first_column, c.range.last_row, c.range.last_column, c.confidence)).collect();
        let expected: Vec<_> = case["candidates"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_u64().unwrap() as u32, v[3].as_u64().unwrap() as u32, v[4].as_f64().unwrap())).collect();
        assert_eq!(regions, expected, "candidates case {index}: {case}");
        let others: Vec<_> = actual.others.iter().map(|v| (v.row, v.column, match v.kind { OtherRegion::Title => "title", OtherRegion::Total => "total", OtherRegion::Note => "note", OtherRegion::Source => "source", OtherRegion::Isolated => "isolated" })).collect();
        let expected: Vec<_> = case["others"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_str().unwrap())).collect();
        assert_eq!(others, expected, "others case {index}: {case}");
        let columns = merge_column_adjacent_regions(&actual.candidates, &cells, &merges).unwrap();
        let forms = merge_form_fragments(&columns, &cells, &merges).unwrap();
        let forms: Vec<_> = forms.iter().map(|c| (c.candidate.range.first_row, c.candidate.range.first_column, c.candidate.range.last_row, c.candidate.range.last_column, c.candidate.confidence, c.headers.rows.clone())).collect();
        let expected_forms: Vec<_> = case["forms"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_u64().unwrap() as u32, v[3].as_u64().unwrap() as u32, v[4].as_f64().unwrap(), v[5].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect::<Vec<_>>())).collect();
        assert_eq!(forms, expected_forms, "form merges case {index}: {case}");
        let columns: Vec<_> = columns.iter().map(|c| (c.candidate.range.first_row, c.candidate.range.first_column, c.candidate.range.last_row, c.candidate.range.last_column, c.candidate.confidence, c.headers.rows.clone())).collect();
        let wanted: Vec<_> = case["columns"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_u64().unwrap() as u32, v[3].as_u64().unwrap() as u32, v[4].as_f64().unwrap(), v[5].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect::<Vec<_>>())).collect();
        assert_eq!(columns, wanted, "column merges case {index}: {case}");
    }
    let cell = LayoutCell { row: 0, column: 0, value: None, bold: false, merged: false, formula: None };
    assert!(preliminary_islands(&[cell.clone(), cell], &[]).is_none());
    // Sparse diagonal inputs produce 25 million potential band intersections;
    // processing should visit stored cells instead of allocating that grid.
    let sparse: Vec<_> = (0..5_000).map(|i| LayoutCell { row: i * 2, column: i * 2, value: Some(LayoutValue::Numeric), bold: false, merged: false, formula: None }).collect();
    let result = preliminary_islands(&sparse, &[]).unwrap();
    assert!(result.candidates.is_empty());
    assert_eq!(result.others.len(), 5_000);
}
