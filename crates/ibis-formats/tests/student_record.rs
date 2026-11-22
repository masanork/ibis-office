// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::{Block, FlowDocument, FlowRunKind};
use ibis_formats::student_record::extract;

fn example() -> FlowDocument {
    let data = include_bytes!("../../../test/fixtures/student-record/elementary-school.docx");
    ibis_formats::read_docx(data)
        .unwrap()
        .flow
        .unwrap()
        .as_ref()
        .clone()
}

#[test]
fn reads_values_and_points_to_the_pupil_cell() {
    let doc = example();
    let values = extract(&doc).unwrap();
    assert_eq!(values.name.text, "紅葉 葵");
    assert_eq!(values.birth_date.text, "平成３１年４月２日生");
    assert_eq!(
        values.next_school.text,
        "架空市立そよかぜ中学校\n架空市ひなた町１丁目"
    );
    let source = ibis_doc_model::flow_command::paragraph(&doc, values.name.paragraph).unwrap();
    assert!(matches!(&source.runs[0].kind, FlowRunKind::Text(value) if value == "紅葉 葵"));
}

#[test]
fn leaves_unfilled_values_blank_even_with_a_guardian_name() {
    let mut doc = example();
    let Block::Table(table) = &mut doc.body[2] else {
        panic!("expected form table")
    };
    for (row, cell, blank) in [(2, 2, ""), (3, 2, "　年　月　日生"), (5, 4, "")] {
        let blocks = &mut table.rows[row].cells[cell].blocks;
        blocks.truncate(1);
        let Block::Paragraph(para) = &mut blocks[0] else {
            panic!("expected paragraph")
        };
        para.runs[0].kind = FlowRunKind::Text(blank.into());
    }
    let values = extract(&doc).unwrap();
    assert!(values.name.text.is_empty());
    assert!(values.birth_date.text.is_empty());
    assert!(values.next_school.text.is_empty());
}

#[test]
fn rejects_duplicate_forms_and_a_missing_pupil_name_label() {
    let mut doc = example();
    doc.body.push(doc.body[2].clone());
    assert!(extract(&doc).is_none());
    doc.body.pop();
    let Block::Table(table) = &mut doc.body[2] else {
        panic!("expected form table")
    };
    table.rows[2].cells[1].blocks.clear();
    assert!(extract(&doc).is_none());
}

#[test]
fn reads_the_full_demo_alongside_the_instruction_records() {
    let bytes = include_bytes!("../../../test/fixtures/student-record/elementary-school-full.docx");
    let doc = ibis_formats::read_docx(bytes).unwrap().flow.unwrap();
    let record = extract(&doc).unwrap();
    assert_eq!(record.name.text, "紅葉 葵");
    assert_eq!(record.birth_date.text, "平成３１年４月２日生");
    assert_eq!(
        record.next_school.text,
        "架空市立そよかぜ中学校\n架空市ひなた町１丁目"
    );
    let source = ibis_doc_model::flow_command::paragraph(&doc, record.name.paragraph).unwrap();
    assert!(matches!(&source.runs[0].kind, FlowRunKind::Text(text) if text == "紅葉 葵"));
}
