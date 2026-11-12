// SPDX-License-Identifier: AGPL-3.0-only
use ibis_doc_model::{Block, FlowDocument, FlowParagraph, FlowRun, FlowRunKind, Link};
use ibis_pptx::docx_convert::export_html;

#[test]
fn html_escapes_text_and_omits_unsafe_links() {
    let run = |text: &str, url: &str| FlowRun {
        kind: FlowRunKind::Text(text.into()),
        link: Some(Link::Url(url.into())),
        ..FlowRun::default()
    };
    let doc = FlowDocument {
        body: vec![Block::Paragraph(FlowParagraph {
            runs: vec![
                run("safe<&", "https://example.test/?a=1&b=2"),
                run("fragment", "#section"),
                run("relative", "folder/page.html"),
                run("script", "jAvAsCrIpT:alert(1)"),
                run("control", "java\nscript:alert(1)"),
                run("local", "file:///etc/passwd"),
                run("data", "data:text/html,hello"),
            ],
            ..FlowParagraph::default()
        })],
        ..FlowDocument::default()
    };
    let html = export_html(&doc);
    assert!(
        html.contains("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">")
    );
    assert!(html.contains("<a href=\"https://example.test/?a=1&amp;b=2\">safe&lt;&amp;</a>"));
    assert!(html.contains("<a href=\"#section\">fragment</a>"));
    assert!(html.contains("<a href=\"folder/page.html\">relative</a>"));
    assert!(html.contains("scriptcontrollocaldata"));
    assert!(!html.to_ascii_lowercase().contains("javascript:"));
    assert!(!html.contains("file:///"));
    assert!(!html.contains("data:text/html"));
}

#[test]
fn workbook_html_contains_visible_sheets_values_and_merges_only() {
    use ibis_pptx::html_export::export_workbook_html;
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test/web/fixtures/html-export.xlsx"
    ))
    .unwrap();
    let p = ibis_pptx::open(bytes).unwrap().presentation;
    let html = String::from_utf8(export_workbook_html(&p, "Book & report").unwrap()).unwrap();
    assert!(html.contains("Book &amp; report"));
    assert!(html.contains("Sales &amp; 実績"));
    assert!(html.contains("売上 &lt;2026&gt;"));
    assert!(html.contains("123.5"));
    assert!(html.contains("colspan=\"2\""));
    assert!(html.contains("Second sheet"));
    assert!(!html.contains("hidden sheet secret"));
    assert!(!html.contains("hidden row secret"));
    assert!(!html.contains("hidden column secret"));
}

#[test]
fn presentation_html_contains_visible_slide_text_image_and_table() {
    use ibis_pptx::html_export::export_slides_html;
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test/web/fixtures/html-export.pptx"
    ))
    .unwrap();
    let p = ibis_pptx::open(bytes).unwrap().presentation;
    let html = String::from_utf8(export_slides_html(&p, "Slides").unwrap()).unwrap();
    assert!(html.contains("Slide &lt;one&gt;") && html.contains("日本語"));
    assert!(html.contains("data:image/png;base64,"));
    assert!(html.contains("Third slide"));
    assert!(html.contains("Cell A") && html.contains("Cell B"));
    assert!(html.contains("background-color:#4F81BD"));
    assert!(!html.contains("hidden slide secret"));
    assert_eq!(html.matches("class=\"slide\"").count(), 2);
}

#[test]
fn workbook_html_rejects_unbounded_sparse_grid() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test/web/fixtures/html-export.xlsx"
    ))
    .unwrap();
    let mut p = ibis_pptx::open(bytes).unwrap().presentation;
    let sheet = p
        .slides
        .iter_mut()
        .flat_map(|s| s.shapes.iter_mut())
        .find_map(|shape| match &mut shape.kind {
            ibis_doc_model::ShapeKind::Sheet(sheet) => Some(std::sync::Arc::make_mut(sheet)),
            _ => None,
        })
        .unwrap();
    let mut far = sheet.cells[0].clone();
    far.row = 1_000_000;
    far.column = 20;
    sheet.cells.push(far);
    sheet.cells.sort_by_key(|c| (c.row, c.column));
    let error = ibis_pptx::html_export::export_workbook_html(&p, "Sparse").unwrap_err();
    assert!(error.to_string().contains("too large or sparse"));
}
