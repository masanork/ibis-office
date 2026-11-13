// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Candidate locations compared with a pinned Harunobu run on synthetic input.
use ibis_doc_model::machine_readability::{candidates, CandidateKind};
use ibis_doc_model::sheet_calc::sheet_of_slide;

#[test]
fn structure_candidates_include_the_reference_locations() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let reference: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-reference.json")).unwrap()).unwrap();
    assert_eq!(reference["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    let p = ibis_pptx::read(&std::fs::read(root.join("structure.xlsx")).unwrap()).unwrap();
    let sheet = sheet_of_slide(&p.slides[0]).unwrap();
    let actual = candidates(sheet);
    let has = |kind, row, col| actual.iter().any(|c| c.kind == kind && c.range.first_row == row && c.range.first_column == col);
    let rules = reference["fixtures"]["structure"]["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 3);
    for result in rules {
        assert_eq!(result["passed"], false);
        for loc in result["locations"].as_array().unwrap() {
            match (result["rule"].as_str().unwrap(), loc.as_str().unwrap()) {
                ("L1-12", "A2:A3") => assert!(has(CandidateKind::MergedCells, 1, 0)),
                ("L1-13", "行3") => assert!(has(CandidateKind::HiddenRow, 2, 0)),
                ("L1-13", "列2") => assert!(has(CandidateKind::HiddenColumn, 0, 1)),
                ("L2-06", "C5") => assert!(has(CandidateKind::Formula, 4, 2)),
                x => panic!("unexpected reference location {x:?}"),
            }
        }
    }
    assert_eq!(actual.len(), 4);
    // A subsequent edit is checked from the live model, not the saved bytes.
    let mut changed = (**sheet).clone();
    changed.merges.clear();
    changed.hidden_rows.clear();
    changed.hidden_columns.clear();
    for c in &mut changed.cells { c.formula = None; }
    assert!(candidates(&changed).is_empty());
}

#[test]
fn header_merge_is_a_candidate_not_an_automatic_violation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let reference: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-reference.json")).unwrap()).unwrap();
    let result = reference["fixtures"]["header"]["rules"].as_array().unwrap().iter().find(|r| r["rule"] == "L1-12").unwrap();
    assert_eq!(result["passed"], true);
    assert_eq!(result["locations"][0], "B1:C1");
    let p = ibis_pptx::read(&std::fs::read(root.join("header-merge.xlsx")).unwrap()).unwrap();
    let candidates = candidates(sheet_of_slide(&p.slides[0]).unwrap());
    assert!(candidates.iter().any(|c| c.kind == CandidateKind::MergedCells && c.range.first_row == 0 && c.range.first_column == 1));
}

#[test]
fn csv_has_no_excel_structure_candidates() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let reference: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-reference.json")).unwrap()).unwrap();
    assert!(reference["fixtures"]["csv"]["rules"].as_array().unwrap().is_empty());
    let text = std::fs::read_to_string(root.join("data.csv")).unwrap();
    let xlsx = ibis_pptx::csv::workbook_from(&text, "data").unwrap();
    let p = ibis_pptx::read(&xlsx).unwrap();
    assert!(candidates(sheet_of_slide(&p.slides[0]).unwrap()).is_empty());
}

