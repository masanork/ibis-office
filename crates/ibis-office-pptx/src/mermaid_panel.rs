// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use gpui_kit::component::{input::TextareaState, v_flex};
use gpui_kit::*;
use ibis_doc_model::{
    Command, EmuRect, MediaFormat, MediaId, Picture, ShapeId, ShapeKind, SlideId,
};
use ibis_formats::mermaid::{MermaidOutput, render_mermaid};

use crate::ext::*;
use crate::shell::{chrome, editor::SlideEditor, i18n::text};

pub(crate) struct MermaidEdit {
    slide: SlideId,
    shape: ShapeId,
    media: MediaId,
    original: String,
    pub(crate) input: Entity<TextareaState>,
    error: Option<&'static str>,
}

#[ibis_office_macros::editor_ext(MermaidPanel)]
impl SlideEditor {
    fn mermaid_picture(&self, slide: SlideId, shape: ShapeId) -> Option<(&Picture, EmuRect, &str)> {
        let slide = self.presentation().slide(slide)?;
        let object = slide.shape(shape)?;
        let ShapeKind::Picture(picture) = &object.kind else {
            return None;
        };
        let alt = slide
            .shape_alt_texts
            .get(&shape)
            .unwrap_or(&picture.alt_text);
        if alt.title != "__mermaid__"
            || self.presentation().media(picture.media)?.format != MediaFormat::Svg
        {
            return None;
        }
        Some((picture, object.frame, &alt.description))
    }

    pub(crate) fn selected_mermaid_picture(&self) -> Option<(SlideId, ShapeId)> {
        let shapes = self.selected_shapes();
        let [shape] = shapes.as_slice() else {
            return None;
        };
        let slide = self.slide().id;
        self.mermaid_picture(slide, shape.id)?;
        Some((slide, shape.id))
    }

    pub(crate) fn open_pptx_mermaid_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_pptx_mark_as_final() || self.is_pptx_read_only_blocked() {
            return;
        }
        let Some((slide, shape)) = self.selected_mermaid_picture() else {
            return;
        };
        let Some((picture, _, source)) = self.mermaid_picture(slide, shape) else {
            return;
        };
        let media = picture.media;
        let original = source.to_string();
        let input = cx.new(|cx| {
            let mut input = TextareaState::new(window, cx).auto_grow(8, 16);
            input.set_value(original.clone(), window, cx);
            input.focus(window, cx);
            input
        });
        if let Some(state) = self.pptx_mut() {
            state.mermaid_edit = Some(MermaidEdit {
                slide,
                shape,
                media,
                original,
                input,
                error: None,
            });
            self.sidebar_open = true;
        }
        cx.notify();
    }

    pub(crate) fn apply_pptx_mermaid_source(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(target_family = "wasm")]
        if let Some(edit) = self.pptx().and_then(|state| state.mermaid_edit.as_ref()) {
            let source = edit.input.read(cx).value().to_string();
            if source != edit.original
                && let Some(MermaidOutput::Svg { bytes, .. }) = render_mermaid(&source)
            {
                let input = edit.input.clone();
                cx.spawn_in(window, async move |this, cx| {
                    let result = crate::shell::web::preload_svg_fonts(bytes).await;
                    let _ = this.update_in(cx, |editor, window, cx| {
                        let Some(edit) = editor.pptx().and_then(|state| state.mermaid_edit.as_ref()) else {
                            return;
                        };
                        if edit.input.entity_id() != input.entity_id() || edit.input.read(cx).value().to_string() != source {
                            return;
                        }
                        if let Err(error) = result {
                            log::warn!("Mermaid font prefetch: {}", crate::shell::web::font_preload_error(error));
                            if let Some(edit) = editor.pptx_mut().and_then(|state| state.mermaid_edit.as_mut()) {
                                edit.error = Some("docx.picture.mermaid-source.rejected");
                            }
                            cx.notify();
                            return;
                        }
                        editor.apply_pptx_mermaid_source_ready(window, cx);
                    });
                }).detach();
                return;
            }
        }
        self.apply_pptx_mermaid_source_ready(window, cx);
    }

    fn apply_pptx_mermaid_source_ready(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.pptx().and_then(|state| state.mermaid_edit.as_ref()) else {
            return;
        };
        let (slide, shape) = (edit.slide, edit.shape);
        let source = edit.input.read(cx).value().to_string();
        let current = self
            .mermaid_picture(slide, shape)
            .filter(|(picture, _, original)| {
                picture.media == edit.media && *original == edit.original
            });
        let error = if let Some((picture, frame, original)) = current {
            if source == original {
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
                && frame.width > 0
                && frame.height > 0
            {
                let mut picture = picture.clone();
                // Keep the diagram inside its current frame and keep the rotation center.
                let scale = (frame.width as f64 / f64::from(width_px))
                    .min(frame.height as f64 / f64::from(height_px));
                let width = (f64::from(width_px) * scale).round().max(1.) as i64;
                let height = (f64::from(height_px) * scale).round().max(1.) as i64;
                let updated = EmuRect::new(
                    frame.x + (frame.width - width) / 2,
                    frame.y + (frame.height - height) / 2,
                    width,
                    height,
                );
                let media = self
                    .document
                    .ids()
                    .add_media(MediaFormat::Svg, bytes.clone());
                self.resources.add_picture(media, MediaFormat::Svg, &bytes);
                picture.media = media;
                let alt_text = ibis_doc_model::AltText {
                    title: "__mermaid__".to_string(),
                    description: source,
                };
                self.run(
                    vec![
                        Command::SetKind {
                            slide,
                            shape,
                            kind: Box::new(ShapeKind::Picture(picture)),
                        },
                        Command::SetShapeAltText {
                            slide,
                            shape,
                            alt_text,
                        },
                        Command::SetFrame {
                            slide,
                            shape,
                            frame: updated,
                        },
                    ],
                    cx,
                );
                if self
                    .mermaid_picture(slide, shape)
                    .is_some_and(|(picture, _, _)| picture.media == media)
                {
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
            .pptx_mut()
            .and_then(|state| state.mermaid_edit.as_mut())
        {
            edit.error = Some(error);
        }
        cx.notify();
    }

    pub(crate) fn pptx_mermaid_dialog(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<chrome::SidebarPanel> {
        let edit = self.pptx()?.mermaid_edit.as_ref()?;
        let body = v_flex()
            .gap_2()
            .p_3()
            .child(chrome::text_area_field(&edit.input))
            .children(edit.error.map(|key| div().child(text(key))));
        let footer =
            chrome::action_bar()
                .action(
                    chrome::ActionDefinition::new(
                        "mermaid-apply",
                        text("_shell.apply"),
                        chrome::ActionButtonKind::Primary,
                    )
                    .on_click(cx.listener(|editor, _, window, cx| {
                        editor.apply_pptx_mermaid_source(window, cx)
                    })),
                )
                .action(chrome::dialog_cancel_definition(cx));
        Some(chrome::SidebarPanel::with_footer(
            text("docx.picture.mermaid-source"),
            body.into_any_element(),
            footer,
        ))
    }
}
