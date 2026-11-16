// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! Review date candidates from selected text cells, without changing formulas.
use crate::ext::*;
use crate::shell::{
    chrome::{ActionButtonKind, ActionDefinition, SidebarPanel, action_bar},
    editor::SlideEditor,
    i18n::text,
};
use gpui_kit::component::v_flex;
use gpui_kit::*;
use ibis_doc_model::japanese_date_review::{DateReview as ReviewResult, review_date};
use ibis_doc_model::{
    CellEdit, CellInput, CellRange, CellValue, Command, Sheet, SheetCommand, SlideId, cell_name,
};

#[derive(Clone)]
struct Candidate {
    row: u32,
    column: u32,
    original: String,
    result: ReviewResult,
}
pub(crate) struct DateReviewPanel {
    slide: SlideId,
    candidates: Vec<Candidate>,
    message: Option<&'static str>,
}

fn candidates(sheet: &Sheet, range: CellRange) -> Vec<Candidate> {
    let mut out: Vec<_> = sheet
        .cells
        .iter()
        .filter_map(|c| {
            if c.row < range.first_row
                || c.row > range.last_row
                || c.column < range.first_column
                || c.column > range.last_column
                || c.formula.is_some()
                || c.runs.is_some()
                || c.phonetic.is_some()
            {
                return None;
            }
            let CellValue::Text(value) = &c.value else {
                return None;
            };
            if value.trim().is_empty() {
                return None;
            }
            Some(Candidate {
                row: c.row,
                column: c.column,
                original: value.clone(),
                result: review_date(value),
            })
        })
        .collect();
    out.sort_by_key(|c| (c.row, c.column));
    out
}

#[ibis_office_macros::editor_ext(DateReview)]
impl SlideEditor {
    pub fn open_sheet_date_review(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.commit_cell_edit(cx);
        let Some(range) = self.selected_range() else {
            return;
        };
        let Some((_, sheet)) = self.current_sheet() else {
            return;
        };
        let panel = DateReviewPanel {
            slide: self.current_slide,
            candidates: candidates(sheet, range),
            message: None,
        };
        let s = self.wb_mut();
        s.custom_format = None;
        s.page_setup = false;
        s.names = None;
        s.validation = None;
        s.find = None;
        s.link = None;
        s.conditional_edit = None;
        s.sort_edit = None;
        s.close_settings();
        s.panels.date_review = Some(panel);
        self.sidebar_open = true;
        cx.notify();
    }

    pub fn apply_sheet_date_review(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self
            .sheet_state()
            .and_then(|s| s.panels.date_review.as_ref())
        else {
            return;
        };
        let slide = panel.slide;
        let candidates = panel.candidates.clone();
        let Some((_, sheet)) = self.current_sheet() else {
            return;
        };
        let mut edits = Vec::new();
        let mut stale = slide != self.current_slide;
        for c in candidates {
            let result = match c.result {
                ReviewResult::Candidate(s) | ReviewResult::Inferred(s) => s,
                _ => continue,
            };
            if result == c.original {
                continue;
            }
            if !sheet.cell(c.row, c.column).is_some_and(|cell| {
                cell.formula.is_none()
                    && cell.runs.is_none()
                    && cell.phonetic.is_none()
                    && matches!(&cell.value, CellValue::Text(value) if value == &c.original)
            }) {
                stale = true;
                break;
            }
            edits.push(CellEdit {
                row: c.row,
                column: c.column,
                input: CellInput::Value(CellValue::Text(result)),
            });
        }
        let message = if self.is_read_only() || self.sheet_protection().is_some() {
            "xlsx.date-review.rejected"
        } else if stale {
            "xlsx.date-review.stale"
        } else if edits.is_empty() {
            "xlsx.date-review.no-changes"
        } else {
            match self
                .document
                .execute(vec![Command::Sheet(SheetCommand::SetCells {
                    slide,
                    edits,
                })]) {
                Ok(_) => {
                    super::sheet_view::release_layouts();
                    self.sync_peers(cx);
                    "xlsx.date-review.applied"
                }
                Err(_) => "xlsx.date-review.rejected",
            }
        };
        if let Some(panel) = self
            .sheet_state_mut()
            .and_then(|s| s.panels.date_review.as_mut())
        {
            panel.message = Some(message);
        }
        cx.notify();
    }

