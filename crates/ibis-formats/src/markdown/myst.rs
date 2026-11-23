// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! MyST constructs that fit the document model without project resolution.

pub(super) fn target(line: &str) -> Option<&str> {
    let label = line.strip_prefix('(')?.strip_suffix(")=")?;
    super::is_word_bookmark(label).then_some(label)
}

/// A colon-fenced alert that the existing paragraph model can represent.
pub(super) fn colon_admonition<'a>(
    lines: &'a [&'a str],
) -> Option<(&'static str, &'a [&'a str], usize)> {
    let first = lines.first()?.trim();
    let length = first.bytes().take_while(|byte| *byte == b':').count();
    if length < 3 {
        return None;
    }
    let alert = alert_kind(first[length..].trim())?;
    for (index, raw) in lines.iter().enumerate().skip(1) {
        let line = raw.trim();
        if line.len() >= length && line.bytes().all(|byte| byte == b':') {
            return (index > 1).then_some((alert, &lines[1..index], index + 1));
        }
        if !simple_body_line(raw) {
            break;
        }
    }
    None
}

pub(super) fn admonition(info: &str, lines: &[&str]) -> Option<&'static str> {
    alert_kind(info)
        .filter(|_| !lines.is_empty() && lines.iter().all(|line| simple_body_line(line)))
}

// The shared alert model accepts a single paragraph of inline Markdown.
fn simple_body_line(raw: &str) -> bool {
    let line = raw.trim();
    !line.is_empty()
        && !line.starts_with([':', '>', '#', '|'])
        && super::parse_list_marker(raw).is_none()
        && super::opening_fence(line).is_none()
        && !super::is_horizontal_rule(line)
        && !raw.ends_with("  ")
        && !raw.ends_with('\\')
}

fn alert_kind(info: &str) -> Option<&'static str> {
    match info {
        "{note}" => Some("[!NOTE]"),
        "{tip}" => Some("[!TIP]"),
        "{important}" => Some("[!IMPORTANT]"),
        "{warning}" => Some("[!WARNING]"),
        "{caution}" => Some("[!CAUTION]"),
        _ => None,
    }
}

pub(super) struct Figure<'a> {
    pub image: String,
    pub caption: String,
    pub label: Option<&'a str>,
    pub align: Option<ibis_doc_model::flow::FlowAlign>,
}

/// The subset that can be represented by an inline picture and caption paragraph.
pub(super) fn figure<'a>(info: &str, lines: &[&'a str]) -> Option<Figure<'a>> {
    let path = info.strip_prefix("{figure} ")?.trim();
    if path.is_empty() || path.contains(char::is_whitespace) || path.contains(['(', ')']) {
        return None;
    }
    let mut label = None;
    let mut alt = "";
    let mut align = None;
    let mut sizes = Vec::new();
    let mut options = lines.iter().peekable();
    while let Some(line) = options.next_if(|line| line.trim_start().starts_with(':')) {
        let (key, value) = line.trim_start().strip_prefix(':')?.split_once(':')?;
        let value = value.trim();
        match key {
            "label" | "name" if label.is_none() && super::is_word_bookmark(value) => {
                label = Some(value)
            }
            "alt" if alt.is_empty() => alt = value,
            "align" if align.is_none() => {
                use ibis_doc_model::flow::FlowAlign;
                align = Some(match value {
                    "left" => FlowAlign::Left,
                    "center" => FlowAlign::Center,
                    "right" => FlowAlign::Right,
                    _ => return None,
                });
            }
            "width" | "height"
                if !value.ends_with('%')
                    && !value.contains(char::is_whitespace)
                    && super::parse_length_emu(value).is_some_and(|size| size > 0) =>
            {
                sizes.push(format!("{key}={value}"));
            }
            _ => return None,
        }
    }
    let caption: Vec<_> = options
        .copied()
        .skip_while(|line| line.trim().is_empty())
        .collect();
    if !caption.iter().all(|line| simple_body_line(line)) {
        return None;
    }
    let mut image = format!("![{}]({path})", super::encode_picture_label(alt));
    if !sizes.is_empty() {
        image.push_str(&format!("{{{}}}", sizes.join(" ")));
    }
    Some(Figure {
        image,
        caption: caption
            .iter()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join(" "),
        label,
        align,
    })
}
