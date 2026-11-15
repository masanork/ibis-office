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

#[test]
fn character_candidates_match_pinned_harunobu_examples() {
    let fixture = include_str!("../../../test/fixtures/machine-readability/harunobu-character-reference.tsv");
    let mut count = 0;
    for line in fixture.lines().skip(1) {
        count += 1;
        let (value, expected) = line.split_once('\t').unwrap();
        let mut sheet = Sheet::default();
        sheet.cells = vec![cell(0, 0, CellValue::Text(value.into()), None)].into();
        let found = candidates(&sheet).iter().any(|candidate| candidate.kind == CandidateKind::PlatformDependentCharacters);
        assert_eq!(found, expected == "true", "{value}");
    }
    assert_eq!(count, 22);
}

#[test]
fn private_use_advisories_cover_all_planes_without_noncharacters() {
    for (codepoint, expected) in [(0xd7ff, false), (0xe000, true), (0xf8ff, true), (0xf900, false),
        (0xeffff, false), (0xf0000, true), (0xffffd, true), (0xffffe, false),
        (0x100000, true), (0x10fffd, true), (0x10fffe, false), (0x10ffff, false)] {
        let character = char::from_u32(codepoint).unwrap();
        let mut sheet = Sheet::default();
        sheet.cells = vec![cell(2, 3, CellValue::Text(character.to_string()), None)].into();
        let findings = candidates(&sheet);
        assert_eq!(findings.iter().any(|candidate| candidate.kind == CandidateKind::PrivateUseCharacters), expected, "U+{codepoint:04X}");
        assert!(!findings.iter().any(|candidate| candidate.kind == CandidateKind::PlatformDependentCharacters), "U+{codepoint:04X}");
    }
}

#[test]
fn one_cell_can_have_both_character_advisories() {
    let mut sheet = Sheet::default();
    sheet.cells = vec![cell(4, 2, CellValue::Text("①\u{e000}".into()), None)].into();
    let findings = candidates(&sheet);
    assert_eq!(findings.iter().map(|f| f.kind).collect::<Vec<_>>(),
        [CandidateKind::PlatformDependentCharacters, CandidateKind::PrivateUseCharacters]);
    assert_eq!(findings[0].range, findings[1].range);
}
