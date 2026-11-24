// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! HWPX package and section reading for document import.

use std::collections::HashMap;

use ibis_doc_model::{
    Block, BorderStyle, Emu, EmuSize, FlowAlign, FlowBorder, FlowCell, FlowDocument, FlowParagraph,
    FlowRow, FlowRun, FlowRunKind, FlowTable, LineHeight, MediaFormat, MediaId, PageSetup,
    ParaFormat, Presentation, Rgb, RowHeight, RunFormat, SectionBreak, SectionSetup, VerticalMerge,
};
use roxmltree::{Document, Node};

use crate::{Error, Kind, Opened, SaveOptions, zip::Archive};

const PACKAGE: &str = "Contents/content.hpf";
const MIME_TYPE: &[u8] = b"application/hwp+zip";
const OPF: &str = "http://www.idpf.org/2007/opf";
const OPF_WITH_SLASH: &str = "http://www.idpf.org/2007/opf/";
const SECTION: &str = "http://www.hancom.co.kr/hwpml/2011/section";
const CORE: &str = "http://www.hancom.co.kr/hwpml/2011/core";
const PARAGRAPH: &str = "http://www.hancom.co.kr/hwpml/2011/paragraph";
const XML_BYTES: usize = 16 * 1024 * 1024;
const TOTAL_SECTION_BYTES: usize = 64 * 1024 * 1024;

/// Opens an HWPX document as a converted Word document.
pub fn open(data: &[u8]) -> Result<Opened, Error> {
    crate::open(to_docx(data)?)
}

/// Converts an HWPX document into an OOXML package using the shared document writer.
pub fn to_docx(data: &[u8]) -> Result<Vec<u8>, Error> {
    document_to_docx(|presentation| read_document(data, presentation))
}

pub(super) fn document_to_docx(
    read: impl FnOnce(&mut Presentation) -> Result<FlowDocument, Error>,
) -> Result<Vec<u8>, Error> {
    let mut blank = crate::open(crate::blank_file(Kind::Document)?)?;
    let mut document = read(&mut blank.presentation)?;
    // The incremental DOCX writer uses the blank document's paragraph as
    // the existing body node around which it inserts new blocks.
    if let Some(baseline) = &blank.presentation.flow {
        document.styles = baseline.styles.clone();
        if let Some(Block::Paragraph(anchor)) = baseline.body.first() {
            let first = document.body.iter_mut().find_map(|block| match block {
                Block::Paragraph(paragraph) => Some(paragraph),
                Block::Table(_) => None,
            });
            if let Some(paragraph) = first {
                paragraph.id = anchor.id;
            } else {
                document.body.push(Block::Paragraph(anchor.clone()));
            }
        }
    }
    blank.presentation.flow = Some(document.into());
    blank
        .source
        .save(&blank.presentation, SaveOptions::default())
}

pub fn is_hwpx(data: &[u8]) -> bool {
    data.starts_with(b"PK")
        && Archive::new(data).ok().is_some_and(|archive| {
            archive
                .read_bounded("mimetype", 64)
                .ok()
                .flatten()
                .as_deref()
                == Some(MIME_TYPE)
        })
}

pub fn is_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("hwpx")
}

