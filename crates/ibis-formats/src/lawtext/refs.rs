// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Statutory reference extraction and 参照条文 (referenced provisions) Markdown
//! generator for [`super::LawDoc`].

use serde_json::{Value, json};

use super::diff::{ArticleChangeKind, diff_laws};
use super::{LawArticle, LawDoc, LawNode, LawSubitem, is_kanji_digit, kanji_num_to_ascii};

/// A statutory reference found in a [`LawDoc`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LawReference {
    /// Source article title where the reference appeared (e.g. `"第一条"`).
    pub source_article: String,
    /// Source paragraph number (`"1"`, `"2"`, ...).
    pub source_paragraph: String,
    /// Raw citation text (e.g. `"行政手続法（平成五年法律第八十八号）第三条"`, `"前条"`, `"第二条"`).
    pub raw_text: String,
    /// External law name if specified (e.g. `"行政手続法"`).
    pub external_law: Option<String>,
    /// External law number if specified (e.g. `"平成五年法律第八十八号"`).
    pub external_law_num: Option<String>,
    /// Target article number in ASCII (e.g. `"3"`, `"5_2"`), if resolved.
    pub target_article_num: Option<String>,
    /// Target paragraph number in ASCII (e.g. `"2"`), if specified or resolved.
    pub target_paragraph_num: Option<String>,
    /// True when the citation is relative (`前条`, `次条`, `前項`, `次項`, `同条`, `同項`).
    pub is_relative: bool,
}

impl LawReference {
    /// Serialize to JSON for AI Assistant or e-Gov lookup pipelines.
    pub fn to_json_value(&self) -> Value {
        json!({
            "source_article": self.source_article,
            "source_paragraph": self.source_paragraph,
            "raw_text": self.raw_text,
            "external_law": self.external_law,
            "external_law_num": self.external_law_num,
            "target_article_num": self.target_article_num,
            "target_paragraph_num": self.target_paragraph_num,
            "is_relative": self.is_relative,
        })
    }
}

/// Extract all statutory references from `doc`.
pub fn extract_references(doc: &LawDoc) -> Vec<LawReference> {
    let articles = collect_articles(&doc.main_provision);
    let mut refs = Vec::new();
    for (art_idx, art) in articles.iter().enumerate() {
        for (p_idx, para) in art.paragraphs.iter().enumerate() {
            let mut texts = vec![para.sentence.as_str()];
            for it in &para.items {
                texts.push(it.sentence.as_str());
                collect_subitem_texts(&it.subitems, &mut texts);
            }
            for text in texts {
                scan_text_references(
                    text,
                    art,
                    art_idx,
                    &articles,
                    &para.num,
                    p_idx,
                    &mut refs,
                );
            }
        }
    }
    refs
}

/// Generate a 参照条文 Markdown document for the amended/added articles between `old` and `new`.
pub fn generate_sansho_markdown(old: &LawDoc, new: &LawDoc) -> String {
    let ir = diff_laws(old, new);
    let all_refs = extract_references(new);
    let new_articles = collect_articles(&new.main_provision);
    let old_articles = collect_articles(&old.main_provision);

    let changed_titles: Vec<String> = ir
        .article_changes
        .iter()
        .filter(|ac| ac.kind != ArticleChangeKind::Deleted)
        .filter_map(|ac| ac.new_title.clone())
        .collect();

    let mut out = String::new();
    let title = if !ir.law_title.is_empty() {
        &ir.law_title
    } else {
        "対象法令"
    };
    out.push_str(&format!("# {} 参照条文\n\n", title));

    let relevant: Vec<&LawReference> = all_refs
        .iter()
        .filter(|r| changed_titles.is_empty() || changed_titles.contains(&r.source_article))
        .collect();

    if relevant.is_empty() {
        out.push_str("改正対象条文に引用されている参照条文は検出されませんでした。\n");
        return out;
    }

    let mut seen_keys = Vec::new();
    for r in relevant {
        let key = (
            r.external_law.clone(),
            r.target_article_num.clone(),
            r.raw_text.clone(),
        );
        if seen_keys.contains(&key) {
            continue;
        }
        seen_keys.push(key);

        if let Some(ext_law) = &r.external_law {
            let num_part = r
                .external_law_num
                .as_deref()
                .map(|n| format!("（{n}）"))
                .unwrap_or_default();
            out.push_str(&format!(
                "## {ext_law}{num_part}（参照元: {}）\n\n",
                r.source_article
            ));
            out.push_str(&format!("> 引用表記: `{}`（外部法令・要条文取得）\n\n", r.raw_text));
        } else if let Some(target_num) = &r.target_article_num {
            if let Some(target_art) = new_articles
                .iter()
                .chain(old_articles.iter())
                .find(|a| a.num == *target_num)
            {
                let cap = target_art
                    .caption
                    .as_deref()
                    .map(|c| format!(" {c}"))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "## {}{cap}（参照元: {} / `{}`）\n\n",
                    target_art.title, r.source_article, r.raw_text
                ));
                for (i, p) in target_art.paragraphs.iter().enumerate() {
                    if i == 0 {
                        out.push_str(&format!("{} {}\n\n", target_art.title, p.sentence));
                    } else {
                        out.push_str(&format!("{} {}\n\n", super::to_fullwidth_digits(&p.num), p.sentence));
                    }
                    for it in &p.items {
                        out.push_str(&format!("- {} {}\n", it.title, it.sentence));
                    }
                    if !p.items.is_empty() {
                        out.push('\n');
                    }
                }
            } else {
                out.push_str(&format!(
                    "## {}（参照元: {}）\n\n> 条文テキストが見つかりません。\n\n",
                    r.raw_text, r.source_article
                ));
            }
        } else {
            out.push_str(&format!(
                "## {}（参照元: {}）\n\n> 相対参照（要文脈確認）\n\n",
                r.raw_text, r.source_article
            ));
        }
    }

    out
}

