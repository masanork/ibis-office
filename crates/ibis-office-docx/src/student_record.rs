// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use gpui_kit::component::v_flex;
use gpui_kit::*;
use ibis_doc_model::FlowPos;

use crate::ext::*;
use crate::shell::chrome::SidebarPanel;
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::text;

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

    pub(crate) fn student_record_panel(&self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        if !self.sidebar_open || !self.doc_state()?.student_record_open {
            return None;
        }

        let mut body = v_flex().w_full().min_w_0().p_3().gap_3();
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
        } else {
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
