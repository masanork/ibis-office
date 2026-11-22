// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/235)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The plain text of a page, from the boxes of its characters ([`PageTextIndex`]).
//!
//! A PDF draws glyphs at positions; it does not say where a line ends or where a space is. The text is
//! rebuilt from the geometry: characters on one baseline make a line, read left to right, and a gap between two characters
//! makes a space. Characters the page sets one below another (縦書き, vertical writing) make columns, read top to
//! bottom, the columns right to left; digits set upright in a column (縦中横) are read inside it.
//!
//! Two layouts, after `pdftotext`: [`TextLayout::Reading`] gives each line its words with single spaces;
//! [`TextLayout::Physical`] places each character in the column of a fixed grid (a half-width character one column,
//! a full-width two), so that the columns of a table stay aligned and a parser can cut the lines at fixed positions.

use crate::scene::{PageTextIndex, estimated_advance_em};

/// How the characters of a line are spaced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextLayout {
    /// Words with single spaces between them, in reading order.
    #[default]
    Reading,
    /// Each character at the column its position on the page gives (like `pdftotext -layout`).
    Physical,
}

/// One drawn character, in page points (y grows downwards).
#[derive(Clone, Copy, Debug)]
struct Glyph {
    ch: char,
    x0: f32,
    x1: f32,
    top: f32,
    base: f32,
    op: usize,
    order: usize,
}

impl Glyph {
    fn height(&self) -> f32 {
        (self.base - self.top).max(1.0)
    }

    fn width(&self) -> f32 {
        let w = self.x1 - self.x0;
        if w > 0.0 { w } else { self.height() * estimated_advance_em(self.ch) }
    }

    fn center_x(&self) -> f32 {
        self.x0 + self.width() / 2.0
    }

    fn bottom(&self) -> f32 {
        self.top + self.height()
    }
}

/// Columns a character takes in the physical layout: a full-width character two, any other one.
fn columns_of(ch: char) -> usize {
    if estimated_advance_em(ch) >= 1.0 { 2 } else { 1 }
}

/// The text of a page. Lines end with `\n`; an empty page gives an empty string.
pub fn page_text(index: &PageTextIndex, layout: TextLayout) -> String {
    let glyphs: Vec<Glyph> = index
        .chars
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.ch.is_whitespace() && !c.ch.is_control())
        .map(|(order, c)| {
            let r = c.rect_pt;
            Glyph {
                ch: c.ch,
                x0: r.x0.min(r.x1),
                x1: r.x0.max(r.x1),
                top: r.y0.min(r.y1),
                base: r.y0.max(r.y1),
                op: c.stream_op_index,
                order,
            }
        })
        .collect();
    if glyphs.is_empty() {
        return String::new();
    }
    let flagged: Vec<bool> = index
        .chars
        .iter()
        .filter(|c| !c.ch.is_whitespace() && !c.ch.is_control())
        .map(|c| c.vertical)
        .collect();
    let vertical = vertical_runs(&glyphs, &flagged);

    let (mut upright, mut flat): (Vec<Glyph>, Vec<Glyph>) = (Vec::new(), Vec::new());
    for (g, v) in glyphs.iter().zip(&vertical) {
        if *v { upright.push(*g) } else { flat.push(*g) }
    }
    let mut segments = column_segments(upright);
    // Upright digits in a column (縦中横) belong to the column.
    flat.retain(|g| !absorb_into_column(&mut segments, *g));
    for s in &mut segments {
        s.glyphs.sort_by(|a, b| a.top.total_cmp(&b.top).then(a.x0.total_cmp(&b.x0)));
    }

    let vertical_text = vertical_text(&segments, layout);
    let horizontal_text = horizontal_text(flat, layout);
    let vertical_count: usize = segments.iter().map(|s| s.glyphs.len()).sum();
    let horizontal_count = glyphs.len() - vertical_count;
    let (first, second) = if vertical_count > horizontal_count {
        (vertical_text, horizontal_text)
    } else {
        (horizontal_text, vertical_text)
    };
    match (first.is_empty(), second.is_empty()) {
        (_, true) => first,
        (true, false) => second,
        (false, false) => format!("{first}\n{second}"),
    }
}

