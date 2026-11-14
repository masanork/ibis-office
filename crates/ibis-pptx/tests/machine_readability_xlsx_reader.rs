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
    use ibis_doc_model::machine_readability_rules::{formula_artifacts, formula_artifacts_report};
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
        for (sheet_index, (actual, sheet_name)) in sheets.iter().zip(names).enumerate() {
            for (list, key) in [(&actual.formulas, "formulas"), (&actual.errors, "errors")] {
                let got: Vec<_> = list.iter().map(|v| (v.row, v.column, v.cell_ref.as_str())).collect();
                let wanted: Vec<_> = case[key].as_array().unwrap().iter().filter(|v| v[0] == *sheet_name).map(|v| (v[1].as_u64().unwrap() as u32, v[2].as_u64().unwrap() as u32, v[3].as_str().unwrap())).collect();
                assert_eq!(got, wanted, "{name}: {sheet_name} {key}");
                if key == "formulas" {
                    let actual_text:Vec<_>=list.iter().map(|v|(v.formula_text.as_deref(),v.formula_type.as_deref())).collect();
                    let expected_text:Vec<_>=case[key].as_array().unwrap().iter().filter(|v|v[0]==*sheet_name).map(|v|(v[4].as_str(),v[5].as_str())).collect();
                    assert_eq!(actual_text,expected_text,"{name}: {sheet_name}");
                }
            }
            let table = CellRange { first_row: 0, first_column: 0, last_row: 10, last_column: 10 };
            let result = formula_artifacts(table, Some((&actual.formulas, &actual.errors)));
            let report = formula_artifacts_report(table, Some((&actual.formulas, &actual.errors))).unwrap();
            let expected_report = &case["reports"][sheet_index];
            assert_eq!(report.message,expected_report["message"].as_str().unwrap(),"{name}: {sheet_name}");
            assert_eq!(report.descriptions,expected_report["descriptions"].as_array().unwrap().iter().map(|v|v.as_str().unwrap()).collect::<Vec<_>>(),"{name}: {sheet_name}");
            assert!(formula_artifacts_report(table,None).is_none());
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
                if file == "xlsx-date-values.xlsx" || file == "xlsx-date-1904.xlsx" || file == "xlsx-duration-values.xlsx" { assert!(got.unavailable.is_empty()); }
                let r = got.table.range;
                assert_eq!([r.first_row, r.first_column, r.last_row, r.last_column], std::array::from_fn(|n| expected["range"][n].as_u64().unwrap() as u32), "{file}");
                assert_eq!(got.header_rows, expected["headers"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect::<Vec<_>>(), "{file}");
                assert_eq!(got.table.confidence, expected["confidence"].as_f64().unwrap(), "{file}");
                let checks = expected["checks"].as_array().unwrap();
                assert_eq!(got.checks.len() + got.unavailable.len(), checks.len(), "{file}");
                for b in checks {
                    let id = b["id"].as_str().unwrap();
                    if got.unavailable.contains(&id) { assert!(got.checks.iter().all(|v| v.result.rule_id != id), "{file}: {id}"); continue; }
                    let a = got.checks.iter().find(|v| v.result.rule_id == id).unwrap_or_else(|| panic!("{file}: missing {id}"));
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
    use ibis_doc_model::machine_readability_rules::{xlsx_data_fragmentation, xlsx_fragmentation_report, DetectedTable};
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
        let report = xlsx_fragmentation_report(&sheet, range, &[DetectedTable { range, confidence: 1.0 }], case["read_only"].as_bool().unwrap()).unwrap();
        assert_eq!(report.evaluation, got, "case {i}");
        assert_eq!(report.message, case["message"].as_str().unwrap(), "case {i}");
        assert_eq!(report.descriptions, case["descriptions"].as_array().unwrap().iter().map(|v|v.as_str().unwrap()).collect::<Vec<_>>(), "case {i}");
        assert_eq!(got.result.passed, case["passed"].as_bool().unwrap(), "case {i}");
        assert_eq!(got.result.confidence, case["confidence"].as_f64().unwrap(), "case {i}");
        assert_eq!(got.findings.iter().map(|v| v.location.as_str()).collect::<Vec<_>>(), case["locations"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect::<Vec<_>>(), "case {i}");
    }
}

#[test]
fn xlsx_column_headers_match_typed_merged_reference_cases() {
    use ibis_doc_model::machine_readability_rules::{xlsx_column_headers, xlsx_column_headers_report};
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
        let mut text_sheet = sheet.clone();
        text_sheet.cells = case["text_cells"].as_array().unwrap().iter().map(|c| { let text=c[2].as_str().unwrap().to_string(); SheetCell { row:c[0].as_u64().unwrap() as u32,column:c[1].as_u64().unwrap() as u32,kind:CellKind::Text,value:CellValue::Text(text.clone()),text,formula:None,style:0,phonetic:None,runs:None } }).collect::<Vec<_>>().into();
        let report = xlsx_column_headers_report(&sheet, &text_sheet, range, &headers, 1, range.last_row, case["read_only"].as_bool().unwrap()).unwrap();
        assert_eq!(report.evaluation, got, "case {i}");
        assert_eq!(report.message, case["message"].as_str().unwrap(), "case {i}");
        assert_eq!(report.descriptions, case["descriptions"].as_array().unwrap().iter().map(|v|v.as_str().unwrap()).collect::<Vec<_>>(), "case {i}");
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

#[test]
fn multiple_xlsx_drawings_follow_relationship_order() {
    use ibis_pptx::machine_readability_xlsx::inspect_xlsx_object_artifacts;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let sheets = inspect_xlsx_object_artifacts(&std::fs::read(root.join("xlsx-multiple-drawings.xlsx")).unwrap()).unwrap();
    let objects: Vec<_> = sheets[0].iter().map(|o| (o.row, o.column, o.drawing_part.as_str())).collect();
    // Harunobu's inspector returns one-based coordinates; these are zero-based.
    assert_eq!(objects, [(1, 1, "xl/drawings/drawing1.xml"), (19, 12, "xl/drawings/drawing1.xml"), (4, 3, "xl/drawings/drawing2.xml"), (19, 12, "xl/drawings/drawing2.xml")]);
}

#[test]
fn xlsx_direct_formats_match_both_reference_reader_modes() {
    use ibis_ooxml::zip::{Archive, Writer, EntryMeta, STORED, crc32};
    use ibis_pptx::machine_readability_xlsx::inspect_xlsx_cell_formats;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-formats-values-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    let raw = std::fs::read(root.join("xlsx-format-variation.xlsx")).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let bytes = if case["padded"].as_bool().unwrap() {
            let source = Archive::new(raw.clone().into()).unwrap(); let mut writer = Writer::new();
            for entry in source.entries() { writer.add(&entry.name, entry.meta, &source.read(entry).unwrap()).unwrap(); }
            let padding = vec![0u8; 5 * 1024 * 1024];
            writer.add_raw("reference-padding.bin", EntryMeta::new(), STORED, crc32(&padding), padding.len(), &padding).unwrap();
            writer.finish().unwrap()
        } else { raw.clone() };
        let formats = inspect_xlsx_cell_formats(&bytes, 0).unwrap();
        for v in case["entries"].as_array().unwrap() {
            let row = v["row"].as_u64().unwrap() as u32; let col = v["col"].as_u64().unwrap() as u32;
            let fmt = formats.get(&(row, col)).unwrap();
            assert_eq!(fmt.font_color.as_deref(), v["font"].as_str(), "{row} {col}");
            assert_eq!(fmt.background.as_deref(), v["bg"].as_str(), "{row} {col}");
            assert_eq!(fmt.bold, v["bold"].as_bool().unwrap(), "{row} {col}");
            assert_eq!(fmt.italic, v["italic"].as_bool().unwrap(), "{row} {col}");
            assert_eq!(fmt.stub_background_nonwhite, v["stub_bg"].as_bool().unwrap(), "{row} {col}");
            assert_eq!(fmt.border_left, v["left"].as_bool().unwrap(), "{row} {col}");
            assert_eq!(fmt.border_right, v["right"].as_bool().unwrap(), "{row} {col}");
        }
    }
}

#[test]
fn saved_xlsx_numeric_lexemes_keep_integer_and_float_distinct() {
    use ibis_pptx::machine_readability_xlsx::{read_xlsx_saved_sheet, XlsxNumericLexeme};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-numeric-lexeme-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    let saved = read_xlsx_saved_sheet(&std::fs::read(root.join("xlsx-numeric-lexemes.xlsx")).unwrap(), 0).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let position = (case["row"].as_u64().unwrap() as u32, case["col"].as_u64().unwrap() as u32);
        let expected = case["raw"].as_str().unwrap().to_string();
        let lexeme = match case["kind"].as_str().unwrap() { "integer" => XlsxNumericLexeme::Integer(expected), "float" => XlsxNumericLexeme::Float(expected), other => panic!("unknown {other}") };
        assert_eq!(saved.numeric_lexemes.get(&position), Some(&lexeme), "{position:?}");
    }
}

#[test]
fn xlsx_numeric_text_matches_reference_for_sampled_values() {
    use ibis_pptx::machine_readability_xlsx::XlsxNumericLexeme;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-number-text-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (i, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let raw = case["raw"].as_str().unwrap().to_string();
        let value = if case["kind"] == "integer" { XlsxNumericLexeme::Integer(raw) } else { XlsxNumericLexeme::Float(raw) };
        assert_eq!(value.reference_text().as_deref(), case["expected"].as_str(), "case {i}");
    }
}

#[test]
fn xlsx_reference_text_view_preserves_numeric_reader_spelling() {
    use ibis_doc_model::sheet::CellValue;
    use ibis_pptx::machine_readability_xlsx::{read_xlsx_saved_sheet, xlsx_reference_text_sheet};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-numeric-lexeme-reference.json")).unwrap()).unwrap();
    let saved = read_xlsx_saved_sheet(&std::fs::read(root.join("xlsx-numeric-lexemes.xlsx")).unwrap(), 0).unwrap();
    let sheet = xlsx_reference_text_sheet(&saved).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let position = (case["row"].as_u64().unwrap() as u32, case["col"].as_u64().unwrap() as u32);
        let cell = sheet.cells.iter().find(|c| (c.row, c.column) == position).unwrap();
        assert_eq!(cell.value, CellValue::Text(case["reference"].as_str().unwrap().to_string()), "{position:?}");
    }
}

