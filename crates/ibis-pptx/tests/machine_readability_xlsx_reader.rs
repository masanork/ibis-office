// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Compare a conservative XLSX layout adapter with pinned Harunobu fixtures.
use ibis_doc_model::machine_readability_layout::{merge_column_adjacent_regions, merge_form_fragments, preliminary_islands, LayoutCell, LayoutValue, OtherRegion};
use ibis_doc_model::machine_readability_xlsx::{prepare_xlsx_layout, XlsxLayoutError, XlsxReadWindow};
use ibis_doc_model::sheet::{CellRange, CellValue};
use ibis_doc_model::sheet_calc::sheet_of_slide;

#[test]
fn xlsx_cells_and_preliminary_layout_match_reference_examples() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-layout-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in data["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let window = XlsxReadWindow::Through { last_row: case["last_row"].as_u64().unwrap() as u32, last_column: case["last_column"].as_u64().unwrap() as u32 };
        let bytes = std::fs::read(root.join(name)).unwrap();
        let input = ibis_pptx::machine_readability_xlsx::read_xlsx_layout(&bytes, 0, window).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let expected: Vec<_> = case["cells"].as_array().unwrap().iter().map(|v| LayoutCell {
            row: v["row"].as_u64().unwrap() as u32, column: v["column"].as_u64().unwrap() as u32,
            value: match v["kind"].as_str() { None => None, Some("text") => Some(LayoutValue::Text(v["text"].as_str().unwrap().into())), Some("numeric") => Some(LayoutValue::Numeric), Some("date") => Some(LayoutValue::Date), other => panic!("bad {other:?}") },
            bold: v["bold"].as_bool().unwrap(), merged: v["merged"].as_bool().unwrap(), formula: v["formula"].as_str().map(str::to_owned),
        }).collect();
        assert_eq!(input.cells, expected, "cells in {name}");
        let expected_merges: Vec<_> = case["merges"].as_array().unwrap().iter().map(|v| CellRange { first_row: v[0].as_u64().unwrap() as u32, first_column: v[1].as_u64().unwrap() as u32, last_row: v[2].as_u64().unwrap() as u32, last_column: v[3].as_u64().unwrap() as u32 }).collect();
        assert_eq!(input.merges, expected_merges, "merges in {name}");
        let preliminary = preliminary_islands(&input.cells, &input.merges).unwrap();
        let columns = merge_column_adjacent_regions(&preliminary.candidates, &input.cells, &input.merges).unwrap();
        let forms = merge_form_fragments(&columns, &input.cells, &input.merges).unwrap();
        let actual: Vec<_> = forms.iter().map(|t| (t.candidate.range.first_row, t.candidate.range.first_column, t.candidate.range.last_row, t.candidate.range.last_column, t.candidate.confidence, t.headers.rows.clone())).collect();
        let expected: Vec<_> = case["tables"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_u64().unwrap() as u32, v[3].as_u64().unwrap() as u32, v[4].as_f64().unwrap(), v[5].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect::<Vec<_>>())).collect();
        assert_eq!(actual, expected, "tables in {name}");
        let actual: Vec<_> = preliminary.others.iter().map(|v| (v.row, v.column, match v.kind { OtherRegion::Title => "title", OtherRegion::Total => "total", OtherRegion::Note => "note", OtherRegion::Source => "source", OtherRegion::Isolated => "isolated" })).collect();
        let expected: Vec<_> = case["others"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_str().unwrap())).collect();
        assert_eq!(actual, expected, "others in {name}");
    }
}

