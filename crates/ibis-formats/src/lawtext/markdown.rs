// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Editable law Markdown and article locators.

use crate::Error;
use super::table_markup::{markdown_tables, restore_tables_with_anchors};
use super::{is_law_xml, parse_law_xml};
use super::model::{
    LawArticle, LawDoc, LawNode, LawParagraphLabels, LawSentenceContent, LawSubitem,
    LawTitleAttributes,
};
use super::syntax::{
    parse_article_header, parse_md_ordered_paragraph, split_article_title_and_caption,
    split_leading_token, to_fullwidth_digits,
};
use super::xml::{
    parse_title_attributes_fragment, encode_suppl_positions, read_encoded_positions, read_encoded_appdx_fragments, read_encoded_toc, write_paragraph_caption,
    write_paragraph_number, write_title_attributes_fragment, write_xml_sentence_content,
};
use super::markup::{
    lawtext_paragraph_text, lawtext_sentence_text, parse_lawtext_paragraph_labels,
    parse_lawtext_sentence_elements,
};
use super::text::{LAWTEXT_SUPPL_APPENDICES, encode_suppl_appendices, is_lawtext, parse_lawtext};

const TABLE_SOURCE_ANCHOR: &str = " {#ibis_law_table_";

/// Export a [`LawDoc`] to Markdown with YAML frontmatter so it can be edited
/// in ibisOffice (DOCX / PPTX) and converted back to Lawtext or e-Gov 法令標準XML.
pub fn to_markdown(doc: &LawDoc) -> String {
    to_markdown_with_articles(doc, true, |_, _, _, _| {})
}

