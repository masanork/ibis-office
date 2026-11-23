// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Japanese provision identifiers and heading syntax.

use super::model::DivisionKind;

pub(super) fn split_title_and_parens(s: &str) -> Option<(String, String)> {
    let inner = s.strip_suffix('）')?;
    let mut nested = 0;
    for (index, ch) in inner.char_indices().rev() {
        match ch {
            '）' => nested += 1,
            '（' if nested == 0 => {
                return Some((inner[..index].trim().to_string(),
                    inner[index + '（'.len_utf8()..].trim().to_string()));
            }
            '（' => nested -= 1,
            _ => {}
        }
    }
    None
}

pub(super) fn is_caption_line(s: &str) -> bool {
    s.starts_with('（') && s.ends_with('）')
}

pub(super) fn split_article_title_and_caption(s: &str) -> Option<(String, Option<String>)> {
    if let Some((art, _, rest)) = parse_article_header(s) {
        if rest.is_empty() {
            return Some((art, None));
        }
        if is_caption_line(rest) {
            return Some((art, Some(rest.to_string())));
        }
    }
    None
}

pub(super) fn parse_suppl_header(s: &str) -> Option<String> {
    let suffix = s.strip_prefix('附')?
        .trim_start_matches([' ', '　'])
        .strip_prefix('則')?;
    (suffix.is_empty() || suffix.starts_with('（')).then(|| s.to_string())
}

pub(super) fn supplementary_item_number(title: &str, sentence: &str) -> Option<String> {
    if sentence == "略" {
        let range = title
            .strip_suffix("まで")
            .and_then(|title| title.split_once("から"))
            .or_else(|| title.split_once("及び"));
        if let Some((first, last)) = range {
            return Some(format!(
                "{}:{}",
                kanji_num_to_ascii(first)?,
                kanji_num_to_ascii(last)?
            ));
        }
    }
    kanji_num_to_ascii(title)
}

pub(super) fn parse_division_header(s: &str) -> Option<(DivisionKind, String)> {
    let rest = s.strip_prefix('第')?;
    let (num_part, suffix) = rest.split_once(|c| matches!(c, '編' | '章' | '節' | '款' | '目'))?;
    let marker = rest.chars().nth(num_part.chars().count())?;
    let kind = match marker {
        '編' => DivisionKind::Part,
        '章' => DivisionKind::Chapter,
        '節' => DivisionKind::Section,
        '款' => DivisionKind::Subsection,
        '目' => DivisionKind::Division,
        _ => return None,
    };
    let num = kanji_num_to_ascii(num_part)?;
    let _ = suffix;
    Some((kind, num))
}

pub(super) fn parse_article_header(s: &str) -> Option<(String, String, &str)> {
    let rest = s.strip_prefix('第')?;
    let jo_idx = rest.find('条')?;
    let main_kanji = &rest[..jo_idx];
    let main_num = kanji_num_to_ascii(main_kanji)?;
    let mut after_jo = &rest[jo_idx + '条'.len_utf8()..];
    let mut num = main_num;
    let mut title_end = '第'.len_utf8() + jo_idx + '条'.len_utf8();
    while let Some(branch_rest) = after_jo.strip_prefix('の') {
        let len = branch_rest
            .chars()
            .take_while(|c| is_kanji_digit(*c) || fullwidth_or_ascii_digit(*c).is_some())
            .map(char::len_utf8)
            .sum::<usize>();
        if len == 0 {
            break;
        }
        let branch_num = kanji_num_to_ascii(&branch_rest[..len])?;
        num.push('_');
        num.push_str(&branch_num);
        title_end += 'の'.len_utf8() + len;
        after_jo = &branch_rest[len..];
    }
    if !after_jo.is_empty() && !after_jo.starts_with(|c: char| c.is_whitespace() || c == '（') {
        return None;
    }
    Some((s[..title_end].to_string(), num, after_jo.trim_start()))
}

pub(super) fn parse_numbered_paragraph(s: &str) -> Option<(String, &str)> {
    let mut chars = s.char_indices();
    let mut digits = String::new();
    let mut end = 0usize;
    for (idx, ch) in &mut chars {
        if let Some(d) = fullwidth_or_ascii_digit(ch) {
            digits.push(d);
            end = idx + ch.len_utf8();
        } else if ch == ' ' || ch == '\u{3000}' {
            if !digits.is_empty() {
                return Some((digits, s[idx + ch.len_utf8()..].trim_start()));
            }
            return None;
        } else {
            return None;
        }
    }
    if digits.is_empty() {
        None
    } else {
        Some((digits, s[end..].trim_start()))
    }
}

pub(super) fn parse_md_ordered_paragraph(s: &str) -> Option<(String, &str)> {
    let (num, rest) = s.split_once(". ").or_else(|| s.strip_suffix('.').map(|num| (num, "")))?;
    if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
        Some((num.to_string(), rest.trim_start()))
    } else {
        None
    }
}

pub(super) fn split_leading_token(s: &str) -> Option<(&str, &str)> {
    s.split_once(|c: char| c == ' ' || c == '\u{3000}')
        .map(|(a, b)| (a.trim(), b.trim_start()))
        .filter(|(a, _)| !a.is_empty())
}

fn fullwidth_or_ascii_digit(c: char) -> Option<char> {
    match c {
        '0'..='9' => Some(c),
        '０'..='９' => char::from_u32(c as u32 - '０' as u32 + '0' as u32),
        _ => None,
    }
}

pub(super) fn to_fullwidth_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '0'..='9' => char::from_u32(c as u32 - '0' as u32 + '０' as u32).unwrap_or(c),
            _ => c,
        })
        .collect()
}

pub(super) fn is_kanji_digit(c: char) -> bool {
    matches!(
        c,
        '〇' | '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十' | '百' | '千'
    )
}

pub(super) fn kanji_num_to_ascii(s: &str) -> Option<String> {
    if s.is_empty() {
        return None;
    }
    let normalized: String = s
        .chars()
        .filter_map(fullwidth_or_ascii_digit)
        .collect();
    if normalized.len() == s.chars().count() {
        return Some(normalized);
    }
    let mut total = 0u32;
    let mut current = 0u32;
    for ch in s.chars() {
        match ch {
            '〇' => current = 0,
            '一' => current = 1,
            '二' => current = 2,
            '三' => current = 3,
            '四' => current = 4,
            '五' => current = 5,
            '六' => current = 6,
            '七' => current = 7,
            '八' => current = 8,
            '九' => current = 9,
            '十' => {
                total += if current == 0 { 1 } else { current } * 10;
                current = 0;
            }
            '百' => {
                total += if current == 0 { 1 } else { current } * 100;
                current = 0;
            }
            '千' => {
                total += if current == 0 { 1 } else { current } * 1000;
                current = 0;
            }
            _ => return None,
        }
    }
    Some((total + current).to_string())
}