fn read_document(data: &[u8], presentation: &mut Presentation) -> Result<FlowDocument, Error> {
    let archive = Archive::new(data)?;
    if archive.read_bounded("mimetype", 64)?.as_deref() != Some(MIME_TYPE) {
        return Err(Error::new("HWPX の mimetype が不正です"));
    }

    let package = xml(&archive, PACKAGE, XML_BYTES)?;
    let package = parse(&package)?;
    let root = package.root_element();
    let namespace = root.tag_name().namespace().unwrap_or_default();
    if root.tag_name().name() != "package" || !matches!(namespace, OPF | OPF_WITH_SLASH) {
        return Err(Error::new("HWPX の package が不正です"));
    }
    let manifest = child(root, namespace, "manifest")?;
    let spine = child(root, namespace, "spine")?;
    let mut items = HashMap::new();
    for item in manifest
        .children()
        .filter(|n| n.has_tag_name((namespace, "item")))
    {
        let id = attribute(item, "id")?;
        let href = attribute(item, "href")?;
        if items.insert(id, href).is_some() {
            return Err(Error::new("HWPX の manifest に重複する ID があります"));
        }
    }

    let mut importer = Importer {
        id: 10,
        runs: HashMap::new(),
        paragraphs: HashMap::new(),
        borders: HashMap::new(),
        archive: &archive,
        items: &items,
        presentation,
        images: HashMap::new(),
        remaining_images: 64 * 1024 * 1024,
        remaining_cells: 100_000,
    };
    let header = items
        .get("header")
        .copied()
        .unwrap_or("Contents/header.xml");
    let header = part_path(header)?;
    if let Some(bytes) = archive.read_bounded(&header, XML_BYTES)? {
        importer.read_styles(parse(&bytes)?.root_element());
    }

    let mut sections = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut remaining = TOTAL_SECTION_BYTES;
    for item in spine
        .children()
        .filter(|n| n.has_tag_name((namespace, "itemref")))
    {
        let id = attribute(item, "idref")?;
        let href = items
            .get(id)
            .ok_or_else(|| Error::new("HWPX の spine が未知の ID を参照しています"))?;
        let path = part_path(href)?;
        if !seen.insert(path.clone()) {
            return Err(Error::new("HWPX の spine に重複する区画があります"));
        }
        let bytes = xml(&archive, &path, remaining.min(XML_BYTES))?;
        remaining -= bytes.len();
        let document = parse(&bytes)?;
        let root = document.root_element();
        // Hancom can list the style header as the first spine item.
        if root.has_tag_name(("http://www.hancom.co.kr/hwpml/2011/head", "head")) {
            continue;
        }
        if !root.has_tag_name((SECTION, "sec")) {
            return Err(Error::new("HWPX の区画 XML が不正です"));
        }
        let page = read_page(root);
        let blocks = importer.blocks(root, 0)?;
        sections.push((page, blocks));
    }
    let Some((page, _)) = sections.last() else {
        return Err(Error::new("HWPX に本文区画がありません"));
    };
    let mut document = FlowDocument {
        page: *page,
        ..Default::default()
    };
    let count = sections.len();
    for (index, (page, mut blocks)) in sections.into_iter().enumerate() {
        if index + 1 < count {
            if !matches!(blocks.last(), Some(Block::Paragraph(_))) {
                blocks.push(Block::Paragraph(importer.paragraph()));
            }
            if let Some(Block::Paragraph(last)) = blocks.last_mut() {
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
        document.body.extend(blocks);
    }
    document.id_floor = importer.id;
    Ok(document)
}

fn part_path(href: &str) -> Result<String, Error> {
    let href = href
        .strip_prefix("../BinData/")
        .map_or(href, |_| &href[3..]);
    if href.contains('\\') || href.split('/').any(|p| matches!(p, "" | "." | "..")) {
        return Err(Error::new("HWPX に無効なパッケージ内パスがあります"));
    }
    Ok(
        if href.starts_with("Contents/") || href.starts_with("BinData/") {
            href.to_owned()
        } else {
            format!("Contents/{href}")
        },
    )
}

fn xml(archive: &Archive<'_>, path: &str, limit: usize) -> Result<Vec<u8>, Error> {
    archive
        .read_bounded(path, limit)?
        .ok_or_else(|| Error::new(format!("HWPX に {path} がありません")))
}

fn parse(bytes: &[u8]) -> Result<Document<'_>, Error> {
    let text = std::str::from_utf8(bytes).map_err(|e| Error::new(format!("HWPX XML: {e}")))?;
    Document::parse(text).map_err(|e| Error::new(format!("HWPX XML: {e}")))
}

fn child<'a, 'input>(
    node: Node<'a, 'input>,
    namespace: &str,
    name: &str,
) -> Result<Node<'a, 'input>, Error> {
    node.children()
        .find(|n| n.has_tag_name((namespace, name)))
        .ok_or_else(|| Error::new(format!("HWPX の {name} がありません")))
}

fn attribute<'a>(node: Node<'a, '_>, name: &str) -> Result<&'a str, Error> {
    node.attribute(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::new(format!("HWPX の {name} 属性がありません")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_korean_body_in_spine_order() {
        let package = format!(
            r#"<package xmlns="{OPF_WITH_SLASH}"><manifest>
            <item id="a" href="Contents/section0.xml"/>
            <item id="b" href="section1.xml"/>
            </manifest><spine><itemref idref="b"/><itemref idref="a"/></spine></package>"#
        );
        let section = |text| {
            format!(
                r#"<hs:sec xmlns:hs="{SECTION}" xmlns:hp="{PARAGRAPH}"><hp:p><hp:run><hp:t>{text}</hp:t></hp:run></hp:p></hs:sec>"#
            )
        };
        let mut writer = ibis_ooxml::zip::Writer::new();
        for (name, data) in [
            ("mimetype", MIME_TYPE),
            (PACKAGE, package.as_bytes()),
            ("Contents/section0.xml", section("본문").as_bytes()),
            (
                "Contents/section1.xml",
                section("신청<hp:tab/>서").as_bytes(),
            ),
        ] {
            writer.add(name, Default::default(), data).unwrap();
        }
        let opened = open(&writer.finish().unwrap()).unwrap();
        let document = opened.presentation.flow.unwrap();
        let paragraphs: Vec<_> = document
            .body
            .iter()
            .filter_map(|block| match block {
                Block::Paragraph(p) => Some(p.text()),
                Block::Table(_) => None,
            })
            .collect();
        assert_eq!(paragraphs, ["신청\t서", "본문"]);
    }
    #[test]
    fn preserves_merged_cells_styles_page_and_picture_in_docx() {
        let package = format!(
            r#"<package xmlns="{OPF}"><manifest>
            <item id="header" href="Contents/header.xml"/>
            <item id="body" href="Contents/section0.xml"/>
            <item id="logo" href="BinData/logo.png"/>
            </manifest><spine><itemref idref="header"/><itemref idref="body"/></spine></package>"#
        );
        let header = format!(
            r##"<hh:head xmlns:hh="{HEAD}">
            <hh:charPr id="1" height="1400" textColor="#112233"><hh:bold/></hh:charPr>
            <hh:paraPr id="1"><hh:align horizontal="CENTER"/></hh:paraPr>
            </hh:head>"##
        );
        let section = format!(
            r#"<hs:sec xmlns:hs="{SECTION}" xmlns:hp="{PARAGRAPH}"
            xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core">
            <hp:p paraPrIDRef="1"><hp:run charPrIDRef="1">
              <hp:secPr><hp:pagePr width="50000" height="70000"><hp:margin left="4300"/></hp:pagePr></hp:secPr>
              <hp:t>안내</hp:t><hp:pic><hp:sz width="3600" height="2400"/><hc:img binaryItemIDRef="logo"/></hp:pic>
            </hp:run></hp:p>
            <hp:p><hp:run><hp:tbl rowCnt="2" colCnt="3">
              <hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="2" rowSpan="2"/>
                <hp:cellSz width="12000" height="3000"/>
                <hp:subList><hp:p><hp:run><hp:t>서식</hp:t></hp:run></hp:p></hp:subList>
              </hp:tc><hp:tc><hp:cellAddr colAddr="2" rowAddr="0"/></hp:tc></hp:tr>
              <hp:tr><hp:tc><hp:cellAddr colAddr="2" rowAddr="1"/></hp:tc></hp:tr>
            </hp:tbl></hp:run></hp:p>
            </hs:sec>"#
        );
        let png = include_bytes!("../../../test/fixtures/markdown-desktop/images/revenue.png");
        let mut writer = ibis_ooxml::zip::Writer::new();
        for (name, data) in [
            ("mimetype", MIME_TYPE),
            (PACKAGE, package.as_bytes()),
            ("Contents/header.xml", header.as_bytes()),
            ("Contents/section0.xml", section.as_bytes()),
            ("BinData/logo.png", png.as_slice()),
        ] {
            writer.add(name, Default::default(), data).unwrap();
        }
        let opened = open(&writer.finish().unwrap()).unwrap();
        let presentation = opened.presentation;
        let flow = presentation.flow.as_ref().unwrap();
        assert_eq!(flow.page.size, EmuSize::new(units(50000), units(70000)));
        assert_eq!(flow.page.left, units(4300));
        let Block::Paragraph(p) = &flow.body[0] else {
            panic!("expected paragraph")
        };
        assert_eq!(p.align, FlowAlign::Center);
        assert!(p.runs[0].style.bold);
        assert_eq!(p.runs[0].style.size, 1400);
        assert_eq!(p.runs[0].style.color, Rgb(0x112233));
        let FlowRunKind::Picture { media, .. } = p.runs[1].kind else {
            panic!("expected picture")
        };
        assert_eq!(presentation.media(media).unwrap().data.as_ref(), png);
        let Block::Table(table) = &flow.body[1] else {
            panic!("expected table")
        };
        assert_eq!(table.rows[0].cells[0].span, 2);
        assert_eq!(table.rows[0].cells[0].merge, VerticalMerge::Start);
        assert_eq!(table.rows[1].cells[0].span, 2);
        assert_eq!(table.rows[1].cells[0].merge, VerticalMerge::Continue);
        let Block::Paragraph(p) = &table.rows[0].cells[0].blocks[0] else {
            panic!("expected paragraph")
        };
        assert_eq!(p.text(), "서식");
    }
}

const HEAD: &str = "http://www.hancom.co.kr/hwpml/2011/head";

fn part<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name((PARAGRAPH, name)))
}

fn number<T: std::str::FromStr>(node: Node<'_, '_>, key: &str) -> Option<T> {
    node.attribute(key)?.parse().ok()
}

pub(super) fn units(value: i64) -> Emu {
    value.clamp(-1_000_000, 1_000_000) * 127
}

// The two formats store line height and the interline gap in HWP units.
pub(super) fn line_pitch(height: i64, gap: i64) -> Option<LineHeight> {
    if !(1..=72_000).contains(&height) || !(-72_000..=72_000).contains(&gap) {
        return None;
    }
    Some(LineHeight::Exact(units(height + gap.max(0))))
}

fn color(value: &str) -> Option<Rgb> {
    let hex = value.strip_prefix('#')?;
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok().map(Rgb))
        .flatten()
}