fn to_markdown_with_articles(
    doc: &LawDoc,
    preserve_editing_metadata: bool,
    mut record: impl FnMut(Option<usize>, &LawArticle, usize, usize),
) -> String {
    let mut out = String::from("---\ntype: lawtext\n");
    if !doc.law_title.is_empty() {
        out.push_str(&format!("law_title: {}\n", doc.law_title));
    }
    if preserve_editing_metadata && doc.law_title_attributes != LawTitleAttributes::default() {
        out.push_str("law_title_attributes: ");
        write_title_attributes_fragment(&mut out, &doc.law_title_attributes);
        out.push('\n');
    }
    if preserve_editing_metadata && let Some(statement) = &doc.enact_statement {
        let encoded = serde_json::Value::String(statement.clone());
        out.push_str(&format!("law_enact_statement: {encoded}\n"));
    }
    if preserve_editing_metadata && let Some(toc) = &doc.toc_xml {
        let encoded = serde_json::Value::String(toc.clone());
        out.push_str(&format!("law_toc: {encoded}\n"));
    }
    let (table_source_ids, table_fragments) = if preserve_editing_metadata {
        markdown_tables(doc)
    } else {
        (Vec::new(), Vec::new())
    };
    if table_source_ids.iter().any(Option::is_some)
        && let Ok(encoded) = serde_json::to_string(&table_fragments)
    {
        out.push_str(&format!("law_appdx_source_ids: true\nlaw_appdx_tables: {encoded}\n"));
    }
    if preserve_editing_metadata && !doc.appdx_fragment_xml.is_empty()
        && let Ok(encoded) = serde_json::to_string(&doc.appdx_fragment_xml)
    {
        out.push_str(&format!("law_appdx_fragments: {encoded}\n"));
    }
    if preserve_editing_metadata && !doc.appdx_fragment_positions.is_empty()
        && let Ok(encoded) = serde_json::to_string(&doc.appdx_fragment_positions)
    {
        out.push_str(&format!("law_appdx_fragment_positions: {encoded}\n"));
    }
    if preserve_editing_metadata && let Some(encoded) = encode_suppl_positions(doc)
    {
        out.push_str(&format!("law_suppl_positions: {encoded}\n"));
    }
    if !doc.law_num.is_empty() {
        out.push_str(&format!("law_num: {}\n", doc.law_num));
    }
    if let Some(era) = doc.era.as_deref() {
        out.push_str(&format!("era: {era}\n"));
    }
    if let Some(year) = doc.year.as_deref() {
        out.push_str(&format!("year: {year}\n"));
    }
    if let Some(month) = doc.promulgate_month.as_deref() {
        out.push_str(&format!("promulgate_month: {month}\n"));
    }
    if let Some(day) = doc.promulgate_day.as_deref() {
        out.push_str(&format!("promulgate_day: {day}\n"));
    }
    if let Some(num) = doc.num.as_deref() {
        out.push_str(&format!("num: {num}\n"));
    }
    if let Some(lt) = doc.law_type.as_deref() {
        out.push_str(&format!("law_type: {lt}\n"));
    }
    if let Some(lang) = doc.lang.as_deref().filter(|lang| *lang != "ja") {
        out.push_str(&format!("lang: {lang}\n"));
    }
    out.push_str("---\n\n");

    if !doc.law_title.is_empty() {
        out.push_str(&format!("# {}\n\n", doc.law_title));
    }
    if !doc.law_num.is_empty() {
        out.push_str(&format!("> 法令番号: {}\n\n", doc.law_num));
    }
    if let Some(preamble) = doc.preamble.as_deref().filter(|text| !text.is_empty()) {
        out.push_str(preamble);
        out.push_str("\n\n");
    }
    for node in &doc.main_provision {
        write_markdown_node(&mut out, node, 2, preserve_editing_metadata, &mut |article, start, end| record(None, article, start, end));
    }
    for (index, suppl) in doc.suppl_provisions.iter().enumerate() {
        out.push_str("## ");
        out.push_str(&suppl.label);
        if let Some(amend) = suppl.amend_law_num.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!("（{amend}）"));
        }
        if suppl.extract {
            out.push_str("（抄）");
        }
        out.push_str("\n\n");
        if preserve_editing_metadata && let Some(encoded) = encode_suppl_appendices(doc.suppl_appendices(index)) {
            // Escape hyphens so fragment text cannot end the Markdown comment.
            let encoded = encoded.replace('-', "\\u002d");
            out.push_str(&format!("<!-- {LAWTEXT_SUPPL_APPENDICES}{encoded} -->\n\n"));
        }
        for node in &suppl.nodes {
            write_markdown_node(&mut out, node, 3, preserve_editing_metadata, &mut |article, start, end| record(Some(index), article, start, end));
        }
    }
    for (index, appdx) in doc.appdx_tables.iter().enumerate() {
        if let Some(rel) = appdx.related_article_num.as_deref() {
            out.push_str(&format!("## {}（{}）", appdx.title, rel));
        } else {
            out.push_str(&format!("## {}", appdx.title));
        }
        if let Some(Some(source_id)) = table_source_ids.get(index) {
            out.push_str(&format!("{TABLE_SOURCE_ANCHOR}{source_id}}}"));
        }
        out.push_str("\n\n");
        if !appdx.rows.is_empty() {
            let cols = appdx.rows.iter().map(|r| r.len()).max().unwrap_or(1);
            for (r_idx, row) in appdx.rows.iter().enumerate() {
                out.push('|');
                for c in 0..cols {
                    let cell = row.get(c).map(|s| s.replace('|', "\\|")).unwrap_or_default();
                    out.push_str(&format!(" {cell} |"));
                }
                out.push('\n');
                if r_idx == 0 {
                    out.push('|');
                    for _ in 0..cols {
                        out.push_str(" :--- |");
                    }
                    out.push('\n');
                }
            }
            out.push('\n');
        }
    }
    out.trim_end().to_string() + "\n"
}

