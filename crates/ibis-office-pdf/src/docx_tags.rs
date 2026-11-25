// SPDX-License-Identifier: AGPL-3.0-only
//! Use page-scoped marked content to reconstruct editable PDF text and tables.

use std::collections::{HashMap, HashSet};

use ibis_doc_model::{Block, FlowCell, FlowDocument, FlowRow, FlowTable, VerticalMerge};
use ibis_pdf::{CharBox, CosDict, CosObj, ObjectId, PageScene, PdfContainer};

use super::docx::paragraph;

// Keep the interpreter's order within each marked sequence. A wide gap or
// a line wrap can stand for a space even when no space glyph was painted.
fn marked_text(chars: &[CharBox]) -> String {
    let mut result = String::new();
    let mut previous: Option<&CharBox> = None;
    for ch in chars.iter().filter(|ch| !ch.ch.is_control()) {
        if let Some(before) = previous
            && before.ch.is_ascii_alphanumeric()
            && ch.ch.is_ascii_alphanumeric()
            && !before.vertical
            && !ch.vertical
            && before.stream_op_index != ch.stream_op_index
        {
            let height = before.rect_pt.height().min(ch.rect_pt.height());
            if (before.rect_pt.y1 - ch.rect_pt.y1).abs() > 0.45 * height
                || ch.rect_pt.x0 - before.rect_pt.x1 > 0.12 * height
            {
                result.push(' ');
            }
        } else if let Some(before) = previous
            && before.ch.is_ascii_punctuation()
            && ch.ch.is_ascii_alphanumeric()
            && (before.rect_pt.y1 - ch.rect_pt.y1).abs() > 0.45 * before.rect_pt.height()
        {
            result.push(' ');
        }
        result.push(ch.ch);
        previous = Some(ch);
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Default)]
struct Node {
    role: Vec<u8>,
    children: Vec<Node>,
    mark: Option<(ObjectId, i32)>,
    rows: usize,
    cols: usize,
}

fn resolve(pdf: &mut PdfContainer, object: &CosObj) -> Option<CosObj> {
    match object {
        CosObj::Ref(id) => pdf.resolve(*id).ok(),
        other => Some(other.clone()),
    }
}

pub(super) struct Tree(Node);

impl Tree {
    pub(super) fn read(pdf: &mut PdfContainer) -> Option<Self> {
        let (_, catalog) = pdf.catalog().ok()?;
        let root = resolve(pdf, catalog.get(b"StructTreeRoot")?)?;
        let root = root.as_dict()?;
        let roles = root.get(b"RoleMap").and_then(|map| resolve(pdf, map));
        let mut reader = Reader {
            pdf,
            roles: roles.as_ref().and_then(CosObj::as_dict),
            seen: HashSet::new(),
            remaining: 100_000,
        };
        Some(Self(reader.node(root.get(b"K")?, None, 0)?))
    }

    pub(super) fn blocks(
        self,
        pages: &[(PageScene, Vec<Option<i32>>)],
    ) -> Option<Vec<Vec<TaggedBlock>>> {
        let indices: HashMap<_, _> = pages
            .iter()
            .enumerate()
            .map(|(i, (scene, _))| (scene.info.object_id, i))
            .collect();
        let mut marks: HashMap<_, Vec<CharBox>> = HashMap::new();
        for (scene, ids) in pages {
            for (ch, mark) in scene.text_index.chars.iter().zip(ids) {
                if let Some(mark) = mark {
                    marks
                        .entry((scene.info.object_id, *mark))
                        .or_default()
                        .push(ch.clone());
                } else if !ch.ch.is_whitespace() {
                    return None;
                }
            }
        }
        let mut ordered = Vec::new();
        self.0.blocks(&mut marks, &mut ordered)?;
        if ordered.is_empty() || marks.values().flatten().any(|ch| !ch.ch.is_whitespace()) {
            return None;
        }
        let mut output: Vec<Vec<_>> = (0..pages.len()).map(|_| Vec::new()).collect();
        let mut previous = 0;
        for (page, block) in ordered {
            let index = *indices.get(&page)?;
            if index < previous {
                return None;
            }
            output[index].push(block);
            previous = index;
        }
        Some(output)
    }
}