fn read_page(root: Node<'_, '_>) -> PageSetup {
    let mut page = PageSetup::default();
    let Some(props) = root
        .children()
        .filter(|n| n.has_tag_name((PARAGRAPH, "p")))
        .flat_map(|p| p.children().filter(|n| n.has_tag_name((PARAGRAPH, "run"))))
        .filter_map(|run| part(run, "secPr"))
        .find_map(|props| part(props, "pagePr"))
    else {
        return page;
    };
    if let (Some(width), Some(height)) = (
        number::<i64>(props, "width"),
        number::<i64>(props, "height"),
    ) && width > 0
        && height > 0
    {
        page.size = EmuSize::new(units(width), units(height));
    }
    if let Some(margin) = part(props, "margin") {
        for (name, field) in [
            ("top", &mut page.top),
            ("right", &mut page.right),
            ("bottom", &mut page.bottom),
            ("left", &mut page.left),
            ("header", &mut page.header),
            ("footer", &mut page.footer),
            ("gutter", &mut page.gutter),
        ] {
            if let Some(value) = number::<i64>(margin, name) {
                *field = units(value.max(0));
            }
        }
    }
    page
}

struct Importer<'a, 'package> {
    id: u32,
    runs: HashMap<u32, RunFormat>,
    paragraphs: HashMap<u32, ParaFormat>,
    borders: HashMap<u32, [Option<FlowBorder>; 4]>,
    archive: &'a Archive<'a>,
    items: &'a HashMap<&'package str, &'package str>,
    presentation: &'a mut Presentation,
    images: HashMap<&'package str, Option<MediaId>>,
    remaining_images: usize,
    remaining_cells: usize,
}