/// Export recognized Lawtext or law XML with semantic article locators.
pub fn export_ai_law(data: &[u8]) -> Result<Option<serde_json::Value>, Error> {
    use serde_json::json;
    let parser = if is_law_xml(data) { parse_law_xml } else if is_lawtext(data) { parse_lawtext } else { return Ok(None) };
    let input = std::str::from_utf8(data).map_err(|_| Error::new("Law source is not UTF-8"))?;
    let doc = parser(input)?;
    let mut articles = Vec::new();
    let markdown = to_markdown_with_articles(&doc, false, |supplement, article, start, end| {
        articles.push((json!({
            "lawNum": doc.law_num,
            "supplementIndex": supplement,
            "amendLawNum": supplement.and_then(|index| doc.suppl_provisions[index].amend_law_num.as_deref()),
            "articleNum": article.num,
            "title": article.title,
        }), start, end));
    });
    let articles: Vec<_> = articles.into_iter().map(|(source, start, end)| {
        json!({ "source": source, "markdownBytes": [start, end.min(markdown.len())] })
    }).collect();
    Ok(Some(json!({
        "version": 1,
        "kind": "law",
        "markdown": markdown,
        "articles": articles,
        "diagnostics": [{ "code": "law_export_uses_supported_structure_only" }],
    })))
}

fn write_markdown_node(
    out: &mut String, node: &LawNode, heading_level: usize,
    preserve_sentences: bool,
    record: &mut impl FnMut(&LawArticle, usize, usize),
) {
    match node {
        LawNode::Division {
            kind,
            title,
            children,
            ..
        } => {
            let lvl = kind.heading_level().max(heading_level).min(6);
            out.push_str(&"#".repeat(lvl));
            out.push(' ');
            out.push_str(title);
            out.push_str("\n\n");
            for child in children {
                write_markdown_node(out, child, (lvl + 1).min(6), preserve_sentences, record);
            }
        }
        LawNode::Article(art) => {
            let start = out.len();
            let lvl = heading_level.min(6);
            out.push_str(&"#".repeat(lvl));
            out.push(' ');
            out.push_str(&art.title);
            if let Some(cap) = art.caption.as_deref().filter(|s| !s.is_empty()) {
                out.push(' ');
                out.push_str(cap);
            }
            out.push_str("\n\n");
            for (idx, para) in art.paragraphs.iter().enumerate() {
                if idx != 0 || para.num != "1" || para.sentence.is_empty() {
                    out.push_str(&format!("{}. ", para.num));
                }
                write_markdown_sentence_comment(out, &para.sentence, lawtext_paragraph_text(para, preserve_sentences));
                out.push_str("\n\n");
                for item in &para.items {
                    out.push_str(&format!("- {} ", item.title));
                    write_markdown_sentence_text(out, &item.sentence, &item.xml_sentence_content, preserve_sentences);
                    out.push('\n');
                    for sub in &item.subitems {
                        write_markdown_subitem(out, sub, preserve_sentences);
                    }
                }
                if !para.items.is_empty() {
                    out.push('\n');
                }
            }
            record(art, start, out.len());
        }
        LawNode::Paragraph(para) => {
            if para.num != "1" || para.sentence.is_empty() {
                out.push_str(&format!("{}. ", para.num));
            }
            write_markdown_sentence_comment(out, &para.sentence, lawtext_paragraph_text(para, preserve_sentences));
            out.push_str("\n\n");
        }
    }
}

pub(crate) const MARKDOWN_SENTENCE_COMMENT: &str = "<!-- ibis-law-sentences ";

fn write_markdown_sentence_text(out: &mut String, text: &str, content: &LawSentenceContent, preserve_sentences: bool) {
    write_markdown_sentence_comment(out, text, lawtext_sentence_text(text, content, preserve_sentences));
}

fn write_markdown_sentence_comment(out: &mut String, text: &str, encoded: std::borrow::Cow<'_, str>) {
    out.push_str(text);
    if !text.contains(['\r', '\n']) && let std::borrow::Cow::Owned(xml) = encoded {
        if !text.is_empty() {
            out.push(' ');
        }
        out.push_str(MARKDOWN_SENTENCE_COMMENT);
        // Keep the payload from ending the HTML comment.
        out.push_str(&xml.replace("--", "&#45;&#45;"));
        out.push_str(" -->");
    }
}