    pub fn sheet_date_review_panel(&mut self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        let p = self.sheet_state()?.panels.date_review.as_ref()?;
        let mut body = v_flex()
            .gap_2()
            .px(px(18.))
            .py(px(8.))
            .child(div().text_sm().child(text("xlsx.date-review.explanation")))
            .child(div().text_sm().child(text("xlsx.date-review.pivot")));
        if p.candidates.is_empty() {
            body = body.child(div().text_sm().child(text("xlsx.date-review.empty")));
        }
        for c in &p.candidates {
            let (result, status) = match &c.result {
                ReviewResult::Candidate(v) => (v.clone(), "xlsx.date-review.candidate"),
                ReviewResult::Inferred(v) => (v.clone(), "xlsx.date-review.inferred"),
                ReviewResult::Invalid => (String::new(), "xlsx.date-review.invalid"),
                ReviewResult::Unrecognized => (String::new(), "xlsx.date-review.unrecognized"),
            };
            body = body.child(
                v_flex()
                    .gap_1()
                    .py(px(4.))
                    .child(div().text_sm().child(format!(
                        "{}: {}",
                        cell_name(c.column, c.row),
                        c.original
                    )))
                    .child(
                        div()
                            .text_sm()
                            .child(format!("{} {}", text(status), result)),
                    ),
            );
        }
        if let Some(message) = p.message {
            body = body.child(div().text_sm().child(text(message)));
        }
        let footer = action_bar()
            .action(
                ActionDefinition::new("date-review-apply", "", ActionButtonKind::Primary)
                    .label(text("xlsx.date-review.apply"))
                    .on_click(cx.listener(|e, _, w, cx| e.apply_sheet_date_review(w, cx))),
            )
            .action(
                ActionDefinition::new("date-review-close", "", ActionButtonKind::Normal)
                    .label(text("_shell.close"))
                    .on_click(cx.listener(|e, _, w, cx| e.close_dialog(w, cx))),
            );
        Some(SidebarPanel::with_footer(
            text("xlsx.date-review.title"),
            body.into_any_element(),
            footer,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ReviewResult, candidates};
    use ibis_doc_model::{CellKind, CellRange, CellValue, Sheet, SheetCell};
    fn cell(row: u32, value: &str) -> SheetCell {
        SheetCell {
            row,
            column: 0,
            value: CellValue::Text(value.into()),
            text: value.into(),
            kind: CellKind::Text,
            formula: None,
            style: 0,
            phonetic: None,
            runs: None,
        }
    }
    #[test]
    fn only_selected_plain_text_cells_are_reviewed() {
        let mut sheet = Sheet::default();
        let mut formula = cell(1, "24/1/2");
        formula.formula = Some("A1".into());
        let mut rich = cell(2, "24/1/2");
        rich.runs = Some(Default::default());
        let mut phonetic = cell(3, "24/1/2");
        phonetic.phonetic = Some("reading".into());
        sheet.cells = vec![
            cell(4, "24/1/2"),
            formula,
            rich,
            phonetic,
            cell(5, "30/1/2"),
            cell(8, "outside"),
        ]
        .into();
        let found = candidates(
            &sheet,
            CellRange {
                first_row: 0,
                last_row: 5,
                first_column: 0,
                last_column: 0,
            },
        );
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].row, found[1].row), (4, 5));
        assert_eq!(found[0].result, ReviewResult::Inferred("2024-01-02".into()));
        assert_eq!(found[1].result, ReviewResult::Inferred("1930-01-02".into()));
    }
}
