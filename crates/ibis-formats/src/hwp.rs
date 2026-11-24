// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Binary Hangul 5.x import from Hancom's published file-format specification.
//!
//! 본 제품은 한컴의 HWP 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::HashMap;

use ibis_doc_model::{
    AutoShape, Block, BorderStyle, CellAnchor, Color, Emu, EmuRect, EmuSize, Fill, FloatingAnchor,
    FlowAlign, FlowBorder, FlowCell, FlowDocument, FlowParagraph, FlowRow, FlowRun, FlowRunKind,
    FlowTable, Geometry, GroupMember, LineDash, LineHeight, ListLevel, ListSuffix, MediaFormat,
    MediaId, Numbering, PageSetup, ParaFormat, PartKind, Presentation, Rgb, RowHeight, RunFormat,
    SectionBreak, SectionSetup, Shape, ShapeGroup, ShapeKind, Stroke, VerticalMerge,
};

use crate::{
    Error, Opened, hangul_structure as shared,
    hwpx::{line_pitch, units},
    ibis_ooxml::cfb,
};

pub const SPEC_CREDIT: &str =
    "본 제품은 한컴의 HWP 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.";

const SIGNATURE: &[u8] = b"HWP Document File\0";
const STREAM_LIMIT: usize = 16 * 1024 * 1024;
const BODY_LIMIT: usize = 64 * 1024 * 1024;
const PARA: u16 = 66;
const TEXT: u16 = 67;
const CHAR_RUNS: u16 = 68;
const CONTROL: u16 = 71;
const LIST: u16 = 72;
const PAGE: u16 = 73;
const TABLE: u16 = 77;
const COMPONENT: u16 = 76;
const PICTURE: u16 = 85;

/// Hancom input formats that open as editable DOCX documents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Hwp,
    Hwpx,
}

impl Format {
    /// Select the importer. An extension hint also routes malformed files to
    /// the format-specific error. Preserve the existing binary HWP precedence.
    pub fn detect(extension: Option<&str>, data: &[u8]) -> Option<Self> {
        if extension.is_some_and(is_extension) || is_hwp(data) {
            Some(Self::Hwp)
        } else if extension.is_some_and(crate::hwpx::is_extension) || crate::hwpx::is_hwpx(data) {
            Some(Self::Hwpx)
        } else {
            None
        }
    }

    pub fn open(self, data: &[u8]) -> Result<Opened, Error> {
        match self {
            Self::Hwp => open(data),
            Self::Hwpx => crate::hwpx::open(data),
        }
    }
}

pub fn is_hwp(data: &[u8]) -> bool {
    cfb::is_cfb(data)
        && cfb::stream_path(data, "FileHeader").is_ok_and(|h| h.starts_with(SIGNATURE))
}

pub fn is_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("hwp")
}

pub fn open(data: &[u8]) -> Result<Opened, Error> {
    crate::open(to_docx(data)?)
}

pub fn to_docx(data: &[u8]) -> Result<Vec<u8>, Error> {
    crate::hwpx::document_to_docx(|presentation| read_document(data, presentation))
}

fn malformed() -> Error {
    Error::new("HWP 5.0 の文書データが不正です")
}

fn word(data: &[u8], offset: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(
        data.get(offset..offset + 2)
            .ok_or_else(malformed)?
            .try_into()
            .unwrap(),
    ))
}

fn dword(data: &[u8], offset: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        data.get(offset..offset + 4)
            .ok_or_else(malformed)?
            .try_into()
            .unwrap(),
    ))
}

fn dimension(data: &[u8], offset: usize) -> Result<Emu, Error> {
    Ok(units(i64::from(dword(data, offset)? as i32)))
}

fn read_stream(data: &[u8], name: &str, compressed: bool, limit: usize) -> Result<Vec<u8>, Error> {
    let bytes = cfb::stream_path(data, name).map_err(|e| Error::new(e.to_string()))?;
    let limit = limit.min(STREAM_LIMIT);
    if compressed {
        miniz_oxide::inflate::decompress_to_vec_with_limit(&bytes, limit)
            .map_err(|_| Error::new(format!("HWP 5.0: {name} を展開できません")))
    } else if bytes.len() <= limit {
        Ok(bytes)
    } else {
        Err(Error::new(format!("HWP 5.0: {name} が大きすぎます")))
    }
}

#[derive(Clone, Copy)]
struct Record<'a> {
    tag: u16,
    level: u16,
    bytes: &'a [u8],
    end: usize,
}

fn records(mut bytes: &[u8]) -> Result<Vec<Record<'_>>, Error> {
    let mut out: Vec<Record<'_>> = Vec::new();
    let mut parents: Vec<usize> = Vec::new();
    while !bytes.is_empty() {
        let header = dword(bytes, 0)?;
        let level = ((header >> 10) & 1023) as u16;
        let size = header >> 20;
        let (offset, size) = if size == 4095 {
            (8, dword(bytes, 4)?)
        } else {
            (4, size)
        };
        bytes = bytes.get(offset..).ok_or_else(malformed)?;
        let (body, tail) = bytes
            .split_at_checked(size as usize)
            .ok_or_else(malformed)?;
        bytes = tail;
        if usize::from(level) > parents.len() || level > 64 || out.len() >= 250_000 {
            return Err(malformed());
        }
        while parents.len() > usize::from(level) {
            out[parents.pop().unwrap()].end = out.len();
        }
        parents.push(out.len());
        out.push(Record {
            tag: (header & 1023) as u16,
            level,
            bytes: body,
            end: 0,
        });
    }
    for parent in parents {
        out[parent].end = out.len();
    }
    Ok(out)
}

fn read_document(data: &[u8], presentation: &mut Presentation) -> Result<FlowDocument, Error> {
    if data.len() > BODY_LIMIT {
        return Err(Error::new("HWP 5.0: ファイルが大きすぎます"));
    }
    let header = cfb::stream_path(data, "FileHeader").map_err(|e| Error::new(e.to_string()))?;
    if !header.starts_with(SIGNATURE) || header.get(35) != Some(&5) {
        return Err(Error::new("HWP 5.0 のファイルではありません"));
    }
    let version = dword(&header, 32)?;
    let flags = dword(&header, 36)?;
    if flags & 0x0516 != 0 {
        return Err(Error::new(
            "暗号化・配布用・DRM 付き HWP 5.0 には対応していません",
        ));
    }
    let compressed = flags & 1 != 0;
    let info = read_stream(data, "DocInfo", compressed, STREAM_LIMIT)?;
    let mut importer = Importer::new(&records(&info)?, version, data, presentation, compressed)?;
    let mut sections: Vec<_> = cfb::directory(data)
        .map_err(|e| Error::new(e.to_string()))?
        .into_iter()
        .filter_map(|entry| {
            let index = entry
                .path
                .strip_prefix("BodyText/Section")?
                .parse::<u32>()
                .ok()?;
            (!entry.is_storage).then_some((index, entry.path))
        })
        .collect();
    sections.sort_unstable();
    if sections.is_empty()
        || sections.len() > 256
        || sections.windows(2).any(|pair| pair[0].0 == pair[1].0)
    {
        return Err(malformed());
    }
    let count = sections.len();
    let mut remaining = BODY_LIMIT;
    let mut document = FlowDocument::default();
    for (index, (_, path)) in sections.into_iter().enumerate() {
        let bytes = read_stream(data, &path, compressed, remaining)?;
        remaining -= bytes.len();
        let section = records(&bytes)?;
        let page = section
            .iter()
            .find(|record| record.tag == PAGE)
            .map(|record| page_setup(record.bytes))
            .transpose()?
            .unwrap_or(document.page);
        let setup = importer.section_setup(&section, page)?;
        document.even_and_odd |= setup.parts[PartKind::HeaderEven.index()].is_some()
            || setup.parts[PartKind::FooterEven.index()].is_some();
        let mut body = Vec::new();
        let mut cursor = 0;
        while cursor < section.len() {
            let record = section[cursor];
            if record.tag == PARA {
                importer.paragraph(&section, cursor, &mut body, 0, false)?;
            }
            cursor = record.end;
        }
        if index + 1 < count {
            if !matches!(body.last(), Some(Block::Paragraph(_))) {
                body.push(Block::Paragraph(importer.new_paragraph()));
            }
            if let Some(Block::Paragraph(last)) = body.last_mut() {
                last.section_break = SectionBreak::NextPage;
                last.section = Some(setup.into());
            }
            document.page = page;
        } else {
            shared::set_last_section(&mut document, setup);
        }
        document.body.extend(body);
    }
    document.shape_groups = importer.shape_groups;
    std::sync::Arc::make_mut(&mut document.styles).numbering = importer.numbering;
    document.id_floor = importer.id;
    Ok(document)
}