/// Which characters stand in a vertical run: three or more drawn one after another, each about one character below
/// the last in the same column. (A character the reader marked vertical counts too.)
fn vertical_runs(glyphs: &[Glyph], flagged: &[bool]) -> Vec<bool> {
    let n = glyphs.len();
    let mut steps = vec![false; n.saturating_sub(1)];
    for i in 0..n.saturating_sub(1) {
        let (a, b) = (&glyphs[i], &glyphs[i + 1]);
        let h = a.height().max(b.height());
        let dx = (a.center_x() - b.center_x()).abs();
        let dy = b.top - a.top;
        steps[i] = dx <= 0.35 * h && dy >= 0.6 * h && dy <= 1.8 * h;
    }
    let mut out: Vec<bool> = flagged.to_vec();
    out.resize(n, false);
    let mut i = 0;
    while i < steps.len() {
        if !steps[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < steps.len() && steps[i] {
            i += 1;
        }
        // `start..=i` are the characters of the run; it has `i - start` steps.
        if i - start >= 2 {
            for v in &mut out[start..=i] {
                *v = true;
            }
        }
    }
    out
}

/// The characters of one stretch of a column, top to bottom.
#[derive(Debug)]
struct Segment {
    center_x: f32,
    width: f32,
    top: f32,
    bottom: f32,
    glyphs: Vec<Glyph>,
}

/// Vertical characters gathered into columns, and the columns cut where a long gap divides them (the tiers of a page).
fn column_segments(mut upright: Vec<Glyph>) -> Vec<Segment> {
    upright.sort_by(|a, b| b.center_x().total_cmp(&a.center_x()).then(a.top.total_cmp(&b.top)));
    let mut columns: Vec<(f32, f32, Vec<Glyph>)> = Vec::new();
    for g in upright {
        let h = g.height();
        match columns.iter_mut().find(|(cx, ch, _)| (cx - g.center_x()).abs() <= 0.5 * ch.max(h)) {
            Some((_, _, glyphs)) => glyphs.push(g),
            None => columns.push((g.center_x(), h, vec![g])),
        }
    }
    let mut segments = Vec::new();
    for (center_x, h, mut glyphs) in columns {
        glyphs.sort_by(|a, b| a.top.total_cmp(&b.top));
        let mut current: Vec<Glyph> = Vec::new();
        for g in glyphs {
            if let Some(last) = current.last()
                && g.top - last.bottom() > 3.0 * h
            {
                segments.push(segment(center_x, h, std::mem::take(&mut current)));
            }
            current.push(g);
        }
        if !current.is_empty() {
            segments.push(segment(center_x, h, current));
        }
    }
    segments
}

fn segment(center_x: f32, width: f32, glyphs: Vec<Glyph>) -> Segment {
    let top = glyphs.iter().map(|g| g.top).fold(f32::INFINITY, f32::min);
    let bottom = glyphs.iter().map(Glyph::bottom).fold(f32::NEG_INFINITY, f32::max);
    Segment { center_x, width, top, bottom, glyphs }
}

/// Put `g` into the column stretch it stands inside, if any.
fn absorb_into_column(segments: &mut [Segment], g: Glyph) -> bool {
    let (cx, cy) = (g.center_x(), (g.top + g.base) / 2.0);
    for s in segments.iter_mut() {
        if (cx - s.center_x).abs() <= 0.6 * s.width && cy >= s.top - 0.5 * s.width && cy <= s.bottom + 0.5 * s.width {
            s.glyphs.push(g);
            return true;
        }
    }
    false
}

/// The columns as lines: tiers top to bottom, in each tier the columns right to left.
fn vertical_text(segments: &[Segment], layout: TextLayout) -> String {
    if segments.is_empty() {
        return String::new();
    }
    // Tiers: stretches whose vertical extents overlap.
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by(|&a, &b| segments[a].top.total_cmp(&segments[b].top));
    let mut tiers: Vec<(f32, f32, Vec<usize>)> = Vec::new();
    for i in order {
        let s = &segments[i];
        match tiers.iter_mut().find(|(top, bottom, _)| s.top < *bottom && s.bottom > *top) {
            Some((top, bottom, members)) => {
                *top = top.min(s.top);
                *bottom = bottom.max(s.bottom);
                members.push(i);
            }
            None => tiers.push((s.top, s.bottom, vec![i])),
        }
    }
    tiers.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = String::new();
    for (t, (_, _, members)) in tiers.iter_mut().enumerate() {
        if t > 0 {
            out.push('\n');
        }
        members.sort_by(|&a, &b| segments[b].center_x.total_cmp(&segments[a].center_x));
        for &i in members.iter() {
            let s = &segments[i];
            let mut prev: Option<&Glyph> = None;
            for g in &s.glyphs {
                if let Some(p) = prev {
                    if same_glyph(p, g) {
                        continue;
                    }
                    let gap = g.top - p.bottom();
                    let h = p.height().max(g.height());
                    match layout {
                        TextLayout::Reading if gap > 0.5 * h => out.push(' '),
                        TextLayout::Physical if gap > 0.5 * h => {
                            for _ in 0..((gap / h).round() as usize).max(1) {
                                out.push('\u{3000}');
                            }
                        }
                        _ => {}
                    }
                }
                out.push(g.ch);
                prev = Some(g);
            }
            out.push('\n');
        }
    }
    out
}

/// A character drawn twice over itself (overprinted for a bold look): the second drawing is not text.
fn same_glyph(a: &Glyph, b: &Glyph) -> bool {
    a.ch == b.ch && a.op != b.op && (a.x0 - b.x0).abs() < 0.2 * a.height() && (a.top - b.top).abs() < 0.2 * a.height()
}

/// The horizontal characters as lines, top to bottom.
fn horizontal_text(mut flat: Vec<Glyph>, layout: TextLayout) -> String {
    if flat.is_empty() {
        return String::new();
    }
    flat.sort_by(|a, b| a.base.total_cmp(&b.base).then(a.order.cmp(&b.order)));
    // Lines: characters whose baselines are within 0.4 of the smaller character's height of the line's.
    let mut lines: Vec<(f32, f32, Vec<Glyph>)> = Vec::new();
    for g in flat {
        let h = g.height();
        match lines.last_mut() {
            Some((base, lh, glyphs)) if (g.base - *base).abs() <= 0.4 * lh.min(h) => {
                *lh = lh.max(h);
                glyphs.push(g);
            }
            _ => lines.push((g.base, h, vec![g])),
        }
    }
    let left = lines
        .iter()
        .flat_map(|(_, _, glyphs)| glyphs.iter().map(|g| g.x0))
        .fold(f32::INFINITY, f32::min);
    let unit = grid_unit(&lines);

    let mut out = String::new();
    let mut prev: Option<(f32, f32)> = None;
    for (base, lh, glyphs) in &mut lines {
        // A blank line where the page leaves one.
        if let Some((pbase, plh)) = prev
            && *base - pbase > 1.8 * plh.max(*lh)
        {
            out.push('\n');
        }
        prev = Some((*base, *lh));
        glyphs.sort_by(|a, b| a.x0.total_cmp(&b.x0).then(a.order.cmp(&b.order)));
        let mut col = 0usize;
        let mut last: Option<Glyph> = None;
        for g in glyphs.iter() {
            if let Some(p) = last {
                if same_glyph(&p, g) {
                    continue;
                }
                let gap = g.x0 - (p.x0 + p.width());
                let spaced = gap > 0.25 * p.height().min(g.height());
                match layout {
                    TextLayout::Reading => {
                        if spaced {
                            out.push(' ');
                        }
                    }
                    // Within a word the characters follow one another (proportional widths would drift off the grid);
                    // after a gap the next one goes to its column, at least one space on.
                    TextLayout::Physical if spaced => {
                        let target = (((g.x0 - left) / unit).round().max(0.0) as usize).max(col + 1);
                        for _ in col..target {
                            out.push(' ');
                        }
                        col = target;
                    }
                    TextLayout::Physical => {}
                }
            } else if layout == TextLayout::Physical {
                let target = ((g.x0 - left) / unit).round().max(0.0) as usize;
                for _ in 0..target {
                    out.push(' ');
                }
                col = target;
            }
            out.push(g.ch);
            col += columns_of(g.ch);
            last = Some(*g);
        }
        out.push('\n');
    }
    out
}

/// The width of one column of the physical layout: half the most common character height of the page.
fn grid_unit(lines: &[(f32, f32, Vec<Glyph>)]) -> f32 {
    let mut heights: Vec<f32> = lines.iter().flat_map(|(_, _, g)| g.iter().map(Glyph::height)).collect();
    heights.sort_by(f32::total_cmp);
    let median = heights.get(heights.len() / 2).copied().unwrap_or(10.0);
    (median / 2.0).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{CharBox, PdfRect};

    fn put(chars: &mut Vec<CharBox>, ch: char, x: f32, top: f32, size: f32, w: f32) {
        let op = chars.len();
        chars.push(CharBox { ch, rect_pt: PdfRect { x0: x, y0: top, x1: x + w, y1: top + size }, vertical: false, stream_op_index: op });
    }

    fn row(chars: &mut Vec<CharBox>, text: &str, x: f32, top: f32, size: f32) {
        let mut x = x;
        for ch in text.chars() {
            let w = size * estimated_advance_em(ch);
            put(chars, ch, x, top, size, w);
            x += w;
        }
    }

    fn column(chars: &mut Vec<CharBox>, text: &str, x: f32, top: f32, size: f32) {
        for (i, ch) in text.chars().enumerate() {
            put(chars, ch, x, top + i as f32 * size, size, size);
        }
    }

    #[test]
    fn lines_are_read_top_to_bottom_and_left_to_right_whatever_the_drawing_order() {
        let mut chars = Vec::new();
        row(&mut chars, "second", 10.0, 34.0, 10.0);
        row(&mut chars, "first", 10.0, 20.0, 10.0);
        let text = page_text(&PageTextIndex { chars }, TextLayout::Reading);
        assert_eq!(text, "first\nsecond\n");
    }

    #[test]
    fn a_gap_between_words_is_a_space_and_runs_that_touch_are_not_split() {
        let mut chars = Vec::new();
        row(&mut chars, "標準", 0.0, 0.0, 10.0);
        // A second run continues the word right where the first ended.
        row(&mut chars, "仕様書", 20.0, 0.0, 10.0);
        row(&mut chars, "第1章", 80.0, 0.0, 10.0);
        let text = page_text(&PageTextIndex { chars }, TextLayout::Reading);
        assert_eq!(text, "標準仕様書 第1章\n");
    }

    #[test]
    fn the_physical_layout_keeps_table_columns_aligned() {
        let mut chars = Vec::new();
        row(&mut chars, "ID", 0.0, 0.0, 10.0);
        row(&mut chars, "名称", 50.0, 0.0, 10.0);
        row(&mut chars, "0010001", 0.0, 12.0, 10.0);
        row(&mut chars, "住民", 50.0, 12.0, 10.0);
        let text = page_text(&PageTextIndex { chars }, TextLayout::Physical);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        // Both cells of the second column start at the same column of the text.
        let at = |line: &str, word: &str| line[..line.find(word).unwrap()].chars().map(columns_of).sum::<usize>();
        assert_eq!(at(lines[0], "名称"), 10);
        assert_eq!(at(lines[1], "住民"), 10);
    }

    #[test]
    fn vertical_columns_are_read_right_to_left_top_to_bottom() {
        let mut chars = Vec::new();
        column(&mut chars, "デジタル庁告示", 100.0, 10.0, 10.0);
        column(&mut chars, "第二十七号", 85.0, 10.0, 10.0);
        let text = page_text(&PageTextIndex { chars }, TextLayout::Reading);
        assert_eq!(text, "デジタル庁告示\n第二十七号\n");
    }

    #[test]
    fn upright_digits_in_a_column_are_read_inside_it() {
        let mut chars = Vec::new();
        column(&mut chars, "令和", 100.0, 10.0, 10.0);
        // 縦中横: "８" drawn as a horizontal glyph in the column's cell.
        put(&mut chars, '8', 102.0, 30.0, 10.0, 5.0);
        column(&mut chars, "年七月", 100.0, 40.0, 10.0);
        let text = page_text(&PageTextIndex { chars }, TextLayout::Reading);
        assert_eq!(text, "令和8年七月\n");
    }

    #[test]
    fn a_page_mostly_vertical_puts_its_columns_before_its_horizontal_lines() {
        let mut chars = Vec::new();
        column(&mut chars, "本文の縦書き", 100.0, 10.0, 10.0);
        row(&mut chars, "1", 50.0, 200.0, 8.0);
        let text = page_text(&PageTextIndex { chars }, TextLayout::Reading);
        assert_eq!(text, "本文の縦書き\n\n1\n");
    }

    #[test]
    fn a_character_overprinted_for_bold_is_read_once() {
        let mut chars = Vec::new();
        row(&mut chars, "告示", 0.0, 0.0, 10.0);
        row(&mut chars, "告示", 0.3, 0.0, 10.0);
        let text = page_text(&PageTextIndex { chars }, TextLayout::Reading);
        assert_eq!(text, "告示\n");
    }

    #[test]
    fn a_short_stack_of_characters_is_not_taken_for_vertical_writing() {
        // Two table cells one below another are lines, not a column.
        let mut chars = Vec::new();
        row(&mut chars, "○", 0.0, 0.0, 10.0);
        row(&mut chars, "○", 0.0, 12.0, 10.0);
        let text = page_text(&PageTextIndex { chars }, TextLayout::Reading);
        assert_eq!(text, "○\n○\n");
    }

    #[test]
    fn spaces_and_an_empty_page() {
        assert_eq!(page_text(&PageTextIndex::default(), TextLayout::Reading), "");
        let mut chars = Vec::new();
        row(&mut chars, "a b", 0.0, 0.0, 10.0);
        // The drawn space is dropped; the gap it leaves makes the space.
        assert_eq!(page_text(&PageTextIndex { chars }, TextLayout::Reading), "a b\n");
    }
}
