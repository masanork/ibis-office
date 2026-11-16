// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! Inspect cluster members without changing the selected cells.
use crate::ext::*;
use crate::shell::{
    chrome::{ActionButtonKind, ActionDefinition, SidebarPanel, action_bar},
    editor::SlideEditor,
    i18n::text,
};
use gpui_kit::component::v_flex;
use gpui_kit::*;
use ibis_doc_model::variant_clusters::{self, Cluster, VariantClusters};
use ibis_doc_model::{CellRange, CellValue, Sheet, cell_name};

const LIMIT: usize = 200;
struct Candidate {
    row: u32,
    column: u32,
    original: char,
    cluster: Cluster,
}
pub(crate) struct VariantReviewPanel {
    available: bool,
    candidates: Vec<Candidate>,
    truncated: bool,
}
fn variation_selector(c: char) -> bool {
    matches!(c as u32, 0xfe00..=0xfe0f | 0xe0100..=0xe01ef)
}
fn candidates(sheet: &Sheet, range: CellRange, dict: &VariantClusters) -> (Vec<Candidate>, bool) {
    let mut cells: Vec<_> = sheet
        .cells
        .iter()
        .filter(|c| {
            c.row >= range.first_row
                && c.row <= range.last_row
                && c.column >= range.first_column
                && c.column <= range.last_column
        })
        .collect();
    cells.sort_by_key(|c| (c.row, c.column));
    let mut out = Vec::new();
    for cell in cells {
        let CellValue::Text(value) = &cell.value else {
            continue;
        };
        let mut chars = value.chars().peekable();
        while let Some(ch) = chars.next() {
            // An IVS is a sequence, not its base character alone.
            if variation_selector(ch) || chars.peek().is_some_and(|c| variation_selector(*c)) {
                continue;
            }
            if let Some(cluster) = dict.cluster(ch) {
                if out.len() == LIMIT {
                    return (out, true);
                }
                out.push(Candidate {
                    row: cell.row,
                    column: cell.column,
                    original: ch,
                    cluster,
                });
            }
        }
    }
    (out, false)
}
#[ibis_office_macros::editor_ext(VariantReview)]
impl SlideEditor {
    pub fn open_sheet_variant_review(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.commit_cell_edit(cx);
        let Some(range) = self.selected_range() else {
            return;
        };
        let Some((_, sheet)) = self.current_sheet() else {
            return;
        };
        let (available, candidates, truncated) = match variant_clusters::embedded() {
            Some(dict) => {
                let (c, t) = candidates(sheet, range, dict);
                (true, c, t)
            }
            None => (false, Vec::new(), false),
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
        s.panels.variant_review = Some(VariantReviewPanel {
            available,
            candidates,
            truncated,
        });
        self.sidebar_open = true;
        cx.notify();
    }
    pub fn sheet_variant_review_panel(&mut self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        let p = self.sheet_state()?.panels.variant_review.as_ref()?;
        let mut body = v_flex().gap_2().px(px(18.)).py(px(8.)).child(
            div()
                .text_sm()
                .child(text("xlsx.variant-review.explanation")),
        );
        if !p.available {
            body = body.child(
                div()
                    .text_sm()
                    .child(text("xlsx.variant-review.unavailable")),
            );
        } else if p.candidates.is_empty() {
            body = body.child(div().text_sm().child(text("xlsx.variant-review.empty")));
        }
        for item in &p.candidates {
            let members = item
                .cluster
                .members
                .iter()
                .map(|ch| format!("{} (U+{:04X})", ch, *ch as u32))
                .collect::<Vec<_>>()
                .join(" ");
            body = body.child(
                v_flex()
                    .gap_1()
                    .py(px(4.))
                    .child(div().text_sm().child(format!(
                        "{}: {} (U+{:04X})",
                        cell_name(item.column, item.row),
                        item.original,
                        item.original as u32
                    )))
                    .child(div().text_sm().child(format!(
                        "{} {}",
                        text("xlsx.variant-review.representative"),
                        item.cluster.representative
                    )))
                    .child(div().text_sm().child(format!(
                        "{} {}",
                        text("xlsx.variant-review.members"),
                        members
                    ))),
            );
        }
        if p.truncated {
            body = body.child(div().text_sm().child(text("xlsx.variant-review.truncated")));
        }
        let footer = action_bar().action(
            ActionDefinition::new("variant-review-close", "", ActionButtonKind::Normal)
                .label(text("_shell.close"))
                .on_click(cx.listener(|e, _, w, cx| e.close_dialog(w, cx))),
        );
        Some(SidebarPanel::with_footer(
            text("xlsx.variant-review.title"),
            body.into_any_element(),
            footer,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{LIMIT, VariantClusters, candidates};
    use ibis_doc_model::{CellKind, CellRange, CellValue, Sheet, SheetCell};
    fn dictionary() -> VariantClusters {
        let mut v = vec![0; 16 + 4352 * 2];
        v[0..4].copy_from_slice(&2u32.to_le_bytes());
        v[4..8].copy_from_slice(&1u32.to_le_bytes());
        v[8..12].copy_from_slice(&1u32.to_le_bytes());
        v[16..18].copy_from_slice(&1u16.to_le_bytes());
        let mut page = [0u8; 36];
        page[12..16].copy_from_slice(&6u32.to_le_bytes());
        v.extend(page);
        v.extend([0u8; 4]);
        for x in [0u32, 2, 0x41, 0x41, 0x42] {
            v.extend(x.to_le_bytes());
        }
        VariantClusters::from_bytes(v).unwrap()
    }
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
    fn range() -> CellRange {
        CellRange {
            first_row: 0,
            last_row: 2,
            first_column: 0,
            last_column: 0,
        }
    }
    #[test]
    fn selected_text_is_read_only_and_variation_sequences_are_skipped() {
        let mut sheet = Sheet::default();
        sheet.cells = vec![cell(0, "AB\u{fe0f}A"), cell(3, "B")].into();
        let (found, truncated) = candidates(&sheet, range(), &dictionary());
        assert!(!truncated);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|c| c.original == 'A'));
        assert_eq!(sheet.cells[0].value, CellValue::Text("AB\u{fe0f}A".into()));
    }
    #[test]
    fn limit_is_reported_only_when_more_results_exist() {
        let mut sheet = Sheet::default();
        sheet.cells = vec![cell(0, &"A".repeat(LIMIT))].into();
        let (found, truncated) = candidates(&sheet, range(), &dictionary());
        assert_eq!(found.len(), LIMIT);
        assert!(!truncated);
        sheet.cells = vec![cell(0, &"A".repeat(LIMIT + 1))].into();
        let (found, truncated) = candidates(&sheet, range(), &dictionary());
        assert_eq!(found.len(), LIMIT);
        assert!(truncated);
    }
}