#[test]
fn unsupported_xlsx_layout_metadata_does_not_silently_pass() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let presentation = ibis_pptx::read(&std::fs::read(root.join("structure.xlsx")).unwrap()).unwrap();
    let sheet = sheet_of_slide(&presentation.slides[0]).unwrap();
    let window = XlsxReadWindow::Through { last_row: 100, last_column: 100 };
    let mut changed = (**sheet).clone();
    let cache: std::collections::BTreeMap<_, _> = changed.cells.iter().filter(|c| c.formula.is_some()).map(|c| ((c.row, c.column), CellValue::Empty)).collect();
    assert_eq!(prepare_xlsx_layout(&changed, window, &std::collections::BTreeMap::new()), Err(XlsxLayoutError::FormulaCacheUnknown));
    let index = changed.cells[0].style as usize;
    changed.cells[0].value = CellValue::Number(1.0);
    changed.styles[index].number_format = "yyyy-mm-dd".into();
    assert_eq!(prepare_xlsx_layout(&changed, window, &cache).unwrap().cells[0].value, Some(LayoutValue::Date));
    changed.cells[0].value = CellValue::Number(-1.0);
    assert_eq!(prepare_xlsx_layout(&changed, window, &cache), Err(XlsxLayoutError::NumberFormatRequiresMetadata));
    changed.styles[index].number_format = "General".into();
    changed.cells[0].value = CellValue::Number(f64::NAN);
    assert_eq!(prepare_xlsx_layout(&changed, window, &cache), Err(XlsxLayoutError::NonFiniteNumber));
    changed.cells[0].value = CellValue::Number(1.0);
    changed.merges = vec![CellRange { first_row: 0, first_column: 0, last_row: 1000, last_column: 1000 }];
    assert_eq!(prepare_xlsx_layout(&changed, window, &cache), Err(XlsxLayoutError::MergeTooLarge));
    changed.merges = vec![CellRange { first_row: 0, first_column: 0, last_row: 0, last_column: 1 }, CellRange { first_row: 0, first_column: 1, last_row: 0, last_column: 2 }];
    assert_eq!(prepare_xlsx_layout(&changed, window, &cache), Err(XlsxLayoutError::OverlappingMerge));
}

#[test]
fn xlsx_layout_uses_original_formula_caches() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-formula-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in data["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let input = ibis_pptx::machine_readability_xlsx::read_xlsx_layout(&bytes, 0, XlsxReadWindow::Through { last_row: 4, last_column: 2 }).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let actual = input.cells.iter().find(|c| c.row == 4 && c.column == 2);
        if case["expected"].is_null() { assert!(actual.is_none(), "{name}: {actual:?}"); continue; }
        let v = &case["expected"];
        let expected = LayoutCell { row: 4, column: 2,
            value: match v["kind"].as_str() { None => None, Some("numeric") => Some(LayoutValue::Numeric), Some("text") => Some(LayoutValue::Text(v["text"].as_str().unwrap().into())), other => panic!("bad {other:?}") },
            bold: v["bold"].as_bool().unwrap(), merged: v["merged"].as_bool().unwrap(), formula: v["formula"].as_str().map(str::to_owned) };
        assert_eq!(actual, Some(&expected), "{name}");
    }
}

#[test]
fn xlsx_number_format_classification_matches_reference() {
    use ibis_doc_model::machine_readability_xlsx::{classify_xlsx_number_format, XlsxNumberFormat};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-formats-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (i, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let expected = match case["kind"].as_str().unwrap() { "numeric" => XlsxNumberFormat::Numeric, "date" => XlsxNumberFormat::DateTime, "duration" => XlsxNumberFormat::Duration, other => panic!("unexpected {other}") };
        assert_eq!(classify_xlsx_number_format(case["format"].as_str().unwrap()), expected, "format case {i}: {case}");
    }
}

#[test]
fn automatic_xlsx_window_and_numeric_types_match_reference() {
    use ibis_pptx::machine_readability_xlsx::read_xlsx_layout_auto;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-window-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    let bytes = std::fs::read(root.join("xlsx-window-cases.xlsx")).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let index = case["index"].as_u64().unwrap() as usize;
        let name = case["name"].as_str().unwrap();
        let (window, input) = read_xlsx_layout_auto(&bytes, index).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(window, XlsxReadWindow::Through { last_row: case["last_row"].as_u64().unwrap() as u32, last_column: case["last_column"].as_u64().unwrap() as u32 }, "{name}");
        for expected in case["typed"].as_array().unwrap() {
            let row = expected["row"].as_u64().unwrap() as u32;
            let column = expected["column"].as_u64().unwrap() as u32;
            let value = &input.cells.iter().find(|c| c.row == row && c.column == column).unwrap().value;
            let kind = match value { Some(LayoutValue::Numeric) => "numeric", Some(LayoutValue::Date) => "date", Some(LayoutValue::Other) => "other", _ => panic!("unexpected {value:?}") };
            assert_eq!(kind, expected["kind"].as_str().unwrap(), "{name} ({row},{column})");
        }
    }
}

