// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! e-Gov law XML parser and writer.

use crate::Error;
use super::model::{
    DivisionKind, LawArticle, LawDoc, LawItem, LawNode, LawParagraph,
    LawParagraphLabels, LawSentence, LawSentenceColumn, LawSentenceContent, LawSubitem,
    LawTitleAttributes, SupplProvision,
};
use super::syntax::to_fullwidth_digits;
use super::table_markup::{SavedAppdxTable, parse_appdx_table};

/// Parse an e-Gov 法令標準XML string into a [`LawDoc`].
pub fn parse_law_xml(xml: &str) -> Result<LawDoc, Error> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| Error::new(format!("法令XMLの解析に失敗しました: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "Law" {
        return Err(Error::new("ルート要素が <Law> ではありません"));
    }

    let mut out = LawDoc {
        era: root.attribute("Era").map(str::to_string),
        year: root.attribute("Year").map(str::to_string),
        promulgate_month: root.attribute("PromulgateMonth").map(str::to_string),
        promulgate_day: root.attribute("PromulgateDay").map(str::to_string),
        num: root.attribute("Num").map(str::to_string),
        law_type: root.attribute("LawType").map(str::to_string),
        lang: root.attribute("Lang").map(str::to_string),
        ..LawDoc::default()
    };

    if let Some(law_num) = child_elem(root, "LawNum") {
        out.law_num = collect_text(law_num);
    }

    let body = child_elem(root, "LawBody")
        .ok_or_else(|| Error::new("<LawBody> 要素が見つかりません"))?;
    if let Some(title) = child_elem(body, "LawTitle") {
        out.law_title = collect_text(title);
        out.law_title_attributes = parse_title_attributes(title);
    }
    if let Some(enact) = child_elem(body, "EnactStatement") {
        let s = collect_text(enact);
        if !s.is_empty() {
            out.enact_statement = Some(s);
        }
    }
    out.toc_xml = child_elem(body, "TOC").and_then(|toc| read_xml_fragment(&xml[toc.range()], &["TOC"]));
    if let Some(pre) = child_elem(body, "Preamble") {
        let s = collect_text(pre);
        if !s.is_empty() {
            out.preamble = Some(s);
        }
    }
    if let Some(main) = child_elem(body, "MainProvision") {
        out.main_provision = parse_xml_nodes(main);
    }
    for suppl in body.children().filter(|n| n.is_element() && n.tag_name().name() == "SupplProvision") {
        let label = child_elem(suppl, "SupplProvisionLabel")
            .map(collect_text)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "附則".to_string());
        out.suppl_provisions.push(SupplProvision {
            amend_law_num: suppl.attribute("AmendLawNum").map(str::to_string),
            extract: suppl.attribute("Extract").is_some_and(|v| v == "true"),
            label,
            nodes: parse_xml_nodes(suppl),
            appendix_xml: suppl.children()
                .filter(|node| node.is_element() && node.tag_name().namespace().is_none())
                .filter(|node| SUPPL_APPDX_NAMES.contains(&node.tag_name().name()))
                .filter_map(|node| read_xml_fragment(&xml[node.range()], SUPPL_APPDX_NAMES))
                .collect(),
        });
    }
    for appendix in body.children().filter(roxmltree::Node::is_element) {
        match appendix.tag_name().name() {
            "AppdxTable" => {
                out.appdx_tables.push(parse_appdx_table(appendix));
                if let Some(snapshot) = SavedAppdxTable::read(&xml[appendix.range()]) {
                    out.appdx_table_xml.push(snapshot);
                }
            }
            name if APPDX_FRAGMENT_NAMES.contains(&name) && appendix.tag_name().namespace().is_none() => {
                if let Some(fragment) = read_xml_fragment(&xml[appendix.range()], APPDX_FRAGMENT_NAMES) {
                    out.appdx_fragment_positions.push(out.appdx_tables.len());
                    out.appdx_fragment_xml.push(fragment);
                }
            }
            _ => {}
        }
    }
    if out.appdx_fragment_positions.first() == Some(&out.appdx_tables.len()) {
        out.appdx_fragment_positions.clear();
    }
    Ok(out)
}