fn restore_markdown_sentence_text(text: &str, allow_columns: bool) -> std::borrow::Cow<'_, str> {
    let Some((plain, xml)) = text.rsplit_once(MARKDOWN_SENTENCE_COMMENT) else {
        return text.into();
    };
    let Some(xml) = xml.strip_suffix(" -->") else {
        return text.into();
    };
    let plain = plain.trim_end();
    let (labels, sentence_xml) = if allow_columns {
        (LawParagraphLabels::default(), xml)
    } else {
        parse_lawtext_paragraph_labels(xml).unwrap_or_else(|| (LawParagraphLabels::default(), xml))
    };
    if parse_lawtext_sentence_elements(sentence_xml, allow_columns).is_some_and(|(text, _)| text == plain) {
        xml.into()
    } else if labels != LawParagraphLabels::default() {
        let mut updated = String::new();
        write_paragraph_caption(&mut updated, &labels);
        if let Some(number) = labels.number {
            write_paragraph_number(&mut updated, &number);
        }
        write_xml_sentence_content(&mut updated, plain, &LawSentenceContent::default());
        updated.into()
    } else {
        plain.into()
    }
}

fn write_markdown_subitem(out: &mut String, sub: &LawSubitem, preserve_sentences: bool) {
    for _ in 0..sub.depth {
        out.push_str("  ");
    }
    out.push_str(&format!("- {} ", sub.title));
    write_markdown_sentence_text(out, &sub.sentence, &sub.xml_sentence_content, preserve_sentences);
    out.push('\n');
    for child in &sub.children {
        write_markdown_subitem(out, child, preserve_sentences);
    }
}

/// Parse a Lawtext-flavoured Markdown document back into a [`LawDoc`].
pub fn parse_markdown_law(md: &str) -> Result<LawDoc, Error> {
    parse_markdown_law_with_diagnostics(md).map(|(doc, _)| doc)
}

