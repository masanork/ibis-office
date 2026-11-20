// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! Read-only comparison of strings under the variant cluster relation.
use crate::ext::*;
use crate::shell::{
    chrome::{ActionButtonKind, ActionDefinition, SidebarPanel, action_bar},
    editor::SlideEditor,
    i18n::text,
};
use gpui_kit::component::v_flex;
use gpui_kit::*;
use ibis_doc_model::variant_clusters::{self, VariantClusters};
use ibis_doc_model::{CellRange, CellValue, Sheet, cell_name};
use std::collections::BTreeMap;

const MAX_CELLS: usize = 100_000;
const MAX_GROUPS: usize = 200;
const MAX_LOCATIONS: usize = 10;
const MAX_FORMS: usize = 20;
const MAX_TEXT_CHARS: usize = 4096;
const MAX_TOTAL_CHARS: usize = 1_000_000;
#[derive(Default)]
struct Form {
    count: usize,
    cells: Vec<(u32, u32)>,
}
struct MatchGroup {
    forms: Vec<(String, Form)>,
    more_forms: usize,
}
pub(crate) struct VariantReviewPanel {
    available: bool,
    matches: Vec<MatchGroup>,
    skipped: usize,
    truncated: bool,
    pup_fonts: super::variant_pup_fonts::PupFonts,
}
fn variation_selector(c: char) -> bool {
    matches!(c as u32, 0xfe00..=0xfe0f | 0xe0100..=0xe01ef)
}
// Cluster numbers are outside the Unicode scalar range, so unmapped characters
// can never collide with a dictionary group. No normalization beyond clustering.
fn key(value: &str, dict: &VariantClusters) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    for c in value.chars() {
        if variation_selector(c) || out.len() == MAX_TEXT_CHARS {
            return None;
        }
        out.push(dict.cluster_index(c).map_or(c as u32, |g| 0x110000 + g));
    }
    Some(out)
}
fn candidates(
    sheet: &Sheet,
    range: CellRange,
    dict: &VariantClusters,
) -> (Vec<MatchGroup>, usize, bool) {
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
    let mut indexed: BTreeMap<Vec<u32>, BTreeMap<String, Form>> = BTreeMap::new();
    let mut skipped = 0;
    let mut visited = 0;
    let mut total_chars = 0;
    let mut truncated = false;
    for cell in cells {
        let CellValue::Text(value) = &cell.value else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        if visited == MAX_CELLS {
            truncated = true;
            break;
        }
        visited += 1;
        let Some(key) = key(value, dict) else {
            skipped += 1;
            continue;
        };
        total_chars += key.len();
        if total_chars > MAX_TOTAL_CHARS {
            truncated = true;
            break;
        }
        let form = indexed
            .entry(key)
            .or_default()
            .entry(value.clone())
            .or_default();
        form.count += 1;
        if form.cells.len() < MAX_LOCATIONS {
            form.cells.push((cell.row, cell.column));
        }
    }
    let mut matches = Vec::new();
    for (_, forms) in indexed {
        if forms.len() < 2 {
            continue;
        }
        if matches.len() == MAX_GROUPS {
            truncated = true;
            break;
        }
        let more_forms = forms.len().saturating_sub(MAX_FORMS);
        matches.push(MatchGroup {
            forms: forms.into_iter().take(MAX_FORMS).collect(),
            more_forms,
        });
    }
    (matches, skipped, truncated)
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
        let (available, matches, skipped, truncated) = match variant_clusters::embedded() {
            Some(dict) => {
                let (m, s, t) = candidates(sheet, range, dict);
                (true, m, s, t)
            }
            None => (false, Vec::new(), 0, false),
        };
        let pup_fonts = super::variant_pup_fonts::PupFonts::discover(cx);
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
            matches,
            skipped,
            truncated,
            pup_fonts,
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
        } else if p.matches.is_empty() {
            body = body.child(div().text_sm().child(text("xlsx.variant-review.empty")));
        }
        for (index, group) in p.matches.iter().enumerate() {
            let mut block = v_flex()
                .gap_1()
                .py(px(4.))
                .child(div().text_sm().child(format!(
                    "{} {}",
                    text("xlsx.variant-review.group"),
                    index + 1
                )));
            let characters: Vec<Vec<char>> = group
                .forms
                .iter()
                .map(|(value, _)| value.chars().collect())
                .collect();
            let different: Vec<usize> = (0..characters[0].len())
                .filter(|&pos| {
                    characters
                        .iter()
                        .skip(1)
                        .any(|row| row[pos] != characters[0][pos])
                })
                .collect();
            for ((value, form), chars) in group.forms.iter().zip(&characters) {
                let mut codes = different
                    .iter()
                    .take(12)
                    .map(|&pos| format!("U+{:04X}", chars[pos] as u32))
                    .collect::<Vec<_>>()
                    .join(" ");
                if different.len() > 12 {
                    codes.push_str(" …");
                }
                let mut cells = form
                    .cells
                    .iter()
                    .map(|(r, c)| cell_name(*c, *r))
                    .collect::<Vec<_>>()
                    .join(", ");
                if form.count > form.cells.len() {
                    cells.push_str(", …");
                }
                let line = format!(
                    "{} [{}]　({}: {} / {})",
                    value,
                    codes,
                    text("xlsx.variant-review.cells"),
                    cells,
                    form.count
                );
                let (line, overrides) = p.pup_fonts.display(&line);
                // Highlights create run boundaries; font-family overrides then preserve
                // the original UTF-8 and inherit the surrounding UI style.
                let boundaries = overrides
                    .iter()
                    .map(|(range, _)| (range.clone(), HighlightStyle::default()))
                    .collect::<Vec<_>>();
                let line = StyledText::new(line)
                    .with_highlights(boundaries)
                    .with_font_family_overrides(overrides);
                block = block.child(div().text_sm().child(line));
            }
            if group.more_forms > 0 {
                block = block.child(div().text_sm().child(format!(
                    "{} {}",
                    text("xlsx.variant-review.more-forms"),
                    group.more_forms
                )));
            }
            body = body.child(block);
        }
        if p.skipped > 0 {
            body = body.child(div().text_sm().child(format!(
                "{} {}",
                text("xlsx.variant-review.skipped"),
                p.skipped
            )));
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
    use super::{MAX_TEXT_CHARS, VariantClusters, candidates, key};
    use ibis_doc_model::{CellKind, CellRange, CellValue, Sheet, SheetCell};
    fn dictionary() -> VariantClusters {
        let mut v = vec![0; 16 + 4352 * 2];
        v[0..4].copy_from_slice(&4u32.to_le_bytes());
        v[4..8].copy_from_slice(&2u32.to_le_bytes());
        v[8..12].copy_from_slice(&1u32.to_le_bytes());
        v[16..18].copy_from_slice(&1u16.to_le_bytes());
        let mut page = [0u8; 36];
        page[12..16].copy_from_slice(&30u32.to_le_bytes());
        v.extend(page);
        for x in [0u16, 0, 1, 1] {
            v.extend(x.to_le_bytes());
        }
        for x in [0u32, 2, 4, 0x41, 0x43, 0x41, 0x42, 0x43, 0x44] {
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
            last_row: 7,
            first_column: 0,
            last_column: 0,
        }
    }
    #[test]
    fn comparison_keeps_unrelated_characters_and_ivs_separate() {
        let d = dictionary();
        assert_eq!(key("AC", &d), key("BD", &d));
        assert_ne!(key("AC", &d), key("BCx", &d));
        assert_ne!(key("A", &d), key("C", &d));
        assert!(key("A\u{fe0f}", &d).is_none());
        assert!(key(&"A".repeat(MAX_TEXT_CHARS + 1), &d).is_none());
    }
    #[test]
    fn selected_distinct_spellings_group_without_modifying_cells() {
        let mut sheet = Sheet::default();
        sheet.cells = vec![
            cell(0, "AC"),
            cell(1, "BD"),
            cell(2, "AC"),
            cell(3, "ADx"),
            cell(4, "AC\u{e0100}"),
            cell(8, "BD"),
        ]
        .into();
        let (m, skipped, truncated) = candidates(&sheet, range(), &dictionary());
        assert_eq!(m.len(), 1);
        assert_eq!(skipped, 1);
        assert!(!truncated);
        assert_eq!(m[0].forms[0].0, "AC");
        assert_eq!(m[0].forms[0].1.count, 2);
        assert_eq!(m[0].forms[1].0, "BD");
        assert_eq!(m[0].forms[1].1.cells, vec![(1, 0)]);
        assert_eq!(sheet.cells[0].value, CellValue::Text("AC".into()));
    }
    #[test]
    fn bundled_gjs_dictionary_groups_high_variants_at_their_original_cells() {
        if option_env!("IBIS_VARIANT_BITMAP").is_some() {
            return;
        }
        let dict = ibis_doc_model::variant_clusters::embedded().expect("bundled gjs dictionary");
        let mut sheet = Sheet::default();
        sheet.cells = vec![
            cell(0, "髙橋花子"),
            SheetCell {
                column: 1,
                ..cell(0, "高橋花子")
            },
            cell(1, "山田花子"),
            cell(2, "高\u{e0100}橋花子"),
            SheetCell {
                column: 2,
                ..cell(2, "髙橋花子")
            },
        ]
        .into();
        let selection = CellRange {
            last_column: 1,
            ..range()
        };
        let (matches, skipped, truncated) = candidates(&sheet, selection, dict);

        assert_eq!(skipped, 1);
        assert!(!truncated);
        assert_eq!(matches.len(), 1);
        let forms: Vec<_> = matches[0]
            .forms
            .iter()
            .map(|(text, form)| (text.as_str(), form.count, form.cells.as_slice()))
            .collect();
        assert_eq!(
            forms,
            [
                ("高橋花子", 1, [(0, 1)].as_slice()),
                ("髙橋花子", 1, [(0, 0)].as_slice()),
            ]
        );
    }

    #[test]
    fn total_character_limit_reports_incomplete_results() {
        let mut sheet = Sheet::default();
        sheet.cells = (0..246)
            .map(|row| cell(row, &"A".repeat(MAX_TEXT_CHARS)))
            .collect::<Vec<_>>()
            .into();
        let all = CellRange {
            first_row: 0,
            last_row: 245,
            first_column: 0,
            last_column: 0,
        };
        let (_, skipped, truncated) = candidates(&sheet, all, &dictionary());
        assert!(truncated);
        assert_eq!(skipped, 0);
    }
    #[test]
    fn identical_values_alone_are_not_reported() {
        let mut sheet = Sheet::default();
        sheet.cells = vec![cell(0, "AC"), cell(1, "AC")].into();
        assert!(candidates(&sheet, range(), &dictionary()).0.is_empty());
    }
}