#[test]
fn xlsx_date_text_matches_reference_across_epochs() {
    use ibis_pptx::machine_readability_xlsx::xlsx_date_reference_text;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-date-text-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (i, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let got = xlsx_date_reference_text(case["value"].as_str().unwrap().parse().unwrap(), case["date1904"].as_bool().unwrap());
        assert_eq!(got.as_deref(), case["expected"].as_str(), "case {i}: {case}");
    }
}

#[test]
fn generated_xlsx_checks_match_thorough_reference() {
    use ibis_doc_model::machine_readability_rules::FindingSeverity;
    use ibis_doc_model::machine_readability_scoring::Severity;
    use ibis_pptx::machine_readability_xlsx::analyze_xlsx_structure_partial;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-generated-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in data["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bytes: Vec<_> = case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect();
        let got = analyze_xlsx_structure_partial(&bytes, 0).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let expected = case["tables"].as_array().unwrap(); assert_eq!(got.len(), expected.len(), "{name}");
        for (table, reference) in got.iter().zip(expected) {
            assert!(table.unavailable.is_empty(), "{name}: {:?}", table.unavailable);
            assert!(table.diagnostics.is_some(), "{name}: missing diagnostics");
            let r=table.table.range;
            assert_eq!([r.first_row,r.first_column,r.last_row,r.last_column],std::array::from_fn(|i| reference["range"][i].as_u64().unwrap() as u32),"{name}");
            assert_eq!(table.header_rows, reference["headers"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect::<Vec<_>>(), "{name}");
            let labels:Vec<_>=table.column_headers.as_ref().unwrap().iter().map(|h|serde_json::json!({"column":h.column,"rows":h.rows,"label":h.label,"merged":h.merged})).collect();
            assert_eq!(labels,*reference["column_headers"].as_array().unwrap(),"{name}: column headers");
            assert_eq!(table.table.confidence, reference["confidence"].as_f64().unwrap(), "{name}");
            for b in reference["checks"].as_array().unwrap() {
                let id=b["id"].as_str().unwrap();
                if table.unavailable.contains(&id) { continue; }
                let a=table.checks.iter().find(|v| v.result.rule_id==id).unwrap_or_else(|| panic!("{name}: missing {id}"));
                let diagnostic=table.diagnostics.as_ref().unwrap().iter().flatten().find(|d|d.evaluation.result.rule_id==id).unwrap();
                assert_eq!(diagnostic.message,b["message"].as_str().unwrap(),"{name}: {id}");
                let mut descriptions:Vec<_>=diagnostic.evaluation.findings.iter().zip(&diagnostic.descriptions).map(|(f,d)|(f.location.as_str(),d.as_str())).collect();
                let mut expected_descriptions:Vec<_>=b["descriptions"].as_array().unwrap().iter().map(|v|(v[0].as_str().unwrap(),v[1].as_str().unwrap())).collect();
                if id=="L2-04" { descriptions.sort();expected_descriptions.sort(); }
                assert_eq!(descriptions,expected_descriptions,"{name}: {id}");
                assert_eq!(a.result.passed,b["passed"].as_bool().unwrap(),"{name}: {id}");
                assert_eq!(a.result.confidence,b["confidence"].as_f64().unwrap(),"{name}: {id}");
                let severity=match a.result.severity.unwrap() { Severity::Fatal=>"fatal",Severity::Critical=>"critical",Severity::Major=>"major",Severity::Minor=>"minor",Severity::Info=>"info" };
                assert_eq!(severity,b["severity"].as_str().unwrap(),"{name}: {id}");
                let mut findings: Vec<_>=a.findings.iter().map(|v|(v.location.as_str(),match v.severity { FindingSeverity::Error=>"error",FindingSeverity::Warning=>"warning",FindingSeverity::Info=>"info" })).collect();
                let mut wanted: Vec<_>=b["findings"].as_array().unwrap().iter().map(|v|(v[0].as_str().unwrap(),v[1].as_str().unwrap())).collect();
                if id=="L2-04" { findings.sort();wanted.sort(); }
                assert_eq!(findings,wanted,"{name}: {id}");
            }
        }
    }
}

#[test]
fn xlsx_vertical_format_excludes_falsy_numeric_headers() {
    use ibis_doc_model::machine_readability_rules::xlsx_vertical_format;
    use ibis_doc_model::sheet::{CellKind, CellRange, CellValue, Sheet, SheetCell};
    let mut sheet = Sheet::default();
    sheet.cells = ["値1", "値2", "値3", "値4", "False"].iter().enumerate().map(|(col, value)| SheetCell { row: 0, column: col as u32, value: CellValue::Text((*value).to_string()), text: (*value).to_string(), kind: CellKind::Text, formula: None, style: 0, phonetic: None, runs: None }).collect::<Vec<_>>().into();
    let table = CellRange { first_row: 0, first_column: 0, last_row: 2, last_column: 4 };
    let falsy = std::collections::BTreeSet::from([(0, 4)]);
    assert!(xlsx_vertical_format(&sheet, table, &[0], &falsy).unwrap().result.passed);
}

#[test]
fn xlsx_duration_text_matches_reference_samples() {
    use ibis_pptx::machine_readability_xlsx::{xlsx_duration_reference_text,xlsx_duration_reference_json};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-duration-text-reference.json")).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (i, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let got = xlsx_duration_reference_text(case["value"].as_str().unwrap().parse().unwrap());
        assert_eq!(got.as_deref(), case["expected"].as_str(), "case {i}: {case}");
        assert_eq!(xlsx_duration_reference_json(case["value"].as_str().unwrap().parse().unwrap()).as_deref(),case["json"].as_str(),"case {i}: {case}");
    }
}

