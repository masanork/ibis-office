// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Text helpers: labels, wrapping, width estimates and XML escaping.


pub(super) fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\''))) {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

pub(super) fn push_centered_label(svg: &mut String, cx: f32, cy: f32, text: &str, font_size: f32, max_width: f32, fill: &str, weight: &str) {
    let lines = label_lines(text, max_width, font_size);
    let line_h = font_size * 1.25;
    let block = lines.len() as f32 * line_h;
    let mut y = cy - block / 2.0 + font_size * 0.8;
    for line in &lines {
        svg.push_str(&format!(
            r##"<text x="{cx:.1}" y="{y:.1}" text-anchor="middle" font-size="{font_size}" font-weight="{weight}" fill="{fill}">{}</text>"##,
            escape_xml(line)
        ));
        y += line_h;
    }
}

/// `<br>` becomes a line break. A line wider than `max_width` wraps, using
/// [`estimate_text_width`] so Japanese characters take a full em.
pub(super) fn label_lines(text: &str, max_width: f32, font_size: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for piece in break_label(text).split('\n') {
        if piece.is_empty() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            continue;
        }
        lines.extend(wrap_piece(piece, max_width.max(8.0), font_size));
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub(super) fn break_label(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let raw = text.as_bytes();
    let folded = lower.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut idx = 0usize;
    while idx < raw.len() {
        if folded[idx..].starts_with(b"<br")
            && let Some(rel) = folded[idx + 3..].iter().position(|ch| *ch == b'>')
        {
            let inside = &folded[idx + 3..idx + 3 + rel];
            if inside.iter().all(|ch| ch.is_ascii_whitespace() || *ch == b'/') {
                out.push('\n');
                idx += 3 + rel + 1;
                continue;
            }
        }
        let ch = text[idx..].chars().next().unwrap_or('\u{FFFD}');
        out.push(ch);
        idx += ch.len_utf8();
    }
    out
}

pub(super) fn wrap_piece(text: &str, max_width: f32, font_size: f32) -> Vec<String> {
    if estimate_text_width(text, font_size) <= max_width {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut width = 0.0f32;
    for token in split_wrap_tokens(text) {
        let token_width = estimate_text_width(&token, font_size);
        if current.is_empty() || width + token_width <= max_width {
            if token_width <= max_width || current.is_empty() && !token.chars().any(|ch| !ch.is_ascii() || ch.is_whitespace()) {
                if token_width <= max_width {
                    current.push_str(&token);
                    width += token_width;
                    continue;
                }
            }
        } else {
            lines.push(std::mem::take(&mut current));
            width = 0.0;
        }
        if estimate_text_width(&token, font_size) <= max_width {
            current = token;
            width = token_width;
            continue;
        }
        for ch in token.chars() {
            let ch_width = estimate_text_width(&ch.to_string(), font_size);
            if !current.is_empty() && width + ch_width > max_width {
                lines.push(std::mem::take(&mut current));
                width = 0.0;
            }
            current.push(ch);
            width += ch_width;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() { vec![String::new()] } else { lines }
}

pub(super) fn split_wrap_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut ascii_word = false;
    for ch in text.chars() {
        let word = ch.is_ascii() && !ch.is_ascii_whitespace();
        if current.is_empty() {
            current.push(ch);
            ascii_word = word;
            continue;
        }
        if ch.is_whitespace() || (ascii_word && word) {
            current.push(ch);
            continue;
        }
        tokens.push(std::mem::take(&mut current));
        current.push(ch);
        ascii_word = word;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub(super) fn estimate_text_width(s: &str, font_size: f32) -> f32 {
    let mut units = 0.0f32;
    for ch in s.chars() {
        if ch.is_ascii() {
            units += 0.56;
        } else {
            units += 1.0;
        }
    }
    units * font_size
}

pub(super) fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}
