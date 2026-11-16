// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! Normalization of backward-compatible Plane 16 Private Use Area (PUP)
//! characters to standard Unicode / IVS / combining sequences for PDF and
//! OOXML output. In-place CSV saves intentionally bypass this module to retain
//! original bytes and encodings.

use std::borrow::Cow;
use std::sync::Arc;

use crate::flow::{Block, FlowDocument, FlowParagraph, FlowRunKind, FlowTable};
use crate::model::{Presentation, Shape, ShapeKind, Slide};
use crate::pup_compat_table::{PUP_COMPAT_FIRST, PUP_COMPAT_OFFSETS, PUP_COMPAT_SECOND};
use crate::sheet::{CellValue, Sheet};
use crate::text::TextBody;

/// Summary of backward-compatible PUP normalization on a document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PupNormalizationReport {
    /// Number of backward-compatible PUP characters replaced by standard Unicode / IVS.
    pub converted: usize,
    /// Number of remaining Plane 16 PUA characters (`U+100000..=U+10FFFD`) without a verified mapping.
    pub unmapped_plane16_pua: usize,
}

/// Look up a backward-compatible Plane 16 PUP character in the verified table.
pub fn lookup_compat_pup(c: char) -> Option<(char, char)> {
    let cp = c as u32;
    if !(0x10022A..=0x104295).contains(&cp) {
        return None;
    }
    let offset = (cp - 0x100000) as u16;
    let idx = PUP_COMPAT_OFFSETS.binary_search(&offset).ok()?;
    let c1 = char::from_u32(PUP_COMPAT_FIRST[idx])?;
    let c2 = char::from_u32(PUP_COMPAT_SECOND[idx])?;
    Some((c1, c2))
}

fn has_plane16_bytes(s: &str) -> bool {
    // Plane 16 scalar values (U+100000..=U+10FFFD) encode as 4-byte UTF-8 starting with 0xF4.
    s.as_bytes().contains(&0xF4)
}

/// Normalize backward-compatible PUP characters in `s` without guessing or
/// dropping unmapped characters.
pub fn normalize_compat_pup_str<'a>(s: &'a str, report: &mut PupNormalizationReport) -> Cow<'a, str> {
    if !has_plane16_bytes(s) {
        return Cow::Borrowed(s);
    }
    let mut out: Option<String> = None;
    for (idx, c) in s.char_indices() {
        if let Some((c1, c2)) = lookup_compat_pup(c) {
            let buf = out.get_or_insert_with(|| String::from(&s[..idx]));
            buf.push(c1);
            buf.push(c2);
            report.converted += 1;
        } else {
            if (0x100000..=0x10FFFD).contains(&(c as u32)) {
                report.unmapped_plane16_pua += 1;
            }
            if let Some(buf) = &mut out {
                buf.push(c);
            }
        }
    }
    match out {
        Some(owned) => Cow::Owned(owned),
        None => Cow::Borrowed(s),
    }
}

fn norm_string(s: &mut String, report: &mut PupNormalizationReport) -> bool {
    let Cow::Owned(next) = normalize_compat_pup_str(s, report) else {
        return false;
    };
    *s = next;
    true
}

fn norm_arc_str(s: &mut Arc<str>, report: &mut PupNormalizationReport) -> bool {
    let Cow::Owned(next) = normalize_compat_pup_str(s, report) else {
        return false;
    };
    *s = Arc::from(next);
    true
}

fn norm_text_body(body: &mut TextBody, report: &mut PupNormalizationReport) -> bool {
    let mut changed = false;
    for p in &mut body.paragraphs {
        for run in &mut p.runs {
            changed |= norm_string(&mut run.text, report);
        }
    }
    changed
}

fn norm_sheet(sheet: &mut Sheet, report: &mut PupNormalizationReport) -> bool {
    let mut changed = false;
    for cell in sheet.cells.iter_mut() {
        if let CellValue::Text(ref mut text) = cell.value {
            changed |= norm_string(text, report);
        }
        changed |= norm_string(&mut cell.text, report);
        if let Some(ref mut formula) = cell.formula {
            changed |= norm_string(formula, report);
        }
        if let Some(ref mut phonetic) = cell.phonetic {
            changed |= norm_arc_str(phonetic, report);
        }
        if let Some(ref mut runs) = cell.runs
            && runs.iter().any(|r| has_plane16_bytes(&r.text))
        {
            for run in Arc::make_mut(runs).iter_mut() {
                changed |= norm_string(&mut run.text, report);
            }
        }
    }
    for comment in &mut sheet.comments {
        changed |= norm_string(&mut comment.text, report);
        changed |= norm_string(&mut comment.author, report);
    }
    changed
}

fn norm_shape(shape: &mut Shape, report: &mut PupNormalizationReport) -> bool {
    let mut changed = false;
    match &mut shape.kind {
        ShapeKind::TextBox(body) => {
            changed |= norm_text_body(body, report);
        }
        ShapeKind::Auto(auto) => {
            if let Some(body) = &mut auto.text {
                changed |= norm_text_body(body, report);
            }
        }
        ShapeKind::Table(table) => {
            for row in &mut table.rows {
                for cell in &mut row.cells {
                    changed |= norm_text_body(&mut cell.text, report);
                }
            }
        }
        ShapeKind::Sheet(sheet) => {
            let mut cloned = (**sheet).clone();
            if norm_sheet(&mut cloned, report) {
                *sheet = Arc::new(cloned);
                changed = true;
            }
        }
        _ => {}
    }
    changed
}

