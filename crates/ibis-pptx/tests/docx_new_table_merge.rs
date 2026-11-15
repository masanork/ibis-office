// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::{Block, FlowCell, FlowParagraph, FlowRow, FlowTable, VerticalMerge};

#[test]
fn new_table_keeps_vertical_merges_when_saved_and_reopened() {
    let opened =
        ibis_pptx::open(ibis_pptx::blank_file(ibis_pptx::Kind::Document).unwrap()).unwrap();
    let mut presentation = opened.presentation.clone();
    let flow = std::sync::Arc::make_mut(presentation.flow.as_mut().unwrap());
    let cell = |id, merge| FlowCell {
        id,
        span: 1,
        merge,
        blocks: vec![Block::Paragraph(FlowParagraph {
            id: id + 1,
            ..Default::default()
        })],
        ..Default::default()
    };
    flow.body.push(Block::Table(FlowTable {
        id: 1000,
        columns: vec![635_000, 635_000],
        rows: vec![
            FlowRow {
                id: 1001,
                cells: vec![
                    cell(1002, VerticalMerge::Start),
                    cell(1004, VerticalMerge::None),
                ],
                ..Default::default()
            },
            FlowRow {
                id: 1006,
                cells: vec![
                    cell(1007, VerticalMerge::Continue),
                    cell(1009, VerticalMerge::None),
                ],
                ..Default::default()
            },
        ],
        ..Default::default()
    }));
    let data = opened
        .source
        .save(&presentation, ibis_pptx::SaveOptions::default())
        .unwrap();
    let reopened = ibis_pptx::read(&data).unwrap();
    let table = reopened
        .flow
        .as_ref()
        .unwrap()
        .body
        .iter()
        .find_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        table
            .rows
            .iter()
            .map(|row| row.cells.iter().map(|cell| cell.merge).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        vec![
            vec![VerticalMerge::Start, VerticalMerge::None],
            vec![VerticalMerge::Continue, VerticalMerge::None]
        ]
    );
}