impl Importer<'_, '_> {
    fn next_id(&mut self) -> u32 {
        self.id += 1;
        self.id
    }

    fn paragraph(&mut self) -> FlowParagraph {
        FlowParagraph {
            id: self.next_id(),
            ..Default::default()
        }
    }

    fn read_styles(&mut self, root: Node<'_, '_>) {
        let mut fonts = HashMap::new();
        for group in root
            .descendants()
            .filter(|n| n.has_tag_name((HEAD, "fontface")))
        {
            let Some(language) = group.attribute("lang") else {
                continue;
            };
            for font in group.children().filter(|n| n.has_tag_name((HEAD, "font"))) {
                if let (Some(id), Some(face)) = (number::<u32>(font, "id"), font.attribute("face"))
                {
                    fonts.insert((language, id), face);
                }
            }
        }
        for style in root
            .descendants()
            .filter(|n| n.tag_name().namespace() == Some(HEAD))
        {
            let Some(id) = number(style, "id") else {
                continue;
            };
            let field = |name| style.children().find(|n| n.has_tag_name((HEAD, name)));
            match style.tag_name().name() {
                "charPr" => {
                    let font = |attribute, language| {
                        let id = number(field("fontRef")?, attribute)?;
                        fonts.get(&(language, id)).map(|font| (*font).to_owned())
                    };
                    let size =
                        number::<u32>(style, "height").filter(|v| (100..=163_800).contains(v));
                    let scale = field("ratio")
                        .and_then(|n| number::<u32>(n, "hangul"))
                        .filter(|v| (50..=200).contains(v));
                    let spacing = field("spacing")
                        .and_then(|n| number::<i64>(n, "hangul"))
                        .filter(|v| (-50..=50).contains(v))
                        .map(|v| units(v * i64::from(size.unwrap_or(1000))) / 100);
                    self.runs.insert(
                        id,
                        RunFormat {
                            font: font("latin", "LATIN"),
                            east_asian_font: font("hangul", "HANGUL"),
                            size,
                            scale,
                            spacing,
                            color: style.attribute("textColor").and_then(color),
                            bold: field("bold").map(|_| true),
                            italic: field("italic").map(|_| true),
                            underline: field("underline")
                                .map(|n| n.attribute("type") != Some("NONE")),
                            ..Default::default()
                        },
                    );
                }
                "paraPr" => {
                    self.paragraphs.insert(id, read_para_format(style));
                }
                "borderFill" => {
                    self.borders.insert(
                        id,
                        ["topBorder", "rightBorder", "bottomBorder", "leftBorder"]
                            .map(|name| field(name).and_then(read_border)),
                    );
                }
                _ => {}
            }
        }
    }

