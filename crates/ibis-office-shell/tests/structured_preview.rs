// SPDX-License-Identifier: AGPL-3.0-only
#![cfg(not(target_family = "wasm"))]

use ibis_doc_model::{Block, FlowRunKind};

#[test]
fn native_file_reader_creates_a_preview_with_literal_values() {
    let path = std::env::temp_dir().join(format!(
        "ibis-structured-preview-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &path,
        br#"{"entry":"[text](https://example.invalid)","id":"00123"}"#,
    )
    .unwrap();
    let loaded = ibis_office_shell::read_file::read_file(&path);
    std::fs::remove_file(path).unwrap();
    let loaded = loaded.unwrap();
    assert!(loaded.structured_preview);
    let flow = loaded.presentation.flow.as_ref().unwrap();
    let runs = flow
        .body
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(&paragraph.runs),
            _ => None,
        })
        .flatten();
    assert!(runs.clone().any(|run| matches!(&run.kind, FlowRunKind::Text(text) if text.contains("[text](https://example.invalid)"))));
    assert!(
        runs.into_iter()
            .any(|run| matches!(&run.kind, FlowRunKind::Text(text) if text.contains("00123")))
    );
}

#[test]
fn native_file_reader_renders_summary_and_auto_tables_for_json_arrays() {
    let path = std::env::temp_dir().join(format!(
        "ibis-structured-preview-table-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &path,
        br#"{"env":"prod","users":[{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}]}"#,
    )
    .unwrap();
    let loaded = ibis_office_shell::read_file::read_file(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let flow = loaded.presentation.flow.as_ref().unwrap();
    let tables = flow
        .body
        .iter()
        .filter(|b| matches!(b, Block::Table(_)))
        .count();
    // 1 summary card table + 1 auto-detected `users` array table.
    assert_eq!(tables, 2, "expected summary table + users table");
}
