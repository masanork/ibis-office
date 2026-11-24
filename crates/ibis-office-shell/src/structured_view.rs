// SPDX-License-Identifier: AGPL-3.0-only

//! Read-only navigation of structured records and document outlines.

use std::{fmt, io::Read, ops::Range, path::PathBuf, sync::Arc};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Root, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;

mod csv;
mod cyclonedx;
mod hed;
mod linkage;
mod row_schema;

const MAX_BYTES: u64 = 128 * 1024 * 1024;
const MAX_RECORDS: usize = 1_000_000;

const PAGE_SIZE: usize = 256;

struct Fields {
    offset: usize,
    limit: usize,
}

impl Fields {
    fn page(offset: usize) -> Self {
        Self {
            offset,
            limit: PAGE_SIZE + 1,
        }
    }
}

fn parse_fields(bytes: &[u8], fields: Fields) -> Vec<(String, &RawValue)> {
    let mut parser = serde_json::Deserializer::from_slice(bytes);
    if bytes.first() == Some(&b'{') {
        parser.deserialize_map(fields)
    } else {
        parser.deserialize_seq(fields)
    }
    .unwrap_or_default()
}
impl<'de> Visitor<'de> for Fields {
    type Value = Vec<(String, &'de RawValue)>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut fields = Vec::new();
        let mut index = 0;
        while let Some(key) = map.next_key()? {
            let raw = map.next_value()?;
            if index >= self.offset && fields.len() < self.limit {
                fields.push((key, raw));
            }
            index += 1;
        }
        Ok(fields)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut fields = Vec::new();
        let mut index = 0;
        while let Some(raw) = sequence.next_element()? {
            if index >= self.offset && fields.len() < self.limit {
                fields.push((format!("[{index}]"), raw));
            }
            index += 1;
        }
        Ok(fields)
    }
}

const SEARCH_LIMIT: usize = 1000;

#[derive(Clone)]
struct SearchHit {
    span: Range<usize>,
    label: String,
    detail: String,
    path: String,
}

struct Source {
    bytes: Vec<u8>,
    records: Vec<Range<usize>>,
    lines: bool,
    truncated: bool,
    inventory: Option<cyclonedx::Inventory>,
    csv: Option<csv::Csv>,
    notice: Option<hed::Check>,
    linkage_review: Option<Result<linkage::Review, &'static str>>,
    row_review: Option<Result<row_schema::Review, &'static str>>,
}

impl Source {
    fn read(path: PathBuf) -> Result<Self, String> {
        let lines = path.extension().is_some_and(|ext| {
            ext.eq_ignore_ascii_case("jsonl") || ext.eq_ignore_ascii_case("ndjson")
        });
        let is_csv = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"));
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(crate::i18n::text("structured.size-limit").to_string());
        }
        if is_csv {
            let mut csv = csv::Csv::read(&bytes)?;
            let linkage_review = linkage::Definition::sidecar(&path).map(|definition| {
                let definition = definition?;
                let review = csv.review(&bytes, &definition)?;
                csv.definition = Some(definition);
                Ok(review)
            });
            let notice = hed::Check::read(&path, bytes.len(), csv.data_count);
            return Ok(Self {
                bytes,
                records: std::mem::take(&mut csv.records),
                lines: true,
                truncated: csv.truncated,
                inventory: None,
                csv: Some(csv),
                notice: Some(notice),
                linkage_review,
                row_review: None,
            });
        }
        let mut source = Self::new(bytes, lines);
        if let Some(schema) = row_schema::sidecar(&path) {
            source.row_review = Some(row_schema::review(&schema, &source));
        } else if !lines {
            source.inventory = cyclonedx::Inventory::read(&source);
        }
        Ok(source)
    }

    fn new(bytes: Vec<u8>, lines: bool) -> Self {
        let start = usize::from(bytes.starts_with(b"\xef\xbb\xbf")) * 3;
        let mut offset = start;
        let records = if lines {
            bytes[start..]
                .split_inclusive(|&byte| byte == b'\n')
                .take(MAX_RECORDS)
                .map(|line| {
                    let begin = offset;
                    offset += line.len();
                    begin..offset
                })
                .collect()
        } else {
            std::iter::once(start..bytes.len()).collect()
        };
        Self {
            truncated: lines && offset < bytes.len(),
            bytes,
            records,
            inventory: None,
            csv: None,
            notice: None,
            linkage_review: None,
            row_review: None,
            lines,
        }
    }

    fn span(&self, raw: &RawValue) -> Range<usize> {
        let text = raw.get();
        let start = text.as_ptr() as usize - self.bytes.as_ptr() as usize;
        start..start + text.len()
    }

    fn node(&self, raw: &RawValue, mut name: String) -> Node {
        let text = raw.get();
        if !self.lines && name.starts_with('[') && text.starts_with('{') {
            let mut parser = serde_json::Deserializer::from_str(text);
            if let Ok(fields) = parser.deserialize_map(Fields::page(0)) {
                let label = ["name", "bom-ref", "ref"].iter().find_map(|key| {
                    fields
                        .iter()
                        .find(|(name, _)| name == key)
                        .and_then(|(_, value)| serde_json::from_str::<String>(value.get()).ok())
                });
                if let Some(label) = label.filter(|label| !label.is_empty()) {
                    name.push(' ');
                    name.push_str(&excerpt(&label, 120));
                }
            }
        }
        let (kind, children) = match text.as_bytes().first() {
            Some(b'{') => ("Object", None),
            Some(b'[') => ("Array", None),
            _ => ("", Some(Vec::new())),
        };
        Node {
            name,
            detail: if kind.is_empty() {
                excerpt(text, 400)
            } else {
                kind.into()
            },
            span: self.span(raw),
            children,
            expanded: false,
            paragraph: None,
            parent: None,
            more: false,
        }
    }

    fn record(&self, index: usize) -> Node {
        let span = self.records[index].clone();
        match serde_json::from_slice::<&RawValue>(&self.bytes[span.clone()]) {
            Ok(raw) => self.node(raw, "$".into()),
            Err(error) => Node {
                name: crate::i18n::text("structured.invalid").to_string(),
                detail: error.to_string(),
                span,
                children: Some(Vec::new()),
                expanded: false,
                paragraph: None,
                parent: None,
                more: false,
            },
        }
    }

    fn field_search(&self, record: usize, query: &str) -> Vec<SearchHit> {
        let Some(span) = self.records.get(record) else {
            return Vec::new();
        };
        let Ok(root) = serde_json::from_slice::<&RawValue>(&self.bytes[span.clone()]) else {
            return Vec::new();
        };
        let mut hits = Vec::new();
        let mut pending = vec![(root, "$".to_owned(), None::<String>)];
        while let Some((raw, path, label)) = pending.pop() {
            let text = raw.get();
            let container = text.starts_with(['{', '[']);
            let decoded = text
                .starts_with('"')
                .then(|| serde_json::from_str::<String>(text).ok())
                .flatten();
            let scalar = decoded.as_deref().unwrap_or(text);
            if let Some(label) = label
                && (label.to_lowercase().contains(query)
                    || (!container && scalar.to_lowercase().contains(query)))
            {
                hits.push(SearchHit {
                    span: self.span(raw),
                    label: excerpt(&label, 120),
                    detail: if container {
                        String::new()
                    } else {
                        excerpt(scalar, 160)
                    },
                    path: path.clone(),
                });
                if hits.len() > SEARCH_LIMIT {
                    break;
                }
            }
            if container {
                let children = parse_fields(
                    text.as_bytes(),
                    Fields {
                        offset: 0,
                        limit: usize::MAX,
                    },
                );
                pending.extend(children.into_iter().rev().map(|(label, child)| {
                    let label = if label.is_empty() {
                        "\"\"".to_owned()
                    } else {
                        label
                    };
                    let path = format!("{path} / {}", excerpt(&label, 120));
                    (child, path, Some(label))
                }));
            }
        }
        hits
    }

    fn children(&self, span: Range<usize>, offset: usize) -> (Vec<Node>, bool) {
        let page = parse_fields(&self.bytes[span], Fields::page(offset));
        let more = page.len() > PAGE_SIZE;
        (
            page.into_iter()
                .take(PAGE_SIZE)
                .map(|(name, raw)| self.node(raw, name))
                .collect(),
            more,
        )
    }
}

