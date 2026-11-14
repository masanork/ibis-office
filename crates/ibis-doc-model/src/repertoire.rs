// SPDX-License-Identifier: AGPL-3.0-only
//! Text that may be displayed anywhere in a document, independent of viewport
//! or pagination. Used to prepare browser fonts before the first layout.
use std::collections::{BTreeSet, HashSet};
use unicode_segmentation::UnicodeSegmentation;
use crate::*;

/// Unique graphemes from all pages, sheets, tables, notes and other text-bearing
/// model objects. Adjacent styled runs are joined before segmentation, retaining
/// IVS and combining sequences split across runs. Media and document metadata
/// that do not display text are not scanned.
pub fn font_repertoire(presentation: &Presentation) -> String {
    let mut collector = Collector::default();
    collector.properties(&presentation.properties);
    if let Some(doc) = &presentation.flow { collector.document(doc); }
    for slide in &presentation.slides {
        if let Some(name) = &slide.name { collector.add(name); }
        collector.add(&slide.notes);
        for comment in &slide.comments {
            collector.add(&comment.text); collector.add(&comment.author.name);
            for reply in &comment.replies { collector.add(&reply.text); collector.add(&reply.author.name); }
        }
        for shape in slide.shapes.iter().chain(&slide.backdrop).chain(&slide.notes_shapes) { collector.shape(shape); }
    }
    for layout in &presentation.layouts {
        for shape in &layout.backdrop { collector.shape(shape); }
    }
    if let Some(book) = &presentation.workbook {
        collector.properties(&book.properties);
        for sheet in &book.sheets {
            collector.add(&sheet.name);
            if let Some(hidden) = &sheet.hidden { collector.sheet(hidden); }
        }
    }
    // Separate clusters so an isolated combining mark cannot attach to a
    // previous, unrelated paragraph after the set is transferred to JavaScript.
    collector.glyphs.into_iter().collect::<Vec<_>>().join("\n")
}

#[derive(Default)]
struct Collector {
    glyphs: BTreeSet<String>,
    documents: HashSet<usize>,
    sheets: HashSet<usize>,
}

impl Collector {
    fn add(&mut self, text: &str) {
        // All-caps and small-caps are display transforms; retain their glyphs
        // as well as the original, without normalizing variation sequences.
        for value in [text.to_string(), text.to_uppercase()] {
            for glyph in value.graphemes(true) {
                if glyph.chars().any(|c| !c.is_control() && !c.is_whitespace()) {
                    self.glyphs.insert(glyph.to_string());
                }
            }
        }
    }

    fn body(&mut self, body: &TextBody) {
        for paragraph in &body.paragraphs {
            if let Some(bullet) = &paragraph.bullet { self.add(&bullet.text); }
            let mut text = String::new();
            for run in &paragraph.runs {
                text.push_str(&run.text);
                if let Some(math) = &run.style.math { self.add(&math.linear()); }
            }
            self.add(&text);
        }
    }

    fn document(&mut self, doc: &FlowDocument) {
        if !self.documents.insert(std::ptr::from_ref(doc) as usize) { return; }
        self.properties(&doc.properties);
        for paragraph in crate::flow_command::all_paragraphs_everywhere(doc) {
            let mut text = String::new();
            for run in &paragraph.runs {
                use FlowRunKind::*;
                match &run.kind {
                    Text(t) | FileName(t) | RichText { text: t, .. } | PlainText { text: t, .. }
                    | Group { text: t, .. } | BuildingBlockGallery { text: t, .. }
                    | DatePicker { text: t, .. } | PageRef { shown: t, .. } | Field { shown: t, .. } => text.push_str(t),
                    Ruby { base, text: ruby, .. } => { text.push_str(base); self.add(ruby); }
                    Shape(shape) => { self.shape(shape); text.push(' '); }
                    Math { math, .. } => { self.add(&math.linear()); text.push(' '); }
                    CheckBox { checked_char, unchecked_char, .. } => { self.add(&checked_char.to_string()); self.add(&unchecked_char.to_string()); }
                    DropDownList { items, .. } => { for (label, _) in items { self.add(label); } }
                    RepeatingSection { section_title, items, .. } => { self.add(section_title); for item in items { self.add(item); } }
                    FormField(field) => match &field.kind {
                        FormFieldKind::TextInput { value, default, .. } => { text.push_str(value); self.add(default); }
                        FormFieldKind::DropDown { items, .. } => { for item in items { self.add(item); } }
                        FormFieldKind::CheckBox { .. } => self.add("☑☐"),
                    },
                    Tab | PositionalTab { .. } | LineBreak | PageBreak | ColumnBreak => text.push(' '),
                    Picture { .. } | PictureControl { .. } | PageNumber | NoteRef { .. } | NoteMark
                    | CommentStart(_) | CommentEnd(_) | CommentRef(_) => {}
                }
            }
            self.add(&text);
        }
        if let Some(watermark) = &doc.watermark { self.add(&watermark.text); }
        for (_, watermark) in &doc.section_watermarks { if let Some(w) = watermark { self.add(&w.text); } }
        for comment in &doc.comments { self.add(&comment.author); self.add(&comment.initials); }
        for label in crate::flow::list_labels(doc).values() { self.add(label); }
    }