const APPDX_FRAGMENT_NAMES: &[&str] = &["Appdx", "AppdxFig", "AppdxFormat", "AppdxNote", "AppdxStyle"];

const SUPPL_APPDX_NAMES: &[&str] = &[
    "SupplProvisionAppdxTable",
    "SupplProvisionAppdxStyle",
    "SupplProvisionAppdx",
];

pub(super) fn read_encoded_suppl_appendices(encoded: &str) -> Vec<String> {
    read_encoded_fragments(encoded, SUPPL_APPDX_NAMES)
}

fn read_xml_fragment(xml: &str, names: &[&str]) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let node = doc.root_element();
    if !names.contains(&node.tag_name().name()) || node.tag_name().namespace().is_some() {
        return None;
    }
    Some(xml[node.range()].to_string())
}

pub(super) fn read_encoded_toc(encoded: &str) -> Option<String> {
    let toc = serde_json::from_str::<String>(encoded).ok()?;
    read_xml_fragment(&toc, &["TOC"])
}

pub(super) fn read_encoded_appdx_fragments(encoded: &str) -> Vec<String> {
    read_encoded_fragments(encoded, APPDX_FRAGMENT_NAMES)
}

fn read_encoded_fragments(encoded: &str, names: &[&str]) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(encoded)
        .unwrap_or_default()
        .iter()
        .filter_map(|fragment| read_xml_fragment(fragment, names))
        .collect()
}

pub(super) fn read_encoded_appdx_fragment_positions(encoded: &str, fragment_count: usize) -> Vec<usize> {
    let positions: Vec<usize> = serde_json::from_str(encoded).unwrap_or_default();
    if positions.len() == fragment_count && positions.is_sorted() {
        positions
    } else {
        Vec::new()
    }
}

fn parse_title_attributes(node: roxmltree::Node<'_, '_>) -> LawTitleAttributes {
    LawTitleAttributes {
        kana: node.attribute("Kana").map(str::to_string),
        abbrev: node.attribute("Abbrev").map(str::to_string),
        abbrev_kana: node.attribute("AbbrevKana").map(str::to_string),
    }
}

pub(super) fn parse_title_attributes_fragment(fragment: &str) -> Option<LawTitleAttributes> {
    let doc = roxmltree::Document::parse(fragment).ok()?;
    let root = doc.root_element();
    if !root.has_tag_name("LawTitle") || root.has_children() {
        return None;
    }
    Some(parse_title_attributes(root))
}

fn write_title_attributes(out: &mut String, attributes: &LawTitleAttributes) {
    for (name, value) in [
        ("Kana", attributes.kana.as_deref()),
        ("Abbrev", attributes.abbrev.as_deref()),
        ("AbbrevKana", attributes.abbrev_kana.as_deref()),
    ] {
        if let Some(value) = value {
            out.push_str(&format!(" {name}=\""));
            for ch in value.chars() {
                match ch {
                    '\t' => out.push_str("&#9;"),
                    '\n' => out.push_str("&#10;"),
                    '\r' => out.push_str("&#13;"),
                    ch => push_xml_escaped(out, ch.encode_utf8(&mut [0; 4])),
                }
            }
            out.push('"');
        }
    }
}

pub(super) fn write_title_attributes_fragment(out: &mut String, attributes: &LawTitleAttributes) {
    out.push_str("<LawTitle");
    write_title_attributes(out, attributes);
    out.push_str("/>");
}

pub(super) fn child_elem<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    parent
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

pub(super) fn collect_text(node: roxmltree::Node<'_, '_>) -> String {
    let mut out = String::new();
    collect_text_into(node, &mut out);
    out.trim().to_string()
}