fn norm_slide(slide: &mut Slide, report: &mut PupNormalizationReport) -> bool {
    let mut changed = false;
    if let Some(name) = &mut slide.name {
        changed |= norm_string(name, report);
    }
    for shape in &mut slide.shapes {
        changed |= norm_shape(shape, report);
    }
    for shape in &mut slide.backdrop {
        changed |= norm_shape(shape, report);
    }
    changed
}

fn norm_flow_paragraph(p: &mut FlowParagraph, report: &mut PupNormalizationReport) -> bool {
    let mut changed = false;
    for run in &mut p.runs {
        if let FlowRunKind::Text(ref mut text) = run.kind {
            changed |= norm_string(text, report);
        }
    }
    changed
}

fn norm_flow_table(table: &mut FlowTable, report: &mut PupNormalizationReport) -> bool {
    let mut changed = false;
    for row in &mut table.rows {
        for cell in &mut row.cells {
            for block in &mut cell.blocks {
                changed |= norm_flow_block(block, report);
            }
        }
    }
    changed
}

fn norm_flow_block(block: &mut Block, report: &mut PupNormalizationReport) -> bool {
    match block {
        Block::Paragraph(p) => norm_flow_paragraph(p, report),
        Block::Table(t) => norm_flow_table(t, report),
    }
}

fn norm_flow_document(flow: &mut FlowDocument, report: &mut PupNormalizationReport) -> bool {
    let mut changed = false;
    for blocks in [
        &mut flow.header,
        &mut flow.footer,
        &mut flow.body,
        &mut flow.header_first,
        &mut flow.footer_first,
        &mut flow.header_even,
        &mut flow.footer_even,
    ] {
        for block in blocks.iter_mut() {
            changed |= norm_flow_block(block, report);
        }
    }
    changed
}

/// Normalize backward-compatible PUP characters in `presentation` for standard
/// Unicode / IVS output (PDF and OOXML). Borrows without cloning when no
/// backward-compatible PUP characters are present.
pub fn normalize_presentation_compat_pup<'a>(
    presentation: &'a Presentation,
) -> (Cow<'a, Presentation>, PupNormalizationReport) {
    let mut report = PupNormalizationReport::default();
    let mut cloned = presentation.clone();
    let mut changed = false;
    for slide in &mut cloned.slides {
        changed |= norm_slide(slide, &mut report);
    }
    if let Some(flow) = &mut cloned.flow {
        let mut flow_cloned = (**flow).clone();
        if norm_flow_document(&mut flow_cloned, &mut report) {
            *flow = Arc::new(flow_cloned);
            changed = true;
        }
    }
    if let Some(wb) = &mut cloned.workbook {
        let mut wb_cloned = (**wb).clone();
        let mut wb_changed = false;
        for sheet in &mut wb_cloned.sheets {
            wb_changed |= norm_string(&mut sheet.name, &mut report);
        }
        if wb_changed {
            *wb = Arc::new(wb_cloned);
            changed = true;
        }
    }
    if changed {
        (Cow::Owned(cloned), report)
    } else {
        (Cow::Borrowed(presentation), report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::{CellKind, SheetCell};
    use crate::{EmuSize, SlideId};

    #[test]
    fn compat_pup_lookup_covers_ivs_and_combining_without_guessing() {
        assert_eq!(lookup_compat_pup('\u{101000}'), Some(('\u{3404}', '\u{E0101}')));
        assert_eq!(lookup_compat_pup('\u{10022A}'), Some(('\u{00E6}', '\u{0300}')));
        // Administrative Plane 16 PUP (outside compat table) is left untouched and counted as unmapped.
        assert_eq!(lookup_compat_pup('\u{10DE92}'), None);

        let mut report = PupNormalizationReport::default();
        let input = "A\u{101000}B\u{10022A}\u{10DE92}";
        let out = normalize_compat_pup_str(input, &mut report);
        assert_eq!(out, "A\u{3404}\u{E0101}B\u{00E6}\u{0300}\u{10DE92}");
        assert_eq!(report.converted, 2);
        assert_eq!(report.unmapped_plane16_pua, 1);
    }

    #[test]
    fn ascii_and_normal_text_borrow_without_allocation() {
        let mut report = PupNormalizationReport::default();
        let s = "通常の漢字とASCII 123";
        assert!(matches!(normalize_compat_pup_str(s, &mut report), Cow::Borrowed(_)));
        assert_eq!(report, PupNormalizationReport::default());
    }

    #[test]
    fn presentation_normalization_updates_cells_and_text_boxes() {
        let mut p = Presentation::default();
        p.slide_size = EmuSize::new(100, 100);
        let mut sheet = Sheet::default();
        sheet.rows.push(100);
        sheet.columns.push(100);
        sheet.put_cell(0, 0, Some(SheetCell {
            row: 0,
            column: 0,
            value: CellValue::Text("\u{101000}".into()),
            text: "\u{101000}".into(),
            kind: CellKind::Text,
            formula: None,
            style: 0,
            phonetic: None,
            runs: None,
        }));
        let mut slide = Slide::new(SlideId(1));
        slide.shapes.push(Shape::new(
            crate::ShapeId(1),
            crate::EmuRect::new(0, 0, 10, 10),
            ShapeKind::Sheet(Arc::new(sheet)),
        ));
        p.slides = vec![slide];

        let (normalized, report) = normalize_presentation_compat_pup(&p);
        assert_eq!(report.converted, 2);
        let ShapeKind::Sheet(norm_sheet) = &normalized.slides[0].shapes[0].kind else {
            panic!("expected sheet");
        };
        let cell = norm_sheet.cell(0, 0).unwrap();
        assert_eq!(cell.value, CellValue::Text("\u{3404}\u{E0101}".into()));
        assert_eq!(cell.text, "\u{3404}\u{E0101}");
    }
}
