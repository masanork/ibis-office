// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop entry point for reviewing named-cell transfers.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::{Disableable as _, Sizable as _, checkbox::Checkbox, h_flex};
use gpui_kit::*;
use serde_json::Value;

use crate::ext::*;
use crate::prefill_copy::{ReviewInput, is_xlsx, read_text, save_reviewed_copy_with_options};
use crate::shell::chrome::SidebarPanel;
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::text;

pub(crate) struct PrefillReview {
    input: Arc<ReviewInput>,
    selected: BTreeSet<String>,
    allow_overwrite: bool,
    saving: bool,
    message: Option<String>,
}

fn can_select(row: &Value, allow_overwrite: bool) -> bool {
    let status_ok = row["status"] == "ready" || (allow_overwrite && row["status"] == "occupied");
    status_ok
        && row["proposed_value"]
            .as_str()
            .is_none_or(|value| value.chars().count() <= 250)
}

fn prompt(extensions: &[&str], message: &str) -> PathPromptOptions {
    PathPromptOptions {
        files: true,
        directories: false,
        extensions: extensions.iter().map(|ext| ext.to_string()).collect(),
        multiple: false,
        prompt: Some(text(message).into()),
    }
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
                    let source_text = read_text(&source)?;
                    let mapping_text = read_text(&mapping)?;
                    let settings: Value = serde_json::from_str(&mapping_text)
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
                    let report = ibis_doc_service::review_prefill(
                        &presentation,
                        slide,
                        signed,
                        &format,
                        &source_text,
                        &settings["mappings"],
                    )
                    .map_err(|error| error.message)?;
                    Ok(PrefillReview {
                        input: Arc::new(ReviewInput {
                            source,
                            mapping,
                            source_text,
                            mapping_text,
                            format,
                            report,
                        }),
                        selected: BTreeSet::new(),
                        allow_overwrite: false,
                        saving: false,
                        message: None,
                    })
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

    fn can_apply_prefill_to_sheet(&self) -> bool {
        !self.is_read_only() && self.protection_password.is_none()
    }

    fn can_save_prefill_copy(&self) -> bool {
        !self.is_read_only()
            && self.protection_password.is_none()
            && self.file_path().is_some_and(|path| is_xlsx(&path))
            && self.source.is_some()
    }

    fn apply_prefill_to_sheet(&mut self, cx: &mut Context<Self>) {
        if !self.can_apply_prefill_to_sheet() {
            if let Some(Ok(review)) = self.wb_mut().panels.prefill_preview.as_mut() {
                review.message = Some(text("xlsx.prefill.apply-unavailable").to_string());
            }
            cx.notify();
            return;
        }
        self.commit_cell_edit(cx);
        let slide = self.current_slide;
        let Some((input, targets, allow_overwrite)) = self
            .sheet_state()
            .and_then(|state| state.panels.prefill_preview.as_ref())
            .and_then(|res| res.as_ref().ok())
            .filter(|review| !review.selected.is_empty() && !review.saving)
            .map(|review| {
                (
                    review.input.clone(),
                    review.selected.clone(),
                    review.allow_overwrite,
                )
            })
        else {
            return;
        };

        let settings: Value = match serde_json::from_str(&input.mapping_text) {
            Ok(val) => val,
            Err(_) => {
                if let Some(Ok(review)) = self.wb_mut().panels.prefill_preview.as_mut() {
                    review.message = Some(text("xlsx.prefill.changed").to_string());
                }
                cx.notify();
                return;
            }
        };
        let signed = self
            .source
            .as_ref()
            .is_some_and(|source| source.has_package_signatures().unwrap_or(true));
        let current = match ibis_doc_service::review_prefill(
            self.presentation(),
            slide,
            signed,
            &input.format,
            &input.source_text,
            &settings["mappings"],
        ) {
            Ok(report) if report == input.report => report,
            _ => {
                if let Some(Ok(review)) = self.wb_mut().panels.prefill_preview.as_mut() {
                    review.message = Some(text("xlsx.prefill.changed").to_string());
                }
                cx.notify();
                return;
            }
        };

        let command = match ibis_doc_service::reviewed_prefill_command_with_options(
            &current,
            slide,
            &serde_json::json!(targets),
            allow_overwrite,
        ) {
            Ok(cmd) => cmd,
            Err(error) => {
                if let Some(Ok(review)) = self.wb_mut().panels.prefill_preview.as_mut() {
                    review.message = Some(error.message);
                }
                cx.notify();
                return;
            }
        };

        let applied_count = targets.len();
        self.run_sheet(vec![command], cx);

        let updated_report = ibis_doc_service::review_prefill(
            self.presentation(),
            slide,
            signed,
            &input.format,
            &input.source_text,
            &settings["mappings"],
        )
        .unwrap_or(current);

        let count_str = applied_count.to_string();
        let msg = crate::shell::i18n::format("xlsx.prefill.applied", &[("count", &count_str)]);
        if let Some(Ok(review)) = self.wb_mut().panels.prefill_preview.as_mut() {
            review.input = Arc::new(ReviewInput {
                source: input.source.clone(),
                mapping: input.mapping.clone(),
                source_text: input.source_text.clone(),
                mapping_text: input.mapping_text.clone(),
                format: input.format.clone(),
                report: updated_report,
            });
            review.selected.clear();
            review.message = Some(msg);
        }
        cx.notify();
    }

    fn save_prefill_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_save_prefill_copy() {
            return;
        }
        let Some(workbook) = self.file_path().filter(|path| is_xlsx(path)) else {
            return;
        };
        let Some(directory) = workbook.parent() else {
            return;
        };
        let context = (self.file_id, self.current_slide);
        let Some(Ok(review)) = self.wb_mut().panels.prefill_preview.as_mut() else {
            return;
        };
        if review.selected.is_empty() || review.saving {
            return;
        }
        review.saving = true;
        review.message = None;
        let input = review.input.clone();
        let targets = review.selected.clone();
        let allow_overwrite = review.allow_overwrite;
        let stem = workbook.file_stem().unwrap_or_default().to_string_lossy();
        let picked = cx.prompt_for_new_path(directory, Some(&format!("{stem}-filled.xlsx")));
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = match picked.await {
                Ok(Ok(Some(output))) => {
                    let Ok(Some(snapshot)) = this.update_in(cx, |this, _, _| {
                        let review = this
                            .sheet_state()?
                            .panels
                            .prefill_preview
                            .as_ref()?
                            .as_ref()
                            .ok()?;
                        if context != (this.file_id, this.current_slide)
                            || !Arc::ptr_eq(&input, &review.input)
                            || this.file_path().as_ref() != Some(&workbook)
                            || !this.can_save_prefill_copy()
                        {
                            return None;
                        }
                        Some((this.presentation().clone(), this.source.clone()))
                    }) else {
                        return;
                    };
                    let input_for_save = input.clone();
                    let name = output.display().to_string();
                    cx.background_executor()
                        .spawn(async move {
                            let (presentation, package) = snapshot;
                            let package = package
                                .ok_or_else(|| text("xlsx.prefill.save-unavailable").to_string())?;
                            save_reviewed_copy_with_options(
                                &input_for_save,
                                presentation,
                                &package,
                                context.1,
                                &workbook,
                                &output,
                                &targets,
                                allow_overwrite,
                            )?;
                            Ok::<_, String>(Some(name))
                        })
                        .await
                }
                Ok(Ok(None)) => Ok(None),
                _ => Err(text("xlsx.prefill.save-unavailable").to_string()),
            };
            this.update_in(cx, |this, _, cx| {
                if context != (this.file_id, this.current_slide) {
                    return;
                }
                let Some(Ok(review)) = this.wb_mut().panels.prefill_preview.as_mut() else {
                    return;
                };
                if !Arc::ptr_eq(&input, &review.input) {
                    return;
                }
                review.saving = false;
                review.message = match result {
                    Ok(Some(path)) => Some(crate::shell::i18n::format(
                        "xlsx.prefill.saved",
                        &[("path", &path)],
                    )),
                    Ok(None) => None,
                    Err(error) => Some(error),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn prefill_review_panel(&self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        let result = self.sheet_state()?.panels.prefill_preview.as_ref()?;
        let mut body = div().flex().flex_col().gap(px(10.)).text_sm();
        match result {
            Err(error) => body = body.child(error.clone()),
            Ok(review) => {
                let report = &review.input.report;
                let can_apply = self.can_apply_prefill_to_sheet();
                let can_save = self.can_save_prefill_copy();
                let can_edit = can_apply || can_save;
                let allow_overwrite = review.allow_overwrite;
                body = body.child(text("xlsx.prefill.read-only"));
                if report["credential_trust"] == "not_checked" {
                    body = body.child(text("xlsx.prefill.unverified"));
                }
                if can_edit {
                    let has_occupied = report["changes"]
                        .as_array()
                        .is_some_and(|rows| rows.iter().any(|r| r["status"] == "occupied"));
                    let mut controls = h_flex().gap_2().flex_wrap().child(
                        crate::shell::chrome::action_button("prefill-select-all")
                            .small()
                            .label(text("xlsx.prefill.select-all"))
                            .disabled(review.saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(Ok(review)) =
                                    this.wb_mut().panels.prefill_preview.as_mut()
                                {
                                    let overwrite = review.allow_overwrite;
                                    if let Some(changes) =
                                        review.input.report["changes"].as_array()
                                    {
                                        for row in changes {
                                            if can_select(row, overwrite)
                                                && let Some(target) = row["target"].as_str()
                                            {
                                                review.selected.insert(target.to_string());
                                            }
                                        }
                                    }
                                }
                                cx.notify();
                            })),
                    );
                    if has_occupied {
                        controls = controls.child(
                            Checkbox::new("prefill-allow-overwrite")
                                .label(text("xlsx.prefill.allow-overwrite"))
                                .checked(allow_overwrite)
                                .disabled(review.saving)
                                .on_change(cx.listener(|this, checked, _, cx| {
                                    if let Some(Ok(review)) =
                                        this.wb_mut().panels.prefill_preview.as_mut()
                                    {
                                        review.allow_overwrite = *checked;
                                        if !*checked
                                            && let Some(changes) =
                                                review.input.report["changes"].as_array()
                                        {
                                            for row in changes {
                                                if row["status"] == "occupied"
                                                    && let Some(target) = row["target"].as_str()
                                                {
                                                    review.selected.remove(target);
                                                }
                                            }
                                        }
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                    body = body.child(controls);
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
                        let status_selectable = row["status"] == "ready"
                            || (allow_overwrite && row["status"] == "occupied");
                        if can_edit && can_select(row, allow_overwrite) {
                            let target = target.to_string();
                            item = item.child(
                                Checkbox::new(SharedString::from(format!("prefill-{cell}")))
                                    .label(text("xlsx.prefill.choose"))
                                    .checked(review.selected.contains(&target))
                                    .disabled(review.saving)
                                    .on_change(cx.listener(move |this, checked, _, cx| {
                                        if let Some(Ok(review)) =
                                            this.wb_mut().panels.prefill_preview.as_mut()
                                        {
                                            if *checked {
                                                review.selected.insert(target.clone());
                                            } else {
                                                review.selected.remove(&target);
                                            }
                                        }
                                        cx.notify();
                                    })),
                            );
                        } else if status_selectable && !can_select(row, allow_overwrite) {
                            item = item.child(text("xlsx.prefill.long-value"));
                        }
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
                if let Some(message) = &review.message {
                    body = body.child(message.clone());
                }
                if can_edit {
                    let mut buttons = h_flex().gap_2().flex_wrap();
                    if can_apply {
                        buttons = buttons.child(
                            crate::shell::chrome::action_button("prefill-apply-sheet")
                                .primary()
                                .label(text("xlsx.prefill.apply-sheet"))
                                .disabled(review.selected.is_empty() || review.saving)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.apply_prefill_to_sheet(cx)
                                })),
                        );
                    }
                    if can_save {
                        buttons = buttons.child(
                            crate::shell::chrome::action_button("prefill-save")
                                .label(text("xlsx.prefill.save-copy"))
                                .disabled(review.selected.is_empty() || review.saving)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_prefill_copy(window, cx)
                                })),
                        );
                    }
                    body = body.child(buttons);
                }
            }
        }
        Some(SidebarPanel {
            title: text("xlsx.prefill.title").into(),
            body: body.into_any_element(),
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_prefill_caption_is_in_both_languages() {
        for table in [
            include_str!("../../ibis-office-base/i18n/ja/xlsx.tsv"),
            include_str!("../../ibis-office-base/i18n/en/xlsx.tsv"),
        ] {
            for id in [
                "xlsx.prefill.title",
                "xlsx.prefill.allow-overwrite",
                "xlsx.prefill.select-all",
                "xlsx.prefill.apply-sheet",
                "xlsx.prefill.applied",
                "xlsx.prefill.apply-unavailable",
                "xlsx.prefill.save-copy",
            ] {
                assert!(
                    table.lines().any(|l| l.split('\t').next() == Some(id)),
                    "{id} is missing"
                );
            }
        }
    }
}
