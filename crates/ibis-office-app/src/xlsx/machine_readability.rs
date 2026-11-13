// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! A structural review of the current in-memory workbook. This is not a score.
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_doc_model::machine_readability::{CandidateKind, candidates};
use ibis_doc_model::sheet_calc::sheet_of_slide;
use ibis_doc_model::SlideId;
use crate::shell::chrome::SidebarPanel;
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::text;

#[derive(Clone)]
pub(crate) struct Finding { sheet: SlideId, name: String, row: u32, col: u32, pub(crate) kind: CandidateKind, address: String }

pub(crate) struct MachineReadabilityPanel { pub(crate) findings: Vec<Finding>, loading: bool }

fn scan(p: &ibis_doc_model::Presentation) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (index, slide) in p.slides.iter().enumerate() {
        let Some(sheet) = sheet_of_slide(slide) else { continue };
        let name = slide.name.clone().unwrap_or_else(|| format!("Sheet{}", index + 1));
        for item in candidates(sheet) {
            let r = item.range;
            let a = format!("{}{}", super::sheet_ui::column_letters(r.first_column), r.first_row + 1);
            let b = format!("{}{}", super::sheet_ui::column_letters(r.last_column), r.last_row + 1);
            let address = if a == b { a } else { format!("{a}:{b}") };
            findings.push(Finding { sheet: slide.id, name: name.clone(), row: r.first_row, col: r.first_column, kind: item.kind, address });
        }
    }
    findings
}

impl SlideEditor {
    pub(crate) fn open_machine_readability(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let panel = MachineReadabilityPanel { findings: scan(self.presentation()), loading: self.is_csv_streaming };
        self.wb_mut().close_settings();
        self.wb_mut().panels.machine_readability = Some(panel);
        self.sidebar_open = true;
        cx.notify();
    }
    pub(crate) fn recheck_machine_readability(&mut self, cx: &mut Context<Self>) {
        let panel = MachineReadabilityPanel { findings: scan(self.presentation()), loading: self.is_csv_streaming };
        self.wb_mut().panels.machine_readability = Some(panel);
        cx.notify();
    }
    pub(crate) fn jump_machine_readability(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(finding) = self.wb().panels.machine_readability.as_ref().and_then(|p| p.findings.get(index)).cloned() else { return };
        self.current_slide = finding.sheet;
        let wb = self.wb_mut();
        wb.cells.insert(finding.sheet, (finding.col, finding.row));
        wb.cell_ends.remove(&finding.sheet);
        wb.jump.pending_scroll = true;
        self.selection = None;
        cx.notify();
    }
    pub(crate) fn machine_readability_panel(&mut self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        let panel = self.wb().panels.machine_readability.as_ref()?;
        let findings = panel.findings.clone();
        let loading = panel.loading;
        let mut content = v_flex().p_4().gap_3()
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(text("xlsx.machine-readability.scope").to_string()))
            .when(loading, |el| el.child(div().text_sm().child(text("xlsx.machine-readability.loading").to_string())));
        if findings.is_empty() {
            content = content.child(div().text_sm().child(text("xlsx.machine-readability.none").to_string()));
        }
        let omitted = findings.len().saturating_sub(200);
        for (index, finding) in findings.into_iter().take(200).enumerate() {
            let label = match finding.kind {
                CandidateKind::MergedCells => "xlsx.machine-readability.merge",
                CandidateKind::HiddenRow => "xlsx.machine-readability.hidden-row",
                CandidateKind::HiddenColumn => "xlsx.machine-readability.hidden-column",
                CandidateKind::Formula => "xlsx.machine-readability.formula",
            };
            let title = format!("{} · {}", finding.name, finding.address);
            content = content.child(v_flex().gap_1().p_2().border_1().border_color(cx.theme().border).rounded_md()
                .child(h_flex().justify_between().child(div().text_sm().child(title))
                    .child(crate::shell::chrome::action_button(("machine-jump", index)).label(text("_shell.view")).small()
                        .on_click(cx.listener(move |this, _, _, cx| this.jump_machine_readability(index, cx)))))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(text(label).to_string())));
        }
        if omitted > 0 {
            content = content.child(div().text_sm().child(format!("{}: {}", text("xlsx.machine-readability.omitted"), omitted)));
        }
        let footer = crate::shell::chrome::action_bar().action(
            crate::shell::chrome::ActionDefinition::new("machine-recheck", "", crate::shell::chrome::ActionButtonKind::Normal)
                .label(text("xlsx.accessibility.recheck")).on_click(cx.listener(|this, _, _, cx| this.recheck_machine_readability(cx))),
        );
        Some(SidebarPanel::with_footer(text("xlsx.machine-readability.title"), content.into_any_element(), footer))
    }
}
