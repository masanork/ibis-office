// SPDX-License-Identifier: AGPL-3.0-only
//! Developer-friendly foldable code/tree view for JSON, JSONL, XML, YAML, and TOML.

use std::ops::Range;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_formats::structured_preview::code_tree::{CodeDocument, TokenKind};

/// Display modes available for a structured file inside [`crate::editor::SlideEditor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredViewMode {
    Code,
    Table,
    Document,
}

impl StructuredViewMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Code => "構造テキスト",
            Self::Table => "表形式",
            Self::Document => "ドキュメント",
        }
    }
}

/// Shared raw payload kept on [`crate::editor::SlideEditor`] so users can switch
/// between views (Code / Table / Document) without re-opening the file.
#[derive(Clone)]
pub struct StructuredSource {
    pub name: SharedString,
    pub extension: String,
    pub bytes: Arc<Vec<u8>>,
    pub available_modes: Vec<StructuredViewMode>,
    pub active_mode: StructuredViewMode,
}

pub struct StructuredCodeView {
    pub title: SharedString,
    doc: CodeDocument,
    visible: Vec<usize>,
    selected_line: Option<usize>,
    search: Entity<InputState>,
    query: String,
    matches: Vec<usize>,
    match_cursor: usize,
    scroll: UniformListScrollHandle,
    on_switch_mode: Option<Box<dyn Fn(StructuredViewMode, &mut Window, &mut App) + 'static>>,
    available_modes: Vec<StructuredViewMode>,
    _subscription: Subscription,
}

impl StructuredCodeView {
    pub fn new(
        title: SharedString,
        extension: &str,
        bytes: &[u8],
        available_modes: Vec<StructuredViewMode>,
        on_switch_mode: Option<Box<dyn Fn(StructuredViewMode, &mut Window, &mut App) + 'static>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Self, String> {
        let doc = CodeDocument::parse(extension, bytes)?;
        let visible = doc.visible_lines();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("検索 (キー / 値 / パス)…"));
        let subscription = cx.subscribe(&search, |view: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.update_search(cx);
            }
        });
        Ok(Self {
            title,
            doc,
            visible,
            selected_line: None,
            search,
            query: String::new(),
            matches: Vec::new(),
            match_cursor: 0,
            scroll: UniformListScrollHandle::default(),
            on_switch_mode,
            available_modes,
            _subscription: subscription,
        })
    }

    fn refresh_visible(&mut self) {
        self.visible = self.doc.visible_lines();
    }

    fn toggle_fold(&mut self, node_id: usize, cx: &mut Context<Self>) {
        self.doc.toggle_fold(node_id);
        self.refresh_visible();
        cx.notify();
    }

    fn expand_all(&mut self, cx: &mut Context<Self>) {
        self.doc.expand_all();
        self.refresh_visible();
        cx.notify();
    }

    fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.doc.collapse_all();
        self.refresh_visible();
        cx.notify();
    }

    fn collapse_depth_2(&mut self, cx: &mut Context<Self>) {
        self.doc.collapse_to_depth(2);
        self.refresh_visible();
        cx.notify();
    }

    fn update_search(&mut self, cx: &mut Context<Self>) {
        self.query = self.search.read(cx).value().to_string();
        self.matches = self.doc.search_matches(&self.query);
        self.match_cursor = 0;
        if let Some(&first) = self.matches.first() {
            self.jump_to_line(first);
        }
        cx.notify();
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        if forward {
            self.match_cursor = (self.match_cursor + 1) % self.matches.len();
        } else if self.match_cursor == 0 {
            self.match_cursor = self.matches.len() - 1;
        } else {
            self.match_cursor -= 1;
        }
        let target = self.matches[self.match_cursor];
        self.jump_to_line(target);
        cx.notify();
    }

    fn jump_to_line(&mut self, line_idx: usize) {
        self.doc.reveal_line(line_idx);
        self.refresh_visible();
        self.selected_line = Some(line_idx);
        if let Some(pos) = self.visible.iter().position(|&idx| idx == line_idx) {
            self.scroll.scroll_to_item(pos, ScrollStrategy::Center);
        }
    }

    fn token_color(kind: TokenKind, is_dark: bool, fg: Hsla, muted: Hsla) -> Hsla {
        match (kind, is_dark) {
            (TokenKind::Key | TokenKind::TagName, true) => rgb(0x7dd3fc).into(),
            (TokenKind::Key | TokenKind::TagName, false) => rgb(0x0369a1).into(),
            (TokenKind::AttrName, true) => rgb(0xfdba74).into(),
            (TokenKind::AttrName, false) => rgb(0xb45309).into(),
            (TokenKind::String | TokenKind::AttrValue, true) => rgb(0x86efac).into(),
            (TokenKind::String | TokenKind::AttrValue, false) => rgb(0x15803d).into(),
            (TokenKind::Number, true) => rgb(0xf9a8d4).into(),
            (TokenKind::Number, false) => rgb(0xbe185d).into(),
            (TokenKind::Boolean | TokenKind::Null, true) => rgb(0xc4b5fd).into(),
            (TokenKind::Boolean | TokenKind::Null, false) => rgb(0x6d28d9).into(),
            (TokenKind::Comment | TokenKind::Summary, _) => muted,
            (TokenKind::Punctuation, _) => muted,
            (TokenKind::Text, _) => fg,
        }
    }
}

