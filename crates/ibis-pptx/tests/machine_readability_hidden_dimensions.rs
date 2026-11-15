// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
use ibis_doc_model::machine_readability::{candidates, CandidateKind};
use ibis_doc_model::sheet_calc::sheet_of_slide;

#[test]
fn zero_dimensions_are_reported_from_a_loaded_workbook() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test/fixtures/machine-readability/zero-dimensions.xlsx");
    let presentation = ibis_pptx::read(&std::fs::read(file).unwrap()).unwrap();
    let sheet = sheet_of_slide(&presentation.slides[0]).unwrap();
    assert!(sheet.is_row_hidden(2));
    assert!(sheet.is_col_hidden(1));
    assert!(!sheet.hidden_rows.contains_key(&2));
    assert!(!sheet.hidden_columns.contains_key(&1));
    let found = candidates(sheet);
    assert!(found.iter().any(|c| c.kind == CandidateKind::HiddenRow && c.range.first_row == 2));
    assert!(found.iter().any(|c| c.kind == CandidateKind::HiddenColumn && c.range.first_column == 1));
}