#[test]
fn partial_xlsx_scores_match_reference_aggregation() {
    use ibis_pptx::machine_readability_xlsx::{analyze_xlsx_structure_partial, score_xlsx_tables_partial};
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data: serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-generated-reference.json")).unwrap()).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let name=case["name"].as_str().unwrap();
        let bytes: Vec<_>=case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
        let tables=analyze_xlsx_structure_partial(&bytes,0).unwrap();
        for (got,wanted) in score_xlsx_tables_partial(&tables).unwrap().iter().zip(case["scores"].as_array().unwrap()) {
            assert_eq!(got.score.map(u64::from),wanted["score"].as_u64(),"{name}");
            assert_eq!(got.passed as u64,wanted["passed"].as_u64().unwrap(),"{name}");
            assert_eq!(got.total as u64,wanted["total"].as_u64().unwrap(),"{name}");
            let mut ids: Vec<_>=got.failed_rules.iter().map(|v|v.rule_id.as_str()).collect();
            let mut expected: Vec<_>=wanted["failed_rules"].as_array().unwrap().iter().map(|v|v["rule_id"].as_str().unwrap()).collect();
            ids.sort(); expected.sort();
            assert_eq!(ids,expected,"{name}");
            assert_eq!(!got.forced_zero_rules.is_empty(),wanted["forced_zero"].as_bool().unwrap(),"{name}");
        }
    }
}

