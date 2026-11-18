// SPDX-License-Identifier: AGPL-3.0-only
use ibis_pptx::markdown;
use serde_json::Value;

fn excerpt<'a>(export: &'a Value, location: &Value) -> &'a str {
    let markdown = export["markdown"].as_str().unwrap();
    let bytes = &location["markdownBytes"];
    &markdown[bytes[0].as_u64().unwrap() as usize..bytes[1].as_u64().unwrap() as usize]
}

#[test]
fn document_ranges_point_to_utf8_markdown_and_footnotes() {
    let original =
        b"# Heading\n\n| A | B |\n| --- | --- |\n| one[^1] | {++new++} |\n\n[^1]: footnote\n";
    let docx = markdown::import_markdown_to_docx_bytes(original).unwrap();
    let mut presentation = ibis_pptx::read(&docx).unwrap();
    let flow = std::sync::Arc::make_mut(presentation.flow.as_mut().unwrap());
    for block in &mut flow.body {
        if let ibis_doc_model::Block::Table(table) = block {
            table.rows[0].cells[0].span = 2;
        }
    }
    let original_export = markdown::export_ai_source(&docx).unwrap();
    assert_eq!(original_export["sourceSha256"].as_str().unwrap().len(), 64);
    let export = markdown::export_ai_document(&presentation).unwrap();
    assert_eq!(
        export["markdown"],
        markdown::export_presentation_to_markdown(&presentation).unwrap()
    );
    let blocks = export["blocks"].as_array().unwrap();
    assert!(
        blocks
            .iter()
            .any(|block| excerpt(&export, block).starts_with("# Heading"))
    );
    assert!(blocks.iter().any(
        |block| block["source"]["kind"] == "table" && excerpt(&export, block).contains("[^1]")
    ));
    assert!(
        blocks
            .iter()
            .any(|block| block["source"]["kind"] == "footnote"
                && excerpt(&export, block).contains("footnote"))
    );
    assert!(
        export["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["code"] == "table_merge_not_preserved")
    );
}

#[test]
fn workbook_cells_reference_their_unescaped_coordinates() {
    let xlsx = markdown::import_markdown_to_xlsx_bytes(
        "| 区分 | 金額 |\n| --- | --- |\n| 国内 | 42 |\n".as_bytes(),
    )
    .unwrap();
    let presentation = ibis_pptx::read(&xlsx).unwrap();
    let export = markdown::export_ai_source(&xlsx).unwrap();
    assert_eq!(
        export["markdown"],
        markdown::export_workbook_markdown(&presentation).unwrap()
    );
    let cells = export["cells"].as_array().unwrap();
    let cell = cells
        .iter()
        .find(|c| c["source"]["row"] == 2 && c["source"]["column"] == 1)
        .unwrap();
    assert_eq!(
        cell["source"]["sheet"],
        presentation.workbook.as_ref().unwrap().sheets[0].name
    );
    assert_eq!(cell["valueType"], "text");
    assert_eq!(cell["value"], "国内");
    assert_eq!(excerpt(&export, cell), "国内");
}

#[test]
fn lawtext_and_bundled_law_xml_have_article_locators() {
    for input in [
        include_bytes!("lawtext/administrative-procedure-act.lawtext").as_slice(),
        include_bytes!("lawtext/321CONSTITUTION.xml").as_slice(),
    ] {
        let export = markdown::export_ai_source(input).unwrap();
        let article = &export["articles"][0];
        assert_eq!(article["source"]["supplementIndex"], Value::Null);
        assert!(!article["source"]["lawNum"].as_str().unwrap().is_empty());
        assert!(excerpt(&export, article).contains("第一条"));
    }
}

#[test]
fn citations_resolve_against_the_original_document_only() {
    let docx =
        markdown::import_markdown_to_docx_bytes("# 見出し\n\n本文です。\n".as_bytes()).unwrap();
    let export = markdown::export_ai_source(&docx).unwrap();
    let text = export["markdown"].as_str().unwrap();
    let quote = "本文";
    let start = text.find(quote).unwrap();
    let mut query = serde_json::json!({
        "version": 1,
        "sourceSha256": export["sourceSha256"],
        "markdownBytes": [start, start + quote.len()],
        "quote": quote,
    });
    let found = markdown::locate_ai_source(&docx, &query).unwrap();
    let matches = found["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["source"]["kind"], "paragraph");
    assert_eq!(matches[0]["source"]["index"], 1);
    assert_eq!(
        u64::from(markdown::locate_ai_paragraph(&docx, &query).unwrap()),
        matches[0]["source"]["modelId"].as_u64().unwrap()
    );
    let cross_block = serde_json::json!({
        "version": 1,
        "sourceSha256": export["sourceSha256"],
        "markdownBytes": [0, text.len()],
        "quote": text,
    });
    assert!(markdown::locate_ai_paragraph(&docx, &cross_block).is_err());

    assert!(
        markdown::locate_ai_source(b"different input", &query)
            .unwrap_err()
            .to_string()
            .contains("SHA-256")
    );
    query["markdownBytes"][0] = serde_json::json!(start + 1);
    assert!(markdown::locate_ai_source(&docx, &query).is_err());
}

#[test]
fn ai_edit_previews_and_round_trips_a_single_paragraph() {
    let docx =
        markdown::import_markdown_to_docx_bytes("# 見出し\n\n修正前。\n\n別の段落。\n".as_bytes())
            .unwrap();
    let export = markdown::export_ai_source(&docx).unwrap();
    let text = export["markdown"].as_str().unwrap();
    let before = "修正前。";
    let start = text.find(before).unwrap();
    let mut proposal = serde_json::json!({
        "version": 1,
        "sourceSha256": export["sourceSha256"],
        "markdownBytes": [start, start + before.len()],
        "quote": before,
        "before": before,
        "replacement": "修正後。",
    });
    let preview = markdown::preview_ai_edit(&docx, &proposal).unwrap();
    assert_eq!(preview["before"], before);
    assert_eq!(preview["after"], "修正後。");
    let edited = markdown::apply_ai_edit(&docx, &proposal).unwrap();
    let remapped = markdown::export_ai_source(&edited).unwrap();
    let after = remapped["markdown"].as_str().unwrap();
    assert!(
        after.contains("# 見出し") && after.contains("修正後。") && after.contains("別の段落。")
    );
    assert!(!after.contains(before));
    assert!(markdown::apply_ai_edit(&edited, &proposal).is_err());

    proposal["replacement"] = serde_json::json!("new\nparagraph");
    assert!(markdown::preview_ai_edit(&docx, &proposal).is_err());
    proposal["replacement"] = serde_json::json!("修正後。");
    proposal["before"] = serde_json::json!("合わない本文");
    assert!(markdown::apply_ai_edit(&docx, &proposal).is_err());
}

#[test]
fn ai_edit_binds_a_partial_replacement_to_the_cited_run() {
    let docx =
        markdown::import_markdown_to_docx_bytes("同じ語と**同じ語の強調**、*斜体*。\n".as_bytes())
            .unwrap();
    let export = markdown::export_ai_source(&docx).unwrap();
    let markdown = export["markdown"].as_str().unwrap();
    let before = "同じ語と同じ語の強調、斜体。";
    let quote = "同じ語";
    let plain_start = before.rfind(quote).unwrap();
    let start = markdown.rfind(quote).unwrap();
    let mut proposal = serde_json::json!({
        "version": 1,
        "sourceSha256": export["sourceSha256"],
        "markdownBytes": [start, start + quote.len()],
        "quote": quote,
        "paragraphBytes": [plain_start, plain_start + quote.len()],
        "before": before,
        "replacement": "別の語",
    });

    let preview = markdown::preview_ai_edit(&docx, &proposal).unwrap();
    assert_eq!(preview["before"], before);
    assert_eq!(preview["after"], "同じ語と別の語の強調、斜体。");
    let edited = markdown::apply_ai_edit(&docx, &proposal).unwrap();
    let export = markdown::export_ai_source(&edited).unwrap();
    assert_eq!(export["markdown"], "同じ語と**別の語の強調**、*斜体*。\n");

    proposal["paragraphBytes"] = serde_json::json!([0, quote.len()]);
    assert!(markdown::apply_ai_edit(&docx, &proposal).is_err());
    proposal["paragraphBytes"] = serde_json::json!([plain_start + 1, plain_start + quote.len()]);
    assert!(markdown::preview_ai_edit(&docx, &proposal).is_err());
    proposal.as_object_mut().unwrap().remove("paragraphBytes");
    assert!(markdown::preview_ai_edit(&docx, &proposal).is_err());
}

#[test]
fn docx_endnotes_export_and_resolve_separately_from_same_id_footnotes() {
    use ibis_doc_model::{Block, flow::FlowRunKind};

    let mut opened = markdown::open_markdown(
        "本文[^1]と補足[^2]。\n\n[^1]: ページ脚注\n[^2]: 文末注記\n"
            .as_bytes()
            .to_vec(),
    )
    .unwrap();
    let flow = std::sync::Arc::make_mut(opened.presentation.flow.as_mut().unwrap());
    let Block::Paragraph(body) = &mut flow.body[0] else {
        panic!("expected the body paragraph");
    };
    for run in &mut body.runs {
        if matches!(run.kind, FlowRunKind::NoteRef { id: 2, .. }) {
            run.kind = FlowRunKind::NoteRef {
                endnote: true,
                id: 1,
            };
        }
    }
    let mut endnote = flow.footnotes.remove(1);
    endnote.id = 1;
    flow.endnotes.push(endnote);
    let docx = opened
        .source
        .save(&opened.presentation, ibis_pptx::SaveOptions::default())
        .unwrap();

    let export = markdown::export_ai_source(&docx).unwrap();
    let text = export["markdown"].as_str().unwrap();
    assert_eq!(markdown::export_docx_to_markdown(&docx).unwrap(), text);
    assert!(text.contains("本文[^1]と補足[^endnote-1]。"));
    assert!(text.contains("[^1]: ページ脚注"));
    assert!(text.contains("[^endnote-1]: 文末注記"));
    assert_eq!(export["diagnostics"], serde_json::json!([]));

    for (quote, part, kind) in [
        ("ページ脚注", "footnotes", "footnote"),
        ("文末注記", "endnotes", "endnote"),
    ] {
        let start = text.find(quote).unwrap();
        let query = serde_json::json!({
            "version": 1,
            "sourceSha256": export["sourceSha256"],
            "markdownBytes": [start, start + quote.len()],
            "quote": quote,
        });
        let found = markdown::locate_ai_source(&docx, &query).unwrap();
        let matches = found["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0]["source"],
            serde_json::json!({"part": part, "kind": kind, "id": 1}),
        );
        assert!(excerpt(&export, &matches[0]).ends_with(&format!("{quote}\n")));
    }
}
