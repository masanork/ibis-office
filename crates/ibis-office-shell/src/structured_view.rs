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
            let notice = hed::Check::read(&path, bytes.len(), csv.data_count);
            return Ok(Self {
                bytes,
                records: std::mem::take(&mut csv.records),
                lines: true,
                truncated: csv.truncated,
                inventory: None,
                csv: Some(csv),
                notice: Some(notice),
            });
        }
        let mut source = Self::new(bytes, lines);
        if !lines {
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
                if let Some(label) = label {
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

type OutlineTarget = (WeakEntity<crate::editor::SlideEditor>, AnyWindowHandle, u64);

struct View {
    title: SharedString,
    source: Option<Arc<Source>>,
    tree: Tree,
    editor: Option<OutlineTarget>,
    search: Entity<InputState>,
    matching: Vec<usize>,
    query: String,
    hits: Option<Vec<SearchHit>>,
    record: usize,
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
        cx.notify();
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        self.query = self.search.read(cx).value().trim().to_lowercase();
        self.hits = None;
        if let Some(source) = self.source.clone()
            && let Some(inventory) = &source.inventory
            && !self.query.is_empty()
        {
            self.hits = Some(inventory.search(&self.query));
        }
        if let Some(source) = self.source.clone().filter(|source| source.lines) {
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
        let visible = self.tree.visible(if lines { "" } else { &self.query });
        let show_hits = inventory.is_some() && !self.query.is_empty();
        let hit_count = self.hits.as_ref().map_or(0, Vec::len);
        let hit_rows = uniform_list(
            "structured-search-hits",
            hit_count.min(SEARCH_LIMIT),
            cx.processor(|view, range: Range<usize>, _, cx| {
                range
                    .filter_map(|position| {
                        let hit = view.hits.as_ref()?.get(position)?.clone();
                        let label = hit.label.clone();
                        let path = hit.path.clone();
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
                                        path.clone(),
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
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        if let Some((tree, selected)) = view.document_tree.take() {
                                            view.tree = tree;
                                            view.selected = selected;
                                            view.query.clear();
                                            view.search.update(cx, |input, cx| {
                                                input.set_value("", window, cx)
                                            });
                                            cx.notify();
                                        }
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
                            .when(!show_hits, |pane| {
                                pane.child(div().flex_1().min_h_0().child(rows))
                            })
                            .when(show_hits, |pane| {
                                let status = match hit_count {
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
                let body = decoded.unwrap_or_else(|| {
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
    editor: Option<OutlineTarget>,
    cx: &mut App,
) {
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
                InputState::new(window, cx).placeholder(crate::i18n::text("structured.search"))
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
                search,
                matching,
                query: String::new(),
                hits: None,
                record: 0,
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

impl crate::editor::SlideEditor {
    pub(crate) fn open_structured_data(
        &mut self,
        path: PathBuf,
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
            let loaded = cx
                .background_executor()
                .spawn(async move { Source::read(path) })
                .await;
            let _ = this.update(cx, |editor, cx| match loaded {
                Ok(source) => open_view(title, Some(Arc::new(source)), Tree::default(), None, cx),
                Err(error) => {
                    editor.show_alert(crate::i18n::text("structured.open"), error.into(), cx)
                }
            });
        })
        .detach();
    }

    pub(crate) fn pick_structured_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            extensions: vec!["json".into(), "jsonl".into(), "ndjson".into(), "csv".into()],
            prompt: Some(crate::i18n::text("structured.open")),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |editor, window, cx| {
                    editor.open_structured_data(path, window, cx)
                });
            }
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
                    Some(owner),
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