fn page_setup(bytes: &[u8]) -> Result<PageSetup, Error> {
    let (width, height) = (dimension(bytes, 0)?, dimension(bytes, 4)?);
    if width <= 0 || height <= 0 {
        return Err(malformed());
    }
    let size = if dword(bytes, 36)? & 1 == 0 {
        EmuSize::new(width, height)
    } else {
        EmuSize::new(height, width)
    };
    let left = dimension(bytes, 8)?.max(0);
    let right = dimension(bytes, 12)?.max(0);
    let header = dimension(bytes, 24)?.max(0);
    let footer = dimension(bytes, 28)?.max(0);
    Ok(PageSetup {
        size,
        left,
        right,
        header,
        footer,
        top: dimension(bytes, 16)?.max(0) + header,
        bottom: dimension(bytes, 20)?.max(0) + footer,
        gutter: dimension(bytes, 32)?.max(0),
        ..Default::default()
    })
}

type CellFill = ([Option<FlowBorder>; 4], Option<Rgb>);

struct EmbeddedPicture {
    path: String,
    format: MediaFormat,
    compressed: bool,
}

struct Importer<'a> {
    id: u32,
    runs: Vec<RunFormat>,
    paragraphs: Vec<ParaFormat>,
    heading_refs: Vec<(u32, u16, u32)>,
    numbering: Numbering,
    list_ids: Vec<u32>,
    bullet_ids: Vec<u32>,
    outline: u16,
    fills: Vec<CellFill>,
    data: &'a [u8],
    presentation: &'a mut Presentation,
    pictures: Vec<Option<EmbeddedPicture>>,
    media: HashMap<u16, MediaId>,
    remaining_media: usize,
    remaining_warp: usize,
    remaining_cells: usize,
    shape_groups: Vec<ShapeGroup>,
}