fn collect_text_into(node: roxmltree::Node<'_, '_>, out: &mut String) {
    if node.is_text() {
        if let Some(t) = node.text().filter(|t| !(t.contains('\n') && t.trim().is_empty())) {
            out.push_str(t);
        }
        return;
    }
    if !node.is_element() {
        return;
    }
    // Skip Ruby annotations <Rt> when flattening plain text so reading stays clean.
    let tag = node.tag_name().name();
    if tag == "Rt" || tag == "Rp" {
        return;
    }
    if tag == "Column" && !out.is_empty() && !out.ends_with(char::is_whitespace) {
        out.push(' ');
    }
    for child in node.children() {
        collect_text_into(child, out);
    }
}

fn parse_xml_nodes(parent: roxmltree::Node<'_, '_>) -> Vec<LawNode> {
    let mut nodes = Vec::new();
    for child in parent.children().filter(|n| n.is_element()) {
        let tag = child.tag_name().name();
        if let Some(kind) = DivisionKind::from_xml_tag(tag) {
            let num = child.attribute("Num").unwrap_or("").to_string();
            let title = child_elem(child, kind.title_tag())
                .map(collect_text)
                .unwrap_or_default();
            let children = parse_xml_nodes(child);
            nodes.push(LawNode::Division {
                kind,
                num,
                title,
                children,
            });
        } else if tag == "Article" {
            nodes.push(LawNode::Article(parse_xml_article(child)));
        } else if tag == "Paragraph" {
            nodes.push(LawNode::Paragraph(parse_xml_paragraph(child)));
        }
    }
    nodes
}

fn parse_xml_article(node: roxmltree::Node<'_, '_>) -> LawArticle {
    let num = node.attribute("Num").unwrap_or("").to_string();
    let caption = child_elem(node, "ArticleCaption")
        .map(collect_text)
        .filter(|s| !s.is_empty());
    let title = child_elem(node, "ArticleTitle")
        .map(collect_text)
        .unwrap_or_default();
    let paragraphs = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Paragraph")
        .map(parse_xml_paragraph)
        .collect();
    LawArticle {
        num,
        caption,
        title,
        paragraphs,
    }
}

pub(super) fn is_sentence_inline(node: roxmltree::Node<'_, '_>) -> bool {
    node.is_text() || (node.has_tag_name("Ruby")
        && node.attributes().len() == 0
        && node.children().all(|child| child.is_text() || (child.has_tag_name("Rt")
            && child.attributes().len() == 0
            && child.children().all(|text| text.is_text()))))
}

