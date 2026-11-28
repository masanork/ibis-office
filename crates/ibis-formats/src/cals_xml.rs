// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust reader for Japanese MLIT/MAFF/municipal CALS/EC electronic delivery XML files
//! (`INDEX_D.XML`, `INDEX_C.XML`, `DRAWING.XML`, `PHOTO.XML`, etc.).

use crate::{Error, Opened};

pub fn is_cals_xml(data: &[u8]) -> bool {
    let text = decode_xml_bytes(data);
    let head: String = text.chars().take(2048).collect();
    if !head.contains('<') {
        return false;
    }
    const MARKERS: [&str; 10] = [
        "<基礎情報",
        "<業務管理項目",
        "<工事管理項目",
        "<図面情報",
        "<写真情報",
        "<報告書ファイル情報",
        "INDEX_D",
        "INDEX_C",
        "DRAWING",
        "PHOTO",
    ];
    head.contains("<?xml") && MARKERS.iter().any(|m| head.contains(m))
}

pub fn decode_xml_bytes(data: &[u8]) -> String {
    let head_ascii: String = data
        .iter()
        .take(256)
        .map(|&b| if b.is_ascii() { b as char } else { ' ' })
        .collect();
    let lower = head_ascii.to_ascii_lowercase();
    if lower.contains("encoding=\"shift_jis\"")
        || lower.contains("encoding='shift_jis'")
        || lower.contains("encoding=\"csシフトjis\"")
        || lower.contains("encoding=\"windows-31j\"")
    {
        let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(data);
        return cow.into_owned();
    }
    if lower.contains("encoding=\"euc-jp\"") || lower.contains("encoding='euc-jp'") {
        let (cow, _, _) = encoding_rs::EUC_JP.decode(data);
        return cow.into_owned();
    }
    match std::str::from_utf8(data) {
        Ok(s) => s.strip_prefix('\u{FEFF}').unwrap_or(s).to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(data);
            cow.into_owned()
        }
    }
}

pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let xml = decode_xml_bytes(data);
    let doc = roxmltree::Document::parse_with_options(
        &xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|e| Error::new(format!("invalid CALS/EC XML: {e}")))?;

    let root = doc.root_element();
    let mut md = format!("# 電子納品管理XML ({})\n\n", root.tag_name().name());

    let mut kv_rows: Vec<(String, String)> = Vec::new();
    let mut list_groups: Vec<(String, Vec<Vec<(String, String)>>)> = Vec::new();

    for child in root.children().filter(|n| n.is_element()) {
        let tag = child.tag_name().name().to_string();
        let sub_elements: Vec<_> = child.children().filter(|n| n.is_element()).collect();
        if sub_elements.is_empty() {
            let text = child.text().unwrap_or("").trim();
            if !text.is_empty() {
                kv_rows.push((tag, text.to_string()));
            }
            continue;
        }
        // Check if this is a repeating record group (e.g., <図面情報> or <写真情報>) or a metadata section (<基礎情報>)
        let has_nested_records = sub_elements
            .iter()
            .any(|el| el.children().any(|c| c.is_element()));
        if has_nested_records {
            for rec in sub_elements {
                let rec_tag = rec.tag_name().name().to_string();
                let mut pairs = Vec::new();
                collect_leaf_pairs(rec, "", &mut pairs);
                if !pairs.is_empty() {
                    if let Some((_, rows)) = list_groups.iter_mut().find(|(g, _)| g == &rec_tag) {
                        rows.push(pairs);
                    } else {
                        list_groups.push((rec_tag, vec![pairs]));
                    }
                }
            }
        } else {
            let mut pairs = Vec::new();
            collect_leaf_pairs(child, &tag, &mut pairs);
            if let Some((_, rows)) = list_groups.iter_mut().find(|(g, _)| g == &tag) {
                rows.push(pairs);
            } else if is_repeating_tag(&tag) {
                list_groups.push((tag, vec![pairs]));
            } else {
                kv_rows.extend(pairs);
            }
        }
    }

    if !kv_rows.is_empty() {
        md.push_str("## 基本情報\n\n| 項目 | 内容 |\n| --- | --- |\n");
        for (k, v) in kv_rows {
            md.push_str(&format!("| {} | {} |\n", escape_md(&k), escape_md(&v)));
        }
        md.push('\n');
    }

    for (group, rows) in list_groups {
        if rows.is_empty() {
            continue;
        }
        let mut headers: Vec<String> = Vec::new();
        for row in &rows {
            for (k, _) in row {
                if !headers.contains(k) {
                    headers.push(k.clone());
                }
            }
        }
        if headers.is_empty() {
            continue;
        }
        md.push_str(&format!("## {group} ({}件)\n\n", rows.len()));
        md.push('|');
        for h in &headers {
            md.push_str(&format!(" {} |", escape_md(h)));
        }
        md.push_str("\n|");
        for _ in &headers {
            md.push_str(" --- |");
        }
        md.push('\n');
        for row in &rows {
            md.push('|');
            for h in &headers {
                let val = row
                    .iter()
                    .find(|(k, _)| k == h)
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
                md.push_str(&format!(" {} |", escape_md(val)));
            }
            md.push('\n');
        }
        md.push('\n');
    }

    Ok(md)
}

fn is_repeating_tag(tag: &str) -> bool {
    tag.ends_with("情報") || tag.ends_with("ファイル")
}

fn collect_leaf_pairs(node: roxmltree::Node<'_, '_>, prefix: &str, out: &mut Vec<(String, String)>) {
    let mut has_child_elem = false;
    for child in node.children().filter(|n| n.is_element()) {
        has_child_elem = true;
        let name = child.tag_name().name();
        let next_prefix = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        collect_leaf_pairs(child, &next_prefix, out);
    }
    if !has_child_elem {
        let text = node.text().unwrap_or("").trim();
        if !text.is_empty() {
            let label = if prefix.is_empty() {
                node.tag_name().name().to_string()
            } else {
                prefix.to_string()
            };
            out.push((label, text.to_string()));
        }
    }
}

fn escape_md(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

pub fn open(data: &[u8]) -> Result<Opened, Error> {
    let md = to_markdown(data)?;
    crate::markdown::open_markdown(md.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shift_jis_cals_index_xml_with_dtd() {
        let xml = concat!(
            "<?xml version=\"1.0\" encoding=\"Shift_JIS\"?>\n",
            "<!DOCTYPE INDEX_D SYSTEM \"INDEX_D05.DTD\">\n",
            "<INDEX_D>\n",
            "  <基礎情報>\n",
            "    <業務名>橋梁詳細設計業務</業務名>\n",
            "    <発注機関名>国土交通省</発注機関名>\n",
            "  </基礎情報>\n",
            "  <図面情報>\n",
            "    <図面番号>01</図面番号>\n",
            "    <図面名称>一般図</図面名称>\n",
            "    <図面ファイル名>01一般図.SFC</図面ファイル名>\n",
            "  </図面情報>\n",
            "  <図面情報>\n",
            "    <図面番号>02</図面番号>\n",
            "    <図面名称>構造図</図面名称>\n",
            "    <図面ファイル名>02構造図.SFC</図面ファイル名>\n",
            "  </図面情報>\n",
            "</INDEX_D>\n",
        );
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode(xml);
        assert!(is_cals_xml(&sjis));
        let md = to_markdown(&sjis).unwrap();
        assert!(md.contains("橋梁詳細設計業務"), "{md}");
        assert!(md.contains("01一般図.SFC"), "{md}");
        assert!(md.contains("図面情報 (2件)"), "{md}");
    }
}