fn excerpt(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        text.to_owned()
    } else {
        format!("{}…", &text[..text.floor_char_boundary(limit)])
    }
}

struct Node {
    name: String,
    detail: String,
    span: Range<usize>,
    children: Option<Vec<usize>>,
    expanded: bool,
    paragraph: Option<u32>,
    parent: Option<usize>,
    more: bool,
}

#[derive(Default)]
struct Tree {
    nodes: Vec<Node>,
    roots: Vec<usize>,
}

impl Tree {
    fn push(&mut self, node: Node) -> usize {
        let index = self.nodes.len();
        self.nodes.push(node);
        index
    }

    fn load_children(&mut self, id: usize, source: &Source) {
        let offset = self.nodes[id].children.as_ref().map_or(0, Vec::len);
        let (nodes, more) = source.children(self.nodes[id].span.clone(), offset);
        let children: Vec<_> = nodes
            .into_iter()
            .map(|mut node| {
                node.parent = Some(id);
                self.push(node)
            })
            .collect();
        let node = &mut self.nodes[id];
        node.more = more;
        node.children.get_or_insert_with(Vec::new).extend(children);
    }

    fn toggle(&mut self, id: usize, source: Option<&Source>) {
        if self.nodes[id].children.is_none()
            && let Some(source) = source
        {
            self.load_children(id, source);
        }
        self.nodes[id].expanded = !self.nodes[id].expanded;
    }

    fn select_schema_path(
        &mut self,
        mut parent: usize,
        path: &[row_schema::Segment],
        source: &Source,
    ) -> usize {
        for segment in path {
            self.nodes[parent].expanded = true;
            if self.nodes[parent].children.is_none() {
                self.load_children(parent, source);
            }
            let bytes = &source.bytes[self.nodes[parent].span.clone()];
            let (name, raw) = match segment {
                row_schema::Segment::Key(key) => {
                    let raw =
                        serde_json::from_slice::<std::collections::HashMap<String, &RawValue>>(
                            bytes,
                        )
                        .ok()
                        .and_then(|fields| fields.get(key).copied());
                    (key.clone(), raw)
                }
                row_schema::Segment::Index(index) => {
                    let raw = serde_json::from_slice::<Vec<&RawValue>>(bytes)
                        .ok()
                        .and_then(|fields| fields.get(*index).copied());
                    (format!("[{index}]"), raw)
                }
            };
            let found = self.nodes[parent]
                .children
                .as_ref()
                .into_iter()
                .flatten()
                .copied()
                .find(|&id| raw.is_some_and(|raw| self.nodes[id].span == source.span(raw)));
            parent = found.unwrap_or_else(|| {
                let mut node = if let Some(raw) = raw {
                    source.node(raw, name)
                } else {
                    Node {
                        name,
                        detail: crate::i18n::text("structured.row-schema-required").to_string(),
                        span: 0..0,
                        children: Some(Vec::new()),
                        expanded: false,
                        paragraph: None,
                        parent: None,
                        more: false,
                    }
                };
                node.parent = Some(parent);
                let id = self.push(node);
                if !self.nodes[parent].more {
                    self.nodes[parent]
                        .children
                        .get_or_insert_with(Vec::new)
                        .push(id);
                }
                id
            });
        }
        parent
    }

