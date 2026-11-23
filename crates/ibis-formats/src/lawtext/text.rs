// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Plain-text Lawtext reader and writer.

use crate::Error;
use super::model::{
    LawAppdxTable, LawArticle, LawDoc, LawItem, LawNode, LawSubitem, LawTitleAttributes,
    SupplProvision,
};
use super::syntax::{
    is_caption_line, parse_article_header, parse_division_header, parse_numbered_paragraph,
    parse_suppl_header, split_leading_token, split_title_and_parens, supplementary_item_number,
    to_fullwidth_digits,
};
use super::xml::{
    parse_title_attributes_fragment, read_encoded_appdx_style_positions, read_encoded_appdx_styles, read_encoded_toc, write_title_attributes_fragment,
};
use super::markup::{
    lawtext_paragraph_text, lawtext_sentence_text, parse_lawtext_paragraph,
    parse_lawtext_sentence_text,
};

use super::table_markup::{encode_tables, restore_tables};

const LAWTEXT_TITLE_ATTRIBUTES: &str = ":law-title-attrs: ";
const LAWTEXT_STYLE_POSITIONS: &str = ":law-appdx-style-positions: ";
const LAWTEXT_STYLES: &str = ":law-appdx-styles: ";
const LAWTEXT_TABLES: &str = ":law-appdx-tables: ";
const LAWTEXT_ENACT_STATEMENT: &str = ":law-enact-statement: ";
const LAWTEXT_TOC: &str = ":law-toc: ";

/// The upstream Lawtext examples put the law number on its own line.
fn standalone_law_number(line: &str) -> Option<&str> {
    let number = line.strip_prefix('（')?.strip_suffix('）')?.trim();
    (number.ends_with('号') && number.rsplit_once('第').is_some_and(|(prefix, ordinal)| {
        !prefix.is_empty() && ordinal.chars().next().is_some_and(|ch| {
            ch.is_ascii_digit() || "０１２３４５６７８９〇零一二三四五六七八九十百千万".contains(ch)
        })
    })).then_some(number)
}

// Lawtext permits root attributes before the title, on separate or combined lines.
fn lawtext_attributes(mut line: &str) -> Option<Vec<(&str, String)>> {
    let mut attributes = Vec::new();
    while let Some(entry) = line.strip_prefix('[') {
        let (name, value) = entry.split_once('=')?;
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return None;
        }
        let mut values = serde_json::Deserializer::from_str(value).into_iter::<String>();
        let parsed = values.next()?.ok()?;
        line = value[values.byte_offset()..].strip_prefix(']')?.trim_start();
        attributes.push((name, parsed));
    }
    (line.is_empty() && !attributes.is_empty()).then_some(attributes)
}

/// Return `true` when `data` looks like a Lawtext document (`.law.txt` / `.lawtext`)
/// or Markdown with `type: lawtext` frontmatter.
pub fn is_lawtext(data: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(data) else {
        return false;
    };
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.is_empty() {
        return false;
    }
    let mut lines = trimmed.lines()
        .filter(|line| !line.trim().is_empty())
        .skip_while(|line| {
            let line = line.trim();
            lawtext_attributes(line).is_some() || line.strip_prefix(LAWTEXT_TITLE_ATTRIBUTES)
                .and_then(parse_title_attributes_fragment).is_some()
                || line.starts_with(LAWTEXT_TOC) || line.starts_with(LAWTEXT_TABLES)
                || line.starts_with(LAWTEXT_ENACT_STATEMENT) || line.starts_with(LAWTEXT_STYLES)
                || line.starts_with(LAWTEXT_STYLE_POSITIONS)
        });
    let Some(first) = lines.next() else {
        return false;
    };
    let has_header = (first.contains('（') && first.contains('）'))
        || lines.next().and_then(standalone_law_number).is_some();
    let has_article = trimmed.lines().any(|l| parse_article_header(l.trim()).is_some());
    has_header && has_article
}

