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
    assert!(html.contains("<a href=\"https://example.test/?a=1&amp;b=2\">safe&lt;&amp;</a>"));
    assert!(html.contains("<a href=\"#section\">fragment</a>"));
    assert!(html.contains("<a href=\"folder/page.html\">relative</a>"));
    assert!(html.contains("scriptcontrollocaldata"));
    assert!(!html.to_ascii_lowercase().contains("javascript:"));
    assert!(!html.contains("file:///"));
    assert!(!html.contains("data:text/html"));
}
