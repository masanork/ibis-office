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