/// Export a [`LawDoc`] to human-readable Lawtext plain text.
pub fn to_lawtext(doc: &LawDoc) -> String {
    let mut out = String::new();
    for (name, value) in [
        ("Era", doc.era.as_deref()),
        ("Year", doc.year.as_deref()),
        ("PromulgateMonth", doc.promulgate_month.as_deref()),
        ("PromulgateDay", doc.promulgate_day.as_deref()),
        ("Num", doc.num.as_deref()),
        ("LawType", doc.law_type.as_deref()),
        ("Lang", doc.lang.as_deref().filter(|lang| *lang != "ja")),
    ] {
        if let Some(value) = value {
            let quoted = serde_json::Value::String(value.to_owned());
            out.push_str(&format!("[{name}={quoted}]\n"));
        }
    }
    if doc.law_title_attributes != LawTitleAttributes::default() {
        out.push_str(LAWTEXT_TITLE_ATTRIBUTES);
        write_title_attributes_fragment(&mut out, &doc.law_title_attributes);
        out.push('\n');
    }
    if let Some(statement) = &doc.enact_statement {
        let encoded = serde_json::Value::String(statement.clone());
        out.push_str(&format!("{LAWTEXT_ENACT_STATEMENT}{encoded}\n"));
    }
    if let Some(toc) = &doc.toc_xml {
        let encoded = serde_json::Value::String(toc.clone());
        out.push_str(&format!("{LAWTEXT_TOC}{encoded}\n"));
    }
    if let Some(encoded) = encode_tables(doc) {
        out.push_str(&format!("{LAWTEXT_TABLES}{encoded}\n"));
    }
    if !doc.appdx_style_xml.is_empty() && let Ok(encoded) = serde_json::to_string(&doc.appdx_style_xml) {
        out.push_str(&format!("{LAWTEXT_STYLES}{encoded}\n"));
    }
    if !doc.appdx_style_positions.is_empty()
        && let Ok(encoded) = serde_json::to_string(&doc.appdx_style_positions)
    {
        out.push_str(&format!("{LAWTEXT_STYLE_POSITIONS}{encoded}\n"));
    }
    if !out.is_empty() {
        out.push('\n');
    }
    if !doc.law_title.is_empty() {
        out.push_str(&doc.law_title);
        if !doc.law_num.is_empty() {
            out.push('（');
            out.push_str(&doc.law_num);
            out.push('）');
        }
        out.push_str("\n\n");
    }
    for node in &doc.main_provision {
        write_lawtext_node(&mut out, node, true);
    }
    for suppl in &doc.suppl_provisions {
        out.push_str(&suppl.label);
        if let Some(amend) = suppl.amend_law_num.as_deref().filter(|s| !s.is_empty()) {
            out.push('（');
            out.push_str(amend);
            out.push('）');
        }
        if suppl.extract {
            out.push_str("（抄）");
        }
        out.push_str("\n\n");
        for node in &suppl.nodes {
            write_lawtext_node(&mut out, node, true);
        }
    }
    for appdx in &doc.appdx_tables {
        out.push_str(&appdx.title);
        if let Some(rel) = appdx.related_article_num.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!("（{rel}）"));
        }
        out.push_str("\n\n");
        for row in &appdx.rows {
            out.push_str("| ");
            let cells: Vec<_> = row.iter().map(|cell| cell.replace('|', "\\|")).collect();
            out.push_str(&cells.join(" | "));
            out.push_str(" |\n");
        }
        if !appdx.rows.is_empty() {
            out.push('\n');
        }
    }
    out.trim_end().to_string() + "\n"
}

