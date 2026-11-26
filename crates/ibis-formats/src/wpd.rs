// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Import the WordPerfect 6.x document stream into editable Word paragraphs.
//! Record framing follows the WordPerfect SDK File Formats documentation.
use ibis_doc_model::flow::{
    Block, FlowDocument, FlowParagraph, FlowRun, FlowRunKind, RunFormat, UnderlineStyle,
    VerticalPosition,
};

use crate::{Error, Opened};

mod appearance;
mod picture;
mod table;
mod wpg;
mod wpg_drawing;

use appearance::Packets;

use table::{CellInfo, Table};

const MAGIC: &[u8] = b"\xffWPC";

pub fn is_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("wpd") || extension.eq_ignore_ascii_case("wp6")
}

pub fn is_wpd(data: &[u8]) -> bool {
    data.starts_with(MAGIC) && data.get(8..10) == Some(&[1, 10])
}

pub fn open(data: &[u8]) -> Result<Opened, Error> {
    let (docx, omitted) = convert(data)?;
    let mut opened = crate::open(docx)?;
    if omitted != 0 {
        opened.import_warning = Some(crate::ImportWarning::WordPerfectImagesOmitted(omitted));
    }
    Ok(opened)
}

pub fn to_docx(data: &[u8]) -> Result<Vec<u8>, Error> {
    convert(data).map(|(docx, _)| docx)
}

fn convert(data: &[u8]) -> Result<(Vec<u8>, usize), Error> {
    let mut omitted = 0;
    let docx = crate::hwpx::document_to_docx(|presentation| {
        let (document, omitted_images) = read(data, presentation)?;
        omitted = omitted_images;
        Ok(document)
    })?;
    Ok((docx, omitted))
}

fn malformed() -> Error {
    Error::new("WordPerfect の文書データが不正です")
}

fn read(
    data: &[u8],
    presentation: &mut ibis_doc_model::Presentation,
) -> Result<(FlowDocument, usize), Error> {
    if !is_wpd(data) {
        return Err(malformed());
    }
    let header = data.get(..24).ok_or_else(malformed)?;
    if header[10] != 2 {
        return Err(Error::new(
            "この WordPerfect 文書の形式には対応していません",
        ));
    }
    if header[12..14] != [0, 0] {
        return Err(Error::new(
            "暗号化された WordPerfect 文書には対応していません",
        ));
    }
    let start = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    // The file-size field excludes any padding after the document stream.
    let size = u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
    let end = if size == 0 { data.len() } else { size };
    let mut body = data
        .get(start..end)
        .filter(|_| start >= header.len())
        .ok_or_else(malformed)?;
    let mut importer = Importer {
        packets: Packets::read(&data[..start]),
        ..Default::default()
    };
    let mut deleted_depth = 0usize;
    while let Some(&code) = body.first() {
        let record = take_record(body, code)?;
        body = &body[record.len()..];
        if code == 0xf1 {
            match record[1] {
                0 => deleted_depth += 1,
                1 => deleted_depth = deleted_depth.checked_sub(1).ok_or_else(malformed)?,
                _ => {}
            }
        } else if deleted_depth == 0 {
            importer.record(code, record, presentation)?;
        }
    }
    if deleted_depth != 0 || importer.table.is_some() {
        return Err(malformed());
    }
    if importer.has_paragraph() || !matches!(importer.document.body.last(), Some(Block::Table(_))) {
        importer.finish_paragraph();
    }
    Ok((importer.document, importer.pictures.omitted))
}

fn take_record(body: &[u8], code: u8) -> Result<&[u8], Error> {
    let length = match code {
        0xd0..=0xef => {
            let size = body.get(2..4).ok_or_else(malformed)?;
            let length = usize::from(u16::from_le_bytes(size.try_into().unwrap()));
            if length < 8 || body.get(length - 3..length - 1) != Some(size) {
                return Err(malformed());
            }
            length
        }
        0xf2..=0xf5 => 3,
        0xf0 | 0xf6..=0xf8 => 4,
        0xf1 | 0xf9..=0xfa => 5,
        0xfb..=0xfc => 6,
        0xfd..=0xfe => 8,
        0xff => return Err(malformed()),
        _ => 1,
    };
    body.get(..length)
        .filter(|record| length == 1 || record.last() == Some(&code))
        .ok_or_else(malformed)
}

fn next_id(ids: &mut u32) -> u32 {
    *ids += 1;
    *ids + 1
}

