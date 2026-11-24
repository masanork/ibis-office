// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Obsidian constructs that fit a single manuscript.

pub(super) fn callout(line: &str) -> Option<(&str, &str, Option<&str>)> {
    let content = line.trim_start_matches(' ');
    if line.len() - content.len() > 3 {
        return None;
    }
    let (kind, rest) = content.strip_prefix("[!")?.split_once(']')?;
    if !kind.starts_with(|ch: char| ch.is_ascii_alphabetic())
        || !kind.bytes().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_'))
    {
        return None;
    }
    let (fold, title) = if rest.starts_with(['+', '-']) {
        rest.split_at(1)
    } else {
        ("", rest)
    };
    if !title.is_empty() && !title.starts_with([' ', '\t']) {
        return None;
    }
    let title = title.trim();
    Some((kind, fold, (!title.is_empty()).then_some(title)))
}

pub(super) fn canonical_callout_kind(kind: &str) -> Option<&'static str> {
    match kind.to_ascii_lowercase().as_str() {
        "note" => Some("NOTE"),
        "tip" | "hint" => Some("TIP"),
        "important" => Some("IMPORTANT"),
        "warning" | "attention" => Some("WARNING"),
        "caution" => Some("CAUTION"),
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

pub(super) fn image_embed(input: &str) -> Option<(usize, &str, Option<i64>, Option<i64>)> {
    fn pixels(value: &str) -> Option<i64> {
        let px: u32 = value.parse().ok()?;
        (px > 0).then(|| i64::from(px) * 9_525)
    }
    let (contents, _) = input.strip_prefix("![[")?.split_once("]]")?;
    let (path, width, height) = match contents.split_once('|') {
        Some((path, size)) => {
            let (width, height) = if let Some((w, h)) = size.split_once('x') {
                (pixels(w)?, Some(pixels(h)?))
            } else {
                (pixels(size)?, None)
            };
            (path, Some(width), height)
        }
        None => (contents, None, None),
    };
    let path = path.trim();
    if path.is_empty() || path.contains(['[', ']', ':', '#', '?', '\\', '\r', '\n']) {
        return None;
    }
    let extension = std::path::Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff"
    )
    .then_some((contents.len() + 5, path, width, height))
}