    fn blocks(&mut self, node: Node<'_, '_>, depth: usize) -> Result<Vec<Block>, Error> {
        if depth > 16 {
            return Err(Error::new("HWPX の表の入れ子が深すぎます"));
        }
        let mut blocks = Vec::new();
        for source in node.children().filter(|n| n.has_tag_name((PARAGRAPH, "p"))) {
            let mut paragraph = self.paragraph();
            paragraph.direct = number(source, "paraPrIDRef")
                .and_then(|id| self.paragraphs.get(&id))
                .cloned()
                .unwrap_or_default();
            if source.attribute("pageBreak") == Some("1") {
                paragraph.direct.page_break_before = Some(true);
            }
            if let Some(lines) = part(source, "linesegarray") {
                let mut pitches = lines
                    .children()
                    .filter(|n| n.has_tag_name((PARAGRAPH, "lineseg")))
                    .map(|line| {
                        let height = number::<i64>(line, "vertsize")?;
                        let extra = number::<i64>(line, "spacing")?;
                        line_pitch(height, extra)
                    });
                if let Some(Some(pitch)) = pitches.next()
                    && pitches.all(|candidate| candidate == Some(pitch))
                {
                    paragraph.direct.line = Some(pitch);
                }
            }
            let mut had_table = false;
            for run in source
                .children()
                .filter(|n| n.has_tag_name((PARAGRAPH, "run")))
            {
                let format = number(run, "charPrIDRef")
                    .and_then(|id| self.runs.get(&id))
                    .cloned()
                    .unwrap_or_default();
                if paragraph.runs.is_empty() {
                    paragraph.mark_format = format.clone();
                }
                for item in run.children() {
                    if item.has_tag_name((PARAGRAPH, "tbl")) {
                        if !paragraph.runs.is_empty()
                            || paragraph.direct.page_break_before == Some(true)
                        {
                            let mut next = self.paragraph();
                            next.direct = paragraph.direct.clone();
                            next.direct.page_break_before = None;
                            paragraph.resolve(&Default::default());
                            blocks.push(Block::Paragraph(std::mem::replace(&mut paragraph, next)));
                        }
                        blocks.push(Block::Table(self.table(item, depth)?));
                        had_table = true;
                    } else if item.has_tag_name((PARAGRAPH, "pic")) {
                        if let Some(kind) = self.picture(item)? {
                            paragraph.runs.push(FlowRun::new(kind, Default::default()));
                        }
                    } else if item.has_tag_name((PARAGRAPH, "t")) {
                        for child in item.children() {
                            let kind = if let Some(text) = child.text().filter(|_| child.is_text())
                            {
                                FlowRunKind::Text(text.to_owned())
                            } else if child.has_tag_name((PARAGRAPH, "tab")) {
                                FlowRunKind::Tab
                            } else if child.has_tag_name((PARAGRAPH, "lineBreak")) {
                                FlowRunKind::LineBreak
                            } else {
                                continue;
                            };
                            let mut run = FlowRun::new(kind, Default::default());
                            run.format = format.clone();
                            paragraph.runs.push(run);
                        }
                    }
                }
            }
            if !had_table || !paragraph.runs.is_empty() {
                paragraph.resolve(&Default::default());
                blocks.push(Block::Paragraph(paragraph));
            }
        }
        Ok(blocks)
    }

