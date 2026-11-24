// SPDX-License-Identifier: AGPL-3.0-only

//! Import the text-document branch of the Chinese Uniform Office Format.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use ibis_doc_model::{
    Block, FlowCell, FlowParagraph, FlowRow, FlowRun, FlowRunKind, FlowStyle, FlowStyles,
    FlowTable, ParaFormat, Presentation, RunFormat, StyleKind, VerticalMerge,
};
use roxmltree::{Document, Node, ParsingOptions};

use crate::{Error, Kind, Opened, SaveOptions};

mod images;

const UOF: &str = "http://schemas.uof.org/cn/2003/uof";
const WORD: &str = "http://schemas.uof.org/cn/2003/uof-wordproc";
const MAX_XML_BYTES: usize = 16 * 1024 * 1024;
const MAX_XML_NODES: u32 = 300_000;
const MAX_TABLE_COLUMNS: usize = 1024;

fn word_child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.has_tag_name((WORD, name)))
}

fn parse(data: &[u8]) -> Result<Document<'_>, Error> {
    if data.len() > MAX_XML_BYTES {
        return Err(Error::new("UOT XML exceeds the import size limit"));
    }
    let text = std::str::from_utf8(data).map_err(|_| Error::new("UOT XML must be UTF-8"))?;
    Document::parse_with_options(
        text.strip_prefix('\u{feff}').unwrap_or(text),
        ParsingOptions {
            nodes_limit: MAX_XML_NODES,
            ..ParsingOptions::default()
        },
    )
    .map_err(|error| Error::new(format!("Invalid UOT XML: {error}")))
}

fn body<'a, 'input>(root: Node<'a, 'input>) -> Option<Node<'a, 'input>> {
    if !root.has_tag_name((UOF, "UOF")) {
        return None;
    }
    let document = root
        .children()
        .find(|node| node.has_tag_name((UOF, "文字处理")))?;
    word_child(document, "主体")
}

pub fn is_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("uot")
}

pub fn is_uot(data: &[u8]) -> bool {
    let prefix = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data);
    prefix
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        == Some(b'<')
        && data.windows(UOF.len()).any(|part| part == UOF.as_bytes())
        && parse(data).is_ok_and(|document| body(document.root_element()).is_some())
}

pub fn open(data: &[u8]) -> Result<Opened, Error> {
    open_with_base_dir(data, None)
}

pub fn open_with_base_dir(data: &[u8], base_dir: Option<&Path>) -> Result<Opened, Error> {
    crate::open(to_docx_with_base_dir(data, base_dir)?)
}

/// Generate a Word document with editable paragraphs, headings, tables and inline images.
pub fn to_docx(data: &[u8]) -> Result<Vec<u8>, Error> {
    to_docx_with_base_dir(data, None)
}

/// Convert UOT to DOCX, resolving local images relative to the source directory.
pub fn to_docx_with_base_dir(data: &[u8], base_dir: Option<&Path>) -> Result<Vec<u8>, Error> {
    let xml = parse(data)?;
    let root = xml.root_element();
    let content = body(root).ok_or_else(|| Error::new("UOT text document body not found"))?;
    let mut blank = crate::open(crate::blank_file(Kind::Document)?)?;
    let mut flow = Arc::unwrap_or_clone(blank.presentation.flow.take().expect("blank DOCX flow"));
    let anchor = match flow.body.first() {
        Some(Block::Paragraph(paragraph)) => paragraph.clone(),
        _ => return Err(Error::new("Missing blank DOCX paragraph")),
    };
    let mut importer = Importer {
        next_id: flow.new_id(),
        paragraph_styles: paragraph_styles(root),
        remaining_continuations: MAX_XML_NODES,
        styles: Arc::make_mut(&mut flow.styles),
        presentation: &mut blank.presentation,
        images: images::Images::new(root, base_dir),
    };
    let blocks = importer.blocks(content, 0)?;
    if !blocks.is_empty() {
        flow.body = blocks;
        // Keep the blank source paragraph as an insertion anchor for the
        // incremental DOCX writer, even when the source begins with a table.
        if let Some(first) = flow.body.iter_mut().find_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            Block::Table(_) => None,
        }) {
            first.id = anchor.id;
        } else {
            flow.body.push(Block::Paragraph(anchor));
        }
    }
    blank.presentation.flow = Some(flow.into());
    blank
        .source
        .save(&blank.presentation, SaveOptions::default())
}

