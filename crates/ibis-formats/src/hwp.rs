// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Binary Hangul 5.x import from Hancom's published file-format specification.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::HashMap;

use ibis_doc_model::{
    Block, BorderStyle, CellAnchor, Emu, EmuSize, FlowAlign, FlowBorder, FlowCell, FlowDocument,
    FlowParagraph, FlowRow, FlowRun, FlowRunKind, FlowTable, LineHeight, MediaFormat, MediaId,
    PageSetup, ParaFormat, Presentation, Rgb, RowHeight, RunFormat, SectionBreak, SectionSetup,
    VerticalMerge,
};

use crate::{
    Error, Opened,
    hwpx::{line_pitch, units},
    ibis_ooxml::cfb,
};

pub const SPEC_CREDIT: &str =
    "본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.";

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
const PICTURE: u16 = 85;

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
        let mut body = Vec::new();
        let mut cursor = 0;
        while cursor < section.len() {
            let record = section[cursor];
            if record.tag == PARA {
                importer.paragraph(&section, cursor, &mut body, 0)?;
            }
            cursor = record.end;
        }
        if index + 1 < count {
            if !matches!(body.last(), Some(Block::Paragraph(_))) {
                body.push(Block::Paragraph(importer.new_paragraph()));
            }
            if let Some(Block::Paragraph(last)) = body.last_mut() {
                last.section_break = SectionBreak::NextPage;
                last.section = Some(
                    SectionSetup {
                        page,
                        ..Default::default()
                    }
                    .into(),
                );
            }
        }
        document.page = page;
        document.body.extend(body);
    }
    document.id_floor = importer.id;
    Ok(document)
}

fn page_setup(bytes: &[u8]) -> Result<PageSetup, Error> {
    let (width, height) = (dimension(bytes, 0)?, dimension(bytes, 4)?);
    if width <= 0 || height <= 0 {
        return Err(malformed());
    }
    let left = dimension(bytes, 8)?.max(0);
    let right = dimension(bytes, 12)?.max(0);
    let header = dimension(bytes, 24)?.max(0);
    let footer = dimension(bytes, 28)?.max(0);
    Ok(PageSetup {
        size: EmuSize::new(width, height),
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
    fills: Vec<CellFill>,
    data: &'a [u8],
    presentation: &'a mut Presentation,
    pictures: Vec<Option<EmbeddedPicture>>,
    media: HashMap<u16, MediaId>,
    remaining_media: usize,
    remaining_cells: usize,
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
            fills: Vec::new(),
            data,
            presentation,
            pictures: Vec::new(),
            media: HashMap::new(),
            remaining_media: BODY_LIMIT,
            remaining_cells: 100_000,
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
                25 => importer.paragraphs.push(para_format(bytes, version)?),
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

    fn paragraph(
        &mut self,
        records: &[Record<'_>],
        start: usize,
        blocks: &mut Vec<Block>,
        depth: usize,
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
                            i64::from(dword(line, 20).ok()? as i32),
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
                if records[control].bytes.starts_with(b" osg") {
                    if let Some(kind) = self.picture(records, control)? {
                        add_run(&mut paragraph, kind, &run_format);
                    }
                } else if records[control].bytes.starts_with(b" lbt") {
                    if !paragraph.runs.is_empty() {
                        let mut next = self.new_paragraph();
                        next.direct = paragraph.direct.clone();
                        next.direct.page_break_before = None;
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
        let id = word(picture.bytes, 71)?;
        let Some(asset) = usize::from(id)
            .checked_sub(1)
            .and_then(|i| self.pictures.get(i))
            .and_then(Option::as_ref)
        else {
            return Ok(None);
        };
        let width = dimension(control.bytes, 16)?;
        let height = dimension(control.bytes, 20)?;
        if width <= 0 || height <= 0 {
            return Ok(None);
        }
        let media = if let Some(&media) = self.media.get(&id) {
            media
        } else {
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
            media
        };
        Ok(Some(FlowRunKind::Picture {
            media,
            size: EmuSize::new(width, height),
            look: Default::default(),
        }))
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
                    self.paragraph(records, cursor, &mut blocks, depth)?;
                    let lines = records[cursor + 1..record.end]
                        .iter()
                        .find(|r| r.level == record.level + 1 && r.tag == 69);
                    match lines.and_then(|r| measured_paragraph_height(r.bytes)) {
                        Some(height) => required = required.saturating_add(height),
                        None => have_measurement = false,
                    }
                }
                cursor = record.end;
            }
            if down == 1 {
                measured[row] &= have_measurement
                    && required <= height
                    && blocks.iter().all(|block| {
                        matches!(block, Block::Paragraph(p)
                        if !p.runs.iter().any(|r| matches!(r.kind, FlowRunKind::Picture { .. })))
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
        let repeat = dword(table.bytes, 0)? & 4 != 0;
        for (index, (mut cells, height)) in grid.into_iter().zip(heights).enumerate() {
            cells.sort_unstable_by_key(|(column, _)| *column);
            let mut row = FlowRow {
                id: self.next_id(),
                header: repeat && index == 0,
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

fn measured_paragraph_height(bytes: &[u8]) -> Option<Emu> {
    if bytes.is_empty() || bytes.len() % 36 != 0 {
        return None;
    }
    let mut total: Emu = 0;
    let mut lines = bytes.chunks_exact(36).peekable();
    while let Some(line) = lines.next() {
        let height = dimension(line, 8).ok()?;
        if height <= 0 {
            return None;
        }
        total = total.saturating_add(height);
        if lines.peek().is_some() {
            total = total.saturating_add(dimension(line, 20).ok()?.max(0));
        }
    }
    Some(total)
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
        control[16..20].copy_from_slice(&3600u32.to_le_bytes());
        control[20..24].copy_from_slice(&2400u32.to_le_bytes());
        record(&mut body, CONTROL, 1, &control);
        let mut picture = [0; 78];
        picture[71] = 1;
        record(&mut body, PICTURE, 2, &picture);
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
