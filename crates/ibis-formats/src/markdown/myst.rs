// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! MyST constructs that fit the document model without project resolution.

pub(super) fn target(line: &str) -> Option<&str> {
    let label = line.strip_prefix('(')?.strip_suffix(")=")?;
    super::is_word_bookmark(label).then_some(label)
}

pub(super) enum Directive<'a> {
    Alert(&'static str, &'a [&'a str]),
    Figure(Figure<'a>),
    Equation { label: &'a str, latex: String },
}

pub(super) fn colon_directive<'a>(lines: &'a [&'a str]) -> Option<(Directive<'a>, usize)> {
    let first = lines.first()?.trim();
    let length = first.bytes().take_while(|byte| *byte == b':').count();
    if length < 3 {
        return None;
    }
    let info = first[length..].trim();
    let alert = alert_kind(info);
    if alert.is_none() && !info.starts_with("{figure} ") && info != "{math}" {
        return None;
    }
    for (index, raw) in lines.iter().enumerate().skip(1) {
        let line = raw.trim();
        if line.len() >= length && line.bytes().all(|byte| byte == b':') {
            let body = &lines[1..index];
            let directive = match alert {
                Some(alert) if !body.is_empty() => Directive::Alert(alert, body),
                Some(_) => return None,
                None if info == "{math}" => {
                    let (label, latex) = equation(info, body)?;
                    Directive::Equation { label, latex }
                }
                None => Directive::Figure(figure(info, body)?),
            };
            return Some((directive, index + 1));
        }
        if line.starts_with(":::") {
            break;
        }
    }
    None
}

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

pub(super) fn alert_kind(info: &str) -> Option<&'static str> {
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

pub(super) fn equation<'a>(info: &str, lines: &[&'a str]) -> Option<(&'a str, String)> {
    if info != "{math}" {
        return None;
    }
    let (option, body) = lines.split_first()?;
    let label = option.trim().strip_prefix(":label:")?.trim();
    let latex = body.join("\n");
    (super::is_word_bookmark(label) && !latex.trim().is_empty()).then_some((label, latex))
}

pub(super) fn reference(source: &str) -> Option<(usize, &str, &str)> {
    let (payload, _) = source.strip_prefix("{ref}`")?.split_once('`')?;
    let (title, label) = payload.strip_suffix('>')?.rsplit_once(" <")?;
    (!title.trim().is_empty() && !title.contains(['<', '>', '\r', '\n'])
        && super::is_word_bookmark(label))
        .then_some(("{ref}`".len() + payload.len() + 1, title, label))
}

// Paragraph normalization can merge adjacent runs that await the same target.
pub(super) fn reference_title(mut source: &str, target: &str) -> Option<String> {
    let mut title = String::new();
    while !source.is_empty() {
        let (length, text, label) = reference(source)?;
        if label != target {
            return None;
        }
        title.push_str(text);
        source = &source[length..];
    }
    Some(title)
}