#[test]
fn large_xlsx_reader_matches_reference_two_pass_semantics() {
    use ibis_pptx::machine_readability_xlsx::read_xlsx_layout_auto;
    use ibis_ooxml::zip::{Archive, Writer, EntryMeta, STORED, crc32};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-window-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    // Add an unreferenced stored part in memory so the same synthetic workbook
    // crosses Harunobu's 5 MiB threshold without committing a large fixture.
    let original = Archive::new(std::fs::read(root.join("xlsx-window-cases.xlsx")).unwrap().into()).unwrap();
    let padded = |padding_len: usize| {
        let mut writer = Writer::new();
        for entry in original.entries() { writer.add(&entry.name, entry.meta, &original.read(entry).unwrap()).unwrap(); }
        let padding = vec![0u8; padding_len];
        writer.add_raw("reference-padding.bin", EntryMeta::new(), STORED, crc32(&padding), padding.len(), &padding).unwrap();
        writer.finish().unwrap()
    };
    let base = padded(0);
    let boundary = padded(5 * 1024 * 1024 - base.len());
    assert_eq!(boundary.len(), 5 * 1024 * 1024);
    let index = data["boundary"]["index"].as_u64().unwrap() as usize;
    let (_, single) = read_xlsx_layout_auto(&boundary, index).unwrap();
    let blank = single.cells.iter().find(|c| c.row == 0 && c.column == 1);
    assert_eq!(blank.is_some(), data["boundary"]["bold_blank_present"].as_bool().unwrap());
    assert_eq!(blank.map(|c| c.bold), data["boundary"]["bold"].as_bool());
    let bytes = padded(5 * 1024 * 1024 - base.len() + 1);
    assert_eq!(bytes.len(), 5 * 1024 * 1024 + 1);
    for case in data["large_cases"].as_array().unwrap() {
        let index = case["index"].as_u64().unwrap() as usize;
        let name = case["name"].as_str().unwrap();
        let (window, input) = read_xlsx_layout_auto(&bytes, index).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(window, XlsxReadWindow::Through { last_row: case["last_row"].as_u64().unwrap() as u32, last_column: case["last_column"].as_u64().unwrap() as u32 }, "{name}");
        let expected: Vec<_> = case["cells"].as_array().unwrap().iter().map(|v| LayoutCell {
            row: v["row"].as_u64().unwrap() as u32, column: v["column"].as_u64().unwrap() as u32,
            value: match v["kind"].as_str() { None => None, Some("text") => Some(LayoutValue::Text(v["text"].as_str().unwrap().into())), Some("numeric") => Some(LayoutValue::Numeric), Some("date") => Some(LayoutValue::Date), Some("other") => Some(LayoutValue::Other), other => panic!("bad {other:?}") },
            bold: v["bold"].as_bool().unwrap(), merged: v["merged"].as_bool().unwrap(), formula: v["formula"].as_str().map(str::to_owned),
        }).collect();
        assert_eq!(input.cells, expected, "cells in {name}");
    }
}

