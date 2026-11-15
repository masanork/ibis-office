// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
use ibis_doc_model::machine_readability::{candidates, CandidateKind};
use ibis_doc_model::machine_readability_one_data::{multiple_data_reason, ColumnType};
use ibis_doc_model::sheet::{CellKind, CellValue, Sheet, SheetCell};

#[test]
fn matches_pinned_harunobu_examples() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../test/fixtures/machine-readability/harunobu-one-data-reference.json")).unwrap();
    assert_eq!(fixture["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 114);
    for case in cases {
        let column = match case["column"].as_str() {
            Some("numeric") => Some(ColumnType::Numeric),
            Some("text") => Some(ColumnType::Other),
            None => None,
            value => panic!("unexpected column {value:?}"),
        };
        assert_eq!(multiple_data_reason(case["value"].as_str().unwrap(), column), case["reason"].as_str(), "{case}");
    }
}

#[test]
fn live_candidates_do_not_assume_a_non_numeric_column() {
    let mut sheet = Sheet::default();
    sheet.cells = ["1,234", "1,2", "東京、大阪、名古屋", "A,B", "first\nsecond"]
        .into_iter().enumerate().map(|(row, value)| SheetCell {
            row: row as u32, column: 0, text: value.into(), value: CellValue::Text(value.into()),
            kind: CellKind::Text, formula: None, style: 0, phonetic: None, runs: None,
        }).collect::<Vec<_>>().into();
    let rows = candidates(&sheet).into_iter().filter(|item| item.kind == CandidateKind::MultipleData)
        .map(|item| item.range.first_row).collect::<Vec<_>>();
    assert_eq!(rows, [2, 3, 4]);
}

#[test]
fn classifies_long_non_numeric_comma_input() {
    let input = format!("{},末尾", "あ".repeat(100_000));
    assert!(multiple_data_reason(&input, Some(ColumnType::Numeric)).is_some());
}
