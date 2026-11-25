// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Shared XML fragments used by Lawtext and editable law Markdown.

use super::model::{LawParagraph, LawParagraphLabels, LawSentenceContent};
use super::xml::{
    is_sentence_inline, parse_paragraph_labels, parse_xml_sentence_container,
    write_paragraph_caption, write_paragraph_number, write_xml_sentence_content,
    write_xml_sentence_elements,
};

pub(super) fn lawtext_paragraph_text(para: &LawParagraph, structured: bool) -> std::borrow::Cow<'_, str> {
    if structured && para.labels != LawParagraphLabels::default() {
        let mut xml = String::new();
        write_paragraph_caption(&mut xml, &para.labels);
        if let Some(label) = &para.labels.number {
            write_paragraph_number(&mut xml, label);
        }
        write_xml_sentence_content(&mut xml, &para.sentence, &para.xml_sentence_content);
        return xml.replace('\r', "&#13;").replace('\n', "&#10;").into();
    }
    lawtext_sentence_text(&para.sentence, &para.xml_sentence_content, structured)
}

pub(super) fn parse_lawtext_paragraph_labels(text: &str) -> Option<(LawParagraphLabels, &str)> {
    if !text.starts_with("<ParagraphCaption") && !text.starts_with("<ParagraphNum") {
        return None;
    }
    const OPEN: &str = "<Paragraph>";
    let xml = format!("{OPEN}{text}</Paragraph>");
    let doc = roxmltree::Document::parse(&xml).ok()?;
    let root = doc.root_element();
    let mut elements = root.children().filter(roxmltree::Node::is_element).peekable();
    if elements.peek()?.has_tag_name("ParagraphCaption") {
        elements.next();
    }
    if elements.peek()?.has_tag_name("ParagraphNum") {
        elements.next();
    }
    let start = elements.peek()?.range().start - OPEN.len();
    let rest = &text[start..];
    if !rest.starts_with("<Sentence") {
        return None;
    }
    Some((parse_paragraph_labels(root), rest))
}

pub(super) fn lawtext_sentence_text<'a>(text: &'a str, content: &LawSentenceContent, structured: bool) -> std::borrow::Cow<'a, str> {
    if structured && content.has_structure() && content.plain_text().as_deref() == Some(text) {
        let mut xml = String::new();
        write_xml_sentence_elements(&mut xml, content);
        return xml.replace('\r', "&#13;").replace('\n', "&#10;").into();
    }
    text.into()
}

pub(super) fn parse_lawtext_paragraph(num: String, text: &str) -> LawParagraph {
    let (labels, text) = parse_lawtext_paragraph_labels(text).unwrap_or_else(|| (LawParagraphLabels::default(), text));
    let (sentence, xml_sentence_content) = parse_lawtext_sentence_text(text, false);
    LawParagraph { num, labels, sentence, xml_sentence_content, items: Vec::new() }
}

pub(super) fn parse_lawtext_sentence_text(text: &str, allow_columns: bool) -> (String, LawSentenceContent) {
    parse_lawtext_sentence_elements(text, allow_columns)
        .unwrap_or_else(|| (text.to_string(), LawSentenceContent::default()))
}

pub(super) fn parse_lawtext_sentence_elements(text: &str, allow_columns: bool) -> Option<(String, LawSentenceContent)> {
    if !text.starts_with("<Sentence") && !(allow_columns && text.starts_with("<Column")) {
        return None;
    }
    let xml = format!("<ParagraphSentence>{text}</ParagraphSentence>");
    let doc = roxmltree::Document::parse(&xml).ok()?;
    let root = doc.root_element();
    let is_spacing = |node: roxmltree::Node<'_, '_>| node.text().is_some_and(|text| node.is_text() && text.trim().is_empty());
    let is_sentence = |node: roxmltree::Node<'_, '_>| {
        node.has_tag_name("Sentence")
            && node.children().all(is_sentence_inline)
            && node.attributes().all(|attr| attr.namespace().is_none()
                && matches!(attr.name(), "Num" | "Function" | "Indent" | "WritingMode"))
    };
    let mut children = root.children().filter(|&node| !is_spacing(node)).peekable();
    let columns = children.peek()?.has_tag_name("Column");
    if columns {
        if !allow_columns || !children.all(|node| node.has_tag_name("Column")
            && node.attributes().all(|attr| attr.namespace().is_none()
                && matches!(attr.name(), "Num" | "LineBreak" | "Align"))
            && node.children().any(is_sentence)
            && node.children().all(|child| is_spacing(child) || is_sentence(child)))
        {
            return None;
        }
    } else if !children.all(is_sentence) {
        return None;
    }
    Some(parse_xml_sentence_container(Some(root), columns))
}