type ParagraphStyles<'a, 'input> = HashMap<&'a str, Node<'a, 'input>>;

fn paragraph_styles<'a, 'input>(root: Node<'a, 'input>) -> ParagraphStyles<'a, 'input> {
    root.children()
        .find(|node| node.has_tag_name((UOF, "式样集")))
        .into_iter()
        .flat_map(|styles| styles.children())
        .filter(|node| node.has_tag_name((UOF, "段落式样")))
        .filter_map(|node| Some((node.attribute((WORD, "标识符"))?, node)))
        .collect()
}

struct Importer<'a, 'xml, 'input, 'path> {
    next_id: u32,
    paragraph_styles: ParagraphStyles<'xml, 'input>,
    remaining_continuations: u32,
    styles: &'a mut FlowStyles,
    presentation: &'a mut Presentation,
    images: images::Images<'xml, 'input, 'path>,
}

impl Importer<'_, '_, '_, '_> {
    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn blocks(&mut self, parent: Node<'_, '_>, depth: usize) -> Result<Vec<Block>, Error> {
        if depth > 16 {
            return Err(Error::new("UOT block nesting exceeds the import limit"));
        }
        let mut blocks = Vec::new();
        for child in parent
            .children()
            .filter(|node| node.tag_name().namespace() == Some(WORD))
        {
            match child.tag_name().name() {
                "段落" => blocks.push(Block::Paragraph(self.paragraph(child))),
                "文字表" => blocks.push(Block::Table(self.table(child, depth + 1)?)),
                "节" | "章节" => blocks.extend(self.blocks(child, depth + 1)?),
                _ => {}
            }
        }
        Ok(blocks)
    }

    fn paragraph(&mut self, node: Node<'_, '_>) -> FlowParagraph {
        let style = word_child(node, "段落属性")
            .and_then(|properties| self.heading_level(properties))
            .map(|level| self.heading_style(level));
        let mut paragraph = FlowParagraph {
            id: self.id(),
            style,
            ..Default::default()
        };
        for child in node.children() {
            if child.has_tag_name((WORD, "句")) {
                for item in child.children() {
                    self.append_run_item(&mut paragraph, item);
                }
            } else if child.has_tag_name((WORD, "锚点")) {
                self.append_run_item(&mut paragraph, child);
            }
        }
        paragraph.resolve(self.styles);
        paragraph.normalize();
        paragraph
    }

    fn append_run_item(&mut self, paragraph: &mut FlowParagraph, item: Node<'_, '_>) {
        let kind = if item.has_tag_name((WORD, "文本串")) {
            item.text().map(|text| FlowRunKind::Text(text.into()))
        } else if item.has_tag_name((WORD, "锚点")) {
            self.images.picture(item, self.presentation)
        } else {
            None
        };
        if let Some(kind) = kind {
            paragraph.runs.push(FlowRun::new(kind, Default::default()));
        }
    }

    fn heading_level(&self, properties: Node<'_, '_>) -> Option<u8> {
        fn level(node: Node<'_, '_>) -> Option<u8> {
            word_child(node, "大纲级别")?.text()?.trim().parse().ok()
        }

        if let Some(level) = level(properties) {
            return (1..=9).contains(&level).then_some(level);
        }
        let mut reference = properties.attribute((WORD, "式样引用"));
        let mut visited = HashSet::new();
        while let Some(id) = reference.filter(|id| visited.len() < 64 && visited.insert(*id)) {
            let style = *self.paragraph_styles.get(id)?;
            if let Some(level) =
                level(style).or_else(|| word_child(style, "段落属性").and_then(level))
            {
                return (1..=9).contains(&level).then_some(level);
            }
            reference = style.attribute((WORD, "基式样引用"));
        }
        None
    }

    fn heading_style(&mut self, level: u8) -> String {
        let name = format!("heading {level}");
        let normal = self.styles.default_paragraph.clone();
        let id = format!("Heading{level}");
        self.styles
            .styles
            .entry(id.clone())
            .or_insert_with(|| FlowStyle {
                kind: StyleKind::Paragraph,
                name,
                based_on: normal.clone(),
                next: normal,
                quick: true,
                para: ParaFormat {
                    keep_next: Some(true),
                    ..Default::default()
                },
                run: RunFormat {
                    bold: Some(true),
                    ..Default::default()
                },
                ..Default::default()
            });
        id
    }