#[test]
fn saved_xlsx_values_preserve_original_formula_caches() {
    use ibis_doc_model::sheet::CellValue;
    use ibis_pptx::machine_readability_xlsx::read_xlsx_saved_sheet;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-saved-values-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in data["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let saved = read_xlsx_saved_sheet(&bytes, 0).unwrap();
        let cell = saved.sheet.cells.iter().find(|c| c.row == 4 && c.column == 2).unwrap();
        assert!(cell.formula.is_some(), "{name}");
        match (&cell.value, case["kind"].as_str().unwrap()) {
            (CellValue::Empty, "none") => {},
            (CellValue::Number(n), "number") => assert_eq!(*n, case["value"].as_f64().unwrap(), "{name}"),
            (CellValue::Bool(v), "bool") => assert_eq!(*v, case["value"].as_bool().unwrap(), "{name}"),
            (CellValue::Text(v), "text") => assert_eq!(v, case["value"].as_str().unwrap(), "{name}"),
            (CellValue::Error(v), "text") => assert_eq!(v.as_str(), case["value"].as_str().unwrap(), "{name}"),
            // The reference treats a cached string beginning '=' as another
            // formula; the raw source adapter must preserve the saved string.
            (CellValue::Text(v), "none") if name.contains("formula_text") => assert_eq!(v, "=cached"),
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn xlsx_formula_artifacts_match_pinned_harunobu() {
    use ibis_doc_model::machine_readability_rules::formula_artifacts;
    use ibis_doc_model::sheet::CellRange;
    use ibis_pptx::machine_readability_xlsx::inspect_xlsx_formula_artifacts;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-artifacts-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in data["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let sheets = inspect_xlsx_formula_artifacts(&std::fs::read(root.join(name)).unwrap()).unwrap();
        let names = case["sheets"].as_array().unwrap();
        assert_eq!(sheets.len(), names.len(), "{name}");
        for (actual, sheet_name) in sheets.iter().zip(names) {
            for (list, key) in [(&actual.formulas, "formulas"), (&actual.errors, "errors")] {
                let got: Vec<_> = list.iter().map(|v| (v.row, v.column, v.cell_ref.as_str())).collect();
                let wanted: Vec<_> = case[key].as_array().unwrap().iter().filter(|v| v[0] == *sheet_name).map(|v| (v[1].as_u64().unwrap() as u32, v[2].as_u64().unwrap() as u32, v[3].as_str().unwrap())).collect();
                assert_eq!(got, wanted, "{name}: {sheet_name} {key}");
            }
            let table = CellRange { first_row: 0, first_column: 0, last_row: 10, last_column: 10 };
            let result = formula_artifacts(table, Some((&actual.formulas, &actual.errors)));
            if name == "xlsx-artifact-errors.xlsx" {
                assert_eq!(result.findings.iter().map(|v| v.location.as_str()).collect::<Vec<_>>(), ["A2", "C5"]);
                assert!(!result.result.passed);
            }
            assert_eq!(formula_artifacts(table, None).result.confidence, 0.0);
        }
    }
}

#[test]
fn saved_xlsx_values_are_limited_to_reference_window() {
    use ibis_pptx::machine_readability_xlsx::read_xlsx_saved_sheet;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let saved = read_xlsx_saved_sheet(&std::fs::read(root.join("xlsx-window-cases.xlsx")).unwrap(), 3).unwrap();
    assert_eq!(saved.window, XlsxReadWindow::Through { last_row: 100, last_column: 5 });
    assert!(saved.sheet.cells.iter().all(|c| c.row <= 100 && c.column <= 5));
}

#[test]
fn native_xlsx_structure_checks_match_reference() {
    use ibis_doc_model::machine_readability_rules::FindingSeverity;
    use ibis_doc_model::machine_readability_scoring::Severity;
    use ibis_pptx::machine_readability_xlsx::analyze_xlsx_structure_partial;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (name, file) in [("structure", "structure.xlsx"), ("header", "header-merge.xlsx")] {
        let actual = analyze_xlsx_structure_partial(&std::fs::read(root.join(file)).unwrap(), 0).unwrap();
        assert_eq!(actual.len(), 1, "{name}");
        assert!(!actual[0].complete);
        for (got, expected) in actual[0].checks.iter().filter(|c| matches!(c.result.rule_id.as_str(), "L1-12" | "L1-13" | "L2-06")).zip(data["fixtures"][name]["rules"].as_array().unwrap()) {
            assert_eq!(got.result.rule_id, expected["rule"].as_str().unwrap(), "{name}");
            assert_eq!(got.result.passed, expected["passed"].as_bool().unwrap(), "{name}: {}", got.result.rule_id);
            let severity = match got.result.severity.unwrap() { Severity::Fatal => "fatal", Severity::Critical => "critical", Severity::Major => "major", Severity::Minor => "minor", Severity::Info => "info" };
            assert_eq!(severity, expected["severity"].as_str().unwrap(), "{name}: {}", got.result.rule_id);
            assert_eq!(got.findings.iter().map(|v| v.location.as_str()).collect::<Vec<_>>(), expected["locations"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect::<Vec<_>>(), "{name}: {}", got.result.rule_id);
            let severities: Vec<_> = got.findings.iter().map(|v| match v.severity { FindingSeverity::Error => "error", FindingSeverity::Warning => "warning", FindingSeverity::Info => "info" }).collect();
            assert_eq!(severities, expected["finding_severities"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect::<Vec<_>>(), "{name}: {}", got.result.rule_id);
        }
    }
}

#[test]
fn detected_xlsx_structure_matches_thorough_reference() {
    use ibis_doc_model::machine_readability_rules::FindingSeverity;
    use ibis_doc_model::machine_readability_scoring::Severity;
    use ibis_pptx::machine_readability_xlsx::analyze_xlsx_structure_partial;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-structure-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in data["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(file)).unwrap();
        for (i, sheet) in case["sheets"].as_array().unwrap().iter().enumerate() {
            let tables = analyze_xlsx_structure_partial(&bytes, i).unwrap_or_else(|e| panic!("{file}: {e:?}"));
            let wanted = sheet["tables"].as_array().unwrap();
            assert_eq!(tables.len(), wanted.len(), "{file} sheet {i}");
            for (got, expected) in tables.iter().zip(wanted) {
                let r = got.table.range;
                assert_eq!([r.first_row, r.first_column, r.last_row, r.last_column], std::array::from_fn(|n| expected["range"][n].as_u64().unwrap() as u32), "{file}");
                assert_eq!(got.header_rows, expected["headers"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect::<Vec<_>>(), "{file}");
                assert_eq!(got.table.confidence, expected["confidence"].as_f64().unwrap(), "{file}");
                let checks = expected["checks"].as_array().unwrap();
                assert_eq!(got.checks.len(), checks.len(), "{file}");
                for (a,b) in got.checks.iter().zip(checks) {
                    assert_eq!(a.result.rule_id, b["id"].as_str().unwrap(), "{file}");
                    assert_eq!(a.result.passed, b["passed"].as_bool().unwrap(), "{file}: {}", a.result.rule_id);
                    assert_eq!(a.result.confidence, b["confidence"].as_f64().unwrap(), "{file}: {}", a.result.rule_id);
                    let severity = match a.result.severity.unwrap() { Severity::Fatal => "fatal", Severity::Critical => "critical", Severity::Major => "major", Severity::Minor => "minor", Severity::Info => "info" };
                    assert_eq!(severity, b["severity"].as_str().unwrap(), "{file}: {}", a.result.rule_id);
                    let findings: Vec<_> = a.findings.iter().map(|v| (v.location.as_str(), match v.severity { FindingSeverity::Error => "error", FindingSeverity::Warning => "warning", FindingSeverity::Info => "info" })).collect();
                    let wanted: Vec<_> = b["findings"].as_array().unwrap().iter().map(|v| (v[0].as_str().unwrap(), v[1].as_str().unwrap())).collect();
                    assert_eq!(findings, wanted, "{file}: {}", a.result.rule_id);
                }
            }
        }
    }
}

#[test]
fn xlsx_fragmentation_matches_merged_reference_cases() {
    use ibis_doc_model::machine_readability_rules::{xlsx_data_fragmentation, DetectedTable};
    use ibis_doc_model::sheet::{CellKind, CellRange, CellValue, Sheet, SheetCell};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-fragmentation-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (i, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let mut sheet = Sheet::default();
        sheet.cells = case["cells"].as_array().unwrap().iter().map(|c| {
            let value = c[2].as_str().unwrap().to_string();
            SheetCell { row: c[0].as_u64().unwrap() as u32, column: c[1].as_u64().unwrap() as u32, value: CellValue::Text(value.clone()), text: value, kind: CellKind::Text, formula: None, style: 0, phonetic: None, runs: None }
        }).collect::<Vec<_>>().into();
        sheet.merges = case["merges"].as_array().unwrap().iter().map(|v| CellRange { first_row: v[0].as_u64().unwrap() as u32, first_column: v[1].as_u64().unwrap() as u32, last_row: v[2].as_u64().unwrap() as u32, last_column: v[3].as_u64().unwrap() as u32 }).collect();
        let range = CellRange { first_row: 0, first_column: 0, last_row: case["rows"].as_u64().unwrap() as u32 - 1, last_column: case["cols"].as_u64().unwrap() as u32 - 1 };
        let got = xlsx_data_fragmentation(&sheet, range, &[DetectedTable { range, confidence: 1.0 }], case["read_only"].as_bool().unwrap()).unwrap();
        assert_eq!(got.result.passed, case["passed"].as_bool().unwrap(), "case {i}");
        assert_eq!(got.result.confidence, case["confidence"].as_f64().unwrap(), "case {i}");
        assert_eq!(got.findings.iter().map(|v| v.location.as_str()).collect::<Vec<_>>(), case["locations"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect::<Vec<_>>(), "case {i}");
    }
}

#[test]
fn xlsx_column_headers_match_typed_merged_reference_cases() {
    use ibis_doc_model::machine_readability_rules::xlsx_column_headers;
    use ibis_doc_model::sheet::{CellKind, CellRange, CellStyle, CellValue, Sheet, SheetCell};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-headers-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (i, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let mut sheet = Sheet::default();
        if sheet.styles.is_empty() { sheet.styles.push(CellStyle::default()); }
        sheet.cells = case["cells"].as_array().unwrap().iter().map(|c| {
            let v = &c[2];
            let value = if let Some(v) = v.as_str() { CellValue::Text(v.into()) } else if let Some(v) = v.as_bool() { CellValue::Bool(v) } else { CellValue::Number(v.as_f64().unwrap()) };
            SheetCell { row: c[0].as_u64().unwrap() as u32, column: c[1].as_u64().unwrap() as u32, kind: CellKind::of(&value), value, text: String::new(), formula: None, style: 0, phonetic: None, runs: None }
        }).collect::<Vec<_>>().into();
        sheet.merges = case["merges"].as_array().unwrap().iter().map(|v| CellRange { first_row: v[0].as_u64().unwrap() as u32, first_column: v[1].as_u64().unwrap() as u32, last_row: v[2].as_u64().unwrap() as u32, last_column: v[3].as_u64().unwrap() as u32 }).collect();
        let range = CellRange { first_row: 0, first_column: 0, last_row: case["rows"].as_u64().unwrap() as u32 - 1, last_column: case["cols"].as_u64().unwrap() as u32 - 1 };
        let headers: Vec<_> = case["headers"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
        let got = xlsx_column_headers(&sheet, range, &headers, 1, range.last_row, case["read_only"].as_bool().unwrap()).unwrap();
        assert_eq!(got.result.passed, case["passed"].as_bool().unwrap(), "case {i}");
        assert_eq!(got.result.confidence, case["confidence"].as_f64().unwrap(), "case {i}");
        assert_eq!(got.findings.iter().map(|v| v.location.as_str()).collect::<Vec<_>>(), case["locations"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect::<Vec<_>>(), "case {i}");
    }
}

#[test]
fn xlsx_object_inspection_respects_table_confidence() {
    use ibis_doc_model::machine_readability_rules::object_artifacts;
    use ibis_doc_model::sheet::CellRange;
    use ibis_pptx::machine_readability_xlsx::inspect_xlsx_object_artifacts;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let sheets = inspect_xlsx_object_artifacts(&std::fs::read(root.join("xlsx-chart-anchors.xlsx")).unwrap()).unwrap();
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].len(), 2);
    let range = CellRange { first_row: 0, first_column: 0, last_row: 3, last_column: 1 };
    let normal = object_artifacts(range, false, Some(&sheets[0]));
    assert_eq!(normal.findings.iter().map(|v| v.location.as_str()).collect::<Vec<_>>(), ["B2"]);
    let uncertain = object_artifacts(range, true, Some(&sheets[0]));
    assert_eq!(uncertain.findings.iter().map(|v| v.location.as_str()).collect::<Vec<_>>(), ["B2", "M20"]);
    assert_eq!(object_artifacts(range, false, None).result.confidence, 0.0);
}