#[test]
fn generated_xlsx_reports_match_reference_for_all_modes() {
    use ibis_pptx::machine_readability_xlsx::analyze_xlsx_report_experimental;
    fn normalize(report:&mut serde_json::Value) {
        for level in ["1","2","3"] { if let Some(score)=report["レベル別スコア"][level].as_object_mut() {
            for key in ["失敗ルール","強制0点ルール"] { if let Some(rules)=score.get_mut(key).and_then(|v|v.as_array_mut()) { rules.sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string()); } }
        }}
        for sheet in report["sheets"].as_array_mut().unwrap() { for table in sheet["評価対象エリア"].as_array_mut().unwrap() {
            table["評価内容"].as_array_mut().unwrap().sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string());
        }}
    }
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let data:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-generated-reference.json")).unwrap()).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let name=case["name"].as_str().unwrap();
        let bytes:Vec<_>=case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
        for mode in ["lite","standard","thorough"] {
            let mut got=analyze_xlsx_report_experimental(&bytes,"fixture.xlsx",mode).unwrap_or_else(|e|panic!("{name} {mode}: {e:?}"));
            let compressed:Vec<_>=case["reports"][mode].as_str().unwrap().as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
            let reference=miniz_oxide::inflate::decompress_to_vec_zlib(&compressed).unwrap();
            let mut expected:serde_json::Value=serde_json::from_slice(&reference).unwrap(); normalize(&mut got);normalize(&mut expected);
            assert_eq!(got,expected,"{name} {mode}");
        }
    }
}

