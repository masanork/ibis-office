// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop entry point for reviewing named-cell transfers.

use std::io::Read;
use std::path::{Path, PathBuf};

use gpui_kit::*;
use serde_json::Value;

use crate::ext::*;
use crate::shell::chrome::SidebarPanel;
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::text;

fn prompt(extensions: &[&str], message: &str) -> PathPromptOptions {
    PathPromptOptions {
        files: true,
        directories: false,
        extensions: extensions.iter().map(|ext| ext.to_string()).collect(),
        multiple: false,
        prompt: Some(text(message).into()),
    }
}

fn read_text(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(1_048_577).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err(text("xlsx.prefill.too-large").to_string());
    }
    String::from_utf8(bytes).map_err(|_| text("xlsx.prefill.utf8").to_string())
}

#[ibis_office_macros::editor_ext(PrefillPanel)]
impl SlideEditor {
    pub(crate) fn open_prefill_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let context = (self.file_id, self.current_slide);
        let picked = cx.prompt_for_paths(prompt(
            &["json", "xml", "sd-jwt", "mdoc"],
            "xlsx.prefill.select-source",
        ));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(source) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                if context == (this.file_id, this.current_slide) {
                    this.pick_prefill_mapping(source, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn pick_prefill_mapping(
        &mut self,
        source: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = (self.file_id, self.current_slide);
        let picked = cx.prompt_for_paths(prompt(&["json"], "xlsx.prefill.select-mapping"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(mapping) = paths.into_iter().next() else {
                return;
            };
            let Ok(Some((presentation, package))) = this.update_in(cx, |this, _, _| {
                (context == (this.file_id, this.current_slide))
                    .then(|| (this.presentation().clone(), this.source.clone()))
            }) else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    let format = source
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    let source = read_text(&source)?;
                    let settings: Value = serde_json::from_str(&read_text(&mapping)?)
                        .map_err(|_| text("xlsx.prefill.invalid-mapping").to_string())?;
                    let slide = context.1;
                    let sheet = presentation
                        .slide(slide)
                        .and_then(|slide| slide.name.as_deref());
                    if settings["sheet"].as_str() != sheet || sheet.is_none() {
                        return Err(text("xlsx.prefill.wrong-sheet").to_string());
                    }
                    let signed = package
                        .as_ref()
                        .is_some_and(|source| source.has_package_signatures().unwrap_or(true));
                    ibis_doc_service::review_prefill(
                        &presentation,
                        slide,
                        signed,
                        &format,
                        &source,
                        &settings["mappings"],
                    )
                    .map_err(|error| error.message)
                })
                .await;
            this.update_in(cx, |this, _, cx| {
                if context != (this.file_id, this.current_slide) {
                    return;
                }
                this.wb_mut().close_settings();
                this.wb_mut().panels.prefill_preview = Some(result);
                this.sidebar_open = true;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn prefill_review_panel(&self) -> Option<SidebarPanel> {
        let result = self.sheet_state()?.panels.prefill_preview.as_ref()?;
        let mut body = div().flex().flex_col().gap(px(10.)).text_sm();
        match result {
            Err(error) => body = body.child(error.clone()),
            Ok(report) => {
                body = body.child(text("xlsx.prefill.read-only"));
                if report["credential_trust"] == "not_checked" {
                    body = body.child(text("xlsx.prefill.unverified"));
                }
                if let Some(changes) = report["changes"].as_array() {
                    for row in changes {
                        let target = row["target"].as_str().unwrap_or("");
                        let cell = row["cell"].as_str().unwrap_or("");
                        let label = row["target_label"].as_str().unwrap_or(target);
                        let status = match row["status"].as_str() {
                            Some("ready") => "xlsx.prefill.ready",
                            Some("manual") => "xlsx.prefill.manual",
                            Some("not_in_source") => "xlsx.prefill.missing",
                            Some("occupied") => "xlsx.prefill.occupied",
                            Some("blocked_target") => "xlsx.prefill.blocked",
                            _ => "xlsx.prefill.unsupported",
                        };
                        let display = |value: &Value| {
                            if value.is_null() {
                                text("xlsx.prefill.empty").to_string()
                            } else if let Some(value) = value.as_str() {
                                let mut chars = value.chars();
                                let mut short: String = chars.by_ref().take(250).collect();
                                if chars.next().is_some() {
                                    short.push_str(" …");
                                }
                                short
                            } else {
                                value.to_string()
                            }
                        };
                        let mut item = div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(format!("{label}  ({cell})"))
                            .child(text(status));
                        if label != target {
                            item = item.child(div().text_xs().child(target.to_string()));
                        }
                        if let Some(path) = row["source_path"].as_str() {
                            item = item.child(format!("{} {path}", text("xlsx.prefill.source")));
                        }
                        item = item.child(format!(
                            "{} {}",
                            text("xlsx.prefill.current"),
                            display(&row["current_value"])
                        ));
                        if !row["proposed_value"].is_null() {
                            item = item.child(format!(
                                "{} {}",
                                text("xlsx.prefill.proposed"),
                                display(&row["proposed_value"])
                            ));
                        }
                        body = body.child(item);
                    }
                }
            }
        }
        Some(SidebarPanel {
            title: text("xlsx.prefill.title").into(),
            body: body.into_any_element(),
        })
    }
}