/// Parse editable law Markdown and report source fragments that could not be reused.
pub fn parse_markdown_law_with_diagnostics(
    md: &str,
) -> Result<(LawDoc, Vec<crate::markdown::SourceDiagnostic>), Error> {
    let mut doc = LawDoc::default();
    let mut table_xml = None;
    let mut source_ids_required = false;
    let mut fragment_positions = None;
    let mut suppl_positions = None;
    let mut body = md.trim_start_matches('\u{feff}');
    if let Some((fm, rest)) = crate::markdown::roundtrip::frontmatter(body) {
        body = rest;
        for line in fm.lines() {
            if let Some((k, v)) = line.split_once(':') {
                let v = v.trim().to_string();
                match k.trim() {
                    "law_title" => doc.law_title = v,
                    "law_title_attributes" => {
                        if let Some(attributes) = parse_title_attributes_fragment(&v) {
                            doc.law_title_attributes = attributes;
                        }
                    }
                    "law_enact_statement" => doc.enact_statement = serde_json::from_str(&v).ok(),
                    "law_toc" => doc.toc_xml = read_encoded_toc(&v),
                    "law_appdx_fragment_positions" | "law_appdx_style_positions" => fragment_positions = Some(v),
                    "law_appdx_fragments" | "law_appdx_styles" => doc.appdx_fragment_xml = read_encoded_appdx_fragments(&v),
                    "law_suppl_positions" => suppl_positions = Some(v),
                    "law_appdx_source_ids" => source_ids_required = v == "true",
                    "law_appdx_tables" => table_xml = Some(v),
                    "law_num" => doc.law_num = v,
                    "era" => doc.era = Some(v),
                    "year" => doc.year = Some(v),
                    "promulgate_month" => doc.promulgate_month = Some(v),
                    "promulgate_day" => doc.promulgate_day = Some(v),
                    "num" => doc.num = Some(v),
                    "law_type" => doc.law_type = Some(v),
                    "lang" => doc.lang = Some(v),
                    _ => {}
                }
            }
        }
    }

    // An editable DOCX keeps the visible heading and number when exporting
    // back to Markdown, even if the law-specific front matter is absent.
    if let Some(number) = body.lines().find_map(|line| line.trim().strip_prefix("> 法令番号:")) {
        if doc.law_num.is_empty() {
            doc.law_num = number.trim().to_owned();
        }
        if doc.law_title.is_empty() {
            if let Some(title) = body.lines().find_map(|line| line.trim().strip_prefix("# ")) {
                doc.law_title = title.trim().to_owned();
            }
        }
    }

    // Reconstruct Lawtext lines from Markdown headings/lists and delegate to `parse_lawtext`
    // so both formats share one structural parser.
    let mut lawtext_lines = Vec::new();
    if !doc.law_title.is_empty() {
        if !doc.law_num.is_empty() {
            lawtext_lines.push(format!("{}（{}）", doc.law_title, doc.law_num));
        } else {
            lawtext_lines.push(doc.law_title.clone());
        }
    }
    let mut seen_first_article = false;
    let mut table_anchors = Vec::new();
    for raw in body.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with("> 法令番号:") {
            continue;
        }
        if let Some(metadata) = trimmed.strip_prefix("<!-- ").and_then(|line| line.strip_suffix(" -->"))
            && metadata.starts_with(LAWTEXT_SUPPL_APPENDICES)
        {
            lawtext_lines.push(metadata.to_owned());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            let heading = rest.trim_start_matches('#').trim();
            if heading == doc.law_title && !seen_first_article {
                continue;
            }
            if let Some((art_title, caption)) = split_article_title_and_caption(heading) {
                seen_first_article = true;
                if let Some(cap) = caption {
                    lawtext_lines.push(cap);
                }
                lawtext_lines.push(art_title);
                continue;
            }
            let (heading, source_id) = heading.strip_suffix('}')
                .and_then(|heading| heading.rsplit_once(TABLE_SOURCE_ANCHOR))
                .map(|(heading, id)| (heading, Some(id.parse::<usize>().map_err(|_| crate::markdown::SourceIssue::Invalid))))
                .unwrap_or((heading, None));
            lawtext_lines.push(heading.to_string());
            if heading.starts_with("別表") && !heading.contains('。') {
                table_anchors.push(source_id);
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let indent = raw.chars().take_while(|c| *c == ' ').count();
            let mut line = " ".repeat((indent / 2).min(10) * 2 + 2);
            if let Some((title, text)) = split_leading_token(rest) {
                line.push_str(title);
                line.push(' ');
                line.push_str(&restore_markdown_sentence_text(text, true));
            } else {
                line.push_str(rest);
            }
            lawtext_lines.push(line);
            continue;
        }
        if let Some((p_num, rest)) = parse_md_ordered_paragraph(trimmed) {
            lawtext_lines.push(format!("{} {}", to_fullwidth_digits(&p_num), restore_markdown_sentence_text(rest, false)));
            continue;
        }
        let trimmed = restore_markdown_sentence_text(trimmed, false);
        // Attach first body paragraph onto the pending `第N条` line if it has no sentence yet.
        if let Some(last) = lawtext_lines.last_mut() {
            if parse_article_header(last.trim()).is_some_and(|(_, _, r)| r.is_empty()) {
                last.push(' ');
                last.push_str(&trimmed);
                continue;
            }
        }
        if trimmed.starts_with("別表") && !trimmed.contains('。') {
            table_anchors.push(None);
        }
        lawtext_lines.push(trimmed.into_owned());
    }

    let mut parsed = parse_lawtext(&lawtext_lines.join("\n"))?;
    doc.take_suppl_provisions_from(&mut parsed);
    if doc.law_title.is_empty() {
        doc.law_title = parsed.law_title;
    }
    if doc.law_num.is_empty() {
        doc.law_num = parsed.law_num;
    }
    doc.main_provision = std::mem::take(&mut parsed.main_provision);
    if let Some(encoded) = suppl_positions {
        doc.suppl_positions = read_encoded_positions(&encoded, doc.suppl_provisions.len());
    }
    if let Some(encoded) = fragment_positions {
        doc.appdx_fragment_positions = read_encoded_positions(&encoded, doc.appdx_fragment_xml.len());
    }
    doc.appdx_tables = std::mem::take(&mut parsed.appdx_tables);
    let diagnostics = restore_tables_with_anchors(
        &mut doc, table_xml.as_deref(), true, &table_anchors, source_ids_required,
    );
    Ok((doc, diagnostics))
}
