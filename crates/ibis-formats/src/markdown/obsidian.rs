// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Obsidian constructs that fit a single manuscript.

pub(super) fn callout(line: &str) -> Option<&'static str> {
    let kind = line.strip_prefix("[!")?.strip_suffix(']')?;
    match kind.to_ascii_lowercase().as_str() {
        "note" => Some("[!NOTE]"),
        "tip" | "hint" => Some("[!TIP]"),
        "important" => Some("[!IMPORTANT]"),
        "warning" | "attention" => Some("[!WARNING]"),
        "caution" => Some("[!CAUTION]"),
        _ => None,
    }
}

pub(super) fn local_link(input: &str) -> Option<(usize, &str, &str)> {
    let (contents, _) = input.strip_prefix("[[#")?.split_once("]]")?;
    let (anchor, alias) = contents.split_once('|').unwrap_or((contents, contents));
    let anchor = anchor.trim();
    if anchor.is_empty()
        || anchor.contains(['[', ']', '#', '\n', '\r'])
        || anchor.starts_with('^')
        || alias.is_empty()
        || alias.contains(['[', ']', '|', '\n', '\r'])
    {
        return None;
    }
    Some((contents.len() + 5, anchor, alias))
}

/// File references remain workspace-relative until a folder resolves them.
pub(super) fn file_link(input: &str) -> Option<(usize, &str, &str)> {
    let (contents, _) = input.strip_prefix("[[")?.split_once("]]")?;
    let (target, alias) = contents.split_once('|').unwrap_or((contents, contents));
    let path = target.split('#').next()?;
    if path.trim().is_empty()
        || target.contains(['[', ']', ':', '\\', '\n', '\r'])
        || path
            .split('/')
            .any(|part| part.trim().is_empty() || part.starts_with('.'))
        || target.split_once('#').is_some_and(|(_, fragment)| {
            fragment.trim().is_empty() || fragment.starts_with('^') || fragment.contains('#')
        })
        || alias.trim().is_empty()
        || alias.contains(['[', ']', '|', '\n', '\r'])
    {
        return None;
    }
    Some((contents.len() + 4, target, alias))
}

pub(super) fn format_file_link(url: &str, label: &str) -> Option<String> {
    let (consumed, target, _) = file_link(url)?;
    if consumed != url.len() || label.contains(['[', ']', '|', '\n', '\r']) {
        return None;
    }
    if label == target {
        Some(format!("[[{target}]]"))
    } else {
        Some(format!("[[{target}|{label}]]"))
    }
}
