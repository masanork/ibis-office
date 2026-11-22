// SPDX-License-Identifier: AGPL-3.0-only
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The text of a document's shapes (text boxes, labelled shapes), as an appendix the import skips.
//!
//! The body of the Markdown keeps a shape out: written into the body, its text would come back as body paragraphs.
//! A reader of the Markdown still wants it (a caption set in a text box beside a figure, the labels of a diagram
//! drawn with shapes), so `--include-shape-text` lists each shape's paragraphs, in the order of the document, after
//! the body.

use std::fmt::Write;

use ibis_doc_model::{FlowDocument, FlowRunKind};

use super::is_fenced_line;

const START: &str = "<!-- ibis-shape-text -->";
const END: &str = "<!-- /ibis-shape-text -->";
const HEADING: &str = "## Text in shapes";
const SHAPE: &str = "### Shape ";

/// The paragraphs of each shape of `flow` with text, in document order (body, then headers and footers).
fn shape_paragraphs(flow: &FlowDocument) -> Vec<Vec<String>> {
    let mut shapes = Vec::new();
    for paragraph in ibis_doc_model::flow_command::all_paragraphs_everywhere(flow) {
        for run in &paragraph.runs {
            let FlowRunKind::Shape(shape) = &run.kind else { continue };
            let Some(body) = shape.text_body() else { continue };
            let lines: Vec<String> = body
                .paragraphs
                .iter()
                .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
                .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|text| !text.is_empty())
                .collect();
            if !lines.is_empty() {
                shapes.push(lines);
            }
        }
    }
    shapes
}

pub(super) fn append(markdown: &mut String, flow: &FlowDocument) {
    let shapes = shape_paragraphs(flow);
    if shapes.is_empty() {
        return;
    }
    let mut appendix = String::new();
    for (index, lines) in shapes.iter().enumerate() {
        let _ = write!(appendix, "{SHAPE}{}\n\n", index + 1);
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                appendix.push_str(">\n");
            }
            let _ = writeln!(appendix, "> {}", escape(line));
        }
        appendix.push('\n');
    }
    let _ = writeln!(markdown, "\n\n{START}\n\n{HEADING}\n\n{appendix}{END}");
}