struct Reader<'a> {
    pdf: &'a mut PdfContainer,
    roles: Option<&'a CosDict>,
    seen: HashSet<ObjectId>,
    remaining: usize,
}

impl Reader<'_> {
    fn node(&mut self, value: &CosObj, page: Option<ObjectId>, depth: usize) -> Option<Node> {
        if depth > 64 || self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        if let CosObj::Ref(id) = value
            && !self.seen.insert(*id)
        {
            return None;
        }
        let value = resolve(self.pdf, value)?;
        let mut node = Node::default();
        match value {
            CosObj::Array(values) => {
                for value in values {
                    node.children.push(self.node(&value, page, depth + 1)?);
                }
            }
            CosObj::Int(mcid) => node.mark = Some((page?, mcid.try_into().ok()?)),
            CosObj::Dict(dict) => {
                let page = dict.get_ref(b"Pg").or(page);
                if dict.get_name(b"Type") == Some(b"MCR") {
                    // Form streams use their own MCID namespace.
                    if dict.get(b"Stm").is_some() {
                        return None;
                    }
                    node.mark = Some((page?, dict.get_int(b"MCID")?.try_into().ok()?));
                    return Some(node);
                }
                if dict.get_name(b"Type") == Some(b"OBJR") {
                    return Some(node);
                }
                let role = dict.get_name(b"S")?;
                node.role = self
                    .roles
                    .and_then(|map| map.get_name(role))
                    .unwrap_or(role)
                    .to_vec();
                let mut attrs = Vec::new();
                if let Some(value) = dict.get(b"A").and_then(|a| resolve(self.pdf, a)) {
                    match value {
                        CosObj::Array(values) => attrs.extend(values),
                        CosObj::Dict(_) => attrs.push(value),
                        _ => {}
                    }
                }
                node.rows = 1;
                node.cols = 1;
                for attr in attrs {
                    let resolved = resolve(self.pdf, &attr)?;
                    if let Some(attr) = resolved.as_dict() {
                        let span = |name| {
                            usize::try_from(attr.get_int(name).unwrap_or(1))
                                .ok()
                                .filter(|n| (1..=128).contains(n))
                        };
                        if attr.get(b"RowSpan").is_some() {
                            node.rows = span(b"RowSpan")?;
                        }
                        if attr.get(b"ColSpan").is_some() {
                            node.cols = span(b"ColSpan")?;
                        }
                    }
                }
                if let Some(kids) = dict.get(b"K") {
                    node.children.push(self.node(kids, page, depth + 1)?);
                }
            }
            _ => return None,
        }
        Some(node)
    }
}

type Marks = HashMap<(ObjectId, i32), Vec<CharBox>>;

impl Node {
    fn children(&self) -> impl Iterator<Item = &Node> {
        self.children.iter().flat_map(|child| {
            if child.role.is_empty() && child.mark.is_none() {
                child.children.as_slice()
            } else {
                std::slice::from_ref(child)
            }
        })
    }

    fn collect_text(
        &self,
        marks: &mut Marks,
        page: &mut Option<ObjectId>,
        chars: &mut Vec<CharBox>,
    ) -> Option<()> {
        if let Some(key) = self.mark {
            if page.is_some_and(|current| current != key.0) {
                return None;
            }
            *page = Some(key.0);
            if let Some(found) = marks.remove(&key) {
                chars.extend(found);
            }
        }
        for child in &self.children {
            child.collect_text(marks, page, chars)?;
        }
        Some(())
    }