    fn breadcrumb(&self, mut id: usize) -> String {
        let mut labels = Vec::new();
        loop {
            let node = &self.nodes[id];
            labels.push(node.name.as_str());
            let Some(parent) = node.parent else { break };
            id = parent;
        }
        labels.reverse();
        labels.join(" / ")
    }

    fn visible(&self, search: &str) -> Vec<(usize, usize)> {
        let mut pending: Vec<_> = self.roots.iter().rev().map(|&id| (id, 0)).collect();
        let mut rows = Vec::new();
        while let Some((id, depth)) = pending.pop() {
            let node = &self.nodes[id];
            if search.is_empty()
                || node.name.to_lowercase().contains(search)
                || node.detail.to_lowercase().contains(search)
            {
                rows.push((id, depth));
            }
            if (node.expanded || !search.is_empty())
                && let Some(children) = &node.children
            {
                pending.extend(children.iter().rev().map(|&child| (child, depth + 1)));
            }
        }
        rows
    }

    fn outline(flow: &ibis_doc_model::FlowDocument) -> Self {
        use ibis_doc_model::flow_command::{all_paragraphs, heading_level};
        let mut tree = Self::default();
        let mut parents: Vec<(u32, usize)> = Vec::new();
        for paragraph in all_paragraphs(flow) {
            let Some(level) = heading_level(&flow.styles, paragraph) else {
                continue;
            };
            while parents.last().is_some_and(|parent| parent.0 >= level) {
                parents.pop();
            }
            let id = tree.push(Node {
                name: paragraph.text(),
                detail: format!("H{level}"),
                span: 0..0,
                children: Some(Vec::new()),
                expanded: true,
                paragraph: Some(paragraph.id),
                parent: parents.last().map(|&(_, parent)| parent),
                more: false,
            });
            if let Some(&(_, parent)) = parents.last() {
                tree.nodes[parent].children.as_mut().unwrap().push(id);
            } else {
                tree.roots.push(id);
            }
            parents.push((level, id));
        }
        tree
    }
}

pub(crate) type EditorTarget = (WeakEntity<crate::editor::SlideEditor>, AnyWindowHandle, u64);

pub(crate) struct InventoryTarget {
    editor: EditorTarget,
    source: crate::read_file::BomSource,
}

enum Origin {
    Outline(EditorTarget),
    Inventory(InventoryTarget),
}

struct View {
    title: SharedString,
    source: Option<Arc<Source>>,
    tree: Tree,
    editor: Option<EditorTarget>,
    table: Option<InventoryTarget>,
    search: Entity<InputState>,
    matching: Vec<usize>,
    query: String,
    hits: Option<Vec<SearchHit>>,
    record: usize,
    record_scroll: UniformListScrollHandle,
    tree_scroll: UniformListScrollHandle,
    selected: Option<usize>,
    document_tree: Option<(Tree, Option<usize>)>,
    _subscription: Subscription,
}

impl View {
    fn navigate_heading(&self, id: usize, toggle_body: bool, cx: &mut Context<Self>) {
        let Some(paragraph) = self.tree.nodes[id].paragraph else {
            return;
        };
        let Some((editor, owner, file_id)) = &self.editor else {
            return;
        };
        let moved = editor
            .update(cx, |editor, cx| {
                if editor.file_id != *file_id {
                    return false;
                }
                crate::with_format!(
                    editor,
                    navigate_outline_heading(editor, paragraph, toggle_body, cx)
                );
                true
            })
            .unwrap_or(false);
        if moved {
            crate::open_guard::raise(*owner, cx);
        }
    }

