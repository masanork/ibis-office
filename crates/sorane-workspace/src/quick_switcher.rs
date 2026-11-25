// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Obsidian-style Quick Switcher (`Cmd/Ctrl+O`) for jumping across Vault files and open tabs.

use std::path::PathBuf;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{badge_for_path_and_kind, fuzzy_score};
use crate::workspace::SoraneWorkspace;

const MAX_RESULTS: usize = 30;

#[derive(Clone, Debug)]
pub(crate) struct SwitcherCandidate {
    pub(crate) label: String,
    pub(crate) subtitle: String,
    pub(crate) path: PathBuf,
    pub(crate) badge: &'static str,
    pub(crate) is_open: bool,
}

pub(crate) struct QuickSwitcherState {
    pub(crate) input: Entity<InputState>,
    pub(crate) selected: usize,
    pub(crate) _sub: Subscription,
}

impl SoraneWorkspace {
    pub(crate) fn open_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_switcher.is_some() {
            self.close_quick_switcher(window, cx);
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("ファイル名で検索… (Enter: 開く / ⌘Enter: 分割して開く)"));
        let sub = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(qs) = this.quick_switcher.as_mut() {
                    qs.selected = 0;
                }
                cx.notify();
            }
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        self.quick_switcher = Some(QuickSwitcherState {
            input,
            selected: 0,
            _sub: sub,
        });
        cx.notify();
    }

    pub(crate) fn close_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_switcher.take().is_some() {
            self.focus_active_editor(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn quick_switcher_candidates(&self, query: &str) -> Vec<SwitcherCandidate> {
        let mut items: Vec<(i32, SwitcherCandidate)> = Vec::new();
        let mut seen_paths: Vec<PathBuf> = Vec::new();

        // 1. Currently open file-backed tabs (boosted slightly so jumping between active docs is instant).
        for pane in &self.panes {
            for tab in &pane.tabs {
                let Some(path) = tab.path.as_ref() else { continue };
                if seen_paths.iter().any(|p| p == path) {
                    continue;
                }
                seen_paths.push(path.clone());
                let rel = self
                    .vault
                    .as_ref()
                    .and_then(|v| v.relative_path_for(path))
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                let Some(score) = fuzzy_score(&rel, query) else { continue };
                items.push((
                    score + 200,
                    SwitcherCandidate {
                        label: tab.title.clone(),
                        subtitle: rel,
                        path: path.clone(),
                        badge: tab.display_badge(),
                        is_open: true,
                    },
                ));
            }
        }

        // 2. Vault files from `folder_context::Folder`.
        if let Some(vault) = self.vault.as_ref() {
            for rel in vault.files() {
                let Some(score) = fuzzy_score(rel, query) else { continue };
                let full = vault.root().join(rel);
                if seen_paths.iter().any(|p| p == &full) {
                    continue;
                }
                seen_paths.push(full.clone());
                let label = rel.rsplit('/').next().unwrap_or(rel).to_owned();
                let kind = full
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(ibis_formats::Kind::from_extension)
                    .unwrap_or(ibis_formats::Kind::Document);
                items.push((
                    score,
                    SwitcherCandidate {
                        label,
                        subtitle: rel.clone(),
                        path: full,
                        badge: badge_for_path_and_kind(Some(std::path::Path::new(rel)), kind),
                        is_open: false,
                    },
                ));
            }
        }

        items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.subtitle.cmp(&b.1.subtitle)));
        items.into_iter().take(MAX_RESULTS).map(|(_, c)| c).collect()
    }

    pub(crate) fn on_quick_switcher_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(qs) = self.quick_switcher.as_ref() else {
            return false;
        };
        let query = qs.input.read(cx).value().to_string();
        let candidates = self.quick_switcher_candidates(&query);
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.close_quick_switcher(window, cx);
                true
            }
            "down" => {
                if !candidates.is_empty()
                    && let Some(qs) = self.quick_switcher.as_mut()
                {
                    qs.selected = (qs.selected + 1) % candidates.len();
                    cx.notify();
                }
                true
            }
            "up" => {
                if !candidates.is_empty()
                    && let Some(qs) = self.quick_switcher.as_mut()
                {
                    qs.selected = (qs.selected + candidates.len() - 1) % candidates.len();
                    cx.notify();
                }
                true
            }
            "enter" => {
                let open_in_split = event.keystroke.modifiers.platform || event.keystroke.modifiers.control;
                if let Some(choice) = candidates.get(qs.selected).cloned() {
                    self.quick_switcher = None;
                    if open_in_split {
                        self.open_path_in_split_pane(choice.path, window, cx);
                    } else {
                        self.open_path_in_active_pane(choice.path, window, cx);
                    }
                } else {
                    self.close_quick_switcher(window, cx);
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn render_quick_switcher(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let qs = self.quick_switcher.as_ref()?;
        let theme = cx.theme().clone();
        let query = qs.input.read(cx).value().to_string();
        let candidates = self.quick_switcher_candidates(&query);
        let selected = qs.selected.min(candidates.len().saturating_sub(1));

        let mut list = v_flex().id("quick-switcher-list").max_h(px(360.)).overflow_y_scroll();
        if candidates.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py_3()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(if self.vault.is_none() {
                        "フォルダ（Vault）を開くとファイルを検索できます"
                    } else {
                        "一致するファイルがありません"
                    }),
            );
        } else {
            for (idx, item) in candidates.into_iter().enumerate() {
                let is_sel = idx == selected;
                let path = item.path.clone();
                let split_path = item.path.clone();
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("qs-row-{idx}")))
                        .px_3()
                        .py(px(6.))
                        .gap_2()
                        .items_center()
                        .cursor_pointer()
                        .when(is_sel, |r| r.bg(theme.accent))
                        .when(!is_sel, |r| r.hover(|h| h.bg(theme.secondary)))
                        .child(
                            div()
                                .px(px(5.))
                                .py(px(1.))
                                .rounded(px(3.))
                                .bg(theme.muted)
                                .text_size(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.muted_foreground)
                                .child(item.badge),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.foreground)
                                        .truncate()
                                        .child(if item.is_open {
                                            format!("{}  •", item.label)
                                        } else {
                                            item.label
                                        }),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .truncate()
                                        .child(item.subtitle),
                                ),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("qs-split-{idx}")))
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .hover(|h| h.bg(theme.background).text_color(theme.foreground))
                                .child("◫")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.quick_switcher = None;
                                    this.open_path_in_split_pane(split_path.clone(), window, cx);
                                })),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.quick_switcher = None;
                            this.open_path_in_active_pane(path.clone(), window, cx);
                        })),
                );
            }
        }

        Some(
            div()
                .id("quick-switcher-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(56.))
                .bg(hsla(0.0, 0.0, 0.0, 0.28))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.close_quick_switcher(window, cx);
                    }),
                )
                .child(
                    v_flex()
                        .id("quick-switcher-panel")
                        .w(px(460.))
                        .max_w(px(520.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.background)
                        .shadow_lg()
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .p_2()
                                .border_b_1()
                                .border_color(theme.border)
                                .child(Input::new(&qs.input).small()),
                        )
                        .child(list),
                )
                .into_any_element(),
        )
    }
}