    fn table(&mut self, node: Node<'_, '_>, depth: usize) -> Result<FlowTable, Error> {
        let row_nodes: Vec<_> = node
            .children()
            .filter(|row| row.has_tag_name((WORD, "行")))
            .collect();
        let mut rows = Vec::with_capacity(row_nodes.len());
        let mut merges = BTreeMap::new();
        let mut width = 0;
        for (row_index, row) in row_nodes.iter().copied().enumerate() {
            let mut continuing = std::mem::take(&mut merges);
            let mut cells = Vec::new();
            let mut column = 0;
            for cell in row
                .children()
                .filter(|cell| cell.has_tag_name((WORD, "单元格")))
            {
                self.continue_cells(
                    &mut continuing,
                    &mut merges,
                    row_index,
                    &mut column,
                    &mut cells,
                )?;
                let span = cell_span(cell, "跨列")?;
                let down = cell_span(cell, "跨行")?;
                let end = column + span;
                if end > MAX_TABLE_COLUMNS
                    || row_index + down > row_nodes.len()
                    || continuing
                        .first_key_value()
                        .is_some_and(|(&start, _)| start < end)
                {
                    return Err(Error::new(
                        "UOT cell span exceeds or overlaps the table grid",
                    ));
                }
                if down > 1 {
                    merges.insert(column, (span, row_index + down));
                }
                let mut blocks = self.blocks(cell, depth)?;
                if blocks.is_empty() {
                    blocks.push(Block::Paragraph(FlowParagraph {
                        id: self.id(),
                        ..Default::default()
                    }));
                }
                cells.push(FlowCell {
                    id: self.id(),
                    span: span as u32,
                    merge: if down > 1 {
                        VerticalMerge::Start
                    } else {
                        VerticalMerge::None
                    },
                    blocks,
                    ..Default::default()
                });
                column = end;
            }
            self.continue_cells(
                &mut continuing,
                &mut merges,
                row_index,
                &mut column,
                &mut cells,
            )?;
            if cells.is_empty() || !continuing.is_empty() || (width != 0 && width != column) {
                return Err(Error::new(
                    "UOT table rows must cover the same number of columns",
                ));
            }
            width = column;
            let header = word_child(row, "表行属性")
                .and_then(|properties| word_child(properties, "表头行"))
                .is_some_and(|flag| flag.attribute((WORD, "值")) != Some("false"));
            rows.push(FlowRow {
                id: self.id(),
                header,
                cells,
                ..Default::default()
            });
        }
        if rows.is_empty() {
            return Err(Error::new("UOT table has no rows"));
        }
        Ok(FlowTable {
            id: self.id(),
            columns: vec![5_400_000 / width as i64; width],
            rows,
            ..Default::default()
        })
    }

    fn continue_cells(
        &mut self,
        continuing: &mut BTreeMap<usize, (usize, usize)>,
        merges: &mut BTreeMap<usize, (usize, usize)>,
        row_index: usize,
        column: &mut usize,
        cells: &mut Vec<FlowCell>,
    ) -> Result<(), Error> {
        // UOF omits covered cells. DOCX and the Markdown annotations need a
        // continuation cell of the same width in each of the covered rows.
        while let Some((span, row_end)) = continuing.remove(column) {
            self.remaining_continuations = self
                .remaining_continuations
                .checked_sub(1)
                .ok_or_else(|| Error::new("UOT merged cells exceed the import limit"))?;
            if row_index + 1 < row_end {
                merges.insert(*column, (span, row_end));
            }
            cells.push(FlowCell {
                id: self.id(),
                span: span as u32,
                merge: VerticalMerge::Continue,
                blocks: vec![Block::Paragraph(FlowParagraph {
                    id: self.id(),
                    ..Default::default()
                })],
                ..Default::default()
            });
            *column += span;
        }
        Ok(())
    }
}