#[derive(Default)]
struct Importer<'a> {
    packets: Packets<'a>,
    pictures: picture::Pictures,
    document: FlowDocument,
    paragraph: FlowParagraph,
    text: String,
    format: RunFormat,
    cell_format: RunFormat,
    ids: u32,
    table: Option<Table>,
}

impl Importer<'_> {
    fn record(
        &mut self,
        code: u8,
        record: &[u8],
        presentation: &mut ibis_doc_model::Presentation,
    ) -> Result<(), Error> {
        if code == 0xd4 && record[4] & 0x40 == 0 {
            match record[1] {
                0x2a => {
                    if self.table.is_some() {
                        return Err(malformed());
                    }
                    if self.has_paragraph() {
                        self.finish_paragraph();
                    }
                    self.table = Some(Table::new(next_id(&mut self.ids), record, self.packets)?);
                    return Ok(());
                }
                0x2b => return self.table.as_mut().ok_or_else(malformed)?.start(),
                0x2c => return self.table.as_mut().ok_or_else(malformed)?.column(record),
                _ => {}
            }
        }
        if self.table.is_some() {
            let boundary = match code {
                0xc6 => Some((false, false, CellInfo::default())),
                0xc0..=0xc5 => Some((true, false, CellInfo::default())),
                0xbd..=0xbf => Some((false, true, CellInfo::default())),
                0xd0 if record[4] & 0x40 == 0 => match record[1] {
                    10..=16 => Some((
                        record[1] != 10,
                        false,
                        CellInfo::read(record, self.packets)?,
                    )),
                    17..=19 => Some((false, true, CellInfo::default())),
                    _ => None,
                },
                _ => None,
            };
            if let Some((new_row, end, info)) = boundary {
                let table = self.table.as_ref().unwrap();
                if table.active {
                    self.finish_paragraph();
                }
                self.cell_format = RunFormat::default();
                let table = self.table.as_mut().unwrap();
                if end {
                    let table = self.table.take().unwrap().finish(&mut self.ids)?;
                    self.document.body.push(Block::Table(table));
                } else {
                    table.cell(new_row, info, &mut self.ids)?;
                    for bit in 0..16 {
                        if table.attributes & (1 << bit) != 0 {
                            attribute(&mut self.cell_format, bit, true);
                        }
                    }
                }
                return Ok(());
            }
            if !self.table.as_ref().unwrap().active {
                return Ok(());
            }
        }

        if code == 0xdf && record[1] <= 2 && record[4] & 0x40 == 0 {
            let id = ibis_doc_model::ShapeId(u64::from(next_id(&mut self.ids)));
            if let Some(image) = self.pictures.read(record, self.packets, presentation, id) {
                self.push_run(image);
            }
            return Ok(());
        }
        match code {
            0x01..=0x20 => self.text.push(INTERNATIONAL[usize::from(code - 1)].1),
            0x21..=0x7e => self.text.push(char::from(code)),
            0x80 | 0xba..=0xbc | 0xcd..=0xcf => self.text.push(' '),
            0x81 => self.text.push('\u{a0}'),
            0x82 | 0x83 => self.text.push('\u{ad}'),
            0x84 => self.text.push('-'),
            0x87 | 0x89 | 0xb7..=0xb9 | 0xca..=0xcc => self.finish_paragraph(),
            0xc7 => self.page_break(),
            0xc8..=0xc9 => self.push_run(FlowRunKind::ColumnBreak),
            0xbf | 0xbd..=0xbe | 0xc0..=0xc5 => self.finish_paragraph(),
            0xc6 => self.push_run(FlowRunKind::Tab),
            0xd0 if record[4] & 0x40 == 0 => match record[1] {
                1..=3 | 20..=22 => self.text.push(' '),
                4..=6 | 11..=19 | 23..=25 => self.finish_paragraph(),
                7..=8 => self.push_run(FlowRunKind::ColumnBreak),
                9 => self.page_break(),
                10 => self.push_run(FlowRunKind::Tab),
                _ => {}
            },
            0xe0 if record[4] & 0x40 == 0 => self.push_run(FlowRunKind::Tab),
            0xf0 => self.text.push(extended_character(record[1], record[2])),
            0xf2 | 0xf3 if record[1] & 0x80 == 0 => {
                self.flush_text();
                attribute(&mut self.format, record[1] & 0x3f, code == 0xf2);
            }
            _ => {}
        }
        Ok(())
    }

    fn page_break(&mut self) {
        if self.has_paragraph() {
            self.finish_paragraph();
        }
        self.paragraph.direct.page_break_before = Some(true);
    }

    fn has_paragraph(&self) -> bool {
        !self.text.is_empty()
            || !self.paragraph.runs.is_empty()
            || self.paragraph.direct.page_break_before == Some(true)
    }

    fn finish_paragraph(&mut self) {
        self.flush_text();
        self.paragraph.id = next_id(&mut self.ids);
        if let Some(table) = &self.table {
            self.paragraph.direct.align = table.align;
        }
        self.paragraph.resolve(&Default::default());
        let paragraph = Block::Paragraph(std::mem::take(&mut self.paragraph));
        if let Some(table) = &mut self.table {
            table.push(paragraph);
        } else {
            self.document.body.push(paragraph);
        }
    }

    fn push_run(&mut self, kind: FlowRunKind) {
        self.flush_text();
        self.append_run(kind);
    }

    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            let text = std::mem::take(&mut self.text);
            self.append_run(FlowRunKind::Text(text));
        }
    }

    fn append_run(&mut self, kind: FlowRunKind) {
        let mut run = FlowRun::new(kind, Default::default());
        run.format = self.format.clone();
        run.format.overlay(&self.cell_format);
        self.paragraph.runs.push(run);
    }
}

