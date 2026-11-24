// SPDX-License-Identifier: AGPL-3.0-only

//! Import the text-document branch of the Chinese Uniform Office Format.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ibis_doc_model::{
    Block, FlowCell, FlowParagraph, FlowRow, FlowRun, FlowRunKind, FlowStyle, FlowStyles,
    FlowTable, ParaFormat, RunFormat, StyleKind,
};
use roxmltree::{Document, Node, ParsingOptions};

use crate::{Error, Kind, Opened, SaveOptions};

const UOF: &str = "http://schemas.uof.org/cn/2003/uof";
const WORD: &str = "http://schemas.uof.org/cn/2003/uof-wordproc";
const MAX_XML_BYTES: usize = 16 * 1024 * 1024;

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
            nodes_limit: 300_000,
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
    crate::open(to_docx(data)?)
}

/// Generate a Word document with editable paragraphs, headings and basic tables.
pub fn to_docx(data: &[u8]) -> Result<Vec<u8>, Error> {
    let xml = parse(data)?;
    let root = xml.root_element();
    let content = body(root).ok_or_else(|| Error::new("UOT text document body not found"))?;
    let mut blank = crate::open(crate::blank_file(Kind::Document)?)?;
    let flow = Arc::make_mut(blank.presentation.flow.as_mut().expect("blank DOCX flow"));
    let anchor = match flow.body.first() {
        Some(Block::Paragraph(paragraph)) => paragraph.clone(),
        _ => return Err(Error::new("Missing blank DOCX paragraph")),
    };
    let mut importer = Importer {
        next_id: flow.new_id(),
        paragraph_styles: paragraph_styles(root),
        styles: Arc::make_mut(&mut flow.styles),
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

struct Importer<'a, 'xml, 'input> {
    next_id: u32,
    paragraph_styles: ParagraphStyles<'xml, 'input>,
    styles: &'a mut FlowStyles,
}

impl Importer<'_, '_, '_> {
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
        for text in node
            .children()
            .filter(|run| run.has_tag_name((WORD, "句")))
            .flat_map(|run| run.children())
            .filter(|text| text.has_tag_name((WORD, "文本串")))
        {
            if let Some(text) = text.text() {
                paragraph.runs.push(FlowRun::new(
                    FlowRunKind::Text(text.into()),
                    Default::default(),
                ));
            }
        }
        paragraph.resolve(self.styles);
        paragraph.normalize();
        paragraph
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
        let mut rows = Vec::new();
        let mut width = 0;
        for row in node.children().filter(|row| row.has_tag_name((WORD, "行"))) {
            let mut cells = Vec::new();
            for cell in row
                .children()
                .filter(|cell| cell.has_tag_name((WORD, "单元格")))
            {
                let mut blocks = self.blocks(cell, depth)?;
                if blocks.is_empty() {
                    blocks.push(Block::Paragraph(FlowParagraph {
                        id: self.id(),
                        ..Default::default()
                    }));
                }
                cells.push(FlowCell {
                    id: self.id(),
                    span: 1,
                    blocks,
                    ..Default::default()
                });
            }
            if cells.is_empty() || (width != 0 && width != cells.len()) {
                return Err(Error::new(
                    "UOT table rows must have the same number of cells",
                ));
            }
            width = cells.len();
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
