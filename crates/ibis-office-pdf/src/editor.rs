// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! [`FormatEditor`] implementation for `Kind::Pdf`.

use std::sync::{Arc, Mutex};

use gpui_kit::component::scroll::{ScrollableElement as _, ScrollbarAxis};
use gpui_kit::{
    AnyElement, Bounds, Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    SharedString, Size, StatefulInteractiveElement as _, Styled, Window, canvas, div, fill, hsla,
    point, px, rgb, size,
};
use ibis_doc_model::{Presentation, SlideId};
use ibis_office_shell::chrome::MenuId;
use ibis_office_shell::commands::KindMask;
use ibis_office_shell::editor::{
    SlideEditor, WorkspaceContent, bounds_observer, wheel_zoom_listener,
};
use ibis_office_shell::format::{
    FormatEditor, FormatInfo, OfficeApp, ShapeRules, impl_format_state,
};
use ibis_office_shell::menu_context::{ContextMenu, ContextState, ContextTone};
use ibis_office_shell::menu_layout::DOCUMENT_MENUS;

use crate::PdfViewerState;

/// PDF document format driver.
pub struct PdfDocument;

/// Per-window PDF viewer state held in `SlideEditor::format`.
pub struct State {
    pub viewer: Arc<Mutex<PdfViewerState>>,
}

impl_format_state!(State, ibis_formats::Kind::Pdf);

const PAGE_GAP_PX: f32 = 24.0;
const PAGE_PAD_PX: f32 = 32.0;

fn page_scale(editor: &SlideEditor) -> f32 {
    editor.current_zoom().max(0.1) * (96.0 / 72.0)
}

fn outer_contexts(_state: &ContextState) -> Vec<ContextMenu> {
    Vec::new()
}

const fn empty_menus() -> &'static [ibis_office_shell::sidebars::SidebarDef] {
    &[]
}