    fn picture(&mut self, node: Node<'_, '_>) -> Result<Option<FlowRunKind>, Error> {
        let Some(id) = node
            .children()
            .find(|n| n.has_tag_name((CORE, "img")))
            .and_then(|n| n.attribute("binaryItemIDRef"))
        else {
            return Ok(None);
        };
        let Some(&href) = self.items.get(id) else {
            return Ok(None);
        };
        let Some(size) = ["sz", "curSz", "orgSz"].into_iter().find_map(|name| {
            let n = part(node, name)?;
            let width = number::<i64>(n, "width")?;
            let height = number::<i64>(n, "height")?;
            (width > 0 && height > 0).then(|| EmuSize::new(units(width), units(height)))
        }) else {
            return Ok(None);
        };

        let media = if let Some(&media) = self.images.get(href) {
            media
        } else {
            let path = part_path(href)?;
            if !path.starts_with("BinData/") {
                return Ok(None);
            }
            let format = MediaFormat::from_path(&path);
            let media = if matches!(
                format,
                MediaFormat::Png | MediaFormat::Jpeg | MediaFormat::Gif | MediaFormat::Bmp
            ) {
                self.archive
                    .read_bounded(&path, self.remaining_images.min(16 * 1024 * 1024))?
                    .map(|bytes| {
                        self.remaining_images -= bytes.len();
                        self.presentation.add_media(format, bytes)
                    })
            } else {
                None
            };
            self.images.insert(href, media);
            media
        };
        Ok(media.map(|media| FlowRunKind::Picture {
            media,
            size,
            look: Default::default(),
        }))
    }

