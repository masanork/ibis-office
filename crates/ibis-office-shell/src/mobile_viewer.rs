// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Compact phone viewing UI (top bar, overflow menu, and floating bottom pill).
//!
//! When a phone-sized viewport (`width < 600`) opens a file, it starts in viewing mode
//! (`phone_view_only`): one-finger drags scroll the document naturally, while the top bar
//! and floating pill provide navigation, search, export/open, fit-to-width, and an
//! explicit switch to editing mode.

use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions::{Open, Save};
use crate::editor::SlideEditor;
use crate::format::with_format;
use crate::icons::Material;

impl SlideEditor {
    /// Whether any menu item with `id` is currently enabled.
    pub(crate) fn has_enabled_menu_command(&mut self, id: &str) -> bool {
        self.menus().find_mut(id).is_some_and(|item| item.enabled())
    }

    /// Execute an enabled menu item by its command id (e.g. `"edit.find-replace"`).
    pub(crate) fn run_menu_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let handler = self.menus().find_mut(id).and_then(|item| item.enabled().then(|| item.handler.clone()).flatten());
        if let Some(handler) = handler {
            crate::chrome::dispatch(&handler, self, window, cx);
            true
        } else {
            false
        }
    }

    /// Whether a sidebar panel allowed during phone viewing mode (AI assistant or Find) is open.
    pub(crate) fn mobile_viewer_sidebar_open(&self) -> bool {
        self.ai_assistant.is_some()
            || self.shape_edit.find.is_some()
            || self.sidebar_open
    }

    /// Short position label for the bottom floating pill (e.g. `1 / 12` or sheet name).
    fn mobile_position_label(&self) -> SharedString {
        let slides = &self.presentation().slides;
        let index = self.presentation().slide_index(self.current_slide).unwrap_or(0);
        let items = with_format!(self, status_items(self));
        if let Some(sheet_or_item) = items.first().filter(|_| self.presentation().workbook.is_some()) {
            return crate::i18n::text(sheet_or_item);
        }
        format!("{} / {}", index + 1, slides.len().max(1)).into()
    }

    /// Compact document controls for phone-sized windows.
    pub(crate) fn render_mobile_viewer_bar(&mut self, narrow_phone: bool, cx: &mut Context<Self>) -> AnyElement {
        let editing = self.is_phone_editing();
        let can_find = !editing && self.has_document && self.has_enabled_menu_command("edit.find-replace");
        let theme = cx.theme();
        let name = self.file_name.clone().unwrap_or_else(|| "soraneDocs".into());
        let can_edit = narrow_phone && !self.file_is_read_only() && self.host.read_only.is_none();
        let ai_open = self.ai_assistant.is_some();

        if !editing {
            return h_flex()
                .h(px(48.))
                .flex_shrink_0()
                .px_2()
                .gap_1()
                .items_center()
                .bg(theme.title_bar)
                .border_b_1()
                .border_color(theme.title_bar_border)
                .when(self.has_document, |bar| {
                    bar.child(
                        div()
                            .id("mobile-back-to-start")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(40.))
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(theme.foreground)
                            .child(Icon::new(Material("arrow_back")).size(px(20.)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.mobile_viewer_menu_open = false;
                                this.back_to_start(window, cx);
                            })),
                    )
                })
                .child(div().flex_1().min_w_0().px_1().truncate().text_sm().font_weight(FontWeight::MEDIUM).child(name))
                .when(can_find, |bar| {
                    bar.child(
                        div()
                            .id("mobile-find")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(40.))
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(theme.foreground)
                            .child(Icon::new(Material("search")).size(px(20.)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.mobile_viewer_menu_open = false;
                                this.run_menu_command("edit.find-replace", window, cx);
                            })),
                    )
                })
                .child(
                    div()
                        .id("mobile-more-menu")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(40.))
                        .rounded_md()
                        .cursor_pointer()
                        .when(self.mobile_viewer_menu_open, |d| d.bg(theme.accent).text_color(theme.accent_foreground))
                        .when(!self.mobile_viewer_menu_open, |d| d.text_color(theme.foreground))
                        .child(Icon::new(Material("more_horiz")).size(px(20.)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.mobile_viewer_menu_open = !this.mobile_viewer_menu_open;
                            cx.notify();
                        })),
                )
                .into_any_element();
        }

        h_flex()
            .h(px(48.))
            .flex_shrink_0()
            .px_2()
            .gap_1()
            .items_center()
            .bg(theme.title_bar)
            .border_b_1()
            .border_color(theme.title_bar_border)
            .child(div().flex_1().min_w_0().px_1().truncate().text_sm().font_weight(FontWeight::MEDIUM).child(name))
            .when(can_edit, |bar| {
                bar.child(
                    div()
                        .id("mobile-save-file")
                        .flex()
                        .items_center()
                        .justify_center()
                        .min_w(px(48.))
                        .h(px(40.))
                        .px_2()
                        .rounded_md()
                        .cursor_pointer()
                        .text_sm()
                        .text_color(theme.foreground)
                        .child(crate::i18n::text("file.save"))
                        .on_click(cx.listener(|this, _, window, cx| this.save(&Save, window, cx))),
                )
            })
            .when(narrow_phone, |bar| {
                bar.child(
                    div()
                        .id("mobile-ai-toggle")
                        .flex()
                        .items_center()
                        .justify_center()
                        .min_w(px(44.))
                        .h(px(40.))
                        .px_2()
                        .rounded_md()
                        .cursor_pointer()
                        .when(ai_open, |d| d.bg(theme.accent).text_color(theme.accent_foreground).font_weight(FontWeight::SEMIBOLD))
                        .when(!ai_open, |d| d.text_color(theme.foreground))
                        .text_sm()
                        .child("AI")
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.ai_assistant.is_some() {
                                this.ai_assistant = None;
                                this.sidebar_open = false;
                                cx.notify();
                            } else {
                                this.open_ai_assistant_panel(window, cx);
                            }
                        })),
                )
            })
            .child(
                div()
                    .id("mobile-open-file")
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(52.))
                    .h(px(40.))
                    .px_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(crate::i18n::text("start.open"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_file(&Open, window, cx))),
            )
            .when(can_edit, |bar| {
                bar.child(
                    div()
                        .id("mobile-switch-mode")
                        .flex()
                        .items_center()
                        .justify_center()
                        .min_w(px(56.))
                        .h(px(40.))
                        .px_2()
                        .rounded_md()
                        .cursor_pointer()
                        .bg(theme.accent)
                        .text_color(theme.accent_foreground)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_sm()
                        .child(crate::i18n::text("docx.interaction.viewing"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.set_phone_editing(false, window, cx);
                        })),
                )
            })
            .into_any_element()
    }

    /// Overflow menu anchored beneath the top-right `⋯` button in phone viewing mode.
    pub(crate) fn render_mobile_viewer_menu(&mut self, narrow_phone: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.mobile_viewer_menu_open || self.is_phone_editing() {
            return None;
        }
        let can_export_pdf = self.has_document && self.has_enabled_menu_command("file.export.pdf");
        let can_edit = narrow_phone && !self.file_is_read_only() && self.host.read_only.is_none();
        let theme = cx.theme();
        let row = |id: &'static str, icon_name: &'static str, label: SharedString| {
            h_flex()
                .id(id)
                .w_full()
                .h(px(44.))
                .px_3()
                .gap_3()
                .items_center()
                .rounded_md()
                .cursor_pointer()
                .text_sm()
                .text_color(theme.popover_foreground)
                .hover(|s| s.bg(theme.list_hover))
                .child(Icon::new(Material(icon_name)).size(px(18.)))
                .child(label)
        };

        Some(
            div()
                .id("mobile-viewer-menu-backdrop")
                .absolute()
                .inset_0()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.mobile_viewer_menu_open = false;
                    cx.notify();
                }))
                .child(
                    v_flex()
                        .id("mobile-viewer-menu")
                        .absolute()
                        .top(px(4.))
                        .right(px(8.))
                        .w(px(210.))
                        .p_1()
                        .gap_0p5()
                        .rounded_lg()
                        .bg(theme.popover)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            row("mobile-menu-open", "folder_open", crate::i18n::text("start.open"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.mobile_viewer_menu_open = false;
                                    this.open_file(&Open, window, cx);
                                })),
                        )
                        .when(can_export_pdf, |menu| {
                            menu.child(
                                row("mobile-menu-export-pdf", "download", crate::i18n::text("file.export.pdf"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.mobile_viewer_menu_open = false;
                                        this.run_menu_command("file.export.pdf", window, cx);
                                    })),
                            )
                        })
                        .when(narrow_phone, |menu| {
                            menu.child(
                                row("mobile-menu-ai", "info", SharedString::from("AI"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.mobile_viewer_menu_open = false;
                                        if this.ai_assistant.is_some() {
                                            this.ai_assistant = None;
                                            this.sidebar_open = false;
                                            cx.notify();
                                        } else {
                                            this.open_ai_assistant_panel(window, cx);
                                        }
                                    })),
                            )
                        })
                        .child(
                            row("mobile-menu-hide-hud", "visibility_off", SharedString::from("全画面表示"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.mobile_viewer_menu_open = false;
                                    this.mobile_hud_hidden = true;
                                    cx.notify();
                                })),
                        )
                        .when(can_edit, |menu| {
                            menu.child(
                                row("mobile-menu-edit", "edit", crate::i18n::text("menu.edit"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.mobile_viewer_menu_open = false;
                                        this.set_phone_editing(true, window, cx);
                                    })),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    /// Floating bottom pill in phone viewing mode (position, fit-to-width, full-screen, and edit).
    pub(crate) fn render_mobile_viewer_pill(&self, narrow_phone: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.phone_view_only() || self.is_phone_editing() || !self.has_document {
            return None;
        }
        let theme = cx.theme();
        if self.mobile_hud_hidden {
            return Some(
                div()
                    .id("mobile-show-hud")
                    .absolute()
                    .right(px(12.))
                    .bottom(px(16.))
                    .size(px(40.))
                    .rounded_full()
                    .bg(theme.popover.opacity(0.92))
                    .border_1()
                    .border_color(theme.border)
                    .shadow_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(theme.popover_foreground)
                    .child(Icon::new(Material("visibility")).size(px(18.)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.mobile_hud_hidden = false;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }

        let position = self.mobile_position_label();
        let zoom_label: SharedString = format!("{:.0}%", self.current_zoom() * 100.0).into();
        let can_edit = narrow_phone && !self.file_is_read_only() && self.host.read_only.is_none();

        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(14.))
                .flex()
                .justify_center()
                .child(
                    h_flex()
                        .id("mobile-viewer-pill")
                        .h(px(42.))
                        .px_2()
                        .gap_1()
                        .items_center()
                        .rounded_full()
                        .bg(theme.popover.opacity(0.95))
                        .border_1()
                        .border_color(theme.border)
                        .shadow_lg()
                        .child(
                            div()
                                .px_2()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.muted_foreground)
                                .child(position),
                        )
                        .child(div().w(px(1.)).h(px(18.)).bg(theme.border))
                        .child(
                            h_flex()
                                .id("mobile-zoom-fit")
                                .h(px(34.))
                                .px_2()
                                .gap_1()
                                .items_center()
                                .rounded_full()
                                .cursor_pointer()
                                .text_xs()
                                .text_color(theme.popover_foreground)
                                .hover(|s| s.bg(theme.list_hover))
                                .child(Icon::new(Material("fit_screen")).size(px(16.)))
                                .child(zoom_label)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_zoom(None, window, cx);
                                })),
                        )
                        .child(
                            div()
                                .id("mobile-hide-hud")
                                .size(px(34.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .cursor_pointer()
                                .text_color(theme.popover_foreground)
                                .hover(|s| s.bg(theme.list_hover))
                                .child(Icon::new(Material("visibility_off")).size(px(16.)))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.mobile_hud_hidden = true;
                                    this.mobile_viewer_menu_open = false;
                                    cx.notify();
                                })),
                        )
                        .when(can_edit, |pill| {
                            pill.child(div().w(px(1.)).h(px(18.)).bg(theme.border)).child(
                                h_flex()
                                    .id("mobile-pill-edit")
                                    .h(px(34.))
                                    .px_2p5()
                                    .gap_1()
                                    .items_center()
                                    .rounded_full()
                                    .cursor_pointer()
                                    .bg(theme.accent)
                                    .text_color(theme.accent_foreground)
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(Icon::new(Material("edit")).size(px(14.)))
                                    .child(crate::i18n::text("menu.edit"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.mobile_viewer_menu_open = false;
                                        this.set_phone_editing(true, window, cx);
                                    })),
                            )
                        }),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn finger_drag_is_disabled_in_viewing_mode() {
        let should_drag = |is_touch: bool, phone_view_only: bool, modal: bool, in_bounds: bool, in_popup: bool| {
            is_touch && !phone_view_only && !modal && in_bounds && !in_popup
        };
        assert!(should_drag(true, false, false, true, false), "editing mode claims one-finger page drag");
        assert!(!should_drag(true, true, false, true, false), "viewing mode leaves one-finger drag to scroll");
        assert!(!should_drag(false, false, false, true, false), "non-touch device never claims touch drag");
    }
}