    fn blocks(&self, marks: &mut Marks, output: &mut Vec<(ObjectId, TaggedBlock)>) -> Option<()> {
        match self.role.as_slice() {
            b"P" | b"H1" | b"H2" | b"H3" | b"H4" | b"H5" | b"H6" => {
                let mut page = None;
                let mut chars = Vec::new();
                self.collect_text(marks, &mut page, &mut chars)?;
                if let Some(page) = page {
                    let heading = (self.role.len() == 2).then(|| self.role[1] - b'0');
                    output.push((page, TaggedBlock::Text(marked_text(&chars), heading)));
                }
            }
            b"Table" => {
                let mut page = None;
                let mut rows = Vec::new();
                for child in self.children() {
                    match child.role.as_slice() {
                        b"TR" => rows.push(child),
                        b"THead" | b"TBody" | b"TFoot" => rows.extend(child.children()),
                        _ => return None,
                    }
                }
                let mut cells = Vec::new();
                let mut occupied = HashSet::new();
                if rows.len() > 512 {
                    return None;
                }
                for (r, row) in rows.iter().enumerate() {
                    if row.role != b"TR" {
                        return None;
                    }
                    let mut c = 0;
                    for cell in row.children() {
                        if !matches!(cell.role.as_slice(), b"TH" | b"TD") {
                            return None;
                        }
                        while occupied.contains(&(r, c)) {
                            c += 1;
                        }
                        if c + cell.cols > 128 || r + cell.rows > rows.len() {
                            return None;
                        }
                        for y in r..r + cell.rows {
                            for x in c..c + cell.cols {
                                if !occupied.insert((y, x)) {
                                    return None;
                                }
                            }
                        }
                        let mut chars = Vec::new();
                        cell.collect_text(marks, &mut page, &mut chars)?;
                        cells.push(Cell {
                            row: r,
                            col: c,
                            height: cell.rows,
                            width: cell.cols,
                            text: marked_text(&chars),
                        });
                        c += cell.cols;
                    }
                }
                let cols = occupied.iter().map(|(_, c)| c + 1).max()?;
                if occupied.len() != rows.len() * cols {
                    return None;
                }
                output.push((
                    page?,
                    TaggedBlock::Table {
                        rows: rows.len(),
                        cols,
                        cells,
                    },
                ));
            }
            _ => {
                for child in &self.children {
                    child.blocks(marks, output)?;
                }
            }
        }
        Some(())
    }
}

pub(super) struct Cell {
    row: usize,
    col: usize,
    height: usize,
    width: usize,
    text: String,
}

pub(super) enum TaggedBlock {
    Text(String, Option<u8>),
    Table {
        rows: usize,
        cols: usize,
        cells: Vec<Cell>,
    },
}

impl TaggedBlock {
    pub(super) fn to_flow(&self, flow: &FlowDocument, id: &impl Fn() -> u32) -> Block {
        match self {
            Self::Text(text, heading) => {
                let mut p = paragraph(flow, id(), text, 0, 0);
                if let Some(level) = heading {
                    p.style = Some(format!("Heading{level}"));
                    p.resolve(&flow.styles);
                }
                Block::Paragraph(p)
            }
            Self::Table { rows, cols, cells } => {
                let width = (flow.page.size.width - flow.page.left - flow.page.right).max(1);
                let mut row_cells: Vec<Vec<_>> = (0..*rows).map(|_| Vec::new()).collect();
                for cell in cells {
                    for offset in 0..cell.height {
                        let merge = match (offset, cell.height) {
                            (0, 1) => VerticalMerge::None,
                            (0, _) => VerticalMerge::Start,
                            _ => VerticalMerge::Continue,
                        };
                        let content = if offset == 0 {
                            cell.text.trim_end()
                        } else {
                            ""
                        };
                        row_cells[cell.row + offset].push((
                            cell.col,
                            FlowCell {
                                id: id(),
                                span: cell.width as u32,
                                merge,
                                blocks: vec![Block::Paragraph(paragraph(
                                    flow,
                                    id(),
                                    content,
                                    0,
                                    0,
                                ))],
                                ..Default::default()
                            },
                        ));
                    }
                }
                Block::Table(FlowTable {
                    id: id(),
                    columns: vec![width / *cols as i64; *cols],
                    rows: row_cells
                        .into_iter()
                        .map(|mut cells| {
                            cells.sort_by_key(|(col, _)| *col);
                            FlowRow {
                                id: id(),
                                cells: cells.into_iter().map(|(_, cell)| cell).collect(),
                                ..Default::default()
                            }
                        })
                        .collect(),
                    ..Default::default()
                })
            }
        }
    }
}
