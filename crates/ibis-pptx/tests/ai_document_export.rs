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
fn docx_ai_export_keeps_table_grid_and_nested_blocks() {
    use ibis_doc_model::{Block, flow::VerticalMerge};

    let mut opened = markdown::open_markdown(
        b"# Report\n\n| Group | Type | Count |\n| --- | --- | --- |\n| A | | 12 |\n| | | 8 |\n\n| Detail |\n| --- |\n| Nested value |\n".to_vec()
    ).unwrap();
    let flow = std::sync::Arc::make_mut(opened.presentation.flow.as_mut().unwrap());
    let nested = flow.body.pop().unwrap();
    assert!(matches!(nested, Block::Table(_)));
    let Block::Table(outer) = &mut flow.body[1] else {
        panic!("expected the outer table");
    };
    for (row, merge) in [(1, VerticalMerge::Start), (2, VerticalMerge::Continue)] {
        let cells = &mut outer.rows[row].cells;
        cells.remove(1);
        cells[0].span = 2;
        cells[0].merge = merge;
    }
    let cell = &mut outer.rows[1].cells[0];
    let mut trailing = cell.blocks[0].clone();
    let Block::Paragraph(paragraph) = &mut trailing else {
        unreachable!()
    };
    paragraph.id = 10_000;
    paragraph.runs.clear();
    cell.blocks.extend([nested, trailing]);
    let docx = opened
        .source
        .save(&opened.presentation, ibis_pptx::SaveOptions::default())
        .unwrap();

    let export = markdown::export_ai_source(&docx).unwrap();
    let table = &export["tables"][0];
    assert_eq!(table["source"], export["blocks"][1]["source"]);
    assert_eq!(table["structure"]["columnCount"], 3);
    let rows = &table["structure"]["rows"];
    assert_eq!(rows[0]["header"], true);
    assert_eq!(rows[1]["row"], 2);
    assert_eq!(rows[1]["cells"][0]["columnSpan"], 2);
    assert_eq!(rows[1]["cells"][0]["verticalMerge"], "start");
    assert_eq!(rows[1]["cells"][1]["column"], 3);
    assert_eq!(rows[2]["cells"][0]["verticalMerge"], "continue");
    let blocks = &rows[1]["cells"][0]["blocks"];
    assert_eq!(blocks[0]["markdown"], "A");
    assert_eq!(blocks[1]["kind"], "table");
    assert_eq!(
        blocks[1]["rows"][1]["cells"][0]["blocks"][0]["markdown"],
        "Nested value",
    );
}

#[test]
fn docx_table_citations_distinguish_equal_text_in_different_cells() {
    let docx = markdown::import_markdown_to_docx_bytes(
        "本文\n\n| A | B | C |\n| --- | --- | --- |\n| 同じ語 | | 同じ語 |\n".as_bytes(),
    )
    .unwrap();
    let export = markdown::export_ai_source(&docx).unwrap();
    let text = export["markdown"].as_str().unwrap();
    let quote = "同じ語";
    let start = text.rfind(quote).unwrap();
    let citation = |start, end| {
        serde_json::json!({
            "version": 1,
            "sourceSha256": export["sourceSha256"],
            "markdownBytes": [start, end],
            "quote": &text[start..end],
        })
    };
    let result = markdown::locate_ai_source(&docx, &citation(start, start + quote.len())).unwrap();
    let matches = result["tableCellMatches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(excerpt(&export, &matches[0]), quote);
    assert_eq!(matches[0]["source"]["row"], 2);
    assert_eq!(matches[0]["source"]["column"], 3);
    assert_eq!(matches[0]["source"]["index"], 1);
    assert_eq!(
        matches[0]["source"]["tableModelId"],
        result["matches"][0]["source"]["modelId"]
    );
    assert_eq!(
        matches[0]["source"]["modelId"],
        export["tables"][0]["structure"]["rows"][1]["cells"][2]["modelId"]
    );
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);

    let first = text.find(quote).unwrap();
    let across = markdown::locate_ai_source(&docx, &citation(first, start + quote.len())).unwrap();
    assert_eq!(across["tableCellMatches"].as_array().unwrap().len(), 2);
    let separators =
        markdown::locate_ai_source(&docx, &citation(first + quote.len(), start)).unwrap();
    assert_eq!(separators["tableCellMatches"], serde_json::json!([]));
}

#[test]
fn ai_cell_edit_checks_the_whole_cell_and_preserves_neighbors() {
    let docx = markdown::import_markdown_to_docx_bytes(
        "| 対象 | 比較 |\n| --- | --- |\n| **東京** | **東京** |\n".as_bytes(),
    )
    .unwrap();
    let export = markdown::export_ai_source(&docx).unwrap();
    let cells = export["tableCells"].as_array().unwrap();
    let cell = &cells[2];
    let quote = excerpt(&export, cell);
    assert_eq!(quote, "**東京**");
    let mut proposal = serde_json::json!({
        "version": 1,
        "target": "tableCell",
        "sourceSha256": export["sourceSha256"],
        "markdownBytes": cell["markdownBytes"],
        "quote": quote,
        "before": "東京",
        "replacement": "大阪",
    });
    let preview = markdown::preview_ai_edit(&docx, &proposal).unwrap();
    assert_eq!(preview["source"]["kind"], "tableCell");
    assert_eq!(preview["source"]["row"], 2);
    assert_eq!(preview["source"]["column"], 1);
    assert_eq!(preview["before"], "東京");
    assert_eq!(preview["after"], "大阪");

    let edited = markdown::apply_ai_edit(&docx, &proposal).unwrap();
    let exported = markdown::export_docx_to_markdown(&edited).unwrap();
    assert!(exported.contains("| **大阪** | **東京** |"));
    assert!(markdown::apply_ai_edit(&edited, &proposal).is_err());

    proposal["before"] = serde_json::json!("名古屋");
    assert!(markdown::preview_ai_edit(&docx, &proposal).is_err());
    proposal["before"] = serde_json::json!("東京");
    let start = cell["markdownBytes"][0].as_u64().unwrap() + 2;
    proposal["markdownBytes"] = serde_json::json!([start, start + "東京".len() as u64]);
    proposal["quote"] = serde_json::json!("東京");
    assert!(markdown::preview_ai_edit(&docx, &proposal).is_err());
}

