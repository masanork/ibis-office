// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::sync::Arc;

use gpui_kit::component::{input::TextareaState, v_flex};
use gpui_kit::*;
use ibis_doc_model::{
    EmuSize, FlowCommand, FlowPos, FlowRunKind, MediaFormat, MediaId, PictureLook,
};
use ibis_pptx::mermaid::{MermaidOutput, render_mermaid};

use crate::ext::*;
use crate::shell::{chrome, editor::SlideEditor, i18n::text};

const MARKER: &str = "__mermaid__";

pub(crate) struct MermaidEdit {
    at: FlowPos,
    media: MediaId,
    original: String,
    pub(crate) input: Entity<TextareaState>,
    error: Option<&'static str>,
}

#[ibis_office_macros::editor_ext(MermaidPanel)]
impl SlideEditor {
    fn mermaid_picture_at(&self, at: FlowPos) -> Option<(MediaId, EmuSize, &PictureLook)> {
        let paragraph = self.paragraph_of(at.para)?;
        let mut offset = 0;
        for run in &paragraph.runs {
            if offset == at.offset
                && let FlowRunKind::Picture { media, size, look } = &run.kind
                && look.alt_text.title == MARKER
                && self
                    .presentation()
                    .media(*media)
                    .is_some_and(|item| item.format == MediaFormat::Svg)
            {
                return Some((*media, *size, look));
            }
            offset += run.len();
            if offset > at.offset {
                break;
            }
        }
        None
    }

    pub(crate) fn can_edit_mermaid_source(&self) -> bool {
        !self.is_jtd_read_only()
            && self
                .picture_at_caret()
                .is_some_and(|(at, _, _)| self.mermaid_picture_at(at).is_some())
    }

    pub(crate) fn open_mermaid_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_edit_mermaid_source() {
            return;
        }
        let Some((at, _, _)) = self.picture_at_caret() else {
            return;
        };
        let Some((media, _, look)) = self.mermaid_picture_at(at) else {
            return;
        };
        let original = look.alt_text.description.clone();
        let input = cx.new(|cx| {
            let mut input = TextareaState::new(window, cx).auto_grow(8, 16);
            input.set_value(original.clone(), window, cx);
            input.focus(window, cx);
            input
        });
        if let Some(state) = self.doc_state_mut() {
            state.mermaid_edit = Some(MermaidEdit {
                at,
                media,
                original,
                input,
                error: None,
            });
            self.sidebar_open = true;
        }
        cx.notify();
    }

    pub(crate) fn apply_mermaid_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_family = "wasm")]
        if let Some(edit) = self.doc_state().and_then(|state| state.mermaid_edit.as_ref()) {
            let source = edit.input.read(cx).value().to_string();
            if source != edit.original
                && let Some(MermaidOutput::Svg { bytes, .. }) = render_mermaid(&source)
            {
                let input = edit.input.clone();
                cx.spawn_in(window, async move |this, cx| {
                    let result = crate::shell::web::preload_svg_fonts(bytes).await;
                    let _ = this.update_in(cx, |editor, window, cx| {
                        let Some(edit) = editor.doc_state().and_then(|state| state.mermaid_edit.as_ref()) else {
                            return;
                        };
                        if edit.input.entity_id() != input.entity_id() || edit.input.read(cx).value().to_string() != source {
                            return;
                        }
                        if let Err(error) = result {
                            log::warn!("Mermaid font prefetch: {}", crate::shell::web::font_preload_error(error));
                            if let Some(edit) = editor.doc_state_mut().and_then(|state| state.mermaid_edit.as_mut()) {
                                edit.error = Some("docx.picture.mermaid-source.rejected");
                            }
                            cx.notify();
                            return;
                        }
                        editor.apply_mermaid_source_ready(window, cx);
                    });
                }).detach();
                return;
            }
        }
        self.apply_mermaid_source_ready(window, cx);
    }

    fn apply_mermaid_source_ready(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self
            .doc_state()
            .and_then(|state| state.mermaid_edit.as_ref())
        else {
            return;
        };
        let source = edit.input.read(cx).value().to_string();
        let at = edit.at;
        let current = self.mermaid_picture_at(at).filter(|(media, _, look)| {
            *media == edit.media && look.alt_text.description == edit.original
        });
        let error = if let Some((_, size, look)) = current {
            if source == edit.original {
                self.close_dialog(window, cx);
                return;
            }
            if let Some(MermaidOutput::Svg {
                bytes,
                width_px,
                height_px,
            }) = render_mermaid(&source)
                && width_px > 0
                && height_px > 0
            {
                let mut look = look.clone();
                look.alt_text.description = source;
                let height = size.width.saturating_mul(i64::from(height_px)) / i64::from(width_px);
                let size = EmuSize::new(size.width, height.max(1));
                let media = self
                    .document
                    .ids()
                    .add_media(MediaFormat::Svg, bytes.clone());
                self.resources
                    .images
                    .insert(media, Arc::new(Image::from_bytes(ImageFormat::Svg, bytes)));
                if self.run_flow(
                    vec![
                        FlowCommand::ReplacePicture {
                            at,
                            media,
                            size: Some(size),
                        },
                        FlowCommand::SetPictureLook { at, look },
                    ],
                    cx,
                ) {
                    self.close_dialog(window, cx);
                    return;
                }
                "docx.picture.mermaid-source.rejected"
            } else {
                "docx.picture.mermaid-source.invalid"
            }
        } else {
            "docx.picture.mermaid-source.stale"
        };
        if let Some(edit) = self
            .doc_state_mut()
            .and_then(|state| state.mermaid_edit.as_mut())
        {
            edit.error = Some(error);
        }
        cx.notify();
    }

    pub(crate) fn mermaid_dialog(&self, cx: &mut Context<Self>) -> Option<chrome::SidebarPanel> {
        let edit = self.doc_state()?.mermaid_edit.as_ref()?;
        let body = v_flex()
            .gap_2()
            .p_3()
            .child(chrome::text_area_field(&edit.input))
            .children(edit.error.map(|key| div().child(text(key))));
        let footer = chrome::action_bar()
            .action(
                chrome::ActionDefinition::new(
                    "mermaid-apply",
                    text("_shell.apply"),
                    chrome::ActionButtonKind::Primary,
                )
                .on_click(
                    cx.listener(|editor, _, window, cx| editor.apply_mermaid_source(window, cx)),
                ),
            )
            .action(chrome::dialog_cancel_definition(cx));
        Some(chrome::SidebarPanel::with_footer(
            text("docx.picture.mermaid-source"),
            body.into_any_element(),
            footer,
        ))
    }
}
