// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
use ibis_doc_model::machine_readability::{candidates, CandidateKind};
use ibis_doc_model::sheet::{CellKind, CellRange, CellValue, Sheet, SheetCell};

fn cell(row: u32, column: u32, value: CellValue, formula: Option<&str>) -> SheetCell {
    SheetCell { row, column, text: String::new(), value, kind: CellKind::Text,
        formula: formula.map(str::to_owned), style: 0, phonetic: None, runs: None }
}

#[test]
fn reports_candidates_only_within_the_data_extent() {
    let mut sheet = Sheet::default();
    sheet.cells = vec![cell(2, 3, CellValue::Text("x".into()), None),
        cell(3, 2, CellValue::Empty, Some("SUM(A1:A2)")),
        cell(9, 9, CellValue::Empty, None)].into();
    sheet.merges.push(CellRange { first_row: 0, last_row: 0, first_column: 0, last_column: 1 });
    sheet.hidden_rows.insert(1, 0);
    sheet.hidden_rows.insert(8, 0);
    sheet.hidden_columns.insert(3, 0);
    sheet.hidden_columns.insert(8, 0);
    let found = candidates(&sheet);
    assert_eq!(found.iter().map(|f| f.kind).collect::<Vec<_>>(),
        [CandidateKind::MergedCells, CandidateKind::Formula, CandidateKind::HiddenRow, CandidateKind::HiddenColumn]);
    assert_eq!(found[1].range, CellRange { first_row: 3, last_row: 3, first_column: 2, last_column: 2 });
    assert_eq!(found[2].range.last_column, 3);
    assert_eq!(found[3].range.last_row, 3);
}

#[test]
fn no_data_does_not_report_unrelated_hidden_rows() {
    let mut sheet = Sheet::default();
    sheet.cells = vec![cell(99, 99, CellValue::Empty, Some("")), cell(100, 100, CellValue::Empty, None)].into();
    sheet.hidden_rows.insert(0, 0);
    sheet.hidden_columns.insert(0, 0);
    assert!(candidates(&sheet).is_empty());
}