impl FormatEditor for PdfDocument {
    const INFO: &'static FormatInfo = &FormatInfo {
        order: 3,
        extension: "pdf",
        new_file_name: || "Document.pdf".into(),
        untitled_key: "_shell.document-1-docx",
        office: OfficeApp {
            name: "PDF",
            setting_key: "office_path.pdf",
            #[cfg(target_os = "macos")]
            bundle: "Preview.app",
            #[cfg(target_os = "windows")]
            exe: "msedge.exe",
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            open_key: "window.open-in-word",
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            logo_svg: b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'/>",
        },
        new_command: "file.new.document",
        command_menus: &[
            MenuId::File,
            MenuId::Edit,
            MenuId::View,
            MenuId::Window,
            MenuId::Help,
        ],
        icon: "description",
        color: 0xc93b32,
        context_tones: [
            ContextTone {
                fg: 0x9c2821,
                bg: 0xc93b32,
                bg_alpha: 0.12,
            },
            ContextTone {
                fg: 0xf48b84,
                bg: 0xf48b84,
                bg_alpha: 0.16,
            },
        ],
        menus: &DOCUMENT_MENUS,
        fold_order: &[],
        sidebars: empty_menus(),
        mask: KindMask::PDF,
        several_windows: None,
        shapes: ShapeRules {
            in_shell: false,
            several: false,
            try_first: false,
            cmyk: false,
            break_is_typed: false,
        },
        outer_contexts,
        font_substitutes: false,
        drawing_canvas: false,
        new_badge_color: 0xc93b32,
        search_history_key: "menu-search.history.pdf",
        px_per_inch_at_100: 96.0,
        #[cfg(not(target_family = "wasm"))]
        save_formats: &[("file.save-format.pdf", "pdf")],
    };

    const PAGE_NOUN: &'static str = "docx.mod.page";
    const FIT_TOOLTIP: &'static str = "status.fit-page";
    const SHAPES: bool = false;

    fn new_state(
        presentation: &Presentation,
        _window: &mut Window,
        _cx: &mut Context<SlideEditor>,
    ) -> Box<dyn ibis_office_shell::format::FormatState> {
        let viewer = presentation
            .pdf_bytes
            .as_ref()
            .filter(|bytes| !bytes.is_empty())
            .and_then(|bytes| PdfViewerState::open(bytes.clone()).ok())
            .unwrap_or_else(PdfViewerState::empty);
        Box::new(State {
            viewer: Arc::new(Mutex::new(viewer)),
        })
    }

    fn workspace_content(editor: &SlideEditor, workspace: Size<Pixels>) -> WorkspaceContent {
        let scale = page_scale(editor);
        let Some(state) = editor.format_state::<State>() else {
            return WorkspaceContent {
                size: workspace,
                slide_origin: point(px(0.0), px(0.0)),
                scale: 1.0,
            };
        };
        let guard = state.viewer.lock().expect("pdf viewer lock");
        let mut max_w = 0.0f32;
        let mut total_h = PAGE_PAD_PX;
        for page in &guard.pages {
            max_w = max_w.max(page.crop_box.width() * scale);
            total_h += page.crop_box.height() * scale + PAGE_GAP_PX;
        }
        total_h += PAGE_PAD_PX;
        let content_w = (max_w + PAGE_PAD_PX * 2.0).max(f32::from(workspace.width));
        let content_h = total_h.max(f32::from(workspace.height));
        let origin_x = ((content_w - max_w) * 0.5).max(PAGE_PAD_PX);
        WorkspaceContent {
            size: size(px(content_w), px(content_h)),
            slide_origin: point(px(origin_x), px(PAGE_PAD_PX)),
            scale,
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn save_refused(editor: &SlideEditor) -> Option<String> {
        editor
            .presentation()
            .pdf_bytes
            .as_deref()
            .filter(|bytes| ibis_xdw::is_xdw(bytes))
            .map(|_| ibis_office_shell::i18n::text("xdw.read-only").to_string())
    }

    fn status_items(editor: &SlideEditor) -> Vec<SharedString> {
        let Some(state) = editor.format_state::<State>() else {
            return Vec::new();
        };
        let guard = state.viewer.lock().expect("pdf viewer lock");
        if guard.is_xdw() {
            return vec![ibis_office_shell::i18n::text("xdw.preview")];
        }
        guard
            .signature_status_text()
            .map(SharedString::from)
            .into_iter()
            .collect()
    }

    fn show_page(
        editor: &mut SlideEditor,
        slide: SlideId,
        _: &mut Window,
        _: &mut Context<SlideEditor>,
    ) -> bool {
        let Some(index) = editor.presentation().slide_index(slide) else {
            return false;
        };
        let Some(state) = editor.format_state::<State>() else {
            return false;
        };
        let scale = page_scale(editor);
        let top = state
            .viewer
            .lock()
            .expect("pdf viewer lock")
            .pages
            .iter()
            .take(index)
            .map(|page| page.crop_box.height() * scale + PAGE_GAP_PX)
            .sum::<f32>();
        let x = editor.workspace_scroll.offset().x;
        editor.workspace_scroll.set_offset(point(x, px(-top)));
        editor.clamp_workspace_scroll();
        editor.current_slide = slide;
        true
    }

    fn before_render(editor: &mut SlideEditor, _: &mut Window, _: &mut Context<SlideEditor>) {
        let Some(workspace) = editor.workspace_bounds else {
            return;
        };
        let Some(state) = editor.format_state::<State>() else {
            return;
        };
        let scale = page_scale(editor);
        let center = f32::from(workspace.size.height * 0.5 - editor.workspace_scroll.offset().y);
        let index = {
            let viewer = state.viewer.lock().expect("pdf viewer lock");
            let mut bottom = PAGE_PAD_PX;
            viewer
                .pages
                .iter()
                .take(viewer.pages.len().saturating_sub(1))
                .take_while(|page| {
                    bottom += page.crop_box.height() * scale + PAGE_GAP_PX;
                    bottom <= center
                })
                .count()
        };
        if let Some(slide) = editor.presentation().slides.get(index) {
            editor.current_slide = slide.id;
        }
    }

    fn render_body(
        editor: &mut SlideEditor,
        _window: &mut Window,
        cx: &mut Context<SlideEditor>,
    ) -> AnyElement {
        let Some(state) = editor.format_state::<State>() else {
            return div().size_full().into_any_element();
        };
        let viewer = state.viewer.clone();
        let scale = page_scale(editor);
        let workspace = editor
            .workspace_bounds
            .map(|b| b.size)
            .unwrap_or(size(px(1200.0), px(800.0)));
        let content = Self::workspace_content(editor, workspace);
        let entity = cx.entity().downgrade();

        div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(rgb(0xf2f3f5))
            .child(
                div()
                    .id("pdf-workspace-scroll")
                    .size_full()
                    .overflow_scroll()
                    .track_scroll(&editor.workspace_scroll)
                    .on_scroll_wheel(cx.listener(SlideEditor::on_workspace_scroll))
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .w(content.size.width)
                            .h(content.size.height)
                            .child(
                                canvas(
                                    |_bounds, _window, _cx| {},
                                    move |bounds: Bounds<Pixels>, _, window: &mut Window, cx| {
                                        let mut guard = viewer.lock().expect("pdf viewer lock");
                                        let page_count = guard.pages.len();
                                        let mut y = f32::from(bounds.origin.y) + PAGE_PAD_PX;
                                        let view_h = f32::from(window.viewport_size().height);

                                        for idx in 0..page_count {
                                            let (pw, ph) = {
                                                let p = &guard.pages[idx];
                                                (
                                                    p.crop_box.width() * scale,
                                                    p.crop_box.height() * scale,
                                                )
                                            };
                                            let page_bottom = y + ph;
                                            if page_bottom >= -100.0 && y <= view_h + 100.0 {
                                                let x = f32::from(bounds.origin.x)
                                                    + ((f32::from(bounds.size.width) - pw) * 0.5)
                                                        .max(PAGE_PAD_PX);
                                                let page_bounds = Bounds {
                                                    origin: point(px(x), px(y)),
                                                    size: size(px(pw), px(ph)),
                                                };
                                                window.paint_quad(
                                                    fill(page_bounds, rgb(0xffffff))
                                                        .border_widths(px(1.0))
                                                        .border_color(hsla(0.0, 0.0, 0.0, 0.14)),
                                                );
                                                let _ = guard.paint_page(
                                                    idx,
                                                    point(px(x), px(y)),
                                                    scale,
                                                    window,
                                                    cx,
                                                );
                                            }
                                            y += ph + PAGE_GAP_PX;
                                        }
                                    },
                                )
                                .size_full(),
                            ),
                    ),
            )
            .child(wheel_zoom_listener(cx))
            .child(bounds_observer(move |bounds, window, cx| {
                let changed = entity.update(cx, |editor, cx| {
                    if editor.workspace_bounds == Some(bounds) {
                        return false;
                    }
                    editor.workspace_bounds = Some(bounds);
                    editor.clamp_workspace_scroll();
                    cx.notify();
                    true
                });
                if matches!(changed, Ok(true)) {
                    window.on_next_frame(|window, _| window.refresh());
                }
            }))
            .scrollbar(&editor.workspace_scroll, ScrollbarAxis::Both)
            .into_any_element()
    }
}