fn write_ruby_inline(out: &mut String, node: roxmltree::Node<'_, '_>) {
    if node.is_text() {
        push_xml_escaped(out, node.text().unwrap_or_default());
        return;
    }
    let tag = node.tag_name().name();
    out.push('<');
    out.push_str(tag);
    out.push('>');
    for child in node.children() {
        write_ruby_inline(out, child);
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

fn parse_xml_sentence(node: roxmltree::Node<'_, '_>) -> LawSentence {
    let ruby_markup = (node.children().any(|child| child.has_tag_name("Ruby"))
        && node.children().all(is_sentence_inline)).then(|| {
            let mut xml = String::new();
            for child in node.children() {
                write_ruby_inline(&mut xml, child);
            }
            xml
        });
    LawSentence {
        text: collect_text(node),
        ruby_markup,
        num: node.attribute("Num").map(str::to_string),
        function: node.attribute("Function").map(str::to_string),
        indent: node.attribute("Indent").map(str::to_string),
        writing_mode: node.attribute("WritingMode").map(str::to_string),
    }
}

pub(super) fn parse_xml_sentence_container(
    container: Option<roxmltree::Node<'_, '_>>,
    allow_columns: bool,
) -> (String, LawSentenceContent) {
    let Some(container) = container else {
        return (String::new(), LawSentenceContent::default());
    };
    let children: Vec<_> = container.children().filter(roxmltree::Node::is_element).collect();
    if allow_columns && !children.is_empty() && children.iter().all(|node| node.has_tag_name("Column")) {
        let columns = children.into_iter().map(|node| LawSentenceColumn {
            num: node.attribute("Num").map(str::to_string),
            line_break: node.attribute("LineBreak").map(str::to_string),
            align: node.attribute("Align").map(str::to_string),
            sentences: node.children().filter(|child| child.has_tag_name("Sentence"))
                .map(parse_xml_sentence).collect(),
        }).collect();
        let content = LawSentenceContent::Columns(columns);
        return (content.plain_text().unwrap_or_default(), content);
    }
    let content = LawSentenceContent::Sentences(children.into_iter()
        .filter(|node| node.has_tag_name("Sentence")).map(parse_xml_sentence).collect());
    let sentence = content.plain_text().filter(|text| !text.is_empty())
        .unwrap_or_else(|| collect_text(container));
    (sentence, content)
}

pub(super) fn parse_paragraph_labels(node: roxmltree::Node<'_, '_>) -> LawParagraphLabels {
    let caption = child_elem(node, "ParagraphCaption");
    LawParagraphLabels {
        caption: caption.map(collect_text),
        common_caption: caption.and_then(|node| node.attribute("CommonCaption")).map(str::to_string),
        number: child_elem(node, "ParagraphNum").map(collect_text),
    }
}

fn default_paragraph_number(num: &str) -> String {
    if num == "1" { String::new() } else { to_fullwidth_digits(num) }
}

fn parse_xml_paragraph(node: roxmltree::Node<'_, '_>) -> LawParagraph {
    let num = node.attribute("Num").unwrap_or("1").to_string();
    let (sentence, xml_sentence_content) = parse_xml_sentence_container(child_elem(node, "ParagraphSentence"), false);
    let items = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Item")
        .map(parse_xml_item)
        .collect();
    let mut labels = parse_paragraph_labels(node);
    if labels.number.as_deref() == Some(&default_paragraph_number(&num)) {
        labels.number = None;
    }
    LawParagraph {
        num,
        labels,
        sentence,
        xml_sentence_content,
        items,
    }
}

fn parse_xml_item(node: roxmltree::Node<'_, '_>) -> LawItem {
    let num = node.attribute("Num").unwrap_or("1").to_string();
    let title = child_elem(node, "ItemTitle")
        .map(collect_text)
        .unwrap_or_default();
    let (sentence, xml_sentence_content) = parse_xml_sentence_container(child_elem(node, "ItemSentence"), true);
    let subitems = parse_xml_subitems(node, 1);
    LawItem {
        num,
        title,
        sentence,
        xml_sentence_content,
        subitems,
    }
}

fn parse_xml_subitems(parent: roxmltree::Node<'_, '_>, depth: u8) -> Vec<LawSubitem> {
    if depth > 10 {
        return Vec::new();
    }
    let tag = format!("Subitem{depth}");
    let title_tag = format!("Subitem{depth}Title");
    let sent_tag = format!("Subitem{depth}Sentence");
    parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == tag)
        .map(|sub| {
            let (sentence, xml_sentence_content) = parse_xml_sentence_container(child_elem(sub, &sent_tag), true);
            LawSubitem {
                depth,
                num: sub.attribute("Num").unwrap_or("1").to_string(),
                title: child_elem(sub, &title_tag).map(collect_text).unwrap_or_default(),
                sentence,
                xml_sentence_content,
                children: parse_xml_subitems(sub, depth + 1),
            }
        })
        .collect()
}

/// Serialize a [`LawDoc`] into an e-Gov 法令標準XML string.
pub fn to_law_xml(doc: &LawDoc) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Law");
    push_opt_attr(&mut out, "Era", doc.era.as_deref());
    push_opt_attr(&mut out, "Lang", doc.lang.as_deref());
    push_opt_attr(&mut out, "LawType", doc.law_type.as_deref());
    push_opt_attr(&mut out, "Num", doc.num.as_deref());
    push_opt_attr(&mut out, "PromulgateMonth", doc.promulgate_month.as_deref());
    push_opt_attr(&mut out, "PromulgateDay", doc.promulgate_day.as_deref());
    push_opt_attr(&mut out, "Year", doc.year.as_deref());
    out.push_str(">\n");
    if !doc.law_num.is_empty() {
        out.push_str("  <LawNum>");
        push_xml_escaped(&mut out, &doc.law_num);
        out.push_str("</LawNum>\n");
    }
    out.push_str("  <LawBody>\n");
    if !doc.law_title.is_empty() || doc.law_title_attributes != LawTitleAttributes::default() {
        out.push_str("    <LawTitle");
        write_title_attributes(&mut out, &doc.law_title_attributes);
        out.push('>');
        push_xml_escaped(&mut out, &doc.law_title);
        out.push_str("</LawTitle>\n");
    }
    if let Some(enact) = doc.enact_statement.as_deref().filter(|s| !s.is_empty()) {
        out.push_str("    <EnactStatement>");
        push_xml_escaped(&mut out, enact);
        out.push_str("</EnactStatement>\n");
    }
    if let Some(toc) = &doc.toc_xml {
        out.push_str("    ");
        out.push_str(toc);
        out.push('\n');
    }
    if let Some(pre) = doc.preamble.as_deref().filter(|s| !s.is_empty()) {
        out.push_str("    <Preamble><Paragraph Num=\"1\"><ParagraphNum/><ParagraphSentence><Sentence>");
        push_xml_escaped(&mut out, pre);
        out.push_str("</Sentence></ParagraphSentence></Paragraph></Preamble>\n");
    }
    out.push_str("    <MainProvision>\n");
    for node in &doc.main_provision {
        write_xml_node(&mut out, node, 3);
    }
    out.push_str("    </MainProvision>\n");
    for suppl in &doc.suppl_provisions {
        out.push_str("    <SupplProvision");
        push_opt_attr(&mut out, "AmendLawNum", suppl.amend_law_num.as_deref());
        if suppl.extract {
            out.push_str(" Extract=\"true\"");
        }
        out.push_str(">\n");
        out.push_str("      <SupplProvisionLabel>");
        push_xml_escaped(&mut out, &suppl.label);
        out.push_str("</SupplProvisionLabel>\n");
        for node in &suppl.nodes {
            write_xml_node(&mut out, node, 3);
        }
        for fragment in &suppl.appendix_xml {
            out.push_str("      ");
            out.push_str(fragment);
            out.push('\n');
        }
        out.push_str("    </SupplProvision>\n");
    }
    let mut fragments = doc.appdx_fragment_xml.iter();
    let mut positions = doc.appdx_fragment_positions.iter().copied().peekable();
    for (index, appdx) in doc.appdx_tables.iter().enumerate() {
        while positions.next_if_eq(&index).is_some() {
            if let Some(fragment) = fragments.next() {
                write_appdx_fragment(&mut out, fragment);
            }
        }
        write_appdx_table(&mut out, doc, appdx);
    }
    for fragment in fragments {
        write_appdx_fragment(&mut out, fragment);
    }
    out.push_str("  </LawBody>\n</Law>\n");
    out
}