fn cell_span(cell: Node<'_, '_>, name: &str) -> Result<usize, Error> {
    let Some(span) =
        word_child(cell, "单元格属性").and_then(|properties| word_child(properties, name))
    else {
        return Ok(1);
    };
    // UOF writes 跨列 as a 值 attribute and 跨行 as element text.
    span.attribute((WORD, "值"))
        .or_else(|| span.text())
        .and_then(|value| value.trim().parse().ok())
        .filter(|value| (1..=MAX_XML_NODES as usize).contains(value))
        .ok_or_else(|| Error::new(format!("Invalid UOT cell span: {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ibis_doc_model::EmuSize;

    const IMAGE: &str = "iVBORw0KGgoAAAANSUhEUgAAAGAAAAAwCAIAAABhdOiYAAAAl0lEQVR4nO3aMRHCUBAA0R8mrrCAEgShBAtxERGpUyGA8FcA75V33c6Vt+zHOfjtNtkhUBMoCBQECgKF9XJ6fzzH/9ner++hCwoCBYGCQEGgIFAQKAgUBAoCBYGCQEGgIFAQKAgUBAoCBYGCQEGgIFAQKAgUBAoCBYGCQEGgIFAQKAgUBAqLR/I5FxQECgIFgYJAQaAx9wGKmAjtUVq7TgAAAABJRU5ErkJggg==";

    fn image_example(object: &str) -> String {
        format!(
            r#"<uof:UOF xmlns:uof="{UOF}" xmlns:w="{WORD}" xmlns:g="http://schemas.uof.org/cn/2003/graph">
  <uof:对象集>
    <g:图形 g:标识符="graphic" g:其他对象="asset"/>
    <uof:其他对象 uof:标识符="asset">{object}</uof:其他对象>
  </uof:对象集>
  <uof:文字处理>
    <w:公用处理规则><w:文档设置><w:度量单位>mm</w:度量单位></w:文档设置></w:公用处理规则>
    <w:主体><w:段落><w:句><w:文本串>前</w:文本串><w:锚点>
      <w:锚点属性><w:宽度>50.8</w:宽度><w:高度>25.4</w:高度></w:锚点属性>
      <w:图形 w:图形引用="graphic"/>
    </w:锚点><w:文本串>後</w:文本串></w:句></w:段落></w:主体>
  </uof:文字处理>
</uof:UOF>"#
        )
    }

    fn assert_image(presentation: &Presentation) {
        let [Block::Paragraph(paragraph)] = presentation.flow.as_ref().unwrap().body.as_slice()
        else {
            panic!("expected one paragraph");
        };
        let [before, picture, after] = paragraph.runs.as_slice() else {
            panic!("expected text, image and text");
        };
        assert_eq!(before.kind, FlowRunKind::Text("前".into()));
        assert_eq!(after.kind, FlowRunKind::Text("後".into()));
        let (media, size, _) = picture.picture().expect("inline image");
        assert_eq!(size, EmuSize::new(1_828_800, 914_400));
        assert_eq!(
            presentation.media(media).unwrap().data.as_ref(),
            STANDARD.decode(IMAGE).unwrap().as_slice()
        );
    }

    #[test]
    fn preserves_inline_images_through_docx_and_markdown() {
        let manuscript = image_example(&format!("<uof:数据>{IMAGE}</uof:数据>"));
        let opened = open(manuscript.as_bytes()).unwrap();
        assert_image(&opened.presentation);
        let docx = opened
            .source
            .save(&opened.presentation, SaveOptions::default())
            .unwrap();
        assert_image(&crate::open(&docx[..]).unwrap().presentation);
        let markdown =
            crate::markdown::export_presentation_to_markdown(&opened.presentation).unwrap();
        assert_image(&crate::markdown::read_markdown(markdown.as_bytes()).unwrap());
        let from_markdown =
            crate::markdown::import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        assert_image(&crate::open(&from_markdown[..]).unwrap().presentation);
    }

    #[test]
    fn preserves_floating_image_placement_and_wrapping_through_docx_and_markdown() {
        for (mode, wrap, side, behind) in [
            ("square", "square", Some("bothSides"), false),
            ("tight", "tight", Some("left"), false),
            ("through", "through", Some("right"), false),
            ("top-bottom", "topAndBottom", None, false),
            ("behindtext", "none", None, true),
            ("infrontoftext", "none", None, false),
        ] {
            let extras = format!(
                r#"<w:位置>
                    <w:水平 w:相对于="margin"><w:绝对 w:值="40.01"/></w:水平>
                    <w:垂直 w:相对于="paragraph"><w:绝对 w:值="-1.01"/></w:垂直>
                </w:位置>
                <w:绕排 w:绕排方式="{mode}" w:环绕文字="{side}"/>
                <w:边距 w:上="1.01" w:下="1.5" w:左="1.99" w:右="2.5"/>
                <w:锁定 w:值="true"/><w:允许重叠 w:值="false"/>"#,
                side = side.unwrap_or(""),
            );
            let manuscript = image_example(&format!("<uof:数据>{IMAGE}</uof:数据>"))
                .replace("<w:锚点>", r#"<w:锚点 w:类型="normal">"#)
                .replace("</w:锚点属性>", &format!("{extras}</w:锚点属性>"));
            let docx = to_docx(manuscript.as_bytes()).unwrap();
            let direct = crate::read(&docx).unwrap();
            let md = crate::markdown::export_presentation_to_markdown(&direct).unwrap();
            assert!(md.contains("docx-anchor="), "{mode}");
            let from_md = crate::markdown::read_markdown(md.as_bytes()).unwrap();
            let rebuilt = crate::markdown::import_markdown_to_docx_bytes(md.as_bytes()).unwrap();
            for presentation in [direct, from_md, crate::read(&rebuilt).unwrap()] {
                let [Block::Paragraph(paragraph)] =
                    presentation.flow.as_ref().unwrap().body.as_slice()
                else {
                    panic!("expected one paragraph");
                };
                let [before, image, after] = paragraph.runs.as_slice() else {
                    panic!("expected text, image, text");
                };
                assert_eq!(before.kind, FlowRunKind::Text("前".into()));
                assert_eq!(after.kind, FlowRunKind::Text("後".into()));
                let FlowRunKind::Shape(shape) = &image.kind else {
                    panic!("expected floating shape");
                };
                let ibis_doc_model::ShapeKind::Picture(picture) = &shape.kind else {
                    panic!("expected picture");
                };
                assert_eq!(
                    presentation.media(picture.media).unwrap().data.as_ref(),
                    STANDARD.decode(IMAGE).unwrap()
                );
                assert_eq!(
                    (shape.frame.width, shape.frame.height),
                    (1_828_800, 914_400)
                );
                let anchor = shape.floating.as_ref().unwrap();
                assert_eq!((anchor.offset_x, anchor.offset_y), (1_440_360, -36_360));
                assert_eq!(
                    (anchor.relative_h.as_str(), anchor.relative_v.as_str()),
                    ("margin", "paragraph")
                );
                assert_eq!(anchor.wrap, wrap);
                assert_eq!(anchor.wrap_text.as_deref(), side);
                assert_eq!(anchor.behind_doc, behind);
                assert_eq!(
                    (anchor.dist_t, anchor.dist_b, anchor.dist_l, anchor.dist_r),
                    (36_360, 54_000, 71_640, 90_000)
                );
                assert!(anchor.locked);
                assert!(!anchor.allow_overlap);
            }
        }
    }

    #[test]
    fn preserves_relative_alignment_for_paragraph_level_float() {
        let manuscript = image_example(&format!("<uof:数据>{IMAGE}</uof:数据>"))
            .replace(r#"uof:标识符="asset""#, r#"uof:标识符="graphic""#)
            .replace("<w:锚点>", r#"</w:句><w:锚点 w:类型="normal">"#)
            .replace("</w:锚点>", "</w:锚点><w:句>")
            .replace(
                "</w:锚点属性>",
                r#"<w:位置>
                <w:水平 w:相对于="page"><w:相对 w:值="right"/></w:水平>
                <w:垂直 w:相对于="margin"><w:相对 w:值="center"/></w:垂直>
            </w:位置></w:锚点属性>"#,
            );
        let opened = open(manuscript.as_bytes()).unwrap();
        let md = crate::markdown::export_presentation_to_markdown(&opened.presentation).unwrap();
        for presentation in [
            opened.presentation,
            crate::markdown::read_markdown(md.as_bytes()).unwrap(),
        ] {
            let flow = presentation.flow.as_ref().unwrap();
            let Block::Paragraph(paragraph) = &flow.body[0] else {
                panic!("expected paragraph");
            };
            let FlowRunKind::Shape(shape) = &paragraph.runs[1].kind else {
                panic!("expected shape");
            };
            let anchor = shape.floating.as_ref().unwrap();
            assert_eq!(anchor.align_h.as_deref(), Some("right"));
            assert_eq!(anchor.align_v.as_deref(), Some("center"));
            assert_eq!(
                (anchor.relative_h.as_str(), anchor.relative_v.as_str()),
                ("page", "margin")
            );
        }
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn resolves_only_images_inside_the_document_directory() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("document");
        std::fs::create_dir(&base).unwrap();
        let png = STANDARD.decode(IMAGE).unwrap();
        std::fs::write(base.join("figure.png"), &png).unwrap();
        std::fs::write(temp.path().join("outside.png"), &png).unwrap();
        for reference in ["figure.png", "../document/figure.png"] {
            let linked = image_example(&format!("<uof:路径>{reference}</uof:路径>"));
            assert_image(
                &open_with_base_dir(linked.as_bytes(), Some(&base))
                    .unwrap()
                    .presentation,
            );
        }
        for reference in [
            "../outside.png",
            "missing.png",
            "https://example.com/figure.png",
        ] {
            let linked = image_example(&format!("<uof:路径>{reference}</uof:路径>"));
            let presentation = open_with_base_dir(linked.as_bytes(), Some(&base))
                .unwrap()
                .presentation;
            assert!(presentation.media.is_empty());
            let [Block::Paragraph(paragraph)] = presentation.flow.as_ref().unwrap().body.as_slice()
            else {
                panic!("expected one paragraph");
            };
            assert_eq!(paragraph.text(), "前後");
        }
    }

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<uof:UOF xmlns:uof="http://schemas.uof.org/cn/2003/uof" xmlns:w="http://schemas.uof.org/cn/2003/uof-wordproc">
  <uof:式样集>
    <uof:段落式样 w:标识符="H1"><w:大纲级别>1</w:大纲级别></uof:段落式样>
    <uof:段落式样 w:标识符="P1" w:基式样引用="H1"/>
  </uof:式样集>
  <uof:文字处理><w:主体>
    <w:段落><w:段落属性 w:式样引用="P1"/><w:句><w:文本串>中文需求</w:文本串></w:句></w:段落>
    <w:段落><w:句><w:文本串>正文と</w:文本串></w:句><w:句><w:文本串>日本語 &amp; 中文。</w:文本串></w:句></w:段落>
    <w:段落><w:段落属性><w:大纲级别>2</w:大纲级别></w:段落属性><w:句><w:文本串>明細</w:文本串></w:句></w:段落>
    <w:文字表>
      <w:行><w:表行属性><w:表头行 w:值="true"/></w:表行属性>
        <w:单元格><w:段落><w:句><w:文本串>编号</w:文本串></w:句></w:段落></w:单元格>
        <w:单元格><w:段落><w:句><w:文本串>说明</w:文本串></w:句></w:段落></w:单元格>
      </w:行>
      <w:行>
        <w:单元格><w:段落><w:句><w:文本串>A1</w:文本串></w:句></w:段落></w:单元格>
        <w:单元格><w:段落><w:句><w:文本串>认証</w:文本串></w:句></w:段落></w:单元格>
      </w:行>
    </w:文字表>
    <w:段落><w:句><w:文本串>结尾。</w:文本串></w:句></w:段落>
  </w:主体></uof:文字处理>
</uof:UOF>"#;

    fn assert_structure(flow: &ibis_doc_model::FlowDocument) {
        let [
            Block::Paragraph(title),
            Block::Paragraph(body),
            Block::Paragraph(section),
            Block::Table(table),
            Block::Paragraph(tail),
        ] = flow.body.as_slice()
        else {
            panic!("expected headings, paragraphs and table in source order");
        };
        use ibis_doc_model::flow_command::heading_level;
        assert_eq!(heading_level(&flow.styles, title), Some(1));
        assert_eq!(heading_level(&flow.styles, section), Some(2));
        assert_eq!(title.text(), "中文需求");
        assert_eq!(body.text(), "正文と日本語 & 中文。");
        assert_eq!(section.text(), "明細");
        assert_eq!(tail.text(), "结尾。");
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.rows.len(), 2);
        assert!(table.rows[0].header);
        assert_eq!(
            table
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .map(|cell| match &cell.blocks[..] {
                            [Block::Paragraph(p)] => p.text(),
                            _ => panic!("expected one paragraph in each cell"),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
            [["编号", "说明"], ["A1", "认証"]]
        );
    }

    #[test]
    fn imports_and_reopens_docx_and_markdown() {
        assert!(is_uot(SAMPLE.as_bytes()));
        let opened = crate::open(SAMPLE.as_bytes()).unwrap();
        assert_structure(opened.presentation.flow.as_deref().unwrap());
        let docx = opened
            .source
            .save(&opened.presentation, SaveOptions::default())
            .unwrap();
        let reopened = crate::open(docx.as_slice()).unwrap();
        assert_structure(reopened.presentation.flow.as_deref().unwrap());
        let styles = crate::zip::Archive::new(&docx)
            .unwrap()
            .read("word/styles.xml")
            .unwrap()
            .unwrap();
        let styles = Document::parse(std::str::from_utf8(&styles).unwrap()).unwrap();
        let word = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        for (id, level) in [("Heading1", "0"), ("Heading2", "1")] {
            let outline = styles
                .descendants()
                .find(|node| {
                    node.has_tag_name((word, "style"))
                        && node.attribute((word, "styleId")) == Some(id)
                })
                .and_then(|style| {
                    style
                        .descendants()
                        .find(|node| node.has_tag_name((word, "outlineLvl")))
                })
                .and_then(|outline| outline.attribute((word, "val")));
            assert_eq!(outline, Some(level));
        }
        let md = crate::markdown::export_docx_to_markdown(&docx).unwrap();
        assert!(md.contains("# 中文需求\n"));
        assert!(md.contains("## 明細\n"));
        let reopened = crate::markdown::read_markdown(md.as_bytes()).unwrap();
        assert_structure(reopened.flow.as_deref().unwrap());
    }

    fn merged_example() -> String {
        let (prefix, after) = SAMPLE.split_once("<w:文字表>").unwrap();
        let (_, suffix) = after.split_once("</w:文字表>").unwrap();
        format!(
            "{prefix}<{table}{suffix}",
            table = r#"w:文字表>
          <w:行><w:表行属性><w:表头行 w:值="true"/></w:表行属性>
            <w:单元格><w:单元格属性><w:跨列 w:值="2"/></w:单元格属性><w:段落><w:句><w:文本串>横結合</w:文本串></w:句></w:段落></w:单元格>
            <w:单元格><w:段落><w:句><w:文本串>備考</w:文本串></w:句></w:段落></w:单元格>
          </w:行>
          <w:行>
            <w:单元格><w:段落><w:句><w:文本串>左上</w:文本串></w:句></w:段落></w:单元格>
            <w:单元格><w:单元格属性><w:跨行>2</w:跨行></w:单元格属性><w:段落><w:句><w:文本串>縦結合</w:文本串></w:句></w:段落></w:单元格>
            <w:单元格><w:段落><w:句><w:文本串>甲</w:文本串></w:句></w:段落></w:单元格>
          </w:行>
          <w:行>
            <w:单元格><w:段落><w:句><w:文本串>左下</w:文本串></w:句></w:段落></w:单元格>
            <w:单元格><w:段落><w:句><w:文本串>乙</w:文本串></w:句></w:段落></w:单元格>
          </w:行>
          <w:行>
            <w:单元格><w:单元格属性><w:跨行>2</w:跨行><w:跨列 w:值="2"/></w:单元格属性><w:段落><w:句><w:文本串>横と縦</w:文本串></w:句></w:段落></w:单元格>
            <w:单元格><w:段落><w:句><w:文本串>丙</w:文本串></w:句></w:段落></w:单元格>
          </w:行>
          <w:行>
            <w:单元格><w:段落><w:句><w:文本串>丁</w:文本串></w:句></w:段落></w:单元格>
          </w:行>
        </w:文字表>"#
        )
    }

    fn assert_merged_table(presentation: &ibis_doc_model::Presentation) {
        use VerticalMerge::{Continue, None as Unmerged, Start};
        let flow = presentation.flow.as_deref().unwrap();
        let Some(Block::Table(table)) = flow.body.get(3) else {
            panic!("expected table after two headings and a paragraph");
        };
        assert_eq!(table.columns.len(), 3);
        assert!(table.rows[0].header);
        let expected = [
            vec![(2, Unmerged, "横結合"), (1, Unmerged, "備考")],
            vec![
                (1, Unmerged, "左上"),
                (1, Start, "縦結合"),
                (1, Unmerged, "甲"),
            ],
            vec![
                (1, Unmerged, "左下"),
                (1, Continue, ""),
                (1, Unmerged, "乙"),
            ],
            vec![(2, Start, "横と縦"), (1, Unmerged, "丙")],
            vec![(2, Continue, ""), (1, Unmerged, "丁")],
        ];
        assert_eq!(table.rows.len(), expected.len());
        for (row, expected) in table.rows.iter().zip(expected) {
            assert_eq!(row.cells.len(), expected.len());
            for (cell, (width, merge, text)) in row.cells.iter().zip(expected) {
                assert_eq!((cell.span, cell.merge), (width, merge));
                let content: String = cell
                    .blocks
                    .iter()
                    .filter_map(|block| match block {
                        Block::Paragraph(paragraph) => Some(paragraph.text()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(content, text);
            }
        }
    }

    #[test]
    fn preserves_horizontal_vertical_and_combined_merges_through_docx_and_markdown() {
        let manuscript = merged_example();
        let opened = open(manuscript.as_bytes()).unwrap();
        assert_merged_table(&opened.presentation);
        let docx = opened
            .source
            .save(&opened.presentation, SaveOptions::default())
            .unwrap();
        assert_merged_table(&crate::open(docx.as_slice()).unwrap().presentation);
        let xml = crate::zip::Archive::new(&docx)
            .unwrap()
            .read("word/document.xml")
            .unwrap()
            .unwrap();
        let document = Document::parse(std::str::from_utf8(&xml).unwrap()).unwrap();
        let word = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        assert!(
            document
                .descendants()
                .filter(|node| node.has_tag_name((word, "tc")))
                .all(|cell| cell.children().any(|node| node.has_tag_name((word, "p"))))
        );
        let markdown = crate::markdown::export_docx_to_markdown(&docx).unwrap();
        assert!(markdown.contains("<!-- colspan:2 --> <!-- vmerge:start -->"));
        assert!(markdown.contains("<!-- colspan:2 --> <!-- vmerge:continue -->"));
        assert_merged_table(&crate::markdown::read_markdown(markdown.as_bytes()).unwrap());
        let from_markdown =
            crate::markdown::import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        assert_merged_table(&crate::open(from_markdown.as_slice()).unwrap().presentation);
    }

    #[test]
    fn rejects_invalid_and_overlapping_cell_spans() {
        let source = merged_example();
        for (old, replacement) in [
            ("<w:跨行>2</w:跨行>", "<w:跨行>0</w:跨行>"),
            ("<w:跨行>2</w:跨行>", "<w:跨行>999</w:跨行>"),
            (
                "<w:单元格><w:段落><w:句><w:文本串>左下",
                "<w:单元格><w:单元格属性><w:跨列 w:值=\"2\"/></w:单元格属性><w:段落><w:句><w:文本串>左下",
            ),
        ] {
            assert!(to_docx(source.replacen(old, replacement, 1).as_bytes()).is_err());
        }
    }

    #[test]
    fn rejects_another_uof_document_kind_and_invalid_xml() {
        let other = SAMPLE.replace("文字处理", "电子表格");
        assert!(!is_uot(other.as_bytes()));
        assert!(to_docx(other.as_bytes()).is_err());
        assert!(to_docx(b"<uot").is_err());
    }

    #[test]
    fn keeps_table_before_following_paragraph() {
        let table_start = SAMPLE.find("<w:文字表>").unwrap();
        let content = SAMPLE.split_once("<w:主体>").unwrap().0;
        let input = format!("{content}<w:主体>{}", &SAMPLE[table_start..]);
        let opened = open(input.as_bytes()).unwrap();
        let flow = opened.presentation.flow.as_deref().unwrap();
        assert!(
            matches!(flow.body.as_slice(), [Block::Table(_), Block::Paragraph(p)] if p.text() == "结尾。")
        );
    }
}