pub(super) fn write_lawtext_node(out: &mut String, node: &LawNode, structured_sentences: bool) {
    match node {
        LawNode::Division { title, children, .. } => {
            if !title.is_empty() {
                out.push_str(title);
                out.push_str("\n\n");
            }
            for child in children {
                write_lawtext_node(out, child, structured_sentences);
            }
        }
        LawNode::Article(art) => {
            if let Some(cap) = art.caption.as_deref().filter(|s| !s.is_empty()) {
                out.push_str(cap);
                out.push('\n');
            }
            for (idx, para) in art.paragraphs.iter().enumerate() {
                let sentence = lawtext_paragraph_text(para, structured_sentences);
                let inline = idx == 0 && para.num == "1" && !sentence.is_empty();
                if idx == 0 {
                    out.push_str(&art.title);
                    if !inline {
                        out.push('\n');
                    }
                }
                if !inline {
                    out.push_str(&to_fullwidth_digits(&para.num));
                }
                if !sentence.is_empty() {
                    out.push(' ');
                    out.push_str(&sentence);
                }
                out.push('\n');
                write_lawtext_items(out, &para.items, structured_sentences);
            }
            out.push('\n');
        }
        LawNode::Paragraph(para) => {
            if para.num != "1" {
                out.push_str(&to_fullwidth_digits(&para.num));
                out.push(' ');
            }
            out.push_str(&lawtext_paragraph_text(para, structured_sentences));
            out.push('\n');
            write_lawtext_items(out, &para.items, structured_sentences);
            out.push('\n');
        }
    }
}

fn write_lawtext_items(out: &mut String, items: &[LawItem], structured: bool) {
    for item in items {
        out.push_str("  ");
        out.push_str(&item.title);
        let sentence = lawtext_sentence_text(&item.sentence, &item.xml_sentence_content, structured);
        if !sentence.is_empty() {
            out.push(' ');
            out.push_str(&sentence);
        }
        out.push('\n');
        for sub in &item.subitems {
            write_lawtext_subitem(out, sub, structured);
        }
    }
}

fn write_lawtext_subitem(out: &mut String, sub: &LawSubitem, structured: bool) {
    for _ in 0..=sub.depth {
        out.push_str("  ");
    }
    out.push_str(&sub.title);
    let sentence = lawtext_sentence_text(&sub.sentence, &sub.xml_sentence_content, structured);
    if !sentence.is_empty() {
        out.push(' ');
        out.push_str(&sentence);
    }
    out.push('\n');
    for child in &sub.children {
        write_lawtext_subitem(out, child, structured);
    }
}