fn write_appdx_fragment(out: &mut String, fragment: &str) {
    out.push_str("    ");
    out.push_str(fragment);
    out.push('\n');
}

fn write_appdx_table(out: &mut String, doc: &LawDoc, appdx: &super::model::LawAppdxTable) {
    let mut snapshots = doc.appdx_table_xml.iter().filter(|snapshot| snapshot.table == *appdx);
    if let Some(snapshot) = snapshots.next() && snapshots.next().is_none() {
        out.push_str("    ");
        out.push_str(&snapshot.xml);
        out.push('\n');
        return;
    }
    out.push_str("    <AppdxTable>\n      <AppdxTableTitle>");
    push_xml_escaped(out, &appdx.title);
    out.push_str("</AppdxTableTitle>\n");
    if let Some(rel) = appdx.related_article_num.as_deref() {
        out.push_str("      <RelatedArticleNum>");
        push_xml_escaped(out, rel);
        out.push_str("</RelatedArticleNum>\n");
    }
    if !appdx.rows.is_empty() {
        out.push_str("      <TableStruct>\n        <Table>\n");
        for row in &appdx.rows {
            out.push_str("          <TableRow>\n");
            for cell in row {
                out.push_str("            <TableColumn><Sentence>");
                push_xml_escaped(out, cell);
                out.push_str("</Sentence></TableColumn>\n");
            }
            out.push_str("          </TableRow>\n");
        }
        out.push_str("        </Table>\n      </TableStruct>\n");
    }
    out.push_str("    </AppdxTable>\n");
}

