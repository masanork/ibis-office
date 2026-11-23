// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! MyST constructs that fit the document model without project resolution.

pub(super) fn target(line: &str) -> Option<&str> {
    let label = line.strip_prefix('(')?.strip_suffix(")=")?;
    super::is_word_bookmark(label).then_some(label)
}

pub(super) fn admonition(info: &str, lines: &[&str]) -> Option<&'static str> {
    // The shared alert model accepts one paragraph of inline Markdown.
    if lines
        .iter()
        .any(|line| line.trim().is_empty() || line.trim_start().starts_with(':'))
        || lines.len() != 1
    {
        return None;
    }
    let line = lines[0].trim();
    if line.starts_with(['>', '#', '|'])
        || super::parse_list_marker(lines[0]).is_some()
        || super::opening_fence(line).is_some()
        || super::is_horizontal_rule(line)
    {
        return None;
    }
    match info {
        "{note}" => Some("[!NOTE]"),
        "{tip}" => Some("[!TIP]"),
        "{important}" => Some("[!IMPORTANT]"),
        "{warning}" => Some("[!WARNING]"),
        "{caution}" => Some("[!CAUTION]"),
        _ => None,
    }
}