    fn shape(&mut self, shape: &Shape) {
        match &shape.kind {
            ShapeKind::TextBox(body) => self.body(body),
            ShapeKind::Auto(auto) => { if let Some(body) = &auto.text { self.body(body); } }
            ShapeKind::Table(table) => for row in &table.rows { for cell in &row.cells { self.body(&cell.text); } },
            ShapeKind::Sheet(sheet) => self.sheet(sheet),
            ShapeKind::Page { document, .. } => self.document(document),
            ShapeKind::Chart(chart) => self.chart(chart),
            ShapeKind::Slicer(slicer) => { self.add(&slicer.caption); for item in &slicer.items { self.add(&item.label); } }
            ShapeKind::Control(control) => { self.add(&control.text); for item in &control.items { self.add(item); } }
            ShapeKind::Timeline(timeline) => self.add(&timeline.caption),
            ShapeKind::Picture(_) | ShapeKind::Line(_) | ShapeKind::Placeholder(_) => {}
        }
    }

    fn sheet(&mut self, sheet: &Sheet) {
        if !self.sheets.insert(std::ptr::from_ref(sheet) as usize) { return; }
        for cell in &sheet.cells {
            self.add(&cell.text);
            if let CellValue::Text(value) = &cell.value { self.add(value); }
            if let Some(runs) = &cell.runs { self.add(&runs.iter().map(|r| r.text.as_str()).collect::<String>()); }
            if let Some(formula) = &cell.formula { self.add(formula); }
            if let Some(phonetic) = &cell.phonetic { self.add(phonetic); }
        }
        for style in &sheet.styles { self.add(&style.number_format); }
        for comment in &sheet.comments { self.add(&comment.author); self.add(&comment.text); }
        for thread in &sheet.threads { for post in &thread.posts { self.add(&post.author); self.add(&post.text); } }
        let hf = &sheet.print.header_footer;
        for text in [&hf.odd_header, &hf.odd_footer, &hf.first_header, &hf.first_footer, &hf.even_header, &hf.even_footer].into_iter().flatten() { self.add(text); }
    }

    fn chart(&mut self, chart: &Chart) {
        if let Some(title) = &chart.title { self.add(&title.text); }
        for axis in &chart.axes {
            if let Some(title) = &axis.title { self.add(&title.text); }
            self.add(&axis.format);
            for level in &axis.category_levels { for (_, _, text) in level { self.add(text); } }
        }
        for group in &chart.groups {
            for series in &group.series {
                self.add(&series.name); self.add(&series.format);
                for category in &series.categories { self.add(category); }
                if let Some(labels) = &series.labels {
                    for point in &labels.points { if let Some(pieces) = &point.text {
                        for piece in pieces { if let LabelPiece::Text(text) = piece { self.add(text); } }
                    } }
                }
            }
        }
        if let Some(ChartEx::Treemap { nodes, .. } | ChartEx::Sunburst(nodes)) = &chart.ex { self.nodes(nodes); }
        if let Some(ChartEx::Box { boxes, .. }) = &chart.ex { for item in boxes { self.add(&item.name); } }
    }

    fn properties(&mut self, properties: &DocProperties) {
        for text in [&properties.title, &properties.subject, &properties.creator, &properties.keywords,
            &properties.description, &properties.last_modified_by, &properties.category, &properties.company, &properties.manager]
            .into_iter().flatten() { self.add(text); }
        for property in &properties.custom {
            if let CustomPropertyValue::Text(text) = &property.value { self.add(text); }
        }
    }

    fn nodes(&mut self, nodes: &[TreeNode]) {
        for node in nodes { self.add(&node.name); self.nodes(&node.children); }
    }
}