fn push_opt_attr(out: &mut String, name: &str, val: Option<&str>) {
    if let Some(v) = val.filter(|s| !s.is_empty()) {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        push_xml_escaped(out, v);
        out.push('"');
    }
}

fn push_xml_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_xml_node(out: &mut String, node: &LawNode, depth: usize) {
    match node {
        LawNode::Division {
            kind,
            num,
            title,
            children,
        } => {
            push_indent(out, depth);
            out.push('<');
            out.push_str(kind.xml_tag());
            if !num.is_empty() {
                push_opt_attr(out, "Num", Some(num));
            }
            out.push_str(">\n");
            push_indent(out, depth + 1);
            out.push('<');
            out.push_str(kind.title_tag());
            out.push('>');
            push_xml_escaped(out, title);
            out.push_str("</");
            out.push_str(kind.title_tag());
            out.push_str(">\n");
            for child in children {
                write_xml_node(out, child, depth + 1);
            }
            push_indent(out, depth);
            out.push_str("</");
            out.push_str(kind.xml_tag());
            out.push_str(">\n");
        }
        LawNode::Article(art) => {
            push_indent(out, depth);
            out.push_str("<Article");
            push_opt_attr(out, "Num", Some(&art.num));
            out.push_str(">\n");
            if let Some(cap) = art.caption.as_deref().filter(|s| !s.is_empty()) {
                push_indent(out, depth + 1);
                out.push_str("<ArticleCaption>");
                push_xml_escaped(out, cap);
                out.push_str("</ArticleCaption>\n");
            }
            if !art.title.is_empty() {
                push_indent(out, depth + 1);
                out.push_str("<ArticleTitle>");
                push_xml_escaped(out, &art.title);
                out.push_str("</ArticleTitle>\n");
            }
            for para in &art.paragraphs {
                write_xml_paragraph(out, para, depth + 1);
            }
            push_indent(out, depth);
            out.push_str("</Article>\n");
        }
        LawNode::Paragraph(para) => write_xml_paragraph(out, para, depth),
    }
}

fn write_xml_sentence(out: &mut String, sentence: &LawSentence) {
    out.push_str("<Sentence");
    push_opt_attr(out, "Num", sentence.num.as_deref());
    push_opt_attr(out, "Function", sentence.function.as_deref());
    push_opt_attr(out, "Indent", sentence.indent.as_deref());
    push_opt_attr(out, "WritingMode", sentence.writing_mode.as_deref());
    out.push('>');
    if let Some(xml) = &sentence.ruby_markup {
        out.push_str(xml);
    } else {
        push_xml_escaped(out, &sentence.text);
    }
    out.push_str("</Sentence>");
}

pub(super) fn write_xml_sentence_content(out: &mut String, text: &str, content: &LawSentenceContent) {
    if content.plain_text().as_deref() == Some(text) {
        write_xml_sentence_elements(out, content);
    } else {
        out.push_str("<Sentence>");
        push_xml_escaped(out, text);
        out.push_str("</Sentence>");
    }
}

