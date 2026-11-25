// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::sync::Arc;

use gpui_kit::component::{input::TextareaState, v_flex};
use gpui_kit::*;
use ibis_doc_model::{
    EmuSize, FlowCommand, FlowPos, FlowRunKind, MediaFormat, MediaId, PictureLook,
};
use ibis_formats::mermaid::{MermaidOutput, render_mermaid};

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
        let error = if current.is_none() {
            "docx.picture.mermaid-source.stale"
        } else {
            match self.replace_mermaid_picture(at, source, cx) {
                Ok(()) => {
                    self.close_dialog(window, cx);
                    return;
                }
                Err(error) => error,
            }
        };
        if let Some(edit) = self
            .doc_state_mut()
            .and_then(|state| state.mermaid_edit.as_mut())
        {
            edit.error = Some(error);
        }
        cx.notify();
    }

    pub(crate) fn ai_selected_mermaid_source(&self) -> Option<(FlowPos, MediaId, String)> {
        let (at, _, _) = self.picture_at_caret()?;
        let (media, _, look) = self.mermaid_picture_at(at)?;
        Some((at, media, look.alt_text.description.clone()))
    }

    pub(crate) fn replace_mermaid_picture(&mut self, at: FlowPos, source: String, cx: &mut Context<Self>) -> Result<(), &'static str> {
        let (_, size, look) = self.mermaid_picture_at(at).ok_or("docx.picture.mermaid-source.stale")?;
        if source == look.alt_text.description {
            return Ok(());
        }
        let Some(MermaidOutput::Svg { bytes, width_px, height_px }) = render_mermaid(&source)
            .filter(|output| matches!(output, MermaidOutput::Svg { width_px, height_px, .. } if *width_px > 0 && *height_px > 0))
        else {
            return Err("docx.picture.mermaid-source.invalid");
        };
        let mut look = look.clone();
        look.alt_text.description = source;
        let height = size.width.saturating_mul(i64::from(height_px)) / i64::from(width_px);
        let size = EmuSize::new(size.width, height.max(1));
        let media = self.document.ids().add_media(MediaFormat::Svg, bytes.clone());
        self.resources.images.insert(media, Arc::new(Image::from_bytes(ImageFormat::Svg, bytes)));
        if self.run_flow(vec![
            FlowCommand::ReplacePicture { at, media, size: Some(size) },
            FlowCommand::SetPictureLook { at, look },
        ], cx) {
            Ok(())
        } else {
            Err("docx.picture.mermaid-source.rejected")
        }
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