impl<'a> Importer<'a> {
    fn new(
        info: &[Record<'_>],
        version: u32,
        data: &'a [u8],
        presentation: &'a mut Presentation,
        compressed: bool,
    ) -> Result<Self, Error> {
        let mut fonts = Vec::new();
        let mut korean_count = 0;
        for record in info {
            match record.tag {
                17 => korean_count = dword(record.bytes, 4)? as usize,
                19 => {
                    let length = usize::from(word(record.bytes, 1)?);
                    let bytes = record.bytes.get(3..3 + 2 * length).ok_or_else(malformed)?;
                    fonts.push(String::from_utf16_lossy(&utf16(bytes)?));
                }
                _ => {}
            }
        }
        let mut importer = Self {
            id: 10,
            runs: Vec::new(),
            paragraphs: Vec::new(),
            heading_refs: Vec::new(),
            numbering: Numbering::default(),
            list_ids: Vec::new(),
            bullet_ids: Vec::new(),
            outline: 0,
            fills: Vec::new(),
            data,
            presentation,
            pictures: Vec::new(),
            media: HashMap::new(),
            remaining_media: BODY_LIMIT,
            remaining_warp: BODY_LIMIT,
            remaining_cells: 100_000,
            shape_groups: Vec::new(),
        };
        for record in info {
            let bytes = record.bytes;
            match record.tag {
                18 => importer.pictures.push(embedded_picture(bytes, compressed)?),
                20 => {
                    let shading = if dword(bytes, 32)? & 1 != 0 {
                        let fill = dword(bytes, 36)?;
                        (fill != u32::MAX).then(|| rgb(fill))
                    } else {
                        None
                    };
                    importer.fills.push((read_borders(bytes)?, shading));
                }
                21 => {
                    let flags = dword(bytes, 46)?;
                    let size = dword(bytes, 42)?.clamp(100, 163_800);
                    let scale = u32::from(bytes[14]).clamp(50, 200);
                    let spacing =
                        units(i64::from(bytes[21] as i8).clamp(-50, 50) * i64::from(size)) / 100;
                    importer.runs.push(RunFormat {
                        font: korean_count
                            .checked_add(usize::from(word(bytes, 2)?))
                            .and_then(|id| fonts.get(id))
                            .cloned(),
                        east_asian_font: fonts.get(usize::from(word(bytes, 0)?)).cloned(),
                        size: Some(size),
                        scale: Some(scale),
                        spacing: Some(spacing),
                        italic: Some(flags & 1 != 0),
                        bold: Some(flags & 2 != 0),
                        underline: Some(matches!((flags >> 2) & 3, 1 | 3)),
                        color: Some(rgb(dword(bytes, 52)?)),
                        ..Default::default()
                    });
                }
                23 => {
                    let id = binary_numbering(bytes, &importer.runs)
                        .ok()
                        .map_or(0, |levels| {
                            shared::add_list(&mut importer.numbering, levels)
                        });
                    importer.list_ids.push(id);
                }
                24 => {
                    let extended = bytes.len() >= 14;
                    let at = if extended { 12 } else { 8 };
                    let id = word(bytes, at)
                        .ok()
                        .and_then(|unit| char::from_u32(u32::from(unit)))
                        .map_or(0, |ch| {
                            let mut level = shared::bullet_level(ch);
                            if extended {
                                level.run = dword(bytes, 8)
                                    .ok()
                                    .and_then(|id| importer.runs.get(id as usize))
                                    .cloned()
                                    .unwrap_or_default();
                            }
                            shared::add_list(&mut importer.numbering, vec![level])
                        });
                    importer.bullet_ids.push(id);
                }
                25 => {
                    importer.paragraphs.push(para_format(bytes, version)?);
                    let flags = dword(bytes, 0)?;
                    importer.heading_refs.push((
                        (flags >> 23) & 3,
                        word(bytes, 30)?,
                        (flags >> 25) & 7,
                    ));
                }
                _ => {}
            }
        }
        Ok(importer)
    }

    fn next_id(&mut self) -> u32 {
        self.id += 1;
        self.id
    }

    fn new_paragraph(&mut self) -> FlowParagraph {
        FlowParagraph {
            id: self.next_id(),
            ..Default::default()
        }
    }

    fn section_setup(
        &mut self,
        records: &[Record<'_>],
        page: PageSetup,
    ) -> Result<SectionSetup, Error> {
        let mut setup = SectionSetup {
            page,
            ..Default::default()
        };
        let definition = records
            .iter()
            .find(|r| r.tag == CONTROL && r.level == 1 && r.bytes.starts_with(b"dces"));
        self.outline = definition.and_then(|r| word(r.bytes, 18).ok()).unwrap_or(0);
        if let Some(restart) = definition
            .and_then(|r| word(r.bytes, 20).ok())
            .filter(|&n| n != 0)
        {
            setup.page.first_page = u32::from(restart);
            setup.restart_numbering = true;
        }
        let mut page_number = None;
        let first_end = records
            .iter()
            .find(|r| r.tag == PARA && r.level == 0)
            .map_or(0, |r| r.end);
        for (index, control) in records
            .iter()
            .enumerate()
            .filter(|(_, r)| r.tag == CONTROL && r.level == 1)
        {
            let bytes = control.bytes;
            match bytes.get(..4) {
                Some(b"onwn")
                    if index < first_end
                        && dword(bytes, 4)? & 15 == 0
                        && !setup.restart_numbering =>
                {
                    let restart = word(bytes, 8)?;
                    if restart != 0 {
                        setup.page.first_page = u32::from(restart);
                        setup.restart_numbering = true;
                    }
                }
                Some(b"daeh" | b"toof") => {
                    let (regular, even) = if bytes.starts_with(b"daeh") {
                        (PartKind::Header, PartKind::HeaderEven)
                    } else {
                        (PartKind::Footer, PartKind::FooterEven)
                    };
                    let apply = dword(bytes, 4)? & 3;
                    for (kind, include) in [(regular, apply != 1), (even, apply != 2)] {
                        if include {
                            let mut blocks = Vec::new();
                            for cursor in index + 1..control.end {
                                let record = records[cursor];
                                if record.tag == PARA && record.level == control.level + 1 {
                                    self.paragraph(records, cursor, &mut blocks, 0, false)?;
                                }
                            }
                            setup.parts[kind.index()] = Some(blocks);
                        } else {
                            setup.parts[kind.index()].get_or_insert_with(Vec::new);
                        }
                    }
                }
                Some(b"pngp") => page_number = Some(bytes),
                _ => {}
            }
        }
        if let Some(bytes) = page_number.filter(|bytes| {
            dword(bytes, 4).is_ok_and(|flags| (1..=10).contains(&((flags >> 8) & 15)))
        }) {
            let flags = dword(bytes, 4)?;
            let position = (flags >> 8) & 15;

            let prefix = decoration(word(bytes, 10)?);
            let suffix = decoration(word(bytes, 12)?);
            let kinds = if matches!(position, 1..=3 | 7 | 9) {
                [PartKind::Header, PartKind::HeaderEven]
            } else {
                [PartKind::Footer, PartKind::FooterEven]
            };
            for (index, kind) in kinds.into_iter().enumerate() {
                let align = match (position, index) {
                    (1 | 4, _) | (7 | 8, 1) | (9 | 10, 0) => FlowAlign::Left,
                    (3 | 6, _) | (7 | 8, 0) | (9 | 10, 1) => FlowAlign::Right,
                    _ => FlowAlign::Center,
                };
                setup.parts[kind.index()]
                    .get_or_insert_with(Vec::new)
                    .push(shared::page_number(self.next_id(), align, &prefix, &suffix));
            }
            setup.page_number_format = Some(shared::number_format(flags & 255).into());
        }
        Ok(setup)
    }

    fn paragraph(
        &mut self,
        records: &[Record<'_>],
        start: usize,
        blocks: &mut Vec<Block>,
        depth: usize,
        last_in_cell: bool,
    ) -> Result<(), Error> {
        if depth > 16 {
            return Err(malformed());
        }
        let record = records[start];
        let mut paragraph = self.new_paragraph();
        paragraph.direct = self
            .paragraphs
            .get(usize::from(word(record.bytes, 8)?))
            .cloned()
            .unwrap_or_default();
        if let Some(&(kind, id, level)) = self.heading_refs.get(usize::from(word(record.bytes, 8)?))
        {
            let (ids, source_id) = match kind {
                1 => (&self.list_ids[..], self.outline),
                2 => (&self.list_ids[..], id),
                3 => (&self.bullet_ids[..], id),
                _ => (&[][..], 0),
            };
            if let Some(&list) = usize::from(source_id)
                .checked_sub(1)
                .and_then(|id| ids.get(id))
                .filter(|&&id| id != 0)
            {
                paragraph.direct.num_id = Some(list);
                paragraph.direct.level = Some(if kind == 3 { 0 } else { level });
            }
        }
        if record.bytes.get(11).ok_or_else(malformed)? & 4 != 0 {
            paragraph.direct.page_break_before = Some(true);
        }
        let mut cursor = start + 1;
        let mut text = &[][..];
        let mut styles = &[][..];
        let mut controls = Vec::new();
        while cursor < record.end {
            let child = records[cursor];
            match child.tag {
                TEXT => text = child.bytes,
                CHAR_RUNS => styles = child.bytes,
                69 if child.bytes.len() % 36 == 0 => {
                    let mut pitches = child.bytes.chunks_exact(36).map(|line| {
                        line_pitch(
                            i64::from(dword(line, 8).ok()? as i32),
                            if last_in_cell && child.bytes.len() == 36 {
                                0
                            } else {
                                i64::from(dword(line, 20).ok()? as i32)
                            },
                        )
                    });
                    if let Some(Some(pitch)) = pitches.next()
                        && pitches.all(|candidate| candidate == Some(pitch))
                    {
                        paragraph.direct.line = Some(pitch);
                    }
                }
                CONTROL => controls.push(cursor),
                _ => {}
            }
            cursor = child.end;
        }
        let text = utf16(text)?;
        if styles.len() % 8 != 0 {
            return Err(malformed());
        }
        let mut style_changes = styles.chunks_exact(8).peekable();
        let mut control_indices = controls.into_iter();
        let mut run_format = RunFormat::default();
        let mut buffered = Vec::new();
        let mut position = 0;
        let mut contained_table = false;
        while position < text.len() {
            while let Some(change) = style_changes.peek() {
                if dword(change, 0)? as usize > position {
                    break;
                }
                append_text(&mut paragraph, &mut buffered, &run_format);
                run_format = self
                    .runs
                    .get(dword(change, 4)? as usize)
                    .cloned()
                    .unwrap_or_default();
                if position == 0 {
                    paragraph.mark_format = run_format.clone();
                }
                style_changes.next();
            }
            let code = text[position];
            if code >= 32 {
                buffered.push(code);
                position += 1;
                continue;
            }
            append_text(&mut paragraph, &mut buffered, &run_format);
            match code {
                9 => add_run(&mut paragraph, FlowRunKind::Tab, &run_format),
                10 => add_run(&mut paragraph, FlowRunKind::LineBreak, &run_format),
                24 => add_run(&mut paragraph, FlowRunKind::Text("-".into()), &run_format),
                30 | 31 => add_run(
                    &mut paragraph,
                    FlowRunKind::Text("\u{a0}".into()),
                    &run_format,
                ),
                _ => {}
            }
            let extended = matches!(code, 2 | 3 | 11 | 12 | 14..=18 | 21..=23);
            if extended && let Some(control) = control_indices.next() {
                let bytes = records[control].bytes;
                if bytes.starts_with(b"onta") && dword(bytes, 4)? & 15 == 0 {
                    add_run(&mut paragraph, FlowRunKind::PageNumber, &run_format);
                } else if bytes.starts_with(b" osg") {
                    for kind in self.drawing(records, control)? {
                        add_run(&mut paragraph, kind, &run_format);
                    }
                } else if records[control].bytes.starts_with(b" lbt") {
                    if !paragraph.runs.is_empty() {
                        let mut next = self.new_paragraph();
                        next.direct = paragraph.direct.clone();
                        next.direct.page_break_before = None;
                        next.direct.num_id = None;
                        next.direct.level = None;
                        flush_paragraph(blocks, std::mem::replace(&mut paragraph, next));
                    }
                    if paragraph.direct.page_break_before.take() == Some(true) {
                        let mut page_break = self.new_paragraph();
                        page_break.direct.page_break_before = Some(true);
                        flush_paragraph(blocks, page_break);
                    }
                    blocks.push(Block::Table(self.table(records, control, depth + 1)?));
                    contained_table = true;
                }
            }
            position += if extended || matches!(code, 4..=9 | 19 | 20) {
                8
            } else {
                1
            };
            if position > text.len() {
                return Err(malformed());
            }
        }
        append_text(&mut paragraph, &mut buffered, &run_format);
        if !contained_table || !paragraph.runs.is_empty() {
            flush_paragraph(blocks, paragraph);
        }
        Ok(())
    }

    fn drawing(&mut self, records: &[Record<'_>], index: usize) -> Result<Vec<FlowRunKind>, Error> {
        let control = records[index];
        let mut runs = Vec::new();
        if let Some(root) = records
            .get(index + 1)
            .filter(|r| index + 1 < control.end && r.tag == COMPONENT)
        {
            let anchor = picture_anchor(control.bytes)?.unwrap_or_default();
            let size = EmuSize::new(dimension(control.bytes, 16)?, dimension(control.bytes, 20)?);
            if let Some(GroupMember::Group(group)) = self.component(
                records,
                index + 1,
                &anchor,
                &anchor,
                Some(size),
                &mut runs,
                0,
            ) {
                self.shape_groups.push(*group);
            }
            debug_assert!(root.end <= control.end);
        }
        if !runs.iter().any(
            |run| matches!(run, FlowRunKind::Shape(s) if matches!(s.kind, ShapeKind::Picture(_))),
        ) && let Some(picture) = self.picture(records, index)?
        {
            runs.push(picture);
        }
        Ok(runs)
    }

    fn component(
        &mut self,
        records: &[Record<'_>],
        index: usize,
        parent: &FloatingAnchor,
        root: &FloatingAnchor,
        root_size: Option<EmuSize>,
        runs: &mut Vec<FlowRunKind>,
        depth: usize,
    ) -> Option<GroupMember> {
        if depth >= 16 {
            return None;
        }
        let record = records[index];
        let kind = record.bytes.get(..4)?;
        let bytes = record
            .bytes
            .get(if root_size.is_some() { 8.. } else { 4.. })?;
        let coord = |offset| dimension(bytes, offset).ok();
        let mut anchor = parent.clone();
        let size = if let Some(size) = root_size {
            size
        } else {
            anchor = shared::offset_anchor(parent, coord(0)?, coord(4)?);
            EmuSize::new(coord(20)?, coord(24)?)
        };
        if kind == b"noc$" {
            if anchor.relative_h.is_empty() {
                anchor = shared::group_anchor();
            }
            let root = if root_size.is_some() { &anchor } else { root };
            let mut members = Vec::new();
            let mut child = index + 1;
            while child < record.end {
                let next = records[child];
                if next.tag == COMPONENT
                    && let Some(member) =
                        self.component(records, child, &anchor, root, None, runs, depth + 1)
                {
                    members.push(member);
                }
                child = next.end;
            }
            return (!members.is_empty())
                .then(|| GroupMember::Group(Box::new(shared::shape_group(members))));
        }
        let primitive_tag = match kind {
            b"nil$" => 78,
            b"cer$" => 79,
            b"lle$" => 80,
            b"ruc$" => 83,
            b"cip$" if root_size.is_none() => PICTURE,
            _ => return None,
        };
        let primitive = records[index + 1..record.end]
            .iter()
            .find(|r| r.tag == primitive_tag && r.level == record.level + 1)?
            .bytes;
        if primitive_tag != 78 && (size.width <= 0 || size.height <= 0) {
            return None;
        }
        let matrices = usize::from(word(bytes, 42).ok()?);
        let style = bytes.get(92 + 96 * matrices..)?;
        let line = dword(style, 8).unwrap_or(0) & 63;
        let stroke = (line != 0).then(|| {
            let color = Color::opaque(rgb(dword(style, 0).unwrap_or(0)));
            let width = word(style, 4).map_or(0, |n| units(i64::from(n as i16)).max(0));
            let dash = match line {
                2 | 5 | 6 => LineDash::Dash,
                3 => LineDash::Dot,
                4 => LineDash::DashDot,
                _ => LineDash::Solid,
            };
            Stroke::solid(color, width).with_dash(dash)
        });
        let fill = if dword(style, 13).ok() == Some(1) && dword(style, 25).ok() == Some(u32::MAX) {
            Fill::Solid(Color::opaque(rgb(dword(style, 17).ok()?)))
        } else {
            Fill::None
        };
        let mut reverse_h = false;
        let mut reverse_v = false;
        let mut shape_kind = match primitive_tag {
            78 => {
                reverse_h = dimension(primitive, 0).ok()? > dimension(primitive, 8).ok()?;
                reverse_v = dimension(primitive, 4).ok()? > dimension(primitive, 12).ok()?;
                ShapeKind::Line(stroke?)
            }
            79 => ShapeKind::Auto(AutoShape::new(Geometry::Rect, fill, stroke)),
            80 => {
                if dword(primitive, 0).ok()? & 2 != 0 {
                    return None;
                }
                ShapeKind::Auto(AutoShape::new(Geometry::Ellipse, fill, stroke))
            }
            83 => {
                let count = dword(primitive, 0).ok()? as usize;
                if !(2..=4097).contains(&count) {
                    return None;
                }
                let points: Option<Vec<_>> = (0..count)
                    .map(|i| {
                        Some((
                            dimension(primitive, 4 + 8 * i).ok()?,
                            dimension(primitive, 8 + 8 * i).ok()?,
                        ))
                    })
                    .collect();
                let curved: Vec<_> = primitive
                    .get(4 + 8 * count..4 + 8 * count + count - 1)?
                    .iter()
                    .map(|&v| v == 1)
                    .collect();
                let original = EmuSize::new(coord(12)?, coord(16)?);
                let geometry = shared::curve_geometry(&points?, &curved, original)?;
                ShapeKind::Auto(AutoShape::new(geometry, fill, stroke))
            }
            PICTURE => ShapeKind::Picture(ibis_doc_model::Picture::new(
                self.picture_media(word(primitive, 71).ok()?).ok()??,
            )),
            _ => unreachable!(),
        };
        if root_size.is_none()
            && let ShapeKind::Auto(auto) = &mut shape_kind
        {
            auto.text = grouped_shape_text(records, index, &self.paragraphs, &self.runs);
        }
        let id = self.presentation.new_shape_id();
        let mut shape = Shape::new(
            id,
            EmuRect::new(0, 0, size.width.max(1), size.height.max(1)),
            shape_kind,
        );
        let flags = dword(bytes, 28).ok()?;
        shape.flip_h = (flags & 1 != 0) ^ reverse_h;
        shape.flip_v = (flags & 2 != 0) ^ reverse_v;
        shape.rotation = i32::from(word(bytes, 32).ok()?).rem_euclid(360) * 60_000;
        if root_size.is_none() || !parent.relative_h.is_empty() {
            shape.floating = Some(anchor);
        }
        if root_size.is_none() && matrices > 0 {
            let values: Option<Vec<_>> = bytes
                .get(44..92 + 96 * matrices)?
                .chunks_exact(48)
                .map(|matrix| {
                    let mut values = [0.0; 6];
                    for (value, number) in values.iter_mut().zip(matrix.chunks_exact(8)) {
                        *value = f64::from_le_bytes(number.try_into().ok()?);
                        if !value.is_finite() {
                            return None;
                        }
                    }
                    Some(values)
                })
                .collect();
            if let (Some(values), Some(w), Some(h)) = (values, coord(12), coord(16)) {
                if shared::place_group_leaf(
                    &mut shape,
                    root,
                    EmuSize::new(w, h),
                    values,
                    |picture, original, frame, matrix| {
                        shared::warp_group_picture(
                            self.presentation,
                            &mut self.remaining_warp,
                            picture,
                            original,
                            frame,
                            matrix,
                        )
                    },
                )
                .is_some()
                {
                    shape.flip_h ^= reverse_h;
                    shape.flip_v ^= reverse_v;
                }
            }
        }
        runs.push(FlowRunKind::Shape(Box::new(shape)));
        Some(GroupMember::Shape(id))
    }

    fn picture(
        &mut self,
        records: &[Record<'_>],
        index: usize,
    ) -> Result<Option<FlowRunKind>, Error> {
        let control = records[index];
        let Some(picture) = records[index + 1..control.end]
            .iter()
            .find(|r| r.tag == PICTURE)
        else {
            return Ok(None);
        };
        let width = dimension(control.bytes, 16)?;
        let height = dimension(control.bytes, 20)?;
        if width <= 0 || height <= 0 {
            return Ok(None);
        }
        let Some(media) = self.picture_media(word(picture.bytes, 71)?)? else {
            return Ok(None);
        };
        // A grouped child uses local coordinates; the control's anchor
        // belongs to the group.
        let floating = if picture.level <= control.level + 2 {
            picture_anchor(control.bytes)?.map(|anchor| (self.presentation.new_shape_id(), anchor))
        } else {
            None
        };
        Ok(Some(shared::imported_picture(
            media,
            EmuSize::new(width, height),
            Default::default(),
            floating,
        )))
    }

    fn picture_media(&mut self, id: u16) -> Result<Option<MediaId>, Error> {
        if let Some(&media) = self.media.get(&id) {
            return Ok(Some(media));
        }
        let Some(asset) = usize::from(id)
            .checked_sub(1)
            .and_then(|i| self.pictures.get(i))
            .and_then(Option::as_ref)
        else {
            return Ok(None);
        };
        let bytes = read_stream(
            self.data,
            &asset.path,
            asset.compressed,
            self.remaining_media,
        )?;
        if bytes.is_empty() {
            return Ok(None);
        }
        self.remaining_media -= bytes.len();
        let media = self.presentation.add_media(asset.format, bytes);
        self.media.insert(id, media);
        Ok(Some(media))
    }

    fn table(
        &mut self,
        records: &[Record<'_>],
        start: usize,
        depth: usize,
    ) -> Result<FlowTable, Error> {
        let parent = records[start];
        let children = &records[start + 1..parent.end];
        let table = children
            .iter()
            .find(|r| r.level == parent.level + 1 && r.tag == TABLE)
            .ok_or_else(malformed)?;
        let rows = usize::from(word(table.bytes, 4)?);
        let cols = usize::from(word(table.bytes, 6)?);
        if rows == 0 || cols == 0 || cols > 256 || rows > 4096 || rows * cols > self.remaining_cells
        {
            return Err(malformed());
        }
        self.remaining_cells -= rows * cols;
        let mut occupied = vec![false; rows * cols];
        let mut grid = vec![Vec::new(); rows];
        let mut heights = vec![0; rows];
        let mut measured = vec![true; rows];
        let mut widths = vec![0; cols];
        let mut constraints = Vec::new();
        let mut cell_indices = children
            .iter()
            .enumerate()
            .filter_map(|(offset, r)| {
                (r.level == parent.level + 1 && r.tag == LIST && r.bytes.len() >= 32)
                    .then_some(start + 1 + offset)
            })
            .peekable();
        while let Some(index) = cell_indices.next() {
            let bytes = records[index].bytes;
            let base = if bytes.len() == 32 { 6 } else { 8 };
            let row = usize::from(word(bytes, base + 2)?);
            let col = usize::from(word(bytes, base)?);
            let across = usize::from(word(bytes, base + 4)?);
            let down = usize::from(word(bytes, base + 6)?);
            if across == 0 || down == 0 || row + down > rows || col + across > cols {
                return Err(malformed());
            }
            let width = dimension(bytes, base + 8)?;
            if width > 0 {
                constraints.push((across, col, width));
            }
            let height = dimension(bytes, base + 12)?;
            if down == 1 {
                heights[row] = heights[row].max(height);
            }
            let margins =
                [20, 18, 22, 16].map(|at| units(i64::from(word(bytes, base + at).unwrap_or(0))));
            let (borders, shading) = usize::from(word(bytes, base + 24)?)
                .checked_sub(1)
                .and_then(|id| self.fills.get(id))
                .copied()
                .unwrap_or_default();
            let anchor = match (dword(bytes, base - 4)? >> 5) & 3 {
                1 => CellAnchor::Center,
                2 => CellAnchor::Bottom,
                _ => CellAnchor::Top,
            };
            let mut blocks = Vec::new();
            // Some writers put cell paragraphs at the same level as the list
            // header. The next cell header is the boundary in either encoding.
            let end = cell_indices.peek().copied().unwrap_or(parent.end);
            let mut cursor = index + 1;
            let mut required = margins[0] + margins[2];
            let mut have_measurement = true;
            while cursor < end {
                let record = records[cursor];
                if record.tag == PARA {
                    self.paragraph(records, cursor, &mut blocks, depth, record.end == end)?;
                    let lines = records[cursor + 1..record.end]
                        .iter()
                        .find(|r| r.level == record.level + 1 && r.tag == 69);
                    match lines.and_then(|r| measured_paragraph_height(r.bytes, record.end != end))
                    {
                        Some(height) => required = required.saturating_add(height),
                        None => have_measurement = false,
                    }
                }
                cursor = record.end;
            }
            for block in &blocks {
                if let Block::Paragraph(p) = block {
                    required = required
                        .saturating_add(p.space_before.max(0))
                        .saturating_add(p.space_after.max(0));
                }
            }
            if down == 1 {
                measured[row] &= have_measurement
                    && required <= height
                    && blocks.iter().all(|block| {
                        matches!(block, Block::Paragraph(p)
                        if !p.runs.iter().any(|r| matches!(r.kind, FlowRunKind::Picture { .. } | FlowRunKind::Shape(_))))
                    });
            }
            for line in row..row + down {
                for slot in &mut occupied[line * cols + col..line * cols + col + across] {
                    if std::mem::replace(slot, true) {
                        return Err(malformed());
                    }
                }
                let merge = match (line == row, down) {
                    (_, 1) => VerticalMerge::None,
                    (true, _) => VerticalMerge::Start,
                    (false, _) => VerticalMerge::Continue,
                };
                grid[line].push((
                    col,
                    FlowCell {
                        id: self.next_id(),
                        span: across as u32,
                        merge,
                        margins,
                        borders,
                        shading,
                        shading_fill: shading,
                        anchor,
                        own_borders: borders.map(Some),
                        blocks: std::mem::take(&mut blocks),
                        ..Default::default()
                    },
                ));
            }
        }
        constraints.sort_unstable();
        for (span, start, width) in constraints {
            let columns = &mut widths[start..start + span];
            let missing = columns.iter().filter(|&&width| width == 0).count();
            if missing != 0 {
                let share = ((width - columns.iter().sum::<Emu>()) / missing as Emu).max(1);
                for column in columns.iter_mut().filter(|width| **width == 0) {
                    *column = share;
                }
            }
        }
        let mut output = Vec::with_capacity(rows);
        let flags = dword(table.bytes, 0)?;
        let repeat = flags & 4 != 0;
        let cant_split = matches!(flags & 3, 0 | 2);
        for (index, (mut cells, height)) in grid.into_iter().zip(heights).enumerate() {
            cells.sort_unstable_by_key(|(column, _)| *column);
            let mut row = FlowRow {
                id: self.next_id(),
                header: repeat && index == 0,
                cant_split,
                height: if height == 0 {
                    RowHeight::Auto
                } else if measured[index] {
                    RowHeight::Exact(height)
                } else {
                    RowHeight::AtLeast(height)
                },
                ..Default::default()
            };
            let mut column = 0;
            for (start, cell) in cells {
                while column < start {
                    row.cells.push(FlowCell {
                        id: self.next_id(),
                        span: 1,
                        ..Default::default()
                    });
                    column += 1;
                }
                column += cell.span as usize;
                row.cells.push(cell);
            }
            while column < cols {
                row.cells.push(FlowCell {
                    id: self.next_id(),
                    span: 1,
                    ..Default::default()
                });
                column += 1;
            }
            output.push(row);
        }
        Ok(FlowTable {
            id: self.next_id(),
            rows: output,
            columns: widths
                .into_iter()
                .map(|n| if n == 0 { units(7200) } else { n })
                .collect(),
            cell_spacing: Some(units(i64::from(word(table.bytes, 8)?))),
            indent: units(i64::from(word(parent.bytes, 28)?)),
            ..Default::default()
        })
    }
}

fn measured_paragraph_height(bytes: &[u8], include_final_gap: bool) -> Option<Emu> {
    if bytes.is_empty() || bytes.len() % 36 != 0 {
        return None;
    }
    let mut total: Emu = 0;
    let include_gap = include_final_gap || bytes.len() > 36;
    for line in bytes.chunks_exact(36) {
        let height = dimension(line, 8).ok()?;
        if height <= 0 {
            return None;
        }
        total = total.saturating_add(height);
        if include_gap {
            total = total.saturating_add(dimension(line, 20).ok()?.max(0));
        }
    }
    Some(total)
}

fn grouped_shape_text(
    records: &[Record<'_>],
    index: usize,
    formats: &[ParaFormat],
    styles: &[RunFormat],
) -> Option<ibis_doc_model::TextBody> {
    let record = records[index];
    let mut children = index + 1..record.end;
    let list_index = children.find(|&i| {
        records[i].tag == LIST
            && records[i].level == record.level + 1
            && records[i].bytes.len() == 20
    })?;
    let list = records[list_index];
    let count = usize::from(word(list.bytes, 0).ok()?).min(256);
    let mut lines = Vec::new();
    let mut cursor = list_index + 1;
    while cursor < record.end && lines.len() < count {
        let paragraph = records[cursor];
        if paragraph.tag != PARA || paragraph.level != list.level {
            break;
        }
        let parts = &records[cursor + 1..paragraph.end];
        let find = |tag| {
            parts
                .iter()
                .find(|r| r.tag == tag && r.level == paragraph.level + 1)
        };
        let format = word(paragraph.bytes, 8)
            .ok()
            .and_then(|id| formats.get(usize::from(id)));
        let pitch = find(69).and_then(|lines| {
            let mut chunks = lines.bytes.chunks_exact(36);
            let first = dimension(chunks.next()?, 4).ok()?;
            let second = dimension(chunks.next()?, 4).ok()?;
            let step = second - first;
            let mut last = second;
            (step > 0
                && chunks.all(|line| {
                    let next = dimension(line, 4).unwrap_or(0);
                    let uniform = next - last == step;
                    last = next;
                    uniform
                }))
            .then_some(LineHeight::Exact(step))
        });
        let mut line = shared::box_paragraph(format, pitch);
        if let Some(text) = find(TEXT) {
            let words = utf16(text.bytes).ok()?;
            let mut changes = find(CHAR_RUNS)
                .map_or(&[][..], |r| r.bytes)
                .chunks_exact(8)
                .filter_map(|r| Some((dword(r, 0).ok()? as usize, dword(r, 4).ok()? as usize)))
                .peekable();
            let mut style = None;
            let mut buffer = Vec::new();
            let mut offset = 0;
            let push = |line: &mut ibis_doc_model::Paragraph, buffer: &mut Vec<u16>, style| {
                if !buffer.is_empty() {
                    line.runs.push(ibis_doc_model::Run {
                        text: String::from_utf16_lossy(buffer),
                        style: shared::box_run_style(style),
                    });
                    buffer.clear();
                }
            };
            while let Some(&code) = words.get(offset) {
                while let Some(&(_, id)) = changes.peek().filter(|(pos, _)| *pos <= offset) {
                    push(&mut line, &mut buffer, style);
                    style = styles.get(id);
                    changes.next();
                }
                if matches!(code, 9 | 10 | 32..) {
                    buffer.push(code);
                }
                offset += if matches!(code, 2..=9 | 11 | 12 | 14..=23) {
                    8
                } else {
                    1
                };
            }
            push(&mut line, &mut buffer, style);
        }
        lines.push(line);
        cursor = paragraph.end;
    }
    if lines.is_empty() {
        return None;
    }
    let flags = dword(list.bytes, 4).ok()?;
    let mut text = ibis_doc_model::TextBody {
        paragraphs: lines,
        anchor: match (flags >> 5) & 3 {
            1 => ibis_doc_model::VerticalAnchor::Middle,
            2 => ibis_doc_model::VerticalAnchor::Bottom,
            _ => ibis_doc_model::VerticalAnchor::Top,
        },
        wrap: (flags >> 3) & 3 == 0,
        vertical: flags & 7 == 1,
        ..Default::default()
    };
    let inset = |offset| word(list.bytes, offset).map_or(0, |n| units(i64::from(n)));
    text.insets.left = inset(8);
    text.insets.right = inset(10);
    text.insets.top = inset(12);
    text.insets.bottom = inset(14);
    Some(text)
}

fn picture_anchor(bytes: &[u8]) -> Result<Option<FloatingAnchor>, Error> {
    let flags = dword(bytes, 4)?;
    if flags & 1 != 0 {
        return Ok(None);
    }
    let offset_x = dimension(bytes, 12)?;
    let offset_y = dimension(bytes, 8)?;
    let relative_h = ["page", "margin", "column", "column"][((flags >> 8) & 3) as usize];
    let relative_v = ["page", "margin", "paragraph", "paragraph"][((flags >> 3) & 3) as usize];
    let align = |offset, code: u32, labels: &[&str]| {
        (offset == 0)
            .then(|| labels.get(code as usize).map(|s| (*s).to_owned()))
            .flatten()
    };
    let align_h = align(
        offset_x,
        (flags >> 10) & 7,
        &["left", "center", "right", "inside", "outside"],
    );
    let align_v = align(
        offset_y,
        (flags >> 5) & 7,
        &["top", "center", "bottom", "inside", "outside"],
    );
    let wrap = match (flags >> 21) & 7 {
        1 => "tight",
        2 => "through",
        3 => "topAndBottom",
        4 | 5 => "none",
        _ => "square",
    };
    let wrap_text = ["bothSides", "left", "right", "largest"][((flags >> 24) & 3) as usize];
    let margin = |offset| word(bytes, offset).map(|n| units(i64::from(n)));
    Ok(Some(FloatingAnchor {
        offset_x,
        offset_y,
        align_h,
        align_v,
        relative_h: relative_h.into(),
        relative_v: relative_v.into(),
        wrap: wrap.into(),
        behind_doc: (flags >> 21) & 7 == 4,
        dist_l: margin(28)?,
        dist_r: margin(30)?,
        dist_t: margin(32)?,
        dist_b: margin(34)?,
        wrap_text: Some(wrap_text.into()),
        locked: false,
        allow_overlap: flags & (1 << 14) != 0,
        layout_in_cell: true,
        relative_height: dword(bytes, 24)?,
        wrap_polygon: None,
        glue: Default::default(),
    }))
}

fn embedded_picture(
    bytes: &[u8],
    default_compressed: bool,
) -> Result<Option<EmbeddedPicture>, Error> {
    let flags = word(bytes, 0)?;
    if flags & 15 != 1 {
        return Ok(None);
    }
    let index = word(bytes, 2)?;
    let chars = usize::from(word(bytes, 4)?);
    let extension =
        String::from_utf16_lossy(&utf16(bytes.get(6..6 + 2 * chars).ok_or_else(malformed)?)?);
    let path = format!("BinData/BIN{index:04X}.{extension}");
    let format = match extension.to_ascii_lowercase().as_str() {
        "png" => MediaFormat::Png,
        "jpg" | "jpeg" => MediaFormat::Jpeg,
        "gif" => MediaFormat::Gif,
        "bmp" => MediaFormat::Bmp,
        _ => return Ok(None),
    };
    let compressed = match flags & 0x30 {
        0x10 => true,
        0x20 => false,
        _ => default_compressed,
    };
    Ok(Some(EmbeddedPicture {
        path,
        format,
        compressed,
    }))
}

fn utf16(bytes: &[u8]) -> Result<Vec<u16>, Error> {
    if bytes.len() % 2 != 0 {
        return Err(malformed());
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect())
}

fn flush_paragraph(blocks: &mut Vec<Block>, mut paragraph: FlowParagraph) {
    paragraph.resolve(&Default::default());
    blocks.push(Block::Paragraph(paragraph));
}

fn add_run(paragraph: &mut FlowParagraph, kind: FlowRunKind, format: &RunFormat) {
    let mut run = FlowRun::new(kind, Default::default());
    run.format = format.clone();
    paragraph.runs.push(run);
}

fn append_text(paragraph: &mut FlowParagraph, text: &mut Vec<u16>, format: &RunFormat) {
    if !text.is_empty() {
        add_run(
            paragraph,
            FlowRunKind::Text(String::from_utf16_lossy(text)),
            format,
        );
        text.clear();
    }
}

fn decoration(unit: u16) -> String {
    char::from_u32(u32::from(unit))
        .filter(|ch| !ch.is_control())
        .map_or_else(String::new, String::from)
}

fn binary_numbering(bytes: &[u8], formats: &[RunFormat]) -> Result<Vec<ListLevel>, Error> {
    let mut levels = Vec::with_capacity(7);
    let mut offset = 0;
    for level in 1..=7 {
        let flags = dword(bytes, offset)?;
        let style = dword(bytes, offset + 8)?;
        let length = usize::from(word(bytes, offset + 12)?);
        let start = offset + 14;
        offset = start + 2 * length;
        let pattern =
            String::from_utf16_lossy(&utf16(bytes.get(start..offset).ok_or_else(malformed)?)?);
        levels.push(ListLevel {
            start: 1,
            format: shared::number_format((flags >> 8) & 15).into(),
            text: shared::list_text(&pattern, level),
            suffix: ListSuffix::Space,
            align: match flags & 3 {
                1 => FlowAlign::Center,
                2 => FlowAlign::Right,
                _ => FlowAlign::Left,
            },
            run: formats.get(style as usize).cloned().unwrap_or_default(),
            ..Default::default()
        });
    }
    if let Ok(start) = word(bytes, offset) {
        levels[0].start = u32::from(start).max(1);
        for (index, level) in levels.iter_mut().enumerate() {
            if let Ok(start) = dword(bytes, offset + 2 + index * 4)
                && start != 0
            {
                level.start = start;
            }
        }
    }
    Ok(levels)
}

fn para_format(bytes: &[u8], version: u32) -> Result<ParaFormat, Error> {
    let flags = dword(bytes, 0)?;
    let (mode, spacing) = if version >= 0x05000205 && bytes.len() >= 54 {
        (dword(bytes, 46)? & 31, dword(bytes, 50)?)
    } else {
        (flags & 3, dword(bytes, 24)?)
    };
    let line = match mode {
        0 if (50..=500).contains(&spacing) => Some(LineHeight::Auto(spacing * 240 / 100)),
        1 if spacing <= 72_000 => Some(LineHeight::Exact(units(i64::from(spacing)))),
        3 if spacing <= 72_000 => Some(LineHeight::AtLeast(units(i64::from(spacing)))),
        _ => None,
    };
    Ok(ParaFormat {
        align: Some(match (flags >> 2) & 7 {
            0 => FlowAlign::Justify,
            2 => FlowAlign::Right,
            3 => FlowAlign::Center,
            4 | 5 => FlowAlign::Distribute,
            _ => FlowAlign::Left,
        }),
        left: Some(dimension(bytes, 4)? / 2),
        right: Some(dimension(bytes, 8)? / 2),
        first_line: Some(dimension(bytes, 12)? / 2),
        before: Some(dimension(bytes, 16)?.max(0) / 2),
        after: Some(dimension(bytes, 20)?.max(0) / 2),
        line,
        keep_next: Some(flags & (1 << 17) != 0),
        keep_lines: Some(flags & (1 << 18) != 0),
        page_break_before: Some(flags & (1 << 19) != 0),
        ..Default::default()
    })
}

fn rgb(color: u32) -> Rgb {
    Rgb(((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255))
}

fn read_borders(bytes: &[u8]) -> Result<[Option<FlowBorder>; 4], Error> {
    let mut borders = [None; 4];
    for (index, offset) in [14, 8, 20, 2].into_iter().enumerate() {
        let kind = *bytes.get(offset).ok_or_else(malformed)?;
        if kind == 0 {
            continue;
        }
        const WIDTHS: [i64; 16] = [
            10, 12, 15, 20, 25, 30, 40, 50, 60, 70, 100, 150, 200, 300, 400, 500,
        ];
        let width = WIDTHS
            .get(usize::from(*bytes.get(offset + 1).ok_or_else(malformed)?))
            .copied()
            .unwrap_or(10);
        borders[index] = Some(FlowBorder {
            width: width * 360,
            color: rgb(dword(bytes, offset + 2)?),
            style: match kind {
                2 => BorderStyle::Dashed,
                3 => BorderStyle::Dotted,
                4 => BorderStyle::DotDash,
                5 => BorderStyle::DotDotDash,
                8 => BorderStyle::Double,
                _ => BorderStyle::Single,
            },
            space: 0,
            art: None,
        });
    }
    Ok(borders)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfb::Item;

    fn record(output: &mut Vec<u8>, tag: u16, level: u16, bytes: &[u8]) {
        let head = (bytes.len() as u32) << 20 | u32::from(level) << 10 | u32::from(tag);
        output.extend(head.to_le_bytes());
        output.extend(bytes);
    }

    fn sample(flags: u32) -> Vec<u8> {
        sample_with_picture_mode(flags, 1)
    }

    fn sample_with_picture_mode(flags: u32, picture_flags: u32) -> Vec<u8> {
        let mut header = vec![0; 256];
        header[..SIGNATURE.len()].copy_from_slice(SIGNATURE);
        header[32..36].copy_from_slice(&0x0501_0001u32.to_le_bytes());
        header[36..40].copy_from_slice(&flags.to_le_bytes());
        let mut body = Vec::new();
        record(&mut body, PARA, 0, &[0; 24]);
        let mut text: Vec<u8> = "신청서".encode_utf16().flat_map(u16::to_le_bytes).collect();
        text.extend([11, 0]);
        text.extend(b" osg");
        text.extend([0; 8]);
        text.extend([11, 0, 13, 0]);
        record(&mut body, TEXT, 1, &text);
        let mut control = [0; 46];
        control[..4].copy_from_slice(b" osg");
        control[4..8].copy_from_slice(&picture_flags.to_le_bytes());
        control[8..12].copy_from_slice(&1200i32.to_le_bytes());
        control[12..16].copy_from_slice(&(-300i32).to_le_bytes());
        control[28..30].copy_from_slice(&140u16.to_le_bytes());
        control[16..20].copy_from_slice(&3600u32.to_le_bytes());
        control[20..24].copy_from_slice(&2400u32.to_le_bytes());
        record(&mut body, CONTROL, 1, &control);
        let mut picture = [0; 78];
        picture[71] = 1;
        record(&mut body, PICTURE, 2, &picture);
        record(&mut body, PARA, 0, &[0; 24]);
        record(&mut body, TEXT, 1, b"\x0b\0 lbt\0\0\0\0\0\0\0\0\x0b\0\r\0");
        control[..4].copy_from_slice(b" lbt");
        record(&mut body, CONTROL, 1, &control);
        let mut table = [0; 22];
        table[0] = 6;
        table[4] = 2;
        table[6] = 1;
        record(&mut body, TABLE, 2, &table);
        for (row, height) in [(0_u8, 2500_u32), (1, 1700)] {
            let mut cell = [0; 34];
            cell[0] = 2;
            cell[10] = row;
            cell[12] = 1;
            cell[14] = 1;
            cell[16..20].copy_from_slice(&7200_u32.to_le_bytes());
            cell[20..24].copy_from_slice(&height.to_le_bytes());
            record(&mut body, LIST, 2, &cell);
            for (text, height, gap) in [(b"A\0\r\0", 900_u32, 400_u32), (b"B\0\r\0", 1000, 500)] {
                record(&mut body, PARA, 2, &[0; 24]);
                record(&mut body, TEXT, 3, text);
                let mut line = [0; 36];
                line[8..12].copy_from_slice(&height.to_le_bytes());
                line[20..24].copy_from_slice(&gap.to_le_bytes());
                record(&mut body, 69, 3, &line);
            }
        }
        let mut doc_info = Vec::new();
        record(&mut doc_info, 16, 0, &[0; 26]);
        record(
            &mut doc_info,
            18,
            1,
            &[0x21, 0, 1, 0, 3, 0, b'p', 0, b'n', 0, b'g', 0],
        );
        if flags & 1 != 0 {
            body = miniz_oxide::deflate::compress_to_vec(&body, 6);
            doc_info = miniz_oxide::deflate::compress_to_vec(&doc_info, 6);
        }
        cfb::write(&[
            Item::stream("FileHeader", header),
            Item::stream("DocInfo", doc_info),
            Item::storage("BodyText"),
            Item::stream("BodyText/Section0", body),
            Item::storage("BinData"),
            Item::stream(
                "BinData/BIN0001.png",
                include_bytes!("../../../test/fixtures/markdown-desktop/images/revenue.png")
                    .to_vec(),
            ),
        ])
    }

    #[test]
    fn hancom_detection_preserves_binary_precedence_and_extension_hints() {
        let binary = sample(0);
        assert_eq!(Format::detect(None, &binary), Some(Format::Hwp));
        assert_eq!(Format::detect(Some("HWPX"), &binary), Some(Format::Hwp));
        assert_eq!(Format::detect(Some("HWP"), b""), Some(Format::Hwp));
        assert_eq!(Format::detect(Some("HWPX"), b""), Some(Format::Hwpx));
        assert_eq!(Format::detect(Some("docx"), b""), None);
    }

    #[test]
    fn vector_group_survives_docx_roundtrip() {
        let sample = sample(0);
        let header = cfb::stream_path(&sample, "FileHeader").unwrap();
        let mut body = Vec::new();
        record(&mut body, PARA, 0, &[0; 24]);
        record(&mut body, TEXT, 1, b"\x0b\0 osg\0\0\0\0\0\0\0\0\x0b\0\r\0");
        let mut control = [0; 44];
        control[..4].copy_from_slice(b" osg");
        control[8..12].copy_from_slice(&900i32.to_le_bytes());
        control[12..16].copy_from_slice(&1600i32.to_le_bytes());
        control[16..20].copy_from_slice(&2000i32.to_le_bytes());
        control[20..24].copy_from_slice(&1000i32.to_le_bytes());
        record(&mut body, CONTROL, 1, &control);
        let mut group = [0; 196];
        group[..8].copy_from_slice(b"noc$noc$");
        group[50] = 1;
        record(&mut body, COMPONENT, 2, &group);
        for (kind, tag, x, primitive) in [
            (b"cer$", 79, 0i32, vec![0; 33]),
            (b"lle$", 80, 700, vec![0; 60]),
            (b"nil$", 78, 1300, {
                let mut line = vec![0; 20];
                line[..4].copy_from_slice(&500i32.to_le_bytes());
                line[12..16].copy_from_slice(&500i32.to_le_bytes());
                line
            }),
            (b"ruc$", 83, 1000, {
                let mut curve: Vec<u8> = [3i32, 0, 0, 250, 500, 500, 0]
                    .into_iter()
                    .flat_map(i32::to_le_bytes)
                    .collect();
                curve.extend([1, 1]);
                curve
            }),
        ] {
            let mut component = vec![0; 4 + 92 + 96 + 33];
            component[..4].copy_from_slice(kind);
            component[4..8].copy_from_slice(&x.to_le_bytes());
            component[24..28].copy_from_slice(&500i32.to_le_bytes());
            component[28..32].copy_from_slice(&500i32.to_le_bytes());
            component[46] = 1;
            if tag == 83 {
                component[16..20].copy_from_slice(&500i32.to_le_bytes());
                component[20..24].copy_from_slice(&500i32.to_le_bytes());
                let matrices = [
                    [1.0f64, 0.0, 1000.0, 0.0, 1.0, 0.0],
                    [2.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                    [0.0, -1.0, 500.0, 1.0, 0.0, 0.0],
                ];
                for (dst, value) in component[48..192]
                    .chunks_exact_mut(8)
                    .zip(matrices.into_iter().flatten())
                {
                    dst.copy_from_slice(&value.to_le_bytes());
                }
            }
            let style = &mut component[192..];
            style[4] = 33;
            style[8] = 0x41;
            style[13] = 1;
            style[17..20].copy_from_slice(&[0x66, 0xcc, 0xff]);
            style[25..29].fill(0xff);
            record(&mut body, COMPONENT, 3, &component);
            if tag == 79 {
                let mut list = [0; 20];
                list[0] = 1;
                list[4] = 0x28;
                list[8] = 40;
                record(&mut body, LIST, 4, &list);
                record(&mut body, PARA, 4, &[0; 24]);
                record(&mut body, TEXT, 5, &[0xa9, 0xba, 13, 0]);
            }
            record(&mut body, tag, 4, &primitive);
        }
        let data = cfb::write(&[
            Item::stream("FileHeader", header),
            Item::stream("DocInfo", Vec::new()),
            Item::storage("BodyText"),
            Item::stream("BodyText/Section0", body),
        ]);
        let flow = open(&data).unwrap().presentation.flow.unwrap();
        let Block::Paragraph(paragraph) = &flow.body[0] else {
            panic!("paragraph")
        };
        let shapes: Vec<_> = paragraph
            .runs
            .iter()
            .filter_map(|run| match &run.kind {
                FlowRunKind::Shape(shape) => Some(shape),
                _ => None,
            })
            .collect();
        assert_eq!(shapes.len(), 4);
        assert_eq!(
            flow.shape_groups[0].leaf_shape_ids(),
            shapes.iter().map(|s| s.id).collect::<Vec<_>>()
        );
        for (shape, expected, dx) in [
            (&shapes[0], Geometry::Rect, 0),
            (&shapes[1], Geometry::Ellipse, 700),
        ] {
            let ShapeKind::Auto(auto) = &shape.kind else {
                panic!("vector shape")
            };
            assert_eq!(auto.geometry, expected);
            assert_eq!(auto.fill, Fill::Solid(Color::opaque(Rgb(0x66_cc_ff))));
            assert_eq!(shape.floating.as_ref().unwrap().offset_x, units(1600 + dx));
        }
        let ShapeKind::Auto(rect) = &shapes[0].kind else {
            panic!("rectangle")
        };
        let text = rect.text.as_ref().unwrap();
        assert_eq!(text.text(), "목");
        assert_eq!(text.anchor, ibis_doc_model::VerticalAnchor::Middle);
        assert!(!text.wrap);
        assert_eq!(text.insets.left, units(40));
        assert!(matches!(shapes[2].kind, ShapeKind::Line(_)));
        assert!(shapes[2].flip_h);
        let curve = &shapes[3];
        assert_eq!(curve.rotation, 90 * 60_000);
        assert_eq!(
            (curve.frame.width, curve.frame.height),
            (units(1000), units(500))
        );
        let ShapeKind::Auto(auto) = &curve.kind else {
            panic!("curve")
        };
        let Geometry::Formula(geometry) = &auto.geometry else {
            panic!("custom geometry")
        };
        assert!(matches!(
            geometry.def.paths[0].ops[1],
            ibis_doc_model::PathOp::CubicTo(_)
        ));
    }

    #[test]
    fn converts_stored_and_compressed_utf16() {
        for flags in [0, 1] {
            let data = sample(flags);
            assert!(is_hwp(&data));
            let opened = open(&data).unwrap();
            let flow = opened.presentation.flow.unwrap();
            let Block::Paragraph(p) = &flow.body[0] else {
                panic!("paragraph expected")
            };
            assert!(p.text().starts_with("신청서"));
            assert!(matches!(p.runs[1].kind, FlowRunKind::Picture { .. }));
            assert_eq!(opened.presentation.media[0].format, MediaFormat::Png);
        }
    }

    #[test]
    fn binary_floating_picture_survives_docx_roundtrip() {
        let flags = (2 << 8) | (2 << 3) | (1 << 14) | (4 << 21);
        let f = open(&sample_with_picture_mode(1, flags))
            .unwrap()
            .presentation
            .flow
            .unwrap();
        let Block::Paragraph(p) = &f.body[0] else {
            panic!("paragraph expected")
        };
        let FlowRunKind::Shape(shape) = &p.runs[1].kind else {
            panic!("floating picture expected")
        };
        assert!(matches!(shape.kind, ibis_doc_model::ShapeKind::Picture(_)));
        let anchor = shape.floating.as_ref().unwrap();
        assert_eq!(
            (anchor.offset_x, anchor.offset_y),
            (units(-300), units(1200))
        );
        assert_eq!(
            (anchor.relative_h.as_str(), anchor.relative_v.as_str()),
            ("column", "paragraph")
        );
        assert!(anchor.behind_doc && anchor.allow_overlap);
        assert_eq!(anchor.dist_l, units(140));
    }

    #[test]
    fn cell_line_gaps_survive_between_paragraphs_and_rows_can_grow() {
        let opened = open(&sample(1)).unwrap();
        let flow = opened.presentation.flow.unwrap();
        let Block::Table(table) = &flow.body[1] else {
            panic!("table expected")
        };
        assert!(table.rows[0].header);
        assert!(!table.rows[1].header);
        assert!(table.rows.iter().all(|row| row.cant_split));
        assert_eq!(table.rows[0].height, RowHeight::Exact(units(2500)));
        assert_eq!(table.rows[1].height, RowHeight::AtLeast(units(1700)));
        for row in &table.rows {
            let [Block::Paragraph(first), Block::Paragraph(last)] = row.cells[0].blocks.as_slice()
            else {
                panic!("two cell paragraphs expected");
            };
            assert_eq!(first.line, LineHeight::Exact(units(1300)));
            assert_eq!(last.line, LineHeight::Exact(units(1000)));
            assert_eq!(format!("{}{}", first.text(), last.text()), "AB");
        }
    }

    #[test]
    fn binary_lists_and_section_stories_survive_docx() {
        let mut info = Vec::new();
        let mut definition = Vec::new();
        for level in 1..=7 {
            definition.extend([0; 8]);
            definition.extend(u32::MAX.to_le_bytes());
            let label: Vec<u8> = format!("^{level})")
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect();
            definition.extend(((label.len() / 2) as u16).to_le_bytes());
            definition.extend(label);
        }
        definition.extend(4u16.to_le_bytes());
        record(&mut info, 23, 0, &definition);
        let mut bullet = [0; 14];
        bullet[12..14].copy_from_slice(&0xf06eu16.to_le_bytes());
        record(&mut info, 24, 0, &bullet);
        for kind in [0u32, 2, 3] {
            let mut para = [0; 54];
            para[..4].copy_from_slice(&(kind << 23).to_le_bytes());
            para[30] = 1;
            record(&mut info, 25, 0, &para);
        }
        let mut body = Vec::new();
        let mut p = [0; 24];
        p[8] = 1;
        record(&mut body, PARA, 0, &p);
        record(&mut body, TEXT, 1, b"A\0\r\0");
        record(&mut body, CONTROL, 1, b"onwn\0\0\0\0\x07\0");
        record(&mut body, CONTROL, 1, b"pngp\0\x05\0\0\0\0-\0-\0");
        record(&mut body, CONTROL, 1, b"daeh\0\0\0\0");
        record(&mut body, PARA, 2, &[0; 24]);
        record(&mut body, TEXT, 3, b"H\0\r\0");
        record(&mut body, CONTROL, 1, b"toof\0\0\0\0");
        record(&mut body, PARA, 2, &[0; 24]);
        record(&mut body, TEXT, 3, b"F\0\r\0");
        p[8] = 2;
        record(&mut body, PARA, 0, &p);
        record(&mut body, TEXT, 1, b"B\0\r\0");
        let mut header = vec![0; 256];
        header[..SIGNATURE.len()].copy_from_slice(SIGNATURE);
        header[35] = 5;
        let data = cfb::write(&[
            Item::stream("FileHeader", header),
            Item::stream("DocInfo", info),
            Item::storage("BodyText"),
            Item::stream("BodyText/Section0", body),
        ]);
        let f = open(&data).unwrap().presentation.flow.unwrap();
        let [Block::Paragraph(a), Block::Paragraph(b)] = f.body.as_slice() else {
            panic!("expected two body paragraphs")
        };
        let a = a.list.as_ref().unwrap();
        let b = b.list.as_ref().unwrap();
        let number = f.styles.numbering.level(a.num_id, a.level).unwrap();
        assert_eq!((number.start, number.text.as_str()), (4, "%1)"));
        assert_eq!(f.styles.numbering.level(b.num_id, 0).unwrap().text, "■");
        assert_eq!(f.page.first_page, 7);
        let Block::Paragraph(header) = &f.header[0] else {
            panic!("expected header")
        };
        assert_eq!(header.text(), "H");
        let [Block::Paragraph(footer), Block::Paragraph(number)] = f.footer.as_slice() else {
            panic!("expected footer and page number")
        };
        assert_eq!(footer.text(), "F");
        assert_eq!(number.align, FlowAlign::Center);
        assert!(matches!(number.runs[1].kind, FlowRunKind::PageNumber));
    }

    #[test]
    fn page_orientation_rotates_paper_but_preserves_edge_margins() {
        let mut page = [0; 40];
        page[..4].copy_from_slice(&59_528u32.to_le_bytes());
        page[4..8].copy_from_slice(&84_188u32.to_le_bytes());
        page[8..12].copy_from_slice(&2_834u32.to_le_bytes());
        page[16..20].copy_from_slice(&4_252u32.to_le_bytes());
        let portrait = page_setup(&page).unwrap();
        page[36] = 1;
        let landscape = page_setup(&page).unwrap();
        assert_eq!(landscape.size.width, portrait.size.height);
        assert_eq!(landscape.size.height, portrait.size.width);
        assert_eq!(landscape.left, portrait.left);
        assert_eq!(landscape.top, portrait.top);
    }

    #[test]
    fn rejects_protected_and_truncated_records() {
        assert!(
            open(&sample(2))
                .err()
                .unwrap()
                .to_string()
                .contains("暗号化")
        );
        let mut truncated = Vec::new();
        record(&mut truncated, PARA, 0, &[0; 24]);
        truncated.pop();
        assert!(records(&truncated).is_err());
    }
}