#[test]
fn live_character_candidates_match_reference_examples() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let reference: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-character-reference.json")).unwrap()).unwrap();
    assert_eq!(reference["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    let cases = reference["cases"].as_array().unwrap();
    let csv = cases.iter().map(|case| case["value"].as_str().unwrap()).collect::<Vec<_>>().join("\n");
    let bytes = ibis_pptx::csv::workbook_from(&csv, "characters").unwrap();
    let p = ibis_pptx::read(&bytes).unwrap();
    let findings = candidates(sheet_of_slide(&p.slides[0]).unwrap());
    for (row, case) in cases.iter().enumerate() {
        let detected = findings.iter().any(|f| f.kind == CandidateKind::PlatformDependentCharacters && f.range.first_row == row as u32 && f.range.first_column == 0);
        assert_eq!(detected, case["detected"].as_bool().unwrap(), "{}", case["value"]);
    }
}

#[test]
fn rust_scoring_matches_pinned_harunobu() {
    use ibis_doc_model::machine_readability_scoring::{score_level, Level, RuleResult, Severity};
    fn severity(v: &serde_json::Value) -> Option<Severity> {
        Some(match v.as_str()? {
            "fatal" => Severity::Fatal, "critical" => Severity::Critical,
            "major" => Severity::Major, "minor" => Severity::Minor, "info" => Severity::Info,
            other => panic!("unknown severity {other}"),
        })
    }
    fn name(s: Severity) -> &'static str {
        match s { Severity::Fatal => "fatal", Severity::Critical => "critical", Severity::Major => "major", Severity::Minor => "minor", Severity::Info => "info" }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-scoring-reference.json")).unwrap()).unwrap();
    assert_eq!(fixture["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in fixture["cases"].as_array().unwrap() {
        let level = match case["level"].as_u64().unwrap() { 1 => Level::One, 2 => Level::Two, 3 => Level::Three, _ => panic!("bad level") };
        let results: Vec<_> = case["rules"].as_array().unwrap().iter().map(|v| RuleResult {
            rule_id: v["rule_id"].as_str().unwrap().to_string(), passed: v["passed"].as_bool().unwrap(),
            confidence: v["confidence"].as_f64().unwrap(), severity: severity(&v["severity"]),
        }).collect();
        let actual = score_level(level, &results);
        let failed = |rules: &[ibis_doc_model::machine_readability_scoring::FailedRule]| rules.iter().map(|r| serde_json::json!({"rule_id": r.rule_id, "severity": name(r.severity)})).collect::<Vec<_>>();
        let actual_json = serde_json::json!({ "level": case["level"], "score": actual.score, "passed": actual.passed, "total": actual.total,
            "forced_zero": !actual.forced_zero_rules.is_empty(), "forced_zero_rules": failed(&actual.forced_zero_rules), "failed_rules": failed(&actual.failed_rules) });
        assert_eq!(actual_json, case["expected"], "level {} input {}", case["level"], case["rules"]);
    }
}

#[test]
fn rust_rules_match_reference_when_table_context_is_supplied() {
    use std::collections::BTreeSet;
    use ibis_doc_model::machine_readability_rules::{merged_cells, hidden_rows_columns, formula_cells, FindingSeverity, UsedExtent};
    use ibis_doc_model::sheet::CellRange;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let reference: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-reference.json")).unwrap()).unwrap();
    for (name, filename, last_row) in [("structure", "structure.xlsx", 4), ("header", "header-merge.xlsx", 3)] {
        let p = ibis_pptx::read(&std::fs::read(root.join(filename)).unwrap()).unwrap();
        let sheet = sheet_of_slide(&p.slides[0]).unwrap();
        let table = CellRange { first_row: 0, first_column: 0, last_row, last_column: 2 };
        let headers = BTreeSet::from([0]);
        let results = [merged_cells(sheet, table, Some(&headers)), hidden_rows_columns(sheet, UsedExtent::Range(table)), formula_cells(sheet, table, true)];
        let expected = reference["fixtures"][name]["rules"].as_array().unwrap();
        for actual in &results {
            let expected_rule = expected.iter().find(|v| v["rule"] == actual.result.rule_id).unwrap();
            assert_eq!(actual.result.passed, expected_rule["passed"].as_bool().unwrap(), "{name} {}", actual.result.rule_id);
            assert_eq!(actual.result.confidence == 0.0, expected_rule["skipped"].as_bool().unwrap());
            let locations: Vec<&str> = actual.findings.iter().map(|f| f.location.as_str()).collect();
            let wanted: Vec<&str> = expected_rule["locations"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
            assert_eq!(locations, wanted, "{name} {}", actual.result.rule_id);
            let severity = match actual.result.severity.unwrap() {
                ibis_doc_model::machine_readability_scoring::Severity::Fatal => "fatal",
                ibis_doc_model::machine_readability_scoring::Severity::Critical => "critical",
                ibis_doc_model::machine_readability_scoring::Severity::Major => "major",
                ibis_doc_model::machine_readability_scoring::Severity::Minor => "minor",
                ibis_doc_model::machine_readability_scoring::Severity::Info => "info",
            };
            assert_eq!(severity, expected_rule["severity"].as_str().unwrap());
            let levels: Vec<&str> = actual.findings.iter().map(|f| match f.severity { FindingSeverity::Error => "error", FindingSeverity::Warning => "warning", FindingSeverity::Info => "info" }).collect();
            let wanted_levels: Vec<&str> = expected_rule["finding_severities"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
            assert_eq!(levels, wanted_levels);
        }
        if name == "header" { assert_eq!(results[0].findings[0].severity, FindingSeverity::Info); }
        let incomplete = formula_cells(sheet, table, false);
        assert_eq!(incomplete.result.confidence, 0.0, "incomplete formula metadata must be skipped");
    }
}

#[test]
fn rust_rule_boundaries_and_missing_data_are_explicit() {
    use std::collections::BTreeSet;
    use ibis_doc_model::machine_readability_rules::{merged_cells, hidden_rows_columns, formula_cells, FindingSeverity, UsedExtent};
    use ibis_doc_model::machine_readability_scoring::Severity;
    use ibis_doc_model::sheet::CellRange;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let p = ibis_pptx::read(&std::fs::read(root.join("structure.xlsx")).unwrap()).unwrap();
    let mut sheet = (**sheet_of_slide(&p.slides[0]).unwrap()).clone();
    let table = CellRange { first_row: 0, first_column: 0, last_row: 4, last_column: 2 };
    // One merge is completely in the header; one touches data; an unrelated merge is outside.
    sheet.merges = vec![CellRange { first_row: 0, last_row: 0, first_column: 0, last_column: 1 },
        CellRange { first_row: 0, last_row: 1, first_column: 1, last_column: 1 },
        CellRange { first_row: 9, last_row: 9, first_column: 0, last_column: 1 }];
    let merged = merged_cells(&sheet, table, Some(&BTreeSet::from([0])));
    assert!(!merged.result.passed);
    assert_eq!(merged.result.severity, Some(Severity::Fatal));
    assert_eq!(merged.findings.iter().map(|f| f.severity).collect::<Vec<_>>(), vec![FindingSeverity::Info, FindingSeverity::Error]);
    let row_height = *sheet.hidden_rows.values().next().unwrap();
    sheet.hidden_rows.clear();
    for row in [1, 2, 3, 10] { sheet.hidden_rows.insert(row, row_height); }
    let hidden = hidden_rows_columns(&sheet, UsedExtent::Range(table));
    assert!(hidden.findings.iter().any(|f| f.location == "行2-4"));
    assert!(!hidden.findings.iter().any(|f| f.location.contains("11")));
    assert_eq!(formula_cells(&sheet, table, false).result.confidence, 0.0);
    assert_eq!(merged_cells(&sheet, table, None).result.confidence, 0.0);
    assert_eq!(hidden_rows_columns(&sheet, UsedExtent::Unknown).result.confidence, 0.0);
    sheet.merges.push(CellRange { first_row: 3, last_row: 2, first_column: 0, last_column: 1 });
    assert_eq!(merged_cells(&sheet, table, Some(&BTreeSet::new())).result.confidence, 0.0);
    let invalid = CellRange { first_row: 3, last_row: 2, first_column: 0, last_column: 1 };
    assert_eq!(hidden_rows_columns(&sheet, UsedExtent::Range(invalid)).result.confidence, 0.0);
    assert_eq!(formula_cells(&sheet, invalid, true).result.confidence, 0.0);
}

#[test]
fn rust_workbook_rules_match_pinned_harunobu() {
    use ibis_doc_model::machine_readability_rules::{file_format, one_table_per_sheet, WorkbookFormat, DetectedTable};
    use ibis_doc_model::sheet::CellRange;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability");
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("harunobu-workbook-rules-reference.json")).unwrap()).unwrap();
    assert_eq!(fixture["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in fixture["formats"].as_array().unwrap() {
        let format = match case["format"].as_str().unwrap() { "xlsx" => WorkbookFormat::Xlsx, "csv" => WorkbookFormat::Csv, "tsv" => WorkbookFormat::Tsv, _ => WorkbookFormat::Other };
        let actual = file_format(format);
        assert_eq!(actual.result.passed, case["passed"].as_bool().unwrap());
        assert_eq!(actual.result.confidence, case["confidence"].as_f64().unwrap());
        assert_eq!(actual.findings.iter().map(|f| f.location.as_str()).collect::<Vec<_>>(), case["locations"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect::<Vec<_>>());
    }
    assert_eq!(file_format(WorkbookFormat::Unknown).result.confidence, 0.0);
    for case in fixture["tables"].as_array().unwrap() {
        let tables: Vec<_> = case["tables"].as_array().unwrap().iter().map(|v| {
            let a = v.as_array().unwrap();
            DetectedTable { range: CellRange { first_row: a[0].as_u64().unwrap() as u32, first_column: a[1].as_u64().unwrap() as u32,
                last_row: a[2].as_u64().unwrap() as u32, last_column: a[3].as_u64().unwrap() as u32 }, confidence: a[4].as_f64().unwrap() }
        }).collect();
        let current = tables[case["current"].as_u64().unwrap() as usize].range;
        let actual = one_table_per_sheet(current, Some(&tables));
        assert_eq!(actual.result.passed, case["passed"].as_bool().unwrap());
        assert_eq!(actual.result.confidence, case["confidence"].as_f64().unwrap());
        assert_eq!(actual.findings.iter().map(|f| f.location.as_str()).collect::<Vec<_>>(), case["locations"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect::<Vec<_>>());
        assert!(actual.findings.iter().all(|f| f.severity == ibis_doc_model::machine_readability_rules::FindingSeverity::Warning));
    }
    assert_eq!(one_table_per_sheet(CellRange::default(), None).result.confidence, 0.0);
    assert_eq!(one_table_per_sheet(CellRange::default(), Some(&[])).result.confidence, 0.0);
    let invalid = DetectedTable { range: CellRange::default(), confidence: f64::NAN };
    assert_eq!(one_table_per_sheet(CellRange::default(), Some(&[invalid])).result.confidence, 0.0);
}
