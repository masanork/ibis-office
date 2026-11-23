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
    if !super::is_word_bookmark(anchor)
        || alias.is_empty()
        || alias.contains(['[', ']', '|', '\n', '\r'])
    {
        return None;
    }
    Some((contents.len() + 5, anchor, alias))
}