fn attribute(format: &mut RunFormat, attribute: u8, enabled: bool) {
    match attribute {
        5 | 6 => {
            let position = match (attribute, enabled) {
                (5, true) => VerticalPosition::Superscript,
                (6, true) => VerticalPosition::Subscript,
                _ => VerticalPosition::Baseline,
            };
            format.position = Some(position);
        }
        8 => {
            format.italic = Some(enabled);
        }
        11 | 14 => {
            let style = if attribute == 11 && enabled {
                UnderlineStyle::Double
            } else {
                UnderlineStyle::Single
            };
            format.underline = Some(enabled);
            format.underline_style = Some(style);
        }
        12 => {
            format.bold = Some(enabled);
        }
        13 => {
            format.strike = Some(enabled);
        }
        15 => {
            format.small_caps = Some(enabled);
        }
        _ => {}
    }
}

// Default single-byte international mapping, in byte order. Each tuple also
// carries the corresponding WP character number in the multinational set.
const INTERNATIONAL: &[(u8, char); 32] = &[
    (35, 'å'),
    (34, 'Å'),
    (37, 'æ'),
    (36, 'Æ'),
    (31, 'ä'),
    (30, 'Ä'),
    (27, 'á'),
    (33, 'à'),
    (29, 'â'),
    (77, 'ã'),
    (76, 'Ã'),
    (39, 'ç'),
    (38, 'Ç'),
    (45, 'ë'),
    (41, 'é'),
    (40, 'É'),
    (47, 'è'),
    (43, 'ê'),
    (49, 'í'),
    (57, 'ñ'),
    (56, 'Ñ'),
    (81, 'ø'),
    (80, 'Ø'),
    (83, 'õ'),
    (82, 'Õ'),
    (63, 'ö'),
    (62, 'Ö'),
    (71, 'ü'),
    (70, 'Ü'),
    (67, 'ú'),
    (73, 'ù'),
    (23, 'ß'),
];