    fn follow_reference(
        &mut self,
        span: Range<usize>,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = &self.source else { return };
        let Ok(raw) = serde_json::from_slice::<&RawValue>(&source.bytes[span]) else {
            return;
        };
        let mut target = Tree::default();
        let root = target.push(source.node(raw, label));
        target.roots.push(root);
        target.toggle(root, Some(source));
        let previous = std::mem::replace(&mut self.tree, target);
        if self.document_tree.is_none() {
            self.document_tree = Some((previous, self.selected));
        }
        self.selected = Some(root);
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn select_record(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(source) = &self.source else { return };
        self.record = index;
        self.document_tree = None;
        self.selected = None;
        if let Some(csv) = &source.csv {
            self.tree = csv.tree(&source.bytes, index, source.records[index].clone());
            cx.notify();
            return;
        }
        self.tree = Tree::default();
        let root = self.tree.push(source.record(index));
        self.tree.roots.push(root);
        self.tree.toggle(root, Some(source));
        self.refresh_hits(cx);
        cx.notify();
    }

    fn select_row_issue(
        &mut self,
        record: usize,
        path: &[row_schema::Segment],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.select_record(record, cx);
        let Some(&root) = self.tree.roots.first() else {
            return;
        };
        if let Ok(position) = self.matching.binary_search(&record) {
            self.record_scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
        let source = self.source.as_ref().unwrap();
        self.selected = Some(self.tree.select_schema_path(root, path, source));
        if let Some(position) = self
            .tree
            .visible("")
            .iter()
            .position(|(id, _)| Some(*id) == self.selected)
        {
            self.tree_scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    fn select_csv_issue(
        &mut self,
        record: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.select_record(record, cx);
        let prefix = format!("[{}] ", column + 1);
        if let Some(id) = self
            .tree
            .nodes
            .iter()
            .position(|node| node.name.starts_with(&prefix))
        {
            let mut parent = self.tree.nodes[id].parent;
            while let Some(id) = parent {
                self.tree.nodes[id].expanded = true;
                parent = self.tree.nodes[id].parent;
            }
            self.selected = Some(id);
        }
        cx.notify();
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        self.query = self.search.read(cx).value().trim().to_lowercase();
        self.refresh_hits(cx);
        if let Some(source) = self
            .source
            .clone()
            .filter(|source| source.lines && source.csv.is_none())
        {
            let query = self.query.clone();
            cx.spawn(async move |this, cx| {
                let search = query.clone();
                let matching = cx
                    .background_executor()
                    .spawn(async move {
                        source
                            .records
                            .iter()
                            .enumerate()
                            .filter_map(|(index, span)| {
                                (search.is_empty()
                                    || String::from_utf8_lossy(&source.bytes[span.clone()])
                                        .to_lowercase()
                                        .contains(&search))
                                .then_some(index)
                            })
                            .collect()
                    })
                    .await;
                let _ = this.update(cx, |view, cx| {
                    if view.query == query {
                        view.matching = matching;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }
    fn refresh_hits(&mut self, cx: &mut Context<Self>) {
        self.hits = None;
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.query.is_empty()
            || (source.inventory.is_none() && (!source.lines || source.csv.is_some()))
        {
            return;
        }
        let query = self.query.clone();
        let record = self.record;
        cx.spawn(async move |this, cx| {
            let search = query.clone();
            let hits = cx
                .background_executor()
                .spawn(async move {
                    if let Some(inventory) = &source.inventory {
                        inventory.search(&search)
                    } else {
                        source.field_search(record, &search)
                    }
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.query == query && view.record == record {
                    view.hits = Some(hits);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn back_to_tree(&mut self, cx: &mut Context<Self>) {
        if let Some((tree, selected)) = self.document_tree.take() {
            self.tree = tree;
            self.selected = selected;
            cx.notify();
        }
    }
}

impl Render for View {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let lines = self.source.as_ref().is_some_and(|source| source.lines);
        let is_csv = self
            .source
            .as_ref()
            .is_some_and(|source| source.csv.is_some());
        let inventory = self
            .source
            .as_ref()
            .and_then(|source| source.inventory.as_ref());
        let visible = self
            .tree
            .visible(if lines && !is_csv { "" } else { &self.query });
        let visible_is_empty = visible.is_empty();
        let show_hits = (inventory.is_some() || (lines && !is_csv)) && !self.query.is_empty();
        let hit_count = self.hits.as_ref().map_or(0, Vec::len);
        let hit_rows = uniform_list(
            "structured-search-hits",
            hit_count.min(SEARCH_LIMIT),
            cx.processor(|view, range: Range<usize>, _, cx| {
                range
                    .filter_map(|position| {
                        let hit = view.hits.as_ref()?.get(position)?.clone();
                        let label = hit.label.clone();
                        let target_label = format!("{} · {}", hit.label, hit.path);
                        Some(
                            v_flex()
                                .id(("structured-hit", position))
                                .h(px(48.))
                                .px_4()
                                .justify_center()
                                .border_b_1()
                                .border_color(cx.theme().border)
                                .cursor_pointer()
                                .hover(|row| row.bg(cx.theme().accent))
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.follow_reference(
                                        hit.span.clone(),
                                        target_label.clone(),
                                        window,
                                        cx,
                                    );
                                }))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(div().truncate().child(label))
                                        .child(div().text_xs().truncate().child(hit.detail)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .truncate()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(hit.path),
                                ),
                        )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .size_full();
        let record_rows = uniform_list(
            "structured-records",
            self.matching.len(),
            cx.processor(|view, range: Range<usize>, _, cx| {
                range
                    .map(|position| {
                        let index = view.matching[position];
                        let source = view.source.as_ref().unwrap();
                        let bytes = &source.bytes[source.records[index].clone()];
                        let sample = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
                        div()
                            .id(("record", index))
                            .h(px(34.))
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_3()
                            .cursor_pointer()
                            .when(view.record == index, |row| row.bg(cx.theme().accent))
                            .hover(|row| row.bg(cx.theme().accent))
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.select_record(index, cx)),
                            )
                            .child(
                                div()
                                    .w(px(55.))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child((index + 1).to_string()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .child(excerpt(sample.trim(), 150)),
                            )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.record_scroll)
        .size_full();
        let rows = uniform_list(
            "structured-tree",
            visible.len(),
            cx.processor(move |view, range: Range<usize>, _, cx| {
                range
                    .map(|position| {
                        let (id, depth) = visible[position];
                        let node = &view.tree.nodes[id];
                        let has_children = node
                            .children
                            .as_ref()
                            .is_none_or(|children| !children.is_empty());
                        let arrow = if node.expanded { "▾" } else { "▸" };
                        h_flex()
                            .id(("node", id))
                            .h(px(32.))
                            .pl(px(12. + depth.min(32) as f32 * 16.))
                            .pr_3()
                            .gap_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .when(view.selected == Some(id), |row| row.bg(cx.theme().accent))
                            .child(div().w(px(25.)).flex_shrink_0().children(has_children.then(
                                || {
                                    Button::new(("toggle", id))
                                        .ghost()
                                        .xsmall()
                                        .label(arrow)
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.tree.toggle(id, view.source.as_deref());
                                            cx.notify();
                                        }))
                                },
                            )))
                            .child(
                                div()
                                    .id(("select", id))
                                    .flex_1()
                                    .min_w_0()
                                    .cursor_pointer()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.selected = Some(id);
                                        view.navigate_heading(id, false, cx);
                                        cx.notify();
                                    }))
                                    .child(div().truncate().text_sm().child(node.name.clone())),
                            )
                            .children(node.paragraph.and_then(|paragraph| {
                                let (editor, _, file_id) = view.editor.as_ref()?;
                                let editor = editor.upgrade()?;
                                let editor = editor.read(cx);
                                if editor.file_id != *file_id {
                                    return None;
                                }
                                let folded = crate::with_format!(
                                    editor,
                                    outline_heading_folded(editor, paragraph)
                                )?;
                                let key = if folded {
                                    "structured.expand-body"
                                } else {
                                    "structured.collapse-body"
                                };
                                Some(
                                    Button::new(("structured-fold-body", id))
                                        .outline()
                                        .xsmall()
                                        .label(crate::i18n::text(key))
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.selected = Some(id);
                                            view.navigate_heading(id, true, cx);
                                            cx.notify();
                                        })),
                                )
                            }))
                            .children((node.expanded && node.more).then(|| {
                                Button::new(("more", id))
                                    .ghost()
                                    .xsmall()
                                    .label(crate::i18n::text("structured.more"))
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if let Some(source) = &view.source {
                                            view.tree.load_children(id, source);
                                        }
                                        cx.notify();
                                    }))
                            }))
                            .child(
                                div()
                                    .w(relative(0.5))
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(node.detail.clone()),
                            )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.tree_scroll)
        .size_full();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .h(px(58.))
                    .px_4()
                    .gap_4()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_lg()
                            .child(self.title.clone()),
                    )
                    .children(self.table.as_ref().map(|_| {
                        Button::new("structured-bom-table")
                            .outline()
                            .xsmall()
                            .label(crate::i18n::text("structured.show-inventory"))
                            .on_click(cx.listener(|view, _, _, cx| {
                                if let Some(table) = &view.table {
                                    let (editor, owner, file_id) = &table.editor;
                                    if editor
                                        .upgrade()
                                        .is_some_and(|editor| editor.read(cx).file_id == *file_id)
                                    {
                                        crate::open_guard::raise(*owner, cx);
                                    } else {
                                        crate::app::open_editor_for(
                                            table.source.path.clone(),
                                            None,
                                            cx,
                                        );
                                    }
                                }
                            }))
                    }))
                    .children((lines && !is_csv && self.document_tree.is_some()).then(|| {
                        Button::new("structured-record-back")
                            .outline()
                            .xsmall()
                            .label(crate::i18n::text("structured.back-to-record"))
                            .on_click(cx.listener(|view, _, _, cx| view.back_to_tree(cx)))
                    }))
                    .child(div().w(px(270.)).child(Input::new(&self.search).small())),
            )
            .children(is_csv.then(|| {
                div()
                    .px_4()
                    .py_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(crate::i18n::text("structured.csv-mode"))
            }))
            .children(self.source.as_ref().and_then(|source| {
                source.row_review.as_ref().map(|result| {
                    let panel = v_flex().px_4().py_2().gap_1().text_sm();
                    let review = match result {
                        Ok(review) => review,
                        Err(message) => return panel.child(crate::i18n::text(message)),
                    };
                    let mut results = v_flex()
                        .id("row-schema-results")
                        .max_h(px(140.))
                        .overflow_y_scroll();
                    for (index, issue) in review.issues.iter().enumerate() {
                        let record = issue.record;
                        let path = issue.path.clone();
                        let field = if path.is_empty() {
                            "$".into()
                        } else {
                            row_schema::pointer(&path)
                        };
                        let label = crate::i18n::format(
                            if source.lines {
                                "structured.row-schema-issue"
                            } else {
                                "structured.document-schema-issue"
                            },
                            &[
                                ("line", &(record + 1)),
                                ("field", &field),
                                ("message", &crate::i18n::text(issue.message)),
                            ],
                        );
                        results = results.child(
                            Button::new(("row-schema-issue", index))
                                .outline()
                                .xsmall()
                                .label(label)
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.select_row_issue(record, &path, window, cx)
                                })),
                        );
                    }
                    for comment in review.comments.iter().take(linkage::DISPLAY_LIMIT) {
                        results = results.child(
                            div()
                                .truncate()
                                .child(excerpt(comment.lines().next().unwrap_or_default(), 220)),
                        );
                    }
                    panel
                        .child(div().truncate().child(review.title.clone()))
                        .child(crate::i18n::format(
                            if source.lines {
                                "structured.row-schema-summary"
                            } else {
                                "structured.document-schema-summary"
                            },
                            &[
                                ("rows", &source.records.len()),
                                ("issues", &review.issue_count),
                            ],
                        ))
                        .child(results)
                })
            }))
            .children(self.source.as_ref().and_then(|source| {
                source.linkage_review.as_ref().map(|review| {
                    let mut panel = v_flex().px_4().py_2().gap_1().text_sm();
                    let review = match review {
                        Ok(review) => review,
                        Err(message) => return panel.child(crate::i18n::text(message)),
                    };
                    panel = panel
                        .child(div().truncate().child(review.label.clone()))
                        .child(crate::i18n::format(
                            "structured.linkage-summary",
                            &[
                                ("checked", &review.checked.to_string()),
                                (
                                    "total",
                                    &source
                                        .csv
                                        .as_ref()
                                        .map_or(0, |csv| csv.data_count)
                                        .to_string(),
                                ),
                                ("issues", &review.issue_count.to_string()),
                            ],
                        ))
                        .child(crate::i18n::text("structured.linkage-scope"));
                    let mut issues = v_flex()
                        .id("linkage-issues")
                        .max_h(px(130.))
                        .overflow_y_scroll();
                    let definition = source.csv.as_ref().and_then(|csv| csv.definition.as_ref());
                    let conditions =
                        definition.map_or(&[][..], |definition| definition.conditions.as_slice());
                    if !review.unavailable.is_empty() {
                        panel = panel.child(crate::i18n::format(
                            "structured.linkage-unavailable-count",
                            &[("count", &review.unavailable.len())],
                        ));
                    }
                    for (index, issue) in review.issues.iter().enumerate() {
                        let (record, column) = (issue.record, issue.column);
                        let message = crate::i18n::text(issue.message);
                        let message = issue
                            .condition
                            .and_then(|index| conditions.get(index))
                            .and_then(|condition| condition.note.as_deref())
                            .and_then(|note| note.lines().next())
                            .map_or_else(
                                || message.to_string(),
                                |note| format!("{message} · {}", excerpt(note, 150)),
                            );
                        issues = issues.child(
                            Button::new(("linkage-issue", index))
                                .outline()
                                .xsmall()
                                .label(crate::i18n::format(
                                    "structured.linkage-issue",
                                    &[
                                        ("record", &(record + 1).to_string()),
                                        ("column", &(column + 1).to_string()),
                                        ("message", &message),
                                    ],
                                ))
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.select_csv_issue(record, column, window, cx)
                                })),
                        );
                    }
                    for &index in review.unavailable.iter().take(linkage::DISPLAY_LIMIT) {
                        let condition = &conditions[index];
                        let note = condition
                            .note
                            .as_deref()
                            .and_then(|note| note.lines().next())
                            .unwrap_or_default();
                        let key = match condition.predicate.as_ref() {
                            Some(predicate)
                                if definition.is_some_and(|definition| {
                                    !definition.has_field(&predicate.field)
                                }) =>
                            {
                                "structured.linkage-missing-reference"
                            }
                            Some(_) => "structured.linkage-unavailable-reference",
                            None => "structured.linkage-unavailable-rule",
                        };
                        issues = issues.child(div().truncate().child(crate::i18n::format(
                            key,
                            &[("target", &condition.target), ("note", &excerpt(note, 150))],
                        )));
                    }
                    panel.child(issues)
                })
            }))
            .children(
                self.source
                    .as_ref()
                    .and_then(|source| source.notice.as_ref())
                    .map(|notice| {
                        let mut panel = v_flex()
                            .px_4()
                            .pb_2()
                            .gap_1()
                            .text_sm()
                            .child(div().child(crate::i18n::text(notice.label())));
                        if let hed::Check::Compared(differences) = notice {
                            for difference in differences {
                                panel = panel.child(div().truncate().child(crate::i18n::format(
                                    "structured.hed-difference",
                                    &[
                                        ("field", &crate::i18n::text(difference.field)),
                                        ("declared", &difference.declared),
                                        ("actual", &difference.actual),
                                    ],
                                )));
                            }
                        }
                        panel
                    }),
            )
            .children(inventory.map(|inventory| {
                let caption = format!("CycloneDX {}", inventory.version);
                v_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(caption),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(inventory.subject.clone()),
                            )
                            .children(self.document_tree.is_some().then(|| {
                                Button::new("structured-document")
                                    .outline()
                                    .xsmall()
                                    .label(crate::i18n::text("structured.back-to-document"))
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.back_to_tree(cx);
                                    }))
                            })),
                    )
                    .child(
                        h_flex().gap_3().children(
                            [
                                ("structured.components", inventory.components),
                                ("structured.crypto-assets", inventory.crypto_assets),
                                ("structured.services", inventory.services),
                                (
                                    "structured.dependency-entries",
                                    inventory.dependency_entries,
                                ),
                            ]
                            .map(|(label, count)| {
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .rounded_md()
                                    .bg(theme.muted)
                                    .px_3()
                                    .py_2()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(count.to_string()),
                                    )
                                    .child(
                                        div().text_xs().truncate().child(crate::i18n::text(label)),
                                    )
                            }),
                        ),
                    )
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .items_start()
                    .when(lines, |body| {
                        body.child(
                            v_flex()
                                .w(px(310.))
                                .h_full()
                                .border_r_1()
                                .border_color(theme.border)
                                .child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(format!(
                                            "{} / {}",
                                            self.matching.len(),
                                            self.source.as_ref().unwrap().records.len()
                                        )),
                                )
                                .children(
                                    self.source.as_ref().filter(|source| source.truncated).map(
                                        |_| {
                                            div().px_3().text_xs().child(crate::i18n::text(
                                                "structured.records-limit",
                                            ))
                                        },
                                    ),
                                )
                                .child(div().flex_1().min_h_0().child(record_rows)),
                        )
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .when(
                                is_csv && !self.query.is_empty() && visible_is_empty,
                                |pane| {
                                    pane.child(
                                        div().px_4().py_2().text_sm().child(crate::i18n::text(
                                            "structured.no-search-results",
                                        )),
                                    )
                                },
                            )
                            .when(!show_hits, |pane| {
                                pane.child(div().flex_1().min_h_0().child(rows))
                            })
                            .when(show_hits, |pane| {
                                let status = match hit_count {
                                    0 if self.hits.is_none() => {
                                        crate::i18n::text("structured.searching").to_string()
                                    }
                                    0 => crate::i18n::text("structured.no-search-results")
                                        .to_string(),
                                    count if count > SEARCH_LIMIT => format!("{SEARCH_LIMIT}+"),
                                    count => count.to_string(),
                                };
                                pane.child(div().px_4().py_2().text_sm().child(status))
                                    .child(div().flex_1().min_h_0().child(hit_rows))
                            }),
                    ),
            )
            .children(self.selected.zip(self.source.as_ref()).map(|(id, source)| {
                let node = &self.tree.nodes[id];
                let raw = &source.bytes[node.span.clone()];
                let decoded = (source.csv.is_none() && raw.len() <= 16_384)
                    .then(|| serde_json::from_slice::<String>(raw).ok())
                    .flatten();
                let parent = node.parent.map(|id| self.tree.nodes[id].name.as_str());
                let target =
                    inventory
                        .zip(decoded.as_deref())
                        .and_then(|(inventory, reference)| {
                            inventory.target(&node.name, parent, reference)
                        });
                let body = decoded
                    .filter(|text| source.row_review.is_none() || !text.is_empty())
                    .unwrap_or_else(|| {
                        if source.row_review.is_some() && node.span.is_empty() {
                            return node.detail.clone();
                        }
                        excerpt(
                            &String::from_utf8_lossy(&raw[..raw.len().min(16_384)]),
                            16_000,
                        )
                    });
                v_flex()
                    .px_4()
                    .py_2()
                    .gap_1()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "{}  ·  {}–{}",
                                self.tree.breadcrumb(id),
                                node.span.start,
                                node.span.end
                            )),
                    )
                    .children(target.map(|target| {
                        let span = target.span.clone();
                        let label = target.label.clone();
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("structured-follow-reference")
                                    .outline()
                                    .xsmall()
                                    .label(crate::i18n::text("structured.follow-reference"))
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.follow_reference(
                                            span.clone(),
                                            label.clone(),
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                            .child(div().text_sm().truncate().child(target.label.clone()))
                    }))
                    .children(inventory.and_then(|inventory| {
                        let rows = inventory.relationships(&node.span);
                        if rows.is_empty() {
                            return None;
                        }
                        let count = rows.len();
                        Some(
                            uniform_list(
                                "structured-bom-relationships",
                                count,
                                cx.processor(move |view, range: Range<usize>, _, cx| {
                                    let inventory =
                                        view.source.as_ref().unwrap().inventory.as_ref().unwrap();
                                    let rows = inventory.relationships(&view.tree.nodes[id].span);
                                    range
                                        .filter_map(|index| {
                                            let (relation, reference) = rows.get(index)?;
                                            let target = inventory.resolve(reference)?;
                                            let span = target.span.clone();
                                            let label = target.label.clone();
                                            Some(
                                                h_flex()
                                                    .h(px(28.))
                                                    .gap_2()
                                                    .child(
                                                        div().w(px(135.)).text_xs().child(
                                                            crate::i18n::text(relation.label()),
                                                        ),
                                                    )
                                                    .child(
                                                        Button::new((
                                                            "structured-bom-related",
                                                            index,
                                                        ))
                                                        .outline()
                                                        .xsmall()
                                                        .label(label.clone())
                                                        .on_click(cx.listener(
                                                            move |view, _, window, cx| {
                                                                view.follow_reference(
                                                                    span.clone(),
                                                                    label.clone(),
                                                                    window,
                                                                    cx,
                                                                );
                                                            },
                                                        )),
                                                    ),
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .h(px(count.min(4) as f32 * 28.)),
                        )
                    }))
                    .child(
                        div()
                            .id("structured-detail")
                            .max_h(px(155.))
                            .overflow_y_scroll()
                            .text_sm()
                            .child(body),
                    )
            }))
    }
}

fn open_view(
    title: SharedString,
    source: Option<Arc<Source>>,
    tree: Tree,
    origin: Option<Origin>,
    cx: &mut App,
) {
    let (editor, table) = match origin {
        Some(Origin::Outline(editor)) => (Some(editor), None),
        Some(Origin::Inventory(table)) => (None, Some(table)),
        None => (None, None),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1010.), px(670.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.clone()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let _ = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| {
            if let Some((owner, _, _)) = &editor
                && let Some(owner) = owner.upgrade()
            {
                cx.observe(&owner, |_: &mut View, _, cx| cx.notify())
                    .detach();
            }
            let search = cx.new(|cx| {
                let key = match source.as_ref() {
                    Some(source) if source.inventory.is_some() => "structured.search-bom",
                    Some(source) if source.csv.is_some() => "structured.search-columns",
                    _ => "structured.search",
                };
                InputState::new(window, cx).placeholder(crate::i18n::text(key))
            });
            let subscription =
                cx.subscribe(&search, |view: &mut View, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        view.search(cx);
                    }
                });
            let matching = source
                .as_ref()
                .filter(|source| source.lines)
                .map(|source| (0..source.records.len()).collect())
                .unwrap_or_default();
            let mut view = View {
                title,
                source,
                tree,
                editor,
                table,
                search,
                matching,
                query: String::new(),
                hits: None,
                record: 0,
                record_scroll: UniformListScrollHandle::default(),
                tree_scroll: UniformListScrollHandle::default(),
                selected: None,
                document_tree: None,
                _subscription: subscription,
            };
            if view
                .source
                .as_ref()
                .is_some_and(|source| !source.records.is_empty())
            {
                view.select_record(0, cx);
            }
            view
        });
        cx.new(|cx| Root::new(view, window, cx))
    });
}