#[test]
fn ai_partial_cell_edit_targets_one_formatted_occurrence() {
    let docx = markdown::import_markdown_to_docx_bytes(
        "| 内容 | 隣のセル |\n| --- | --- |\n| 東京と**東京** | 東京 |\n".as_bytes(),
    )
    .unwrap();
    let export = markdown::export_ai_source(&docx).unwrap();
    let text = export["markdown"].as_str().unwrap();
    let before = "東京と東京";
    let quote = "東京";
    let start = text.find("**東京**").unwrap() + 2;
    let local = before.rfind(quote).unwrap();
    let mut proposal = serde_json::json!({
        "version": 1,
        "target": "tableCell",
        "sourceSha256": export["sourceSha256"],
        "markdownBytes": [start, start + quote.len()],
        "paragraphBytes": [local, local + quote.len()],
        "quote": quote,
        "before": before,
        "replacement": "大阪",
    });
    let preview = markdown::preview_ai_edit(&docx, &proposal).unwrap();
    assert_eq!(preview["after"], "東京と大阪");
    let edited = markdown::apply_ai_edit(&docx, &proposal).unwrap();
    assert!(
        markdown::export_docx_to_markdown(&edited)
            .unwrap()
            .contains("| 東京と**大阪** | 東京 |")
    );

    proposal["paragraphBytes"] = serde_json::json!([0, quote.len()]);
    assert!(markdown::preview_ai_edit(&docx, &proposal).is_err());
}

#[test]
fn ai_batch_edits_validate_before_writing_and_keep_edit_order() {
    let docx = markdown::import_markdown_to_docx_bytes(
        b"Original sentence.\n\n| Heading |\n| --- |\n| Original cell |\n",
    )
    .unwrap();
    let export = markdown::export_ai_source(&docx).unwrap();
    let text = export["markdown"].as_str().unwrap();
    let start = text.find("Original sentence.").unwrap();
    let mut batch = serde_json::json!({
        "version": 1,
        "sourceSha256": export["sourceSha256"],
        "edits": [
            {
                "markdownBytes": [start, start + "Original sentence.".len()],
                "quote": "Original sentence.",
                "before": "Original sentence.",
                "replacement": "Reviewed sentence.",
            },
            {
                "target": "tableCell",
                "markdownBytes": export["tableCells"][1]["markdownBytes"],
                "quote": "Original cell",
                "before": "Original cell",
                "replacement": "Reviewed cell",
            },
        ],
    });
    let preview = markdown::preview_ai_edit(&docx, &batch).unwrap();
    assert_eq!(preview["edits"][0]["after"], "Reviewed sentence.");
    assert_eq!(preview["edits"][1]["after"], "Reviewed cell");
    let edited = markdown::apply_ai_edit(&docx, &batch).unwrap();
    let output = markdown::export_docx_to_markdown(&edited).unwrap();
    assert!(output.contains("Reviewed sentence."));
    assert!(output.contains("| Reviewed cell |"));

    batch["edits"][1]["before"] = serde_json::json!("Wrong");
    assert!(
        markdown::apply_ai_edit(&docx, &batch)
            .unwrap_err()
            .to_string()
            .contains("Edit 2")
    );
    batch["edits"][1] = batch["edits"][0].clone();
    assert!(
        markdown::preview_ai_edit(&docx, &batch)
            .unwrap_err()
            .to_string()
            .contains("repeats")
    );
}

#[test]
fn ai_batch_combines_edits_to_distinct_runs_in_one_cell() {
    let docx = markdown::import_markdown_to_docx_bytes(
        "| 支店 |\n| --- |\n| **東京**と*大阪* |\n".as_bytes(),
    )
    .unwrap();
    let export = markdown::export_ai_source(&docx).unwrap();
    let text = export["markdown"].as_str().unwrap();
    let before = "東京と大阪";
    let edit = |quote: &str, replacement: &str| {
        let start = text.find(quote).unwrap();
        let local = before.find(quote).unwrap();
        serde_json::json!({
            "target": "tableCell",
            "markdownBytes": [start, start + quote.len()],
            "paragraphBytes": [local, local + quote.len()],
            "quote": quote,
            "before": before,
            "replacement": replacement,
        })
    };
    let mut batch = serde_json::json!({
        "version": 1,
        "sourceSha256": export["sourceSha256"],
        "edits": [edit("大阪", "札幌"), edit("東京", "横浜市")],
    });
    let preview = markdown::preview_ai_edit(&docx, &batch).unwrap();
    assert_eq!(preview["edits"][0]["after"], "東京と札幌");
    assert_eq!(preview["edits"][1]["after"], "横浜市と大阪");
    let edited = markdown::apply_ai_edit(&docx, &batch).unwrap();
    assert!(
        markdown::export_docx_to_markdown(&edited)
            .unwrap()
            .contains("| **横浜市**と*札幌* |")
    );

    // Distinct slices of the same run still conflict.
    batch["edits"] = serde_json::json!([edit("東", "西"), edit("京", "都")]);
    assert!(
        markdown::apply_ai_edit(&docx, &batch)
            .unwrap_err()
            .to_string()
            .contains("repeats a text run")
    );
}