#[test]
fn hidden_worksheets_are_analyzed_in_workbook_order() {
    use ibis_pptx::machine_readability_xlsx::analyze_xlsx_report_experimental;
    fn normalize(report:&mut serde_json::Value) {
        for level in ["1","2","3"] { if let Some(score)=report["レベル別スコア"][level].as_object_mut() {
            for key in ["失敗ルール","強制0点ルール"] { if let Some(rules)=score.get_mut(key).and_then(|v|v.as_array_mut()) { rules.sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string()); } }
        }}
        for sheet in report["sheets"].as_array_mut().unwrap() { for table in sheet["評価対象エリア"].as_array_mut().unwrap() {
            table["評価内容"].as_array_mut().unwrap().sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string());
        }}
    }
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let case:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-hidden-reference.json")).unwrap()).unwrap();
    let bytes:Vec<_>=case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
    for mode in ["lite","standard","thorough"] {
        let mut got=analyze_xlsx_report_experimental(&bytes,"hidden.xlsx",mode).unwrap();
        let mut expected=case["reports"][mode].clone();normalize(&mut got);normalize(&mut expected);
        assert_eq!(got,expected,"{mode}");
    }
}

#[test]
fn chart_sheets_are_inspected_but_excluded_from_sheet_reports() {
    use ibis_pptx::machine_readability_xlsx::analyze_xlsx_report_experimental;
    fn normalize(report:&mut serde_json::Value) {
        for level in ["1","2","3"] { if let Some(score)=report["レベル別スコア"][level].as_object_mut() {
            for key in ["失敗ルール","強制0点ルール"] { if let Some(rules)=score.get_mut(key).and_then(|v|v.as_array_mut()) { rules.sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string()); } }
        }}
        for sheet in report["sheets"].as_array_mut().unwrap() { for table in sheet["評価対象エリア"].as_array_mut().unwrap() {
            table["評価内容"].as_array_mut().unwrap().sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string());
        }}
    }
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let case:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-chart-reference.json")).unwrap()).unwrap();
    let bytes:Vec<_>=case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
    for mode in ["lite","standard","thorough"] {
        let mut got=analyze_xlsx_report_experimental(&bytes,"chart.xlsx",mode).unwrap();
        let mut expected=case["reports"][mode].clone();normalize(&mut got);normalize(&mut expected);
        assert_eq!(got,expected,"{mode}");
    }
}