// Quote the text as plain text so characters in a shape cannot add Markdown or HTML.
fn escape(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            let _ = write!(escaped, "&#{};", u32::from(ch));
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

// Drop only a complete generated appendix, and only at the end of the file.
pub(super) fn strip(markdown: &str) -> &str {
    let Some(before_end) = markdown.trim_end().strip_suffix(END) else {
        return markdown;
    };
    let Some((body, appendix)) = before_end.rsplit_once(START) else {
        return markdown;
    };
    if !body.trim_end_matches([' ', '\t']).ends_with('\n') {
        return markdown;
    }
    let mut lines = appendix.lines().filter(|line| !line.trim().is_empty());
    if lines.next() != Some(HEADING) {
        return markdown;
    }
    let mut index = 0;
    for line in lines {
        if let Some(number) = line.strip_prefix(SHAPE) {
            if number.parse::<usize>() != Ok(index + 1) {
                return markdown;
            }
            index += 1;
        } else if index == 0 || !(line == ">" || line.starts_with("> ")) {
            return markdown;
        }
    }
    // A matching marker in an unterminated code fence belongs to the source.
    let mut fence = None;
    for line in body.lines() {
        is_fenced_line(line.trim(), &mut fence);
    }
    if index > 0 && fence.is_none() {
        body.strip_suffix("\r\n\r\n")
            .or_else(|| body.strip_suffix("\n\n"))
            .unwrap_or(body)
    } else {
        markdown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{
        export_document_markdown_with, export_presentation_to_markdown, import_markdown_to_docx_bytes,
        DocumentMarkdownOptions,
    };

    /// A document whose body has `before`, a text box with `paragraphs`, and `after`.
    fn docx_with_text_box(paragraphs: &[&str]) -> Vec<u8> {
        docx_with_text_box_in(b"Before.\n\nAfter.\n", paragraphs)
    }

    fn docx_with_text_box_in(markdown: &[u8], paragraphs: &[&str]) -> Vec<u8> {
        let docx = import_markdown_to_docx_bytes(markdown).unwrap();
        let mut package = ibis_ooxml::Package::open(docx).unwrap();
        let part = "word/document.xml";
        let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
        let body: String = paragraphs
            .iter()
            .map(|p| format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", p.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")))
            .collect();
        let drawing = format!(
            r#"<w:r><w:drawing xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:anchor behindDoc="0" relativeHeight="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="3600000" cy="900000"/><wp:wrapNone/><wp:docPr id="99" name="Caption"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:cNvSpPr txBox="1"/><wps:spPr/><wps:txbx><w:txbxContent>{body}</w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
        );
        // Into the first paragraph, after its text.
        let at = xml.find("Before.").unwrap();
        let at = at + xml[at..].find("</w:r>").unwrap() + "</w:r>".len();
        let mut xml = xml;
        xml.insert_str(at, &drawing);
        package.set_bytes(part, xml.into_bytes());
        package.save(ibis_ooxml::SaveOptions::default()).unwrap()
    }

    #[test]
    fn the_text_of_a_text_box_follows_the_body_and_the_import_skips_it() {
        let docx = docx_with_text_box(&["デジタル庁資料「標準化」より抜粋", "", "<b>*2*</b>"]);
        let presentation = crate::read_docx(&docx).unwrap();
        let plain = export_presentation_to_markdown(&presentation).unwrap();
        assert!(!plain.contains("抜粋"));
        let output = export_document_markdown_with(
            &presentation,
            DocumentMarkdownOptions { shape_text: true, ..Default::default() },
        )
        .unwrap();
        assert!(output.contains(&format!(
            "{HEADING}\n\n{SHAPE}1\n\n> デジタル庁資料「標準化」より抜粋\n>\n> &#60;b&#62;&#42;2&#42;&#60;&#47;b&#62;\n"
        )));
        assert_eq!(strip(&output), plain);
        let rebuilt = crate::read_docx(&import_markdown_to_docx_bytes(output.as_bytes()).unwrap()).unwrap();
        assert_eq!(export_presentation_to_markdown(&rebuilt).unwrap(), export_presentation_to_markdown(
            &crate::read_docx(&import_markdown_to_docx_bytes(plain.as_bytes()).unwrap()).unwrap()
        ).unwrap());
    }

    #[test]
    fn the_shape_text_and_the_metafile_previews_both_come_off_on_import() {
        let emf = include_bytes!("../../../ibis-emf/tests/fixtures/shapes.emf");
        let body = format!("Before.\n\n![Drawing](data:image/x-emf;base64,{}){{width=20mm height=20mm}}\n", crate::markdown::encode_base64(emf));
        let docx = docx_with_text_box_in(body.as_bytes(), &["注記"]);
        let presentation = crate::read_docx(&docx).unwrap();
        let plain = export_presentation_to_markdown(&presentation).unwrap();
        let output = export_document_markdown_with(
            &presentation,
            DocumentMarkdownOptions { shape_text: true, metafile_previews: true },
        )
        .unwrap();
        assert!(output.contains("> 注記") && output.contains("## Metafile previews"));
        assert!(output.find(HEADING) < output.find("## Metafile previews"));
        let rebuilt = crate::read_docx(&import_markdown_to_docx_bytes(output.as_bytes()).unwrap()).unwrap();
        let expected = crate::read_docx(&import_markdown_to_docx_bytes(plain.as_bytes()).unwrap()).unwrap();
        assert_eq!(export_presentation_to_markdown(&rebuilt).unwrap(), export_presentation_to_markdown(&expected).unwrap());
    }

    #[test]
    fn a_document_without_shape_text_has_no_appendix() {
        let docx = import_markdown_to_docx_bytes(b"Only body.\n").unwrap();
        let presentation = crate::read_docx(&docx).unwrap();
        let output = export_document_markdown_with(
            &presentation,
            DocumentMarkdownOptions { shape_text: true, ..Default::default() },
        )
        .unwrap();
        assert!(!output.contains(START));
    }

    #[test]
    fn leaves_literal_or_incomplete_appendices_in_the_source() {
        let appendix = format!("{START}\n\n{HEADING}\n\n{SHAPE}1\n\n> text\n\n{END}");
        let examples = [
            format!("```md\n{appendix}"),
            format!("body\n\n{appendix}\n\nMore text."),
            format!("body\n\n{START}\n\n{HEADING}\n\nUnrelated text\n\n{END}"),
            format!("body\n\n{START}\n\n{HEADING}\n\n{SHAPE}2\n\n> text\n\n{END}"),
        ];
        for source in &examples {
            assert_eq!(strip(source), source);
        }
    }
}
