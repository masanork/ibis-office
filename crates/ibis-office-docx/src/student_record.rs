// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

#[cfg(not(target_family = "wasm"))]
use gpui_kit::component::Sizable as _;
use gpui_kit::component::v_flex;
use gpui_kit::*;
use ibis_doc_model::FlowPos;

use crate::ext::*;
use crate::shell::chrome::SidebarPanel;
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::text;

#[cfg(not(target_family = "wasm"))]
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(not(target_family = "wasm"))]
use ibis_doc_model::FlowDocument;
#[cfg(not(target_family = "wasm"))]
use ibis_formats::form_template::{self, Truncation};

#[cfg(not(target_family = "wasm"))]
#[derive(Default)]
pub(crate) struct FormCheckState {
    definition: Option<PathBuf>,
    record: Option<PathBuf>,
    result: Option<(Arc<FlowDocument>, Result<Vec<Truncation>, String>)>,
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy)]
enum FormInput {
    Definition,
    Record,
}

#[cfg(not(target_family = "wasm"))]
impl FormInput {
    fn label(self) -> &'static str {
        match self {
            Self::Definition => "docx.student-record.form-definition",
            Self::Record => "docx.student-record.form-data",
        }
    }
}

#[ibis_office_macros::editor_ext(StudentRecordExt)]
impl SlideEditor {
    pub(crate) fn open_student_record(&mut self, cx: &mut Context<Self>) {
        self.close_doc_panels();
        let Some(state) = self.doc_state_mut() else {
            return;
        };
        state.student_record_open = true;
        state.accessibility_panel = false;
        state.selection_pane = false;
        state.tab_edit = None;
        self.sidebar_open = true;
        cx.notify();
    }