fn collect_articles<'a>(nodes: &'a [LawNode]) -> Vec<&'a LawArticle> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            LawNode::Division { children, .. } => out.extend(collect_articles(children)),
            LawNode::Article(a) => out.push(a),
            LawNode::Paragraph(_) => {}
        }
    }
    out
}

fn collect_subitem_texts<'a>(subs: &'a [LawSubitem], out: &mut Vec<&'a str>) {
    for s in subs {
        out.push(s.sentence.as_str());
        collect_subitem_texts(&s.children, out);
    }
}

fn scan_text_references(
    text: &str,
    art: &LawArticle,
    art_idx: usize,
    articles: &[&LawArticle],
    para_num: &str,
    para_idx: usize,
    refs: &mut Vec<LawReference>,
) {
    let mut idx = 0usize;
    let mut last_ext_law: Option<(String, Option<String>)> = None;

    while idx < text.len() {
        let rest = &text[idx..];

        // 1. Check relative citations: 前条, 次条, 前項, 次項, 同条, 同項
        if let Some(rel) = ["前条", "次条", "前項", "次項", "同条", "同項"]
            .into_iter()
            .find(|r| rest.starts_with(r))
        {
            let (target_art, target_para) = match rel {
                "前条" => (
                    art_idx.checked_sub(1).and_then(|i| articles.get(i)).map(|a| a.num.clone()),
                    None,
                ),
                "次条" => (articles.get(art_idx + 1).map(|a| a.num.clone()), None),
                "同条" => (Some(art.num.clone()), None),
                "前項" => (
                    Some(art.num.clone()),
                    para_idx.checked_sub(1).and_then(|i| art.paragraphs.get(i)).map(|p| p.num.clone()),
                ),
                "次項" => (
                    Some(art.num.clone()),
                    art.paragraphs.get(para_idx + 1).map(|p| p.num.clone()),
                ),
                "同項" => (Some(art.num.clone()), Some(para_num.to_string())),
                _ => (None, None),
            };
            refs.push(LawReference {
                source_article: art.title.clone(),
                source_paragraph: para_num.to_string(),
                raw_text: rel.to_string(),
                external_law: None,
                external_law_num: None,
                target_article_num: target_art,
                target_paragraph_num: target_para,
                is_relative: true,
            });
            idx += rel.len();
            continue;
        }

        // 2. Check external law citation: <Name>（<EraYearLawNum>）
        if let Some((law_name, law_num, consumed)) = try_parse_external_law_citation(rest) {
            last_ext_law = Some((law_name.clone(), Some(law_num.clone())));
            // Look ahead for optional 第○条...
            let after = &rest[consumed..];
            if let Some((art_num, para_opt, art_consumed)) = try_parse_article_ref(after) {
                let raw = rest[..consumed + art_consumed].to_string();
                refs.push(LawReference {
                    source_article: art.title.clone(),
                    source_paragraph: para_num.to_string(),
                    raw_text: raw,
                    external_law: Some(law_name),
                    external_law_num: Some(law_num),
                    target_article_num: Some(art_num),
                    target_paragraph_num: para_opt,
                    is_relative: false,
                });
                idx += consumed + art_consumed;
                continue;
            }
            refs.push(LawReference {
                source_article: art.title.clone(),
                source_paragraph: para_num.to_string(),
                raw_text: rest[..consumed].to_string(),
                external_law: Some(law_name),
                external_law_num: Some(law_num),
                target_article_num: None,
                target_paragraph_num: None,
                is_relative: false,
            });
            idx += consumed;
            continue;
        }

        // 3. Check internal article citation: 第○条(の○)(第○項)
        if rest.starts_with('第') {
            if let Some((art_num, para_opt, consumed)) = try_parse_article_ref(rest) {
                let raw = rest[..consumed].to_string();
                let (ext_law, ext_num) = if idx > 0 && text[..idx].ends_with("同法") {
                    last_ext_law.clone().map(|(l, n)| (Some(l), n)).unwrap_or((None, None))
                } else {
                    (None, None)
                };
                refs.push(LawReference {
                    source_article: art.title.clone(),
                    source_paragraph: para_num.to_string(),
                    raw_text: raw,
                    external_law: ext_law,
                    external_law_num: ext_num,
                    target_article_num: Some(art_num),
                    target_paragraph_num: para_opt,
                    is_relative: false,
                });
                idx += consumed;
                continue;
            }
        }

        let ch = rest.chars().next().unwrap();
        idx += ch.len_utf8();
    }
}

