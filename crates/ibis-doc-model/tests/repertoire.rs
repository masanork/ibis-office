// SPDX-License-Identifier: AGPL-3.0-only
use ibis_doc_model::*;
use ibis_doc_model::repertoire::font_repertoire;
use std::sync::Arc;

fn paragraph(text: &str) -> Block {
    Block::Paragraph(FlowParagraph {
        runs: vec![FlowRun::new(FlowRunKind::Text(text.into()), FlowRunStyle::default())],
        ..FlowParagraph::default()
    })
}

#[test]
fn collects_offscreen_nested_tables_headers_sections_notes_ruby_and_inline_shapes() {
    let mut doc = FlowDocument::default();
    doc.body.push(paragraph("先頭"));
    doc.body.push(Block::Table(FlowTable { rows: vec![FlowRow { cells: vec![FlowCell {
        blocks: vec![paragraph("末尾表"), Block::Table(FlowTable { rows: vec![FlowRow { cells: vec![FlowCell {
            blocks: vec![paragraph("深層")], ..FlowCell::default()
        }], ..FlowRow::default() }], ..FlowTable::default() })], ..FlowCell::default()
    }], ..FlowRow::default() }], ..FlowTable::default() }));
    doc.header_even.push(paragraph("偶数頁眉"));
    doc.footer_first.push(paragraph("初頁脚"));
    doc.footnotes.push(Note { id: 1, blocks: vec![paragraph("注釈") ] });
    doc.endnotes.push(Note { id: 2, blocks: vec![paragraph("後註") ] });
    doc.comments.push(Comment { blocks: vec![paragraph("評言")], ..Comment::default() });
    let mut section = SectionSetup::default(); section.parts[0] = Some(vec![paragraph("節固有")]);
    doc.body.push(Block::Paragraph(FlowParagraph {
        section: Some(Arc::new(section)),
        runs: vec![
            FlowRun::new(FlowRunKind::Ruby { base: "親".into(), text: "おや".into(), size: 500, raise: 200, align: RubyAlign::Center }, FlowRunStyle::default()),
            FlowRun::new(FlowRunKind::Shape(Box::new(Shape::text_box(ShapeId(1), EmuRect::new(0, 0, 10, 10), "図形"))), FlowRunStyle::default()),
        ], ..FlowParagraph::default()
    }));
    let mut p = Presentation::empty(EmuSize::new(100, 100)); p.flow = Some(Arc::new(doc));
    let result = font_repertoire(&p);
    for c in "先頭末尾表深層偶数頁眉初頁脚注釈後註評言節固有親おや図形".chars() { assert!(result.contains(c), "{c}"); }
}

#[test]
fn preserves_graphemes_across_style_boundaries_and_deduplicates_whole_document() {
    let mut doc = FlowDocument::default();
    doc.body.push(Block::Paragraph(FlowParagraph {
        runs: ["葛", "\u{e0100}", "e", "\u{301}", "葛\u{e0100}"].into_iter()
            .map(|t| FlowRun::new(FlowRunKind::Text(t.into()), FlowRunStyle::default())).collect(),
        ..FlowParagraph::default()
    }));
    let mut p = Presentation::empty(EmuSize::new(100, 100)); p.flow = Some(Arc::new(doc));
    let text = font_repertoire(&p); let glyphs: Vec<_> = text.lines().collect();
    assert_eq!(glyphs.iter().filter(|g| **g == "葛\u{e0100}").count(), 1);
    assert!(glyphs.contains(&"e\u{301}"));
    assert!(glyphs.contains(&"E\u{301}"));
    assert!(!glyphs.contains(&"\u{e0100}"));
}

#[test]
fn visits_later_slides_and_hidden_sheets_outside_the_current_viewport() {
    let mut p = Presentation::empty(EmuSize::new(100, 100));
    for (id, text) in [(1, "前頁"), (50, "遠頁龜")] {
        let mut slide = Slide::new(SlideId(id));
        slide.shapes.push(Shape::text_box(ShapeId(id), EmuRect::new(0, 0, 10, 10), text));
        p.slides.push(slide);
    }
    let mut sheet = Sheet::default();
    sheet.cells.push(SheetCell { row: 100_000, column: 99, value: CellValue::Text("奥地".into()), text: "遠方".into(),
        kind: ibis_doc_model::sheet::CellKind::Text, formula: None, style: 0, phonetic: None, runs: None });
    p.workbook = Some(Arc::new(ibis_doc_model::sheet::Workbook { sheets: vec![ibis_doc_model::sheet::WorkbookSheet {
        name: "秘匿頁".into(), state: ibis_doc_model::sheet::SheetState::Hidden, slide: None, hidden: Some(Arc::new(sheet)), other: false,
    }], ..ibis_doc_model::sheet::Workbook::default() }));
    let text = font_repertoire(&p);
    for c in "前頁遠龜奥地方秘匿".chars() { assert!(text.contains(c), "{c}"); }
}