/// Parse Lawtext plain text into a [`LawDoc`].
pub fn parse_lawtext(text: &str) -> Result<LawDoc, Error> {
    let mut doc = LawDoc::default();
    let mut in_suppl = false;
    let mut in_appdx = false;
    let mut pending_caption: Option<String> = None;
    let mut cur_article: Option<LawArticle> = None;
    let mut div_stack: Vec<LawNode> = Vec::new();

    let flush_article = |cur_article: &mut Option<LawArticle>,
                         div_stack: &mut Vec<LawNode>,
                         doc: &mut LawDoc,
                         in_suppl: bool| {
        if let Some(art) = cur_article.take() {
            push_node_to_target(LawNode::Article(art), div_stack, doc, in_suppl);
        }
    };

    let lines = text.trim_start_matches('\u{feff}').lines()
        .filter(|line| !line.trim().is_empty());
    let mut lines = lines.peekable();
    while let Some(attributes) = lines.peek().and_then(|line| lawtext_attributes(line.trim())) {
        for (name, value) in attributes {
            match name {
                "Era" => doc.era = Some(value),
                "Year" => doc.year = Some(value),
                "PromulgateMonth" => doc.promulgate_month = Some(value),
                "PromulgateDay" => doc.promulgate_day = Some(value),
                "Num" => doc.num = Some(value),
                "LawType" => doc.law_type = Some(value),
                "Lang" => doc.lang = Some(value),
                _ => {}
            }
        }
        lines.next();
    }
    if let Some(attributes) = lines.peek()
        .and_then(|line| line.trim().strip_prefix(LAWTEXT_TITLE_ATTRIBUTES))
        .and_then(parse_title_attributes_fragment)
    {
        doc.law_title_attributes = attributes;
        lines.next();
    }
    if let Some(encoded) = lines.peek()
        .and_then(|line| line.trim().strip_prefix(LAWTEXT_ENACT_STATEMENT))
    {
        doc.enact_statement = serde_json::from_str(encoded).ok();
        lines.next();
    }
    if let Some(toc) = lines.peek()
        .and_then(|line| line.trim().strip_prefix(LAWTEXT_TOC))
        .and_then(read_encoded_toc)
    {
        doc.toc_xml = Some(toc);
        lines.next();
    }
    let table_xml = lines.peek().and_then(|line| line.trim().strip_prefix(LAWTEXT_TABLES));
    if table_xml.is_some() {
        lines.next();
    }
    if let Some(encoded) = lines.peek().and_then(|line| line.trim().strip_prefix(LAWTEXT_STYLES)) {
        doc.appdx_style_xml = read_encoded_appdx_styles(encoded);
        lines.next();
    }
    if let Some(encoded) = lines.peek().and_then(|line| line.trim().strip_prefix(LAWTEXT_STYLE_POSITIONS)) {
        doc.appdx_style_positions = read_encoded_appdx_style_positions(encoded, doc.appdx_style_xml.len());
        lines.next();
    }
    let mut header = lines.clone();
    if let (Some(title), Some(number)) = (
        header.next().map(str::trim).filter(|title| !title.contains('（') && !title.starts_with('第')),
        header.next().map(str::trim).and_then(standalone_law_number),
    ) {
        doc.law_title = title.to_string();
        doc.law_num = number.to_string();
        lines = header;
    }

    for raw_line in lines {
        let trimmed = raw_line.trim();
        if doc.law_title.is_empty() && cur_article.is_none() && div_stack.is_empty() {
            if let Some((title, law_num)) = split_title_and_parens(trimmed) {
                if !is_caption_line(trimmed) && !trimmed.starts_with('第') {
                    doc.law_title = title;
                    doc.law_num = law_num;
                    continue;
                }
            }
        }
        if trimmed.starts_with("別表") && !trimmed.contains('。') {
            flush_article(&mut cur_article, &mut div_stack, &mut doc, in_suppl);
            flush_all_divisions(&mut div_stack, &mut doc, in_suppl);
            in_appdx = true;
            let (title, rel) = split_title_and_parens(trimmed)
                .map(|(t, r)| (t, Some(r)))
                .unwrap_or_else(|| (trimmed.to_string(), None));
            doc.appdx_tables.push(LawAppdxTable {
                title,
                related_article_num: rel,
                rows: Vec::new(),
            });
            continue;
        }
        if in_appdx && trimmed.starts_with('|') && trimmed.ends_with('|') {
            let cells = crate::markdown::split_table_row(trimmed);
            let is_sep = !cells.is_empty()
                && cells
                    .iter()
                    .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')));
            if !is_sep && !cells.is_empty() {
                if let Some(appdx) = doc.appdx_tables.last_mut() {
                    appdx.rows.push(cells);
                }
            }
            continue;
        }
        if let Some(suppl_label) = parse_suppl_header(trimmed) {
            in_appdx = false;
            flush_article(&mut cur_article, &mut div_stack, &mut doc, in_suppl);
            flush_all_divisions(&mut div_stack, &mut doc, in_suppl);
            in_suppl = true;
            let (header, extract) = suppl_label
                .strip_suffix("（抄）")
                .map_or((suppl_label.as_str(), false), |header| (header, true));
            let (label, amend) = split_title_and_parens(header)
                .map(|(label, amend)| (label, Some(amend)))
                .unwrap_or_else(|| (header.to_string(), None));
            doc.suppl_provisions.push(SupplProvision {
                amend_law_num: amend,
                extract,
                label,
                nodes: Vec::new(),
            });
            continue;
        }
        if let Some((kind, num)) = parse_division_header(trimmed) {
            flush_article(&mut cur_article, &mut div_stack, &mut doc, in_suppl);
            while let Some(LawNode::Division { kind: top_kind, .. }) = div_stack.last() {
                if *top_kind as u8 >= kind as u8 {
                    let popped = div_stack.pop().unwrap();
                    push_node_to_target(popped, &mut div_stack, &mut doc, in_suppl);
                } else {
                    break;
                }
            }
            div_stack.push(LawNode::Division {
                kind,
                num,
                title: trimmed.to_string(),
                children: Vec::new(),
            });
            continue;
        }
        if is_caption_line(trimmed) && cur_article.as_ref().is_none_or(|a| !a.paragraphs.is_empty())
        {
            flush_article(&mut cur_article, &mut div_stack, &mut doc, in_suppl);
            pending_caption = Some(trimmed.to_string());
            continue;
        }
        if let Some((art_title, art_num, rest)) = parse_article_header(trimmed) {
            flush_article(&mut cur_article, &mut div_stack, &mut doc, in_suppl);
            let mut paragraphs = Vec::new();
            if !rest.is_empty() {
                paragraphs.push(parse_lawtext_paragraph("1".to_string(), rest));
            }
            cur_article = Some(LawArticle {
                num: art_num,
                caption: pending_caption.take(),
                title: art_title,
                paragraphs,
            });
            continue;
        }
        if let Some(art) = cur_article.as_mut() {
            if let Some((p_num, rest)) = parse_numbered_paragraph(trimmed) {
                art.paragraphs.push(parse_lawtext_paragraph(p_num, rest));
                continue;
            }
            let indent = raw_line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
            if let Some(para) = art.paragraphs.last_mut() {
                if indent >= 4 {
                    if let Some(item) = para.items.last_mut() {
                        if let Some((sub_title, sub_sent)) = split_leading_token(trimmed) {
                            let depth = (indent / 2 - 1).min(10) as u8;
                            append_lawtext_subitem(&mut item.subitems, depth, 1, sub_title, sub_sent);
                            continue;
                        }
                    }
                }
                if indent >= 2 {
                    if let Some((it_title, it_sent)) = split_leading_token(trimmed) {
                        let (sentence, xml_sentence_content) = parse_lawtext_sentence_text(it_sent, true);
                        let num = if in_suppl {
                            supplementary_item_number(it_title, &sentence)
                        } else {
                            None
                        }
                        .unwrap_or_else(|| (para.items.len() + 1).to_string());
                        para.items.push(LawItem {
                            num,
                            title: it_title.to_string(),
                            sentence,
                            xml_sentence_content,
                            subitems: Vec::new(),
                        });
                        continue;
                    }
                }
                if !para.sentence.is_empty() {
                    para.sentence.push_str(trimmed);
                } else {
                    para.sentence = trimmed.to_string();
                }
            } else {
                art.paragraphs.push(parse_lawtext_paragraph("1".to_string(), trimmed));
            }
        } else if in_suppl {
            let (p_num, rest) = parse_numbered_paragraph(trimmed)
                .unwrap_or_else(|| ("1".to_string(), trimmed));
            push_node_to_target(
                LawNode::Paragraph(parse_lawtext_paragraph(p_num, rest)),
                &mut div_stack,
                &mut doc,
                true,
            );
        }
    }

    flush_article(&mut cur_article, &mut div_stack, &mut doc, in_suppl);
    flush_all_divisions(&mut div_stack, &mut doc, in_suppl);
    if let Some(encoded) = table_xml {
        restore_tables(&mut doc, encoded, false);
    }
    Ok(doc)
}