pub(super) fn write_xml_sentence_elements(out: &mut String, content: &LawSentenceContent) {
    match content {
        LawSentenceContent::Sentences(sentences) => {
            for sentence in sentences {
                write_xml_sentence(out, sentence);
            }
        }
        LawSentenceContent::Columns(columns) => {
            for column in columns {
                out.push_str("<Column");
                push_opt_attr(out, "Num", column.num.as_deref());
                push_opt_attr(out, "LineBreak", column.line_break.as_deref());
                push_opt_attr(out, "Align", column.align.as_deref());
                out.push('>');
                for sentence in &column.sentences {
                    write_xml_sentence(out, sentence);
                }
                out.push_str("</Column>");
            }
        }
    }
}

pub(super) fn write_paragraph_caption(out: &mut String, labels: &LawParagraphLabels) {
    if let Some(caption) = &labels.caption {
        out.push_str("<ParagraphCaption");
        push_opt_attr(out, "CommonCaption", labels.common_caption.as_deref());
        out.push('>');
        push_xml_escaped(out, caption);
        out.push_str("</ParagraphCaption>");
    }
}

pub(super) fn write_paragraph_number(out: &mut String, label: &str) {
    out.push_str("<ParagraphNum>");
    push_xml_escaped(out, label);
    out.push_str("</ParagraphNum>");
}

fn write_xml_paragraph(out: &mut String, para: &LawParagraph, depth: usize) {
    push_indent(out, depth);
    out.push_str("<Paragraph");
    push_opt_attr(out, "Num", Some(&para.num));
    out.push_str(">\n");
    if para.labels.caption.is_some() {
        push_indent(out, depth + 1);
        write_paragraph_caption(out, &para.labels);
        out.push('\n');
    }
    push_indent(out, depth + 1);
    let default_number = default_paragraph_number(&para.num);
    write_paragraph_number(out, para.labels.number.as_deref().unwrap_or(&default_number));
    out.push('\n');
    push_indent(out, depth + 1);
    out.push_str("<ParagraphSentence>");
    write_xml_sentence_content(out, &para.sentence, &para.xml_sentence_content);
    out.push_str("</ParagraphSentence>\n");
    for item in &para.items {
        push_indent(out, depth + 1);
        out.push_str("<Item");
        push_opt_attr(out, "Num", Some(&item.num));
        out.push_str(">\n");
        push_indent(out, depth + 2);
        out.push_str("<ItemTitle>");
        push_xml_escaped(out, &item.title);
        out.push_str("</ItemTitle>\n");
        push_indent(out, depth + 2);
        out.push_str("<ItemSentence>");
        write_xml_sentence_content(out, &item.sentence, &item.xml_sentence_content);
        out.push_str("</ItemSentence>\n");
        for sub in &item.subitems {
            write_xml_subitem(out, sub, depth + 2);
        }
        push_indent(out, depth + 1);
        out.push_str("</Item>\n");
    }
    push_indent(out, depth);
    out.push_str("</Paragraph>\n");
}

fn write_xml_subitem(out: &mut String, sub: &LawSubitem, depth: usize) {
    let tag = format!("Subitem{}", sub.depth);
    let title_tag = format!("Subitem{}Title", sub.depth);
    let sent_tag = format!("Subitem{}Sentence", sub.depth);
    push_indent(out, depth);
    out.push('<');
    out.push_str(&tag);
    push_opt_attr(out, "Num", Some(&sub.num));
    out.push_str(">\n");
    push_indent(out, depth + 1);
    out.push('<');
    out.push_str(&title_tag);
    out.push('>');
    push_xml_escaped(out, &sub.title);
    out.push_str("</");
    out.push_str(&title_tag);
    out.push_str(">\n");
    push_indent(out, depth + 1);
    out.push('<');
    out.push_str(&sent_tag);
    out.push('>');
    write_xml_sentence_content(out, &sub.sentence, &sub.xml_sentence_content);
    out.push_str("</");
    out.push_str(&sent_tag);
    out.push_str(">\n");
    for child in &sub.children {
        write_xml_subitem(out, child, depth + 1);
    }
    push_indent(out, depth);
    out.push_str("</");
    out.push_str(&tag);
    out.push_str(">\n");
}