impl Render for StructuredCodeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let visible_len = self.visible.len();
        let match_count = self.matches.len();
        let active_match_line = self.matches.get(self.match_cursor).copied();
        let selected_path = self
            .selected_line
            .and_then(|idx| self.doc.lines.get(idx))
            .map(|line| line.path.clone())
            .unwrap_or_else(|| self.title.to_string());

        let header = h_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded_sm()
                            .bg(theme.accent)
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .child(self.doc.format_label),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!(
                                "{} 行 · {} B",
                                self.doc.lines.len(),
                                self.doc.byte_size
                            )),
                    )
                    .child(
                        Button::new("code-expand-all")
                            .ghost()
                            .xsmall()
                            .label("すべて展開")
                            .on_click(cx.listener(|view, _, _, cx| view.expand_all(cx))),
                    )
                    .child(
                        Button::new("code-collapse-l2")
                            .ghost()
                            .xsmall()
                            .label("L2")
                            .on_click(cx.listener(|view, _, _, cx| view.collapse_depth_2(cx))),
                    )
                    .child(
                        Button::new("code-collapse-all")
                            .ghost()
                            .xsmall()
                            .label("すべて折りたたむ")
                            .on_click(cx.listener(|view, _, _, cx| view.collapse_all(cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .when(self.available_modes.len() > 1, |bar| {
                        bar.child(h_flex().gap_1().children(self.available_modes.iter().copied().map(
                            |mode| {
                                let is_active = mode == StructuredViewMode::Code;
                                Button::new(("mode-switch", mode as usize))
                                    .xsmall()
                                    .when(is_active, |b| b.primary())
                                    .when(!is_active, |b| b.outline())
                                    .label(mode.label())
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        if mode != StructuredViewMode::Code
                                            && let Some(cb) = &view.on_switch_mode
                                        {
                                            cb(mode, window, cx);
                                        }
                                    }))
                            },
                        )))
                    })
                    .child(div().w(px(220.)).child(Input::new(&self.search).small()))
                    .when(!self.query.is_empty(), |bar| {
                        bar.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(if match_count == 0 {
                                    "0 件".to_string()
                                } else {
                                    format!("{}/{}", self.match_cursor + 1, match_count)
                                }),
                        )
                        .child(
                            Button::new("code-prev-match")
                                .ghost()
                                .xsmall()
                                .label("▲")
                                .on_click(cx.listener(|view, _, _, cx| view.step_match(false, cx))),
                        )
                        .child(
                            Button::new("code-next-match")
                                .ghost()
                                .xsmall()
                                .label("▼")
                                .on_click(cx.listener(|view, _, _, cx| view.step_match(true, cx))),
                        )
                    }),
            );

        let list = uniform_list(
            "structured-code-lines",
            visible_len,
            cx.processor(move |view, range: Range<usize>, _, cx| {
                let theme = cx.theme();
                let is_dark = theme.is_dark();
                let fg = theme.foreground;
                let muted = theme.muted_foreground;
                let border = theme.border;
                let accent = theme.accent;
                let query_lower = view.query.trim().to_ascii_lowercase();

                range
                    .map(|pos| {
                        let line_idx = view.visible[pos];
                        let line = &view.doc.lines[line_idx];
                        let is_selected = view.selected_line == Some(line_idx);
                        let is_active_match = active_match_line == Some(line_idx);
                        let is_match = !query_lower.is_empty()
                            && (line.plain_text.to_ascii_lowercase().contains(&query_lower)
                                || line.path.to_ascii_lowercase().contains(&query_lower));
                        let node_id = line.node_id;
                        let is_folded = node_id.is_some_and(|id| view.doc.is_folded(id));
                        let indent = px((line.depth.min(24) * 18) as f32);

                        h_flex()
                            .id(("code-line", line_idx))
                            .h(px(24.))
                            .w_full()
                            .items_center()
                            .px_2()
                            .font_family("monospace")
                            .text_xs()
                            .when(is_selected || is_active_match, |row| row.bg(accent))
                            .when(!is_selected && !is_active_match && is_match, |row| {
                                row.bg(if is_dark {
                                    hsla(48. / 360., 0.7, 0.25, 0.45)
                                } else {
                                    hsla(48. / 360., 0.95, 0.85, 0.7)
                                })
                            })
                            .hover(|row| row.bg(accent.opacity(0.45)))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.selected_line = Some(line_idx);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .w(px(48.))
                                    .flex_shrink_0()
                                    .pr_2()
                                    .text_right()
                                    .text_color(muted)
                                    .child(line.line_number.to_string()),
                            )
                            .child(
                                div()
                                    .w(px(22.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .children(node_id.map(|id| {
                                        Button::new(("fold-btn", id))
                                            .ghost()
                                            .xsmall()
                                            .label(if is_folded { "▸" } else { "▾" })
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                view.toggle_fold(id, cx);
                                            }))
                                    })),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .pl(indent)
                                    .border_l_1()
                                    .border_color(border.opacity(0.4))
                                    .overflow_hidden()
                                    .children(line.tokens.iter().map(|tok| {
                                        let color = Self::token_color(tok.kind, is_dark, fg, muted);
                                        div()
                                            .flex_shrink_0()
                                            .text_color(color)
                                            .child(tok.text.clone())
                                    }))
                                    .when(is_folded, |row| {
                                        row.children(line.summary.as_ref().map(|summary| {
                                            div()
                                                .ml_1()
                                                .px_1()
                                                .rounded_sm()
                                                .bg(accent)
                                                .text_color(muted)
                                                .child(summary.clone())
                                        }))
                                    }),
                            )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();

        let footer = h_flex()
            .w_full()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .text_xs()
            .text_color(muted)
            .justify_between()
            .child(div().truncate().child(selected_path))
            .child(div().child(format!("表示中: {visible_len} 行")));

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(fg)
            .child(header)
            .child(div().flex_1().min_h_0().w_full().child(list))
            .child(footer)
    }
}