fn append_lawtext_subitem(
    siblings: &mut Vec<LawSubitem>,
    depth: u8,
    level: u8,
    title: &str,
    text: &str,
) {
    if depth > level && let Some(parent) = siblings.last_mut() {
        append_lawtext_subitem(&mut parent.children, depth, level + 1, title, text);
        return;
    }
    let (sentence, xml_sentence_content) = parse_lawtext_sentence_text(text, true);
    siblings.push(LawSubitem {
        depth: level,
        num: (siblings.len() + 1).to_string(),
        title: title.to_string(),
        sentence,
        xml_sentence_content,
        children: Vec::new(),
    });
}

fn push_node_to_target(
    node: LawNode,
    div_stack: &mut [LawNode],
    doc: &mut LawDoc,
    in_suppl: bool,
) {
    if let Some(LawNode::Division { children, .. }) = div_stack.last_mut() {
        children.push(node);
    } else if in_suppl {
        if let Some(suppl) = doc.suppl_provisions.last_mut() {
            suppl.nodes.push(node);
        }
    } else {
        doc.main_provision.push(node);
    }
}

fn flush_all_divisions(div_stack: &mut Vec<LawNode>, doc: &mut LawDoc, in_suppl: bool) {
    while let Some(popped) = div_stack.pop() {
        push_node_to_target(popped, div_stack, doc, in_suppl);
    }
}