pub(crate) fn has_row_schema(path: &std::path::Path) -> bool {
    row_schema::sidecar(path).is_some()
}

impl crate::editor::SlideEditor {
    pub(crate) fn open_bom_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.bom_source.clone() else {
            return;
        };
        let path = source.path.clone();
        let table = InventoryTarget {
            editor: (cx.weak_entity(), window.window_handle(), self.file_id),
            source,
        };
        self.open_structured_data(path, Some(table), window, cx);
    }

    pub(crate) fn open_structured_data(
        &mut self,
        path: PathBuf,
        table: Option<InventoryTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title: SharedString = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
            .into();
        cx.spawn_in(window, async move |this, cx| {
            let expected = table.as_ref().map(|table| table.source.digest);
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    let source = Source::read(path)?;
                    if let Some(expected) = expected {
                        use sha2::{Digest, Sha256};
                        if <[u8; 32]>::from(Sha256::digest(&source.bytes)) != expected {
                            return Err(crate::i18n::text("structured.bom-changed").to_string());
                        }
                    }
                    Ok(source)
                })
                .await;
            let _ = this.update(cx, |editor, cx| match loaded {
                Ok(source) => open_view(
                    title,
                    Some(Arc::new(source)),
                    Tree::default(),
                    table.map(Origin::Inventory),
                    cx,
                ),
                Err(error) => {
                    editor.show_alert(crate::i18n::text("structured.open"), error.into(), cx)
                }
            });
        })
        .detach();
    }

    pub(crate) fn open_structured_outline(&mut self, window: &Window, cx: &mut Context<Self>) {
        if let Some(flow) = self.presentation().flow.as_ref() {
            let tree = Tree::outline(flow);
            let owner = (cx.weak_entity(), window.window_handle(), self.file_id);
            // The first frame reads fold state from the owner.
            cx.defer(move |cx| {
                open_view(
                    crate::i18n::text("structured.outline"),
                    None,
                    tree,
                    Some(Origin::Outline(owner)),
                    cx,
                );
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PAGE_SIZE, Source, Tree};

    #[test]
    fn schema_path_expands_nested_arrays_to_the_selected_field() {
        use super::row_schema::Segment::{Index, Key};
        let source = Source::new(br#"{"team":{"contacts":[{"name":""}]}}"#.to_vec(), false);
        let mut tree = Tree::default();
        let root = tree.push(source.record(0));
        tree.roots.push(root);
        let contacts = [Key("team".into()), Key("contacts".into())];
        let array = tree.select_schema_path(root, &contacts, &source);
        assert_eq!(
            &source.bytes[tree.nodes[array].span.clone()],
            br#"[{"name":""}]"#
        );
        let field = tree.select_schema_path(
            root,
            &[
                contacts[0].clone(),
                contacts[1].clone(),
                Index(0),
                Key("name".into()),
            ],
            &source,
        );
        assert_eq!(tree.breadcrumb(field), "$ / team / contacts / [0] / name");
        assert_eq!(&source.bytes[tree.nodes[field].span.clone()], br#""""#);
        assert!(tree.visible("").iter().any(|(id, _)| *id == field));
    }

    #[test]
    fn searches_nested_fields_without_loading_tree_pages() {
        let source = Source::new(
            br#"{"meta":{"data":[{"name":"first"},{"name":"deeper"}]},"number":15}"#.to_vec(),
            true,
        );
        let hits = source.field_search(0, "deeper");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "$ / meta / data / [1] / name");
        assert_eq!(&source.bytes[hits[0].span.clone()], br#""deeper""#);
        assert_eq!(source.field_search(0, "name").len(), 2);
    }

    #[test]
    fn retains_record_order_and_duplicate_nested_fields() {
        let source = Source::new(
            b"{\"z\":{\"a\":null,\"a\":\"\",\"n\":900719925474099312345},\"b\":1}\n{bad}\n[42]"
                .to_vec(),
            true,
        );
        let children = source.children(source.record(0).span, 0).0;
        assert_eq!(
            children
                .iter()
                .map(|node| node.name.as_str())
                .collect::<Vec<_>>(),
            ["z", "b"]
        );
        let nested = source.children(children[0].span.clone(), 0).0;
        assert_eq!(
            nested
                .iter()
                .map(|node| node.detail.as_str())
                .collect::<Vec<_>>(),
            ["null", "\"\"", "900719925474099312345"]
        );
        assert_eq!(source.record(1).children.unwrap().len(), 0);
        assert_eq!(source.children(source.record(2).span, 0).0[0].detail, "42");
    }

    #[test]
    fn pages_children_without_losing_array_indices() {
        let values = vec![0; PAGE_SIZE + 2];
        let source = Source::new(serde_json::to_vec(&values).unwrap(), false);
        let root = source.record(0);
        let (first, more) = source.children(root.span.clone(), 0);
        assert_eq!(first.len(), PAGE_SIZE);
        assert!(more);
        let (last, more) = source.children(root.span, PAGE_SIZE);
        assert_eq!(last[0].name, format!("[{PAGE_SIZE}]"));
        assert_eq!(last.len(), 2);
        assert!(!more);
    }

    #[test]
    fn outlines_markdown_by_level_and_paragraph_id() {
        let opened =
            ibis_formats::markdown::open_markdown(b"# Same\n\n## Child\n\n# Same\n".to_vec())
                .unwrap();
        let mut tree = Tree::outline(opened.presentation.flow.as_ref().unwrap());
        assert_eq!(tree.roots.len(), 2);
        let (first, second) = (tree.roots[0], tree.roots[1]);
        assert_ne!(tree.nodes[first].paragraph, tree.nodes[second].paragraph);
        assert_eq!(tree.visible("").len(), 3);
        tree.toggle(first, None);
        assert_eq!(tree.visible("").len(), 2);
    }

    #[test]
    fn cyclonedx_components_have_names_in_the_tree() {
        let source = Source::new(
            include_bytes!("../../../test/fixtures/cbom/payment-demo.cdx.json").to_vec(),
            false,
        );
        let root = source.record(0);
        let components = source
            .children(root.span, 0)
            .0
            .into_iter()
            .find(|node| node.name == "components")
            .unwrap();
        let assets = source.children(components.span, 0).0;
        assert!(assets.iter().any(|node| node.name.starts_with("[0] ")));
    }
}