#[test]
fn xlsx_large_integer_matches_reference_without_rounding() {
    use ibis_pptx::machine_readability_xlsx::analyze_xlsx_report_experimental;
    fn normalize(report:&mut serde_json::Value) {
        for level in ["1","2","3"] { if let Some(score)=report["レベル別スコア"][level].as_object_mut() {
            for key in ["失敗ルール","強制0点ルール"] { if let Some(rules)=score.get_mut(key).and_then(|v|v.as_array_mut()) { rules.sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string()); } }
        }}
        for sheet in report["sheets"].as_array_mut().unwrap() { for table in sheet["評価対象エリア"].as_array_mut().unwrap() {
            table["評価内容"].as_array_mut().unwrap().sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string());
        }}
    }
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let case:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-large-integer-reference.json")).unwrap()).unwrap();
    let bytes:Vec<_>=case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
    for mode in ["lite","standard","thorough"] {
        let mut got=analyze_xlsx_report_experimental(&bytes,"integer.xlsx",mode).unwrap();
        let mut expected=case["reports"][mode].clone();normalize(&mut got);normalize(&mut expected);
        assert_eq!(got,expected,"{mode}");
    }
}

#[test]
fn xlsx_iso_date_cells_match_reference() {
    use ibis_pptx::machine_readability_xlsx::analyze_xlsx_report_experimental;
    fn normalize(report:&mut serde_json::Value) {
        for level in ["1","2","3"] { if let Some(score)=report["レベル別スコア"][level].as_object_mut() {
            for key in ["失敗ルール","強制0点ルール"] { if let Some(rules)=score.get_mut(key).and_then(|v|v.as_array_mut()) { rules.sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string()); } }
        }}
        for sheet in report["sheets"].as_array_mut().unwrap() { for table in sheet["評価対象エリア"].as_array_mut().unwrap() {
            table["評価内容"].as_array_mut().unwrap().sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string());
        }}
    }
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let case:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("harunobu-xlsx-iso-date-reference.json")).unwrap()).unwrap();
    let bytes:Vec<_>=case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
    for mode in ["lite","standard","thorough"] {
        let mut got=analyze_xlsx_report_experimental(&bytes,"iso-date.xlsx",mode).unwrap();
        let mut expected=case["reports"][mode].clone();normalize(&mut got);normalize(&mut expected);
        assert_eq!(got,expected,"{mode}");
    }
}
