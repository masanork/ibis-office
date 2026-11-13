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
