// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Marp syntax that can be represented by editable PowerPoint objects.

use ibis_doc_model::EmuSize;

pub(super) fn enabled(metadata: &str) -> bool {
    metadata
        .lines()
        .filter_map(setting)
        .any(|(key, value)| key.eq_ignore_ascii_case("marp") && value.eq_ignore_ascii_case("true"))
}

pub(super) fn size(metadata: &str) -> Option<EmuSize> {
    // 96 CSS pixels per inch, 914400 EMU per inch.
    let (_, preset) = metadata
        .lines()
        .filter_map(setting)
        .find(|(key, _)| *key == "size")?;
    let width = match preset {
        "16:9" => 1280,
        "4:3" => 960,
        _ => return None,
    };
    Some(EmuSize::new(width * 9525, 720 * 9525))
}

pub(super) fn comment_size(body: &str) -> Option<EmuSize> {
    let mut lines = body.lines();
    let mut fence = None;
    let mut selected = None;
    while let Some(line) = lines.next() {
        if !super::is_fenced_line(line.trim(), &mut fence)
            && let Some(comment) = block_comment(line, lines.clone())
        {
            if !comment.text.trim_start().starts_with("notes:") {
                selected = size(&comment.text).or(selected);
            }
            lines.by_ref().take(comment.following_lines).for_each(drop);
        }
    }
    selected
}

pub(super) fn size_preset(size: EmuSize) -> Option<&'static str> {
    let (width, height) = (i128::from(size.width), i128::from(size.height));
    if width * 9 == height * 16 {
        Some("16:9")
    } else if width * 3 == height * 4 {
        Some("4:3")
    } else {
        None
    }
}

fn setting(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    Some((key.trim(), value.trim().trim_matches(['"', '\''])))
}

pub(super) struct BlockComment {
    pub following_lines: usize,
    text: String,
}

pub(super) fn block_comment<'a>(
    line: &'a str,
    following: std::str::Lines<'a>,
) -> Option<BlockComment> {
    let first = line.trim_start().strip_prefix("<!--")?;
    let mut text = String::new();
    for (following_lines, line) in std::iter::once(first).chain(following).enumerate() {
        if let Some((end, suffix)) = line.split_once("-->") {
            if !suffix.trim().is_empty() {
                return None;
            }
            text.push_str(end);
            return Some(BlockComment {
                following_lines,
                text,
            });
        }
        text.push_str(line);
        text.push('\n');
    }
    None
}

fn is_directive(text: &str) -> bool {
    text.lines().filter_map(setting).any(|(key, _)| {
        matches!(
            key.trim_start_matches('_'),
            "marp"
                | "theme"
                | "size"
                | "headingDivider"
                | "math"
                | "paginate"
                | "header"
                | "footer"
                | "class"
                | "color"
                | "backgroundColor"
                | "backgroundImage"
                | "backgroundPosition"
                | "backgroundRepeat"
                | "backgroundSize"
                | "style"
                | "title"
                | "description"
                | "author"
                | "keywords"
                | "url"
                | "image"
                | "lang"
                | "transition"
        )
    })
}

pub(super) fn write_notes(notes: &str, out: &mut String) {
    let notes = notes.trim();
    if notes.is_empty() {
        return;
    }
    out.push_str("<!-- ");
    if notes.starts_with("pptx-") || notes.starts_with("notes:") || is_directive(notes) {
        out.push_str("notes: ");
    }
    out.push_str(notes);
    out.push_str(" -->\n\n");
}

pub(super) fn extract_notes(chunk: &str, is_marp: bool) -> (String, Option<String>) {
    let mut body = Vec::new();
    let mut notes = Vec::new();
    let mut fence = None;
    let mut lines = chunk.lines();
    while let Some(line) = lines.next() {
        if !super::is_fenced_line(line.trim(), &mut fence)
            && let Some(comment) = block_comment(line, lines.clone())
        {
            let text = comment.text.trim();
            let directive = is_marp && is_directive(text);
            let note = text
                .strip_prefix("notes:")
                .or_else(|| (is_marp && !text.starts_with("pptx-") && !directive).then_some(text));
            if let Some(note) = note {
                let note = note.trim();
                if !note.is_empty() {
                    notes.push(note.to_owned());
                }
            }
            if note.is_some() || directive {
                lines.by_ref().take(comment.following_lines).for_each(drop);
                continue;
            }
            body.push(line);
            body.extend(lines.by_ref().take(comment.following_lines));
        } else {
            body.push(line);
        }
    }
    (
        body.join("\n"),
        (!notes.is_empty()).then(|| notes.join("\n")),
    )
}