    fn table(&mut self, table: Node<'_, '_>, depth: usize) -> Result<FlowTable, Error> {
        let invalid = || Error::new("HWPX の表のセル配置が不正です");
        let rows = number::<usize>(table, "rowCnt")
            .filter(|&n| (1..=4096).contains(&n))
            .ok_or_else(invalid)?;
        let cols = number::<usize>(table, "colCnt")
            .filter(|&n| (1..=256).contains(&n))
            .ok_or_else(invalid)?;
        if rows * cols > 65_536 || rows * cols > self.remaining_cells {
            return Err(invalid());
        }
        self.remaining_cells -= rows * cols;
        let mut occupied = vec![false; rows * cols];
        let mut grid = vec![Vec::new(); rows];
        let mut widths = vec![0; cols];
        let mut heights = vec![0; rows];
        let mut constraints = Vec::new();
        for cell in table
            .children()
            .filter(|n| n.has_tag_name((PARAGRAPH, "tr")))
            .flat_map(|row| row.children().filter(|n| n.has_tag_name((PARAGRAPH, "tc"))))
        {
            let addr = part(cell, "cellAddr").ok_or_else(invalid)?;
            let col = number::<usize>(addr, "colAddr")
                .filter(|&n| n < cols)
                .ok_or_else(invalid)?;
            let row = number::<usize>(addr, "rowAddr")
                .filter(|&n| n < rows)
                .ok_or_else(invalid)?;
            let span = part(cell, "cellSpan");
            let across = span
                .and_then(|n| number::<usize>(n, "colSpan"))
                .unwrap_or(1);
            let down = span
                .and_then(|n| number::<usize>(n, "rowSpan"))
                .unwrap_or(1);
            if !(1..=cols - col).contains(&across) || !(1..=rows - row).contains(&down) {
                return Err(invalid());
            }
            let borders = number(cell, "borderFillIDRef")
                .and_then(|id| self.borders.get(&id))
                .copied()
                .unwrap_or_default();
            let size = part(cell, "cellSz");
            if let Some(width) = size
                .and_then(|n| number::<i64>(n, "width"))
                .filter(|&n| n > 0)
            {
                constraints.push((across, col, units(width)));
            }
            if down == 1
                && let Some(height) = size
                    .and_then(|n| number::<i64>(n, "height"))
                    .filter(|&n| n > 0)
            {
                heights[row] = heights[row].max(units(height));
            }
            let mut blocks = match part(cell, "subList") {
                Some(content) => self.blocks(content, depth + 1)?,
                None => Vec::new(),
            };
            let margins = ["top", "right", "bottom", "left"].map(|name| {
                part(cell, "cellMargin")
                    .and_then(|n| number::<i64>(n, name))
                    .map_or(0, |n| units(n.max(0)))
            });
            for line in row..row + down {
                for slot in &mut occupied[line * cols + col..line * cols + col + across] {
                    if std::mem::replace(slot, true) {
                        return Err(invalid());
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
                        own_borders: borders.map(Some),
                        blocks: std::mem::take(&mut blocks),
                        ..Default::default()
                    },
                ));
            }
        }
        constraints.sort_unstable();
        for (span, start, width) in constraints {
            let slice = &mut widths[start..start + span];
            let used: Emu = slice.iter().sum();
            let missing = slice.iter().filter(|&&n| n == 0).count();
            if missing > 0 {
                for cell in slice.iter_mut().filter(|n| **n == 0) {
                    *cell = ((width - used) / missing as Emu).max(1);
                }
            }
        }
        let columns = widths
            .into_iter()
            .map(|n| if n == 0 { units(7200) } else { n })
            .collect();
        let mut table_rows = Vec::with_capacity(rows);
        for (mut cells, height) in grid.into_iter().zip(heights) {
            cells.sort_unstable_by_key(|(col, _)| *col);
            let mut row = FlowRow {
                id: self.next_id(),
                height: if height == 0 {
                    RowHeight::Auto
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
            table_rows.push(row);
        }
        Ok(FlowTable {
            id: self.next_id(),
            columns,
            rows: table_rows,
            indent: part(table, "outMargin")
                .and_then(|m| number::<i64>(m, "left"))
                .map_or(0, units),
            ..Default::default()
        })
    }
}

fn read_para_format(style: Node<'_, '_>) -> ParaFormat {
    let field = |name| style.children().find(|n| n.has_tag_name((HEAD, name)));
    let alternative = part(style, "switch").and_then(|switch| {
        switch.children().find(|n| {
            n.has_tag_name((PARAGRAPH, "case"))
                && n.attribute((PARAGRAPH, "required-namespace"))
                    == Some("http://www.hancom.co.kr/hwpml/2016/HwpUnitChar")
        })
    });
    let measurement = |name| {
        field(name).or_else(|| {
            alternative?
                .children()
                .find(|n| n.has_tag_name((HEAD, name)))
        })
    };
    let margin = |name| {
        let margin = measurement("margin")?
            .children()
            .find(|n| n.has_tag_name((CORE, name)))?;
        (margin.attribute("unit") == Some("HWPUNIT")).then_some(())?;
        number::<i64>(margin, "value").map(units)
    };
    let line = measurement("lineSpacing").and_then(|line| {
        let value = number::<u32>(line, "value")?;
        match line.attribute("type")? {
            "PERCENT" if (50..=500).contains(&value) => Some(LineHeight::Auto(value * 240 / 100)),
            "FIXED" if value <= 72_000 => Some(LineHeight::Exact(units(i64::from(value)))),
            "AT_LEAST" if value <= 72_000 => Some(LineHeight::AtLeast(units(i64::from(value)))),
            _ => None,
        }
    });
    let align = field("align")
        .and_then(|n| n.attribute("horizontal"))
        .map(|align| match align {
            "CENTER" => FlowAlign::Center,
            "RIGHT" => FlowAlign::Right,
            "JUSTIFY" => FlowAlign::Justify,
            "DISTRIBUTE" | "DISTRIBUTE_SPACE" => FlowAlign::Distribute,
            _ => FlowAlign::Left,
        });
    let flag = |name| {
        field("breakSetting")?
            .attribute(name)
            .map(|value| value == "1")
    };
    ParaFormat {
        align,
        left: margin("left"),
        right: margin("right"),
        first_line: margin("intent"),
        before: margin("prev"),
        after: margin("next"),
        line,
        keep_next: flag("keepWithNext"),
        keep_lines: flag("keepLines"),
        page_break_before: flag("pageBreakBefore"),
        ..Default::default()
    }
}

fn read_border(node: Node<'_, '_>) -> Option<FlowBorder> {
    let style = match node.attribute("type")? {
        "NONE" => return None,
        "DASH" => BorderStyle::Dashed,
        "DOT" => BorderStyle::Dotted,
        _ => BorderStyle::Single,
    };
    let width = node
        .attribute("width")?
        .strip_suffix(" mm")?
        .parse::<f64>()
        .ok()?;
    if !(0.0..=10.0).contains(&width) {
        return None;
    }
    Some(FlowBorder {
        style,
        width: (width * 36_000.0).round() as Emu,
        color: node
            .attribute("color")
            .and_then(color)
            .unwrap_or(Rgb::BLACK),
        space: 0,
        art: None,
    })
}