fn try_parse_external_law_citation(s: &str) -> Option<(String, String, usize)> {
    let open = s.find('（')?;
    if open == 0 || open > 80 {
        return None;
    }
    let name = &s[..open];
    if !(name.ends_with('法') || name.ends_with('令') || name.ends_with("規則")) {
        return None;
    }
    // Trim leading non-title particles/punctuation if any
    let mut clean_name = name
        .rsplit(|c: char| matches!(c, '、' | '。' | '「' | '」' | 'は' | 'を' | 'に' | 'の'))
        .next()
        .unwrap_or(name);
    for sep in ["及び", "又は", "並びに", "若しくは"] {
        if let Some((_, tail)) = clean_name.rsplit_once(sep) {
            clean_name = tail;
        }
    }
    if clean_name.is_empty() {
        return None;
    }
    let after_open = &s[open + '（'.len_utf8()..];
    let close = after_open.find('）')?;
    let inner = &after_open[..close];
    if !(inner.contains("法律") || inner.contains("政令") || inner.contains("省令") || inner.contains("規則")) {
        return None;
    }
    let consumed = open + '（'.len_utf8() + close + '）'.len_utf8();
    Some((clean_name.to_string(), inner.to_string(), consumed))
}

fn try_parse_article_ref(s: &str) -> Option<(String, Option<String>, usize)> {
    let after_dai = s.strip_prefix('第')?;
    let kanji_len: usize = after_dai
        .chars()
        .take_while(|c| is_kanji_digit(*c))
        .map(char::len_utf8)
        .sum();
    if kanji_len == 0 {
        return None;
    }
    let main_num = kanji_num_to_ascii(&after_dai[..kanji_len])?;
    let after_main = &after_dai[kanji_len..];
    let mut rest = after_main.strip_prefix('条')?;
    let mut consumed = '第'.len_utf8() + kanji_len + '条'.len_utf8();

    let mut full_num = main_num;
    if let Some(after_no) = rest.strip_prefix('の') {
        let sub_len: usize = after_no
            .chars()
            .take_while(|c| is_kanji_digit(*c))
            .map(char::len_utf8)
            .sum();
        if sub_len > 0 {
            if let Some(sub_num) = kanji_num_to_ascii(&after_no[..sub_len]) {
                full_num = format!("{full_num}_{sub_num}");
                consumed += 'の'.len_utf8() + sub_len;
                rest = &after_no[sub_len..];
            }
        }
    }

    let mut para_opt = None;
    if let Some(after_p_dai) = rest.strip_prefix('第') {
        let p_len: usize = after_p_dai
            .chars()
            .take_while(|c| is_kanji_digit(*c))
            .map(char::len_utf8)
            .sum();
        if p_len > 0 {
            if let Some(after_kou) = after_p_dai[p_len..].strip_prefix('項') {
                let _ = after_kou;
                if let Some(p_num) = kanji_num_to_ascii(&after_p_dai[..p_len]) {
                    para_opt = Some(p_num);
                    consumed += '第'.len_utf8() + p_len + '項'.len_utf8();
                }
            }
        }
    }

    Some((full_num, para_opt, consumed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lawtext::parse_lawtext;

    #[test]
    fn extracts_internal_external_and_relative_references() {
        let old_lt = "\
テスト法（令和七年法律第一号）
（目的）
第一条 この法律は、基本事項を定める。
（適用）
第二条 前条の規定は、行政手続法（平成五年法律第八十八号）第三条に規定する処分に適用する。";
        let new_lt = "\
テスト法（令和七年法律第一号）
（目的）
第一条 この法律は、基本事項及び応用事項を定める。
（適用）
第二条 前条の規定は、行政手続法（平成五年法律第八十八号）第三条及び第一条に規定する処分に適用する。";

        let old_doc = parse_lawtext(old_lt).unwrap();
        let new_doc = parse_lawtext(new_lt).unwrap();

        let refs = extract_references(&new_doc);
        assert!(refs.iter().any(|r| r.is_relative && r.raw_text == "前条" && r.target_article_num.as_deref() == Some("1")));
        assert!(refs.iter().any(|r| r.external_law.as_deref() == Some("行政手続法") && r.target_article_num.as_deref() == Some("3")));
        assert!(refs.iter().any(|r| r.external_law.is_none() && r.raw_text == "第一条" && r.target_article_num.as_deref() == Some("1")));

        let md = generate_sansho_markdown(&old_doc, &new_doc);
        assert!(md.contains("# テスト法 参照条文"));
        assert!(md.contains("## 行政手続法（平成五年法律第八十八号）"));
        assert!(md.contains("基本事項及び応用事項を定める。"));
    }
}