fn extended_character(character: u8, set: u8) -> char {
    match (set, character) {
        (0, 0x20..=0x7e) => char::from(character),
        (1, _) => INTERNATIONAL
            .iter()
            .find_map(|&(number, glyph)| (number == character).then_some(glyph))
            .unwrap_or('\u{fffd}'),
        _ => '\u{fffd}',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(data: &[u8]) -> Result<FlowDocument, Error> {
        super::read(data, &mut ibis_doc_model::Presentation::default())
            .map(|(document, _)| document)
    }
    use ibis_doc_model::flow::FlowRunStyle;

    fn fixture(body: &[u8]) -> Vec<u8> {
        let mut data = vec![0; 526];
        data[..4].copy_from_slice(MAGIC);
        data[4..8].copy_from_slice(&526u32.to_le_bytes());
        data[8..12].copy_from_slice(&[1, 10, 2, 1]);
        data[14..16].copy_from_slice(&512u16.to_le_bytes());
        data[20..24].copy_from_slice(&(526 + body.len() as u32).to_le_bytes());
        data.extend(body);
        data
    }

    #[test]
    fn imports_text_and_formatting_into_reopenable_docx() {
        let data = fixture(b"Normal\x80\xf2\x0c\xf2Bold\xf3\x0c\xf3\x80\xf2\x08\xf2Italic\xf3\x08\xf3\x80\xf2\x0e\xf2Underlined\xf3\x0e\xf3\xccCaf\x0f\xc7Next");
        let opened = open(&data).unwrap();
        let flow = opened.presentation.flow.unwrap();
        let [
            Block::Paragraph(first),
            Block::Paragraph(second),
            Block::Paragraph(third),
        ] = flow.body.as_slice()
        else {
            panic!(
                "expected three editable paragraphs, got {}",
                flow.body.len()
            );
        };
        let texts = |paragraph: &FlowParagraph| {
            paragraph
                .runs
                .iter()
                .filter_map(|run| match &run.kind {
                    FlowRunKind::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>()
        };
        assert_eq!(texts(first), "Normal Bold Italic Underlined");
        assert_eq!(texts(second), "Café");
        assert_eq!(texts(third), "Next");
        assert!(third.page_break_before);
        for (word, check) in [
            (
                "Bold",
                (|s: &FlowRunStyle| s.bold) as fn(&FlowRunStyle) -> bool,
            ),
            ("Italic", |s| s.italic),
            ("Underlined", |s| s.underline),
        ] {
            assert!(
                first
                    .runs
                    .iter()
                    .any(|run| { run.kind == FlowRunKind::Text(word.into()) && check(&run.style) }),
                "missing formatting for {word}"
            );
        }
    }

    fn record(group: u8, kind: u8, flags: u8, payload: &[u8]) -> Vec<u8> {
        let size = (10 + payload.len() as u16).to_le_bytes();
        let mut record = vec![group, kind, size[0], size[1], flags];
        record.extend((payload.len() as u16).to_le_bytes());
        record.extend(payload);
        record.extend(size);
        record.push(group);
        record
    }

    fn table_fixture(across: u8, explicit_coverage: bool) -> Vec<u8> {
        let mut body = b"Before\xcc".to_vec();
        body.extend(record(0xd4, 0x2a, 3, &[0; 17]));
        let mut column = [0; 17];
        column[1..3].copy_from_slice(&1500u16.to_le_bytes());
        for _ in 0..3 {
            body.extend(record(0xd4, 0x2c, 0, &column));
        }
        body.extend(record(0xd4, 0x2b, 3, &[]));
        // First grid cell spans two columns and two rows. Covered slots can
        // appear as explicit markers, or be implied by the first cell.
        body.extend(record(0xd0, 11, 0, &[0, 0, 0x85, across, 2, 0x85]));
        body.extend(b"\xf2\x0c\xf2Merged\xf3\x0c\xf3\xccLabel");
        if explicit_coverage {
            body.extend(record(0xd0, 10, 0, &[0, 0, 0x85, 0x82, 2, 0x85]));
        }
        body.extend(b"\xc6Right");
        if explicit_coverage {
            body.extend(record(0xd0, 11, 0, &[0, 0, 0x85, 2, 0x82, 0x85]));
            body.extend(record(0xd0, 10, 0, &[0, 0, 0x85, 0x82, 0x82, 0x85]));
        }
        // This cell uses its own italic default, centered horizontally and vertically.
        body.extend(record(
            0xd0,
            if explicit_coverage { 10 } else { 11 },
            0,
            &[0, 0, 0x84, 3, 2, 1, 0, 1, 0, 0, 0x84],
        ));
        body.extend(b"Other\xc5One\xc6\xf2\x0e\xf2Two\xf3\x0e\xf3\xc6Three\xbfAfter");
        fixture(&body)
    }

    #[test]
    fn preserves_cell_grid_merges_and_format_in_docx() {
        use ibis_doc_model::flow::{CellAnchor, FlowAlign, VerticalMerge};

        for explicit_coverage in [true, false] {
            let opened = open(&table_fixture(2, explicit_coverage)).unwrap();
            let flow = opened.presentation.flow.unwrap();
            let [
                Block::Paragraph(_),
                Block::Table(table),
                Block::Paragraph(_),
            ] = flow.body.as_slice()
            else {
                panic!("expected one table between paragraphs");
            };
            assert_eq!(table.columns, vec![1_143_000; 3]);
            assert_eq!(
                table
                    .rows
                    .iter()
                    .map(|row| row.cells.len())
                    .collect::<Vec<_>>(),
                [2, 2, 3]
            );
            for (row, expected) in [(0, VerticalMerge::Start), (1, VerticalMerge::Continue)] {
                let cell = &table.rows[row].cells[0];
                assert_eq!((cell.span, cell.merge), (2, expected));
                if expected == VerticalMerge::Continue {
                    assert!(
                        matches!(cell.blocks.as_slice(), [Block::Paragraph(p)] if p.runs.is_empty())
                    );
                }
            }
            let merged = &table.rows[0].cells[0];
            let [Block::Paragraph(first), Block::Paragraph(second)] = merged.blocks.as_slice()
            else {
                panic!("expected two paragraphs inside the merged cell");
            };
            assert_eq!(first.runs[0].kind, FlowRunKind::Text("Merged".into()));
            assert!(first.runs[0].style.bold);
            assert_eq!(second.runs[0].kind, FlowRunKind::Text("Label".into()));
            assert!(!second.runs[0].style.bold);

            let other = &table.rows[1].cells[1];
            assert_eq!(other.anchor, CellAnchor::Center);
            let Block::Paragraph(paragraph) = &other.blocks[0] else {
                panic!("expected a paragraph")
            };
            assert_eq!(paragraph.align, FlowAlign::Center);
            assert_eq!(paragraph.runs[0].kind, FlowRunKind::Text("Other".into()));
            assert!(paragraph.runs[0].style.italic);
            let Block::Paragraph(paragraph) = &table.rows[2].cells[1].blocks[0] else {
                panic!("expected a paragraph");
            };
            assert_eq!(paragraph.runs[0].kind, FlowRunKind::Text("Two".into()));
            assert!(paragraph.runs[0].style.underline);
        }
        assert!(open(&table_fixture(4, false)).is_err());
    }

    #[test]
    fn saves_wp6_table_colors_and_line_styles_through_docx() {
        use ibis_doc_model::{
            Rgb,
            flow::{BorderStyle, VerticalMerge},
        };

        let data = include_bytes!("../../../test/fixtures/wordperfect/appearance.wpd");
        let opened = open(data).unwrap();
        assert_eq!(opened.import_warning, None);
        let flow = opened.presentation.flow.unwrap();
        let [
            Block::Paragraph(_),
            Block::Table(table),
            Block::Paragraph(_),
        ] = flow.body.as_slice()
        else {
            panic!("expected the WordPerfect table between body paragraphs");
        };
        let anchor = &table.rows[0].cells[0];
        let continuation = &table.rows[1].cells[0];
        assert_eq!((anchor.span, anchor.merge), (2, VerticalMerge::Start));
        assert_eq!(
            (continuation.span, continuation.merge),
            (2, VerticalMerge::Continue)
        );
        assert_eq!(anchor.shading, Some(Rgb(0xffe599)));
        assert_eq!(continuation.shading, anchor.shading);
        assert_eq!(table.rows[0].cells[1].shading, Some(Rgb(0xe2f0d9)));
        assert_eq!(table.rows[1].cells[1].shading, Some(Rgb(0xddebf7)));

        for (cell, side, style, eighth_points) in [
            (anchor, 0, BorderStyle::Double, 9),
            (anchor, 3, BorderStyle::Dotted, 6),
            (continuation, 2, BorderStyle::Dashed, 12),
        ] {
            let border = cell.own_borders[side].flatten().unwrap();
            assert_eq!(border.style, style);
            assert_eq!(border.color, Rgb(0x6400b4));
            assert_eq!((border.width * 8 + 6350) / 12700, eighth_points);
        }
        assert_eq!(continuation.own_borders[0], Some(None));
    }

    #[test]
    fn saves_inline_and_floating_wp6_pictures_in_docx() {
        use ibis_doc_model::{EmuSize, MediaFormat, ShapeKind};

        let data = include_bytes!("../../../test/fixtures/wordperfect/images.wpd");
        let opened = open(data).unwrap();
        let flow = opened.presentation.flow.as_ref().unwrap();
        let [
            Block::Paragraph(inline),
            Block::Paragraph(floating),
            Block::Paragraph(_),
        ] = flow.body.as_slice()
        else {
            panic!("expected two image-bearing paragraphs");
        };
        let (media, size) = inline
            .runs
            .iter()
            .find_map(|run| match run.kind {
                FlowRunKind::Picture { media, size, .. } => Some((media, size)),
                _ => None,
            })
            .unwrap();
        assert_eq!(size, EmuSize::new(1_371_600, 1_028_700));
        let floating = floating
            .runs
            .iter()
            .find_map(|run| match &run.kind {
                FlowRunKind::Shape(shape) => Some(shape),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            EmuSize::new(floating.frame.width, floating.frame.height),
            EmuSize::new(1_828_800, 1_371_600)
        );
        assert!(matches!(&floating.kind, ShapeKind::Picture(_)));
        let anchor = floating.floating.as_ref().unwrap();
        assert_eq!(
            (anchor.relative_h.as_str(), anchor.offset_x),
            ("page", 2_514_600)
        );
        assert_eq!(
            (anchor.relative_v.as_str(), anchor.offset_y),
            ("paragraph", 266_700)
        );

        let picture = opened
            .presentation
            .media
            .iter()
            .find(|picture| picture.id == media)
            .unwrap();
        assert_eq!(picture.format, MediaFormat::Png);
        let bitmap = image::load_from_memory(&picture.data).unwrap().to_rgba8();
        assert_eq!(bitmap.dimensions(), (8, 6));
        for (x, y, expected) in [
            (0, 0, [220, 30, 50, 255]),
            (7, 0, [40, 180, 60, 255]),
            (0, 5, [35, 70, 210, 255]),
            (7, 5, [244, 195, 30, 255]),
        ] {
            assert_eq!(bitmap.get_pixel(x, y).0, expected);
        }
    }

    #[test]
    fn preserves_wpg2_vector_and_bitmap_paint_order_in_docx() {
        use ibis_doc_model::{EmuSize, MediaFormat};

        let data = include_bytes!("../../../test/fixtures/wordperfect/mixed-wpg2.wpd");
        let opened = open(data).unwrap();
        let flow = opened.presentation.flow.as_ref().unwrap();
        let Block::Paragraph(paragraph) = &flow.body[0] else {
            panic!("expected inline picture paragraph");
        };
        let (media, size) = paragraph
            .runs
            .iter()
            .find_map(|run| match run.kind {
                FlowRunKind::Picture { media, size, .. } => Some((media, size)),
                _ => None,
            })
            .unwrap();
        assert_eq!(size, EmuSize::new(1_371_600, 1_028_700));
        let media = opened
            .presentation
            .media
            .iter()
            .find(|item| item.id == media)
            .unwrap();
        assert_eq!(media.format, MediaFormat::Png);
        let pixels = image::load_from_memory(&media.data).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (300, 225));
        for (x, y, expected) in [
            (0, 0, [0, 0, 0, 0]),
            (15, 45, [236, 236, 236, 255]),
            (50, 40, [214, 46, 70, 255]),
            (205, 52, [24, 133, 205, 255]),
            (230, 120, [131, 47, 182, 255]),
            (60, 110, [220, 30, 50, 255]),
            (150, 110, [40, 180, 60, 255]),
            (60, 175, [35, 70, 210, 255]),
            (151, 175, [244, 195, 30, 255]),
            (130, 150, [232, 136, 32, 255]),
            (237, 190, [15, 110, 80, 255]),
        ] {
            assert_eq!(pixels.get_pixel(x, y).0, expected, "pixel at ({x}, {y})");
        }
    }

    #[test]
    fn transforms_nested_wpg2_bitmap_and_vectors_in_paint_order() {
        let data = include_bytes!("../../../test/fixtures/wordperfect/grouped-wpg2.wpd");
        let opened = open(data).unwrap();
        let flow = opened.presentation.flow.as_ref().unwrap();
        let Block::Paragraph(paragraph) = &flow.body[0] else {
            panic!("expected inline picture paragraph");
        };
        let media = paragraph
            .runs
            .iter()
            .find_map(|run| match run.kind {
                FlowRunKind::Picture { media, .. } => Some(media),
                _ => None,
            })
            .unwrap();
        let media = opened
            .presentation
            .media
            .iter()
            .find(|item| item.id == media)
            .unwrap();
        let picture = image::load_from_memory(&media.data).unwrap().to_rgba8();
        assert_eq!(picture.dimensions(), (300, 225));
        for (x, y, expected) in [
            (35, 50, [24, 133, 205, 255]), // Outside the group: unchanged.
            (110, 160, [0, 0, 0, 0]),
            (172, 30, [236, 236, 236, 255]),
            (179, 109, [131, 47, 182, 255]),
            (206, 75, [40, 180, 60, 255]),
            (206, 137, [220, 30, 50, 255]),
            (242, 53, [244, 195, 30, 255]),
            (242, 143, [35, 70, 210, 255]),
            (222, 90, [232, 136, 32, 255]), // Vector in front of the transformed bitmap.
        ] {
            assert_eq!(picture.get_pixel(x, y).0, expected, "pixel at ({x}, {y})");
        }
    }

    #[test]
    fn wpg2_linear_stops_and_centered_radial_fill_survive_docx_conversion() {
        let data = include_bytes!("../../../test/fixtures/wordperfect/gradient-stops-wpg2.wpd");
        let opened = open(data).unwrap();
        assert_eq!(opened.import_warning, None);
        let media = opened.presentation.media.first().unwrap();
        let pixels = image::load_from_memory(&media.data).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (300, 225));

        // The green stop is 37.5% across the rectangle, before its midpoint.
        let green = pixels.get_pixel(63, 112).0;
        assert!(green[0] < 30 && green[1] > 215 && green[2].abs_diff(100) <= 4);
        assert!(pixels.get_pixel(30, 112).0[2] > 170);
        assert!(pixels.get_pixel(78, 112).0[1] < 200);
        assert!(pixels.get_pixel(130, 112).0[0] > 200);

        // The circle has a red center and a blue rim.
        let center = pixels.get_pixel(220, 112).0;
        assert!(center[0] > 225 && center[2] < 51);
        for (x, y) in [(249, 112), (220, 142), (190, 112), (220, 82)] {
            let pixel = pixels.get_pixel(x, y).0;
            assert!(pixel[0].abs_diff(131) <= 4 && pixel[2].abs_diff(125) <= 4);
        }
        assert!(pixels.get_pixel(275, 112).0[2] > 188);
        assert_eq!(pixels.get_pixel(284, 112).0, [0, 0, 0, 0]);
    }

    #[test]
    fn wpg2_linear_gradients_survive_docx_conversion() {
        let data = include_bytes!("../../../test/fixtures/wordperfect/linear-gradient-wpg2.wpd");
        let opened = open(data).unwrap();
        assert_eq!(opened.import_warning, None);
        let media = opened.presentation.media.first().unwrap();
        let pixels = image::load_from_memory(&media.data).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (300, 225));
        for (x, y, rgb) in [
            (70, 58, [131, 80, 125]),
            (215, 58, [123, 81, 131]),
            (78, 166, [131, 80, 125]),
            (215, 165, [55, 150, 80]),
        ] {
            let pixel = pixels.get_pixel(x, y).0;
            assert_eq!(pixel[3], 255);
            for (actual, expected) in pixel[..3].iter().zip(rgb) {
                assert!(
                    actual.abs_diff(expected) <= 4,
                    "pixel at ({x}, {y}): {pixel:?}"
                );
            }
        }
        assert!(pixels.get_pixel(70, 20).0[0] > 205);
        assert!(pixels.get_pixel(70, 97).0[0] < 45);
        assert!(pixels.get_pixel(165, 58).0[0] < 45);
        assert!(pixels.get_pixel(275, 58).0[0] > 215);
        assert!(pixels.get_pixel(130, 125).0[0] > 205);
        assert!(pixels.get_pixel(20, 205).0[0] < 55);
    }

    #[test]
    fn wpg2_pen_patterns_survive_docx_conversion() {
        let dashed = include_bytes!("../../../test/fixtures/wordperfect/dashed-grid-wpg2.wpd");
        let dash_dot = include_bytes!("../../../test/fixtures/wordperfect/dash-dot-grid-wpg2.wpd");
        for (data, samples) in [
            (
                dashed.as_slice(),
                [
                    (45, 55, true),
                    (90, 55, false),
                    (90, 130, true),
                    (110, 55, true),
                ],
            ),
            (
                dash_dot.as_slice(),
                [
                    (85, 55, true),
                    (100, 55, false),
                    (125, 55, true),
                    (76, 130, false),
                ],
            ),
        ] {
            let opened = open(data).unwrap();
            assert_eq!(opened.import_warning, None);
            let media = opened.presentation.media.first().unwrap();
            let pixels = image::load_from_memory(&media.data).unwrap().to_rgba8();
            for (x, y, ink) in samples {
                let expected = if ink {
                    [15, 110, 80, 255]
                } else {
                    [0, 0, 0, 0]
                };
                assert_eq!(pixels.get_pixel(x, y).0, expected, "pixel at ({x}, {y})");
            }
        }
    }

    #[test]
    fn reports_omitted_graphics_and_keeps_the_other_image_and_text() {
        let data =
            include_bytes!("../../../test/fixtures/wordperfect/partially-unsupported-wpg2.wpd");
        let opened = open(data).unwrap();
        assert_eq!(
            opened.import_warning,
            Some(crate::ImportWarning::WordPerfectImagesOmitted(1)),
        );
        let bytes = to_docx(data).unwrap();
        let saved = crate::open(bytes).unwrap();
        assert_eq!(saved.import_warning, None);
        for document in [opened, saved] {
            let flow = document.presentation.flow.as_ref().unwrap();
            let [
                Block::Paragraph(inline),
                Block::Paragraph(floating),
                Block::Paragraph(last),
            ] = flow.body.as_slice()
            else {
                panic!("expected all three paragraphs");
            };
            assert!(
                inline
                    .runs
                    .iter()
                    .any(|run| matches!(run.kind, FlowRunKind::Picture { .. }))
            );
            assert!(
                floating
                    .runs
                    .iter()
                    .all(|run| !matches!(run.kind, FlowRunKind::Shape(_)))
            );
            for (paragraph, expected) in [
                (inline, "Inline:"),
                (floating, "Floating anchor"),
                (last, "Last"),
            ] {
                let text: String = paragraph
                    .runs
                    .iter()
                    .filter_map(|run| match &run.kind {
                        FlowRunKind::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                assert!(text.contains(expected), "{text:?}");
            }
        }
    }

    #[test]
    fn wpg2_curves_and_compound_holes_survive_docx_conversion() {
        let data = include_bytes!("../../../test/fixtures/wordperfect/paths-wpg2.wpd");
        let opened = open(data).unwrap();
        let flow = opened.presentation.flow.as_ref().unwrap();
        let Block::Paragraph(paragraph) = &flow.body[0] else {
            panic!("expected inline picture paragraph");
        };
        let media = paragraph
            .runs
            .iter()
            .find_map(|run| match run.kind {
                FlowRunKind::Picture { media, .. } => Some(media),
                _ => None,
            })
            .unwrap();
        let media = opened
            .presentation
            .media
            .iter()
            .find(|item| item.id == media)
            .unwrap();
        let pixels = image::load_from_memory(&media.data).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (300, 225));
        for (x, y, expected) in [
            (84, 33, [192, 46, 78, 255]),   // Cubic stroke.
            (211, 29, [25, 110, 192, 255]), // The top of the open arc.
            (211, 33, [236, 236, 236, 255]),
            (25, 150, [112, 57, 184, 255]),
            (85, 145, [240, 184, 62, 255]), // Odd-even hole over a prior object.
            (180, 150, [35, 153, 96, 255]),
            (225, 145, [236, 236, 236, 255]), // Opposite winding leaves a hole.
            (145, 175, [222, 89, 28, 255]),   // Later object covers the compound edge.
        ] {
            assert_eq!(pixels.get_pixel(x, y).0, expected, "pixel at ({x}, {y})");
        }
    }

    #[test]
    fn handles_framed_breaks_and_rejects_invalid_records() {
        let data =
            fixture(b"A\xd0\x04\x09\x00\x00\x00\x09\x00\xd0\xd0\x09\x09\x00\x00\x00\x09\x00\xd0B");
        let flow = read(&data).unwrap();
        assert_eq!(flow.body.len(), 2);
        assert!(matches!(&flow.body[1], Block::Paragraph(p) if p.page_break_before));

        for body in [
            b"\xf2\x0c".as_slice(),
            b"\xf2\x0c\xf3",
            b"\xd0\x04\x01\x00",
            b"\xd0\x04\x08\x00\x00\x00\x00\xd0",
        ] {
            assert!(read(&fixture(body)).is_err());
        }
        let mut encrypted = fixture(b"Text");
        encrypted[12] = 1;
        assert!(read(&encrypted).is_err());
        encrypted[12] = 0;
        encrypted[10] = 0;
        assert!(read(&encrypted).is_err());
        encrypted[10] = 2;
        encrypted[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read(&encrypted).is_err());
    }
}