    #[cfg(not(target_family = "wasm"))]
    fn pick_student_form_input(
        &mut self,
        input: FormInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.flow().cloned() else {
            return;
        };
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            extensions: vec!["json".into(), "yaml".into(), "yml".into()],
            multiple: false,
            prompt: Some(text(input.label())),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |editor, _, cx| {
                if !editor
                    .flow()
                    .is_some_and(|current| Arc::ptr_eq(current, &document))
                {
                    return;
                }
                let Some(state) = editor
                    .doc_state_mut()
                    .filter(|state| state.student_record_open)
                else {
                    return;
                };
                let check = &mut state.student_form_check;
                match input {
                    FormInput::Definition => check.definition = Some(path),
                    FormInput::Record => check.record = Some(path),
                }
                check.result = None;
                cx.notify();
            });
        })
        .detach();
    }

    #[cfg(not(target_family = "wasm"))]
    fn check_student_form(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.flow().cloned() else {
            return;
        };
        let Some(state) = self.doc_state() else {
            return;
        };
        let check = &state.student_form_check;
        let (Some(definition), Some(record)) = (&check.definition, &check.record) else {
            return;
        };
        let result = (|| {
            fn read(path: &Path) -> Result<String, String> {
                std::fs::read_to_string(path)
                    .map_err(|_| text("docx.student-record.form-read-error").to_string())
            }
            fn yaml(path: &Path) -> bool {
                matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("yaml" | "yml")
                )
            }
            let definition = form_template::parse_input(&read(definition)?, yaml(definition))
                .map_err(|_| text("docx.student-record.form-parse-error").to_string())?;
            let record = form_template::parse_input(&read(record)?, yaml(record))
                .map_err(|_| text("docx.student-record.form-parse-error").to_string())?;
            form_template::fill_with_diagnostics(&self.current_bytes()?, &definition, &record)
                .map(|(_, diagnostics)| diagnostics)
                .map_err(|error| error.to_string())
        })();
        if let Some(state) = self.doc_state_mut() {
            state.student_form_check.result = Some((document, result));
        }
        cx.notify();
    }

    #[cfg(not(target_family = "wasm"))]
    fn student_form_diagnostics(&self, cx: &mut Context<Self>) -> Option<Div> {
        use crate::shell::chrome::action_button;
        let document = self.flow()?;
        let positions = form_template::field_positions(document);
        if positions.is_empty() {
            return None;
        }
        let check = &self.doc_state()?.student_form_check;
        let mut body = v_flex().gap_2().child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(text("docx.student-record.form-preflight")),
        );
        for (index, (input, path)) in [
            (FormInput::Definition, &check.definition),
            (FormInput::Record, &check.record),
        ]
        .into_iter()
        .enumerate()
        {
            body = body.child(
                action_button(format!("student-form-input-{index}"))
                    .small()
                    .label(text(input.label()))
                    .on_click(cx.listener(move |editor, _, window, cx| {
                        editor.pick_student_form_input(input, window, cx)
                    })),
            );
            if let Some(name) = path.as_ref().and_then(|p| p.file_name()) {
                body = body.child(div().text_sm().child(name.to_string_lossy().into_owned()));
            }
        }
        if check.definition.is_some() && check.record.is_some() {
            body = body.child(
                action_button("student-form-check")
                    .small()
                    .label(text("docx.student-record.form-check"))
                    .on_click(cx.listener(|editor, _, _, cx| editor.check_student_form(cx))),
            );
        }
        if let Some((checked, result)) = &check.result {
            if !Arc::ptr_eq(checked, document) {
                body = body.child(
                    div()
                        .text_sm()
                        .child(text("docx.student-record.form-stale")),
                );
            } else {
                match result {
                    Err(error) => body = body.child(div().text_sm().child(error.clone())),
                    Ok(items) if items.is_empty() => {
                        body = body.child(
                            div()
                                .text_sm()
                                .child(text("docx.student-record.form-clear")),
                        )
                    }
                    Ok(items) => {
                        for (index, item) in items.iter().enumerate() {
                            let label = crate::shell::i18n::format(
                                "docx.student-record.form-warning",
                                &[
                                    ("field", &item.field_id),
                                    ("actual", &item.actual_chars.to_string()),
                                    ("max", &item.max_chars.to_string()),
                                ],
                            );
                            if let Some(&at) = positions.get(&item.field_id) {
                                body = body.child(
                                    action_button(format!("student-form-warning-{index}"))
                                        .small()
                                        .label(label)
                                        .on_click(cx.listener(move |editor, _, _, cx| {
                                            editor.set_caret(at, false, cx);
                                            editor.scroll_to_caret(cx);
                                        })),
                                );
                            } else {
                                body = body.child(div().text_sm().child(label));
                            }
                        }
                    }
                }
            }
        }
        Some(body.into())
    }

    pub(crate) fn student_record_panel(&self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        if !self.sidebar_open || !self.doc_state()?.student_record_open {
            return None;
        }

        let mut body = v_flex().w_full().min_w_0().p_3().gap_3();
        #[cfg(not(target_family = "wasm"))]
        let form = self.student_form_diagnostics(cx);
        #[cfg(target_family = "wasm")]
        let form: Option<Div> = None;
        let has_form = form.is_some();
        if let Some(form) = form {
            body = body.child(form);
        }
        if let Some(record) = self
            .flow()
            .map(AsRef::as_ref)
            .and_then(ibis_formats::student_record::extract)
        {
            body = body.child(
                div()
                    .text_sm()
                    .child(text("docx.student-record.instructions")),
            );
            for (index, (label, value)) in [
                ("docx.student-record.name", record.name),
                ("docx.student-record.birth-date", record.birth_date),
                ("docx.student-record.next-school", record.next_school),
            ]
            .into_iter()
            .enumerate()
            {
                let at = FlowPos::new(value.paragraph, 0);
                let mut lines = v_flex().gap_1().child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(text(label)),
                );
                if value.text.is_empty() {
                    lines = lines.child(div().text_sm().child(text("docx.student-record.blank")));
                } else {
                    for line in value.text.lines() {
                        lines = lines.child(div().text_sm().child(line.to_owned()));
                    }
                }
                body = body.child(
                    div()
                        .id(("student-record-field", index))
                        .p_2()
                        .cursor_pointer()
                        .child(lines)
                        .on_click(cx.listener(move |editor, _, _, cx| {
                            editor.set_caret(at, false, cx);
                            editor.scroll_to_caret(cx);
                        })),
                );
            }
        } else if !has_form {
            body = body.child(
                div()
                    .text_sm()
                    .child(text("docx.student-record.unsupported")),
            );
        }
        Some(SidebarPanel {
            title: text("docx.student-record.title"),
            body: body.into_any_element(),
        })
    }
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn scenario_form_preflight_pane() {
    use crate::shell::scenario::{Scenario, button_probe};

    const FIXTURE: &str = "test/fixtures/docx-form/full-school-record";
    let _probe = button_probe::Probe::new();
    let mut scenario = Scenario::open(&format!("{FIXTURE}/template.docx"));
    let record = scenario
        .screenshot_path("long-record")
        .with_extension("json");
    let definition = scenario.screenshot_path("form").with_extension("yaml");
    std::fs::write(
        &definition,
        include_str!("../../../test/fixtures/docx-form/full-school-record/form.yaml"),
    )
    .unwrap();
    let mut data = form_template::parse_input(
        include_str!("../../../test/fixtures/docx-form/full-school-record/record-b.json"),
        false,
    )
    .unwrap();
    data["data"]["grades"]["6"]["remarks"] = "図書委員会の活動を振り返った。".repeat(15).into();
    std::fs::write(&record, data.to_string()).unwrap();

    scenario.menu("docx.student-record.review");
    assert!(button_probe::bounds("student-form-input-0").is_some());
    assert!(button_probe::bounds("student-form-input-1").is_some());
    scenario.update(|editor, _, cx| {
        let check = &mut editor.doc_state_mut().unwrap().student_form_check;
        check.definition = Some(definition);
        check.record = Some(record);
        cx.notify();
    });
    button_probe::click(&mut scenario, "student-form-check");
    let position = scenario.with(|editor, _| {
        let state = &editor.doc_state().unwrap().student_form_check;
        let (_, result) = state.result.as_ref().expect("preflight result");
        assert_eq!(
            result.as_ref().unwrap(),
            &[Truncation {
                field_id: "grades.6.remarks".into(),
                actual_chars: 225,
                max_chars: 160,
            }]
        );
        form_template::field_positions(editor.flow().unwrap())["grades.6.remarks"]
    });
    button_probe::click(&mut scenario, "student-form-warning-0");
    assert_eq!(
        scenario.with(|editor, _| editor.doc_state().unwrap().caret),
        Some(position)
    );
    assert!(!scenario.with(|editor, _| editor.is_dirty()));
    scenario.screenshot("form-preflight-sixth-grade");
}
