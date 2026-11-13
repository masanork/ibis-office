// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! A structural review of the current in-memory workbook. This is not a score.
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
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

pub(crate) struct MachineReadabilityPanel {
    pub(crate) findings: Vec<Finding>, loading: bool,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) full: Option<std::sync::Arc<super::harunobu::Report>>,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) full_error: Option<String>,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) full_busy: Option<u64>,
}

impl MachineReadabilityPanel {
    fn new(editor: &SlideEditor) -> Self {
        Self { findings: scan(editor.presentation()), loading: editor.is_csv_streaming,
            #[cfg(not(target_family = "wasm"))] full: None,
            #[cfg(not(target_family = "wasm"))] full_error: None,
            #[cfg(not(target_family = "wasm"))] full_busy: None,
        }
    }
}

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
        let panel = MachineReadabilityPanel::new(self);
        self.wb_mut().close_settings();
        self.wb_mut().panels.machine_readability = Some(panel);
        self.sidebar_open = true;
        cx.notify();
    }
    pub(crate) fn recheck_machine_readability(&mut self, cx: &mut Context<Self>) {
        let panel = MachineReadabilityPanel::new(self);
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
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn run_harunobu(&mut self, mode: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let validation = if self.is_dirty() { Err(text("xlsx.harunobu.save-first").to_string()) }
            else if self.is_csv_streaming { Err(text("xlsx.machine-readability.loading").to_string()) }
            else { self.path.clone().ok_or_else(|| text("xlsx.harunobu.save-first").to_string()) };
        let path = match validation {
            Ok(path) => path,
            Err(error) => { if let Some(p) = self.wb_mut().panels.machine_readability.as_mut() { p.full = None; p.full_error = Some(error); p.full_busy = None; } cx.notify(); return; }
        };
        self.start_harunobu(vec![path], false, mode, window, cx);
    }
    #[cfg(not(target_family = "wasm"))]
    fn pick_harunobu_bulk(&mut self, mode: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let document_id = self.document_id;
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false,
            extensions: ["xlsx", "xlsm", "csv", "tsv"].iter().map(|v| v.to_string()).collect(),
            multiple: true, prompt: Some(text("xlsx.harunobu.pick-bulk")) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return; };
            this.update_in(cx, |this, window, cx| {
                if this.document_id != document_id { return; }
                if paths.len() < 2 { this.set_status(text("xlsx.harunobu.bulk-minimum").to_string(), cx); return; }
                this.start_harunobu(paths, true, mode, window, cx);
            }).ok();
        }).detach();
    }
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn start_harunobu(&mut self, paths: Vec<std::path::PathBuf>, bulk: bool, mode: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let job = NEXT.fetch_add(1, Ordering::Relaxed);
        let document_id = self.document_id;
        if let Some(p) = self.wb_mut().panels.machine_readability.as_mut() { p.full = None; p.full_error = None; p.full_busy = Some(job); }
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                super::harunobu::runtime().and_then(|bin| {
                    if bulk { super::harunobu::analyze_bulk(&bin, &paths, mode) }
                    else { super::harunobu::analyze(&bin, &paths[0], mode) }
                })
            }).await;
            this.update_in(cx, |this, _window, cx| {
                if this.document_id != document_id { return; }
                let Some(panel) = this.wb_mut().panels.machine_readability.as_mut() else { return; };
                if panel.full_busy != Some(job) { return; }
                panel.full_busy = None;
                match result {
                    Ok(report) => panel.full = Some(std::sync::Arc::new(report)),
                    Err(error) => {
                        let message = match error.as_str() {
                            "Harunobu runtime is not installed" | "Harunobu executable is missing" => text("xlsx.harunobu.runtime-missing").to_string(),
                            "Harunobu runtime version does not match" | "Harunobu runtime source is modified" | "Cannot verify Harunobu runtime" => text("xlsx.harunobu.runtime-invalid").to_string(),
                            "Unsupported input format" => text("xlsx.harunobu.unsupported").to_string(),
                            "Harunobu timed out" => text("xlsx.harunobu.timeout").to_string(),
                            _ => error,
                        };
                        panel.full_error = Some(message);
                    }
                }
                cx.notify();
            }).ok();
        }).detach();
    }
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn jump_harunobu(&mut self, sheet: SlideId, row: u32, col: u32, cx: &mut Context<Self>) {
        self.current_slide = sheet;
        let wb = self.wb_mut();
        wb.cells.insert(sheet, (col, row));
        wb.cell_ends.remove(&sheet);
        wb.jump.pending_scroll = true;
        self.selection = None;
        cx.notify();
    }
    #[cfg(not(target_family = "wasm"))]
    fn export_harunobu(&mut self, csv: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(report) = self.wb().panels.machine_readability.as_ref().and_then(|p| p.full.clone()) else { return };
        let sources = report.sources.clone();
        let bytes = if csv { report.csv.as_bytes().to_vec() } else {
            match serde_json::to_vec_pretty(&report.json) { Ok(bytes) => bytes, Err(error) => { self.set_status(error.to_string(), cx); return; } }
        };
        let directory = self.path.as_ref().and_then(|p| p.parent()).map(std::path::Path::to_path_buf)
            .or_else(|| std::env::var_os("HOME").map(Into::into)).unwrap_or_else(|| ".".into());
        let name = if csv { "harunobu-report.csv" } else { "harunobu-report.json" };
        let picked = cx.prompt_for_new_path(&directory, Some(name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = picked.await else { return };
            this.update(cx, |this, cx| {
                let same = sources.iter().any(|original| original == &path || same_file::is_same_file(original, &path).unwrap_or(false));
                if same { this.set_status(text("xlsx.harunobu.cannot-overwrite").to_string(), cx); return; }
                match std::fs::write(&path, &bytes) {
                    Ok(()) => this.set_status(text("xlsx.harunobu.exported").to_string(), cx),
                    Err(error) => this.set_status(error.to_string(), cx),
                }
            }).ok();
        }).detach();
    }
    pub(crate) fn machine_readability_panel(&mut self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        let panel = self.wb().panels.machine_readability.as_ref()?;
        let findings = panel.findings.clone();
        let loading = panel.loading;
        #[cfg(not(target_family = "wasm"))]
        let (full, full_error, full_busy) = (panel.full.clone(), panel.full_error.clone(), panel.full_busy.is_some());
        #[cfg(not(target_family = "wasm"))]
        let dirty = self.is_dirty();
        let mut content = v_flex().p_4().gap_3()
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(text("xlsx.machine-readability.scope").to_string()))
            .when(loading, |el| el.child(div().text_sm().child(text("xlsx.machine-readability.loading").to_string())));
        if findings.is_empty() {
            content = content.child(div().text_sm().child(text("xlsx.machine-readability.none").to_string()));
        }
        let omitted = findings.len().saturating_sub(200);
        let mut candidates_view = v_flex().gap_3();
        for (index, finding) in findings.into_iter().take(200).enumerate() {
            let label = match finding.kind {
                CandidateKind::MergedCells => "xlsx.machine-readability.merge",
                CandidateKind::HiddenRow => "xlsx.machine-readability.hidden-row",
                CandidateKind::HiddenColumn => "xlsx.machine-readability.hidden-column",
                CandidateKind::Formula => "xlsx.machine-readability.formula",
                CandidateKind::PlatformDependentCharacters => "xlsx.machine-readability.platform-characters",
            };
            let title = format!("{} · {}", finding.name, finding.address);
            candidates_view = candidates_view.child(v_flex().gap_1().p_2().border_1().border_color(cx.theme().border).rounded_md()
                .child(h_flex().justify_between().child(div().text_sm().child(title))
                    .child(crate::shell::chrome::action_button(("machine-jump", index)).label(text("_shell.view")).small()
                        .on_click(cx.listener(move |this, _, _, cx| this.jump_machine_readability(index, cx)))))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(text(label).to_string())));
        }
        if omitted > 0 {
            candidates_view = candidates_view.child(div().text_sm().child(format!("{}: {}", text("xlsx.machine-readability.omitted"), omitted)));
        }
        #[cfg(not(target_family = "wasm"))]
        {
            content = content.child(div().text_sm().child(text("xlsx.harunobu.scope").to_string()));
            let mut single_modes = h_flex().gap_1();
            for (index, (mode, label)) in [("lite", "L1"), ("standard", "L1–L2"), ("thorough", "L1–L3")].into_iter().enumerate() {
                single_modes = single_modes.child(crate::shell::chrome::action_button(("harunobu-mode", index)).label(label).small().disabled(full_busy)
                    .on_click(cx.listener(move |this, _, window, cx| this.run_harunobu(mode, window, cx))));
            }
            content = content.child(single_modes).child(div().text_sm().child(text("xlsx.harunobu.bulk-scope").to_string()));
            let mut bulk_modes = h_flex().gap_1();
            for (index, (mode, label)) in [("lite", "L1"), ("standard", "L1–L2"), ("thorough", "L1–L3")].into_iter().enumerate() {
                bulk_modes = bulk_modes.child(crate::shell::chrome::action_button(("harunobu-bulk", index)).label(label).small().disabled(full_busy)
                    .on_click(cx.listener(move |this, _, window, cx| this.pick_harunobu_bulk(mode, window, cx))));
            }
            content = content.child(bulk_modes);
            if full_busy { content = content.child(div().text_sm().child(text("xlsx.harunobu.running").to_string())); }
            if let Some(error) = full_error { content = content.child(div().text_sm().text_color(cx.theme().danger).child(format!("{}: {error}", text("xlsx.harunobu.failed")))); }
            if let Some(report) = full {
                if dirty && !report.bulk { content = content.child(div().text_sm().text_color(cx.theme().warning).child(text("xlsx.harunobu.stale").to_string())); }
                for (index, (csv, key)) in [(false, "xlsx.harunobu.export-json"), (true, "xlsx.harunobu.export-csv")].into_iter().enumerate() {
                    content = content.child(crate::shell::chrome::action_button(("harunobu-export", index)).label(text(key)).small()
                        .on_click(cx.listener(move |this, _, window, cx| this.export_harunobu(csv, window, cx))));
                }
                if report.bulk {
                    if let Some(total) = report.json["複数ファイル総合スコア"].as_i64() {
                        content = content.child(div().text_sm().child(format!("{}: {total}/100", text("xlsx.harunobu.bulk-score"))));
                    }
                    if let Some(checks) = report.json["ファイル横断チェック"].as_object() {
                        for (id, rule) in checks {
                            let skipped = rule["判定対象外"].as_bool().unwrap_or(false);
                            let passed = rule["合否"].as_bool().unwrap_or(false);
                            let status = if skipped { text("xlsx.harunobu.skipped") } else if passed { text("xlsx.harunobu.info") } else { text("xlsx.harunobu.issue") };
                            content = content.child(div().text_sm().child(format!("{id} · {status}: {}", rule["メッセージ"].as_str().unwrap_or(""))));
                            if let Some(violations) = rule["違反"].as_array() {
                                for violation in violations.iter().take(10) {
                                    content = content.child(div().text_sm().child(format!("{}: {}", violation["セル範囲"].as_str().unwrap_or(""), violation["説明"].as_str().unwrap_or(""))));
                                }
                                if violations.len() > 10 { content = content.child(div().text_sm().child(format!("{}: {}", text("xlsx.machine-readability.omitted"), violations.len() - 10))); }
                            }
                        }
                    }
                }
                let reports: Vec<&serde_json::Value> = if report.bulk { report.json["ファイル別結果"].as_array().map(|v| v.iter().collect()).unwrap_or_default() } else { vec![&report.json] };
                let mut count = 0usize;
                for file_report in reports {
                if report.bulk { content = content.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(file_report["inputファイル名"].as_str().unwrap_or("").to_string())); }
                let scores = &file_report["レベル別スコア"];
                for level in ["1", "2", "3"] {
                    if let Some(score) = scores[level]["スコア"].as_i64() {
                        let passed = scores[level]["合格"].as_i64().unwrap_or(0);
                        let total = scores[level]["合計"].as_i64().unwrap_or(0);
                        content = content.child(div().text_sm().child(format!("L{level}: {score}/100 ({passed}/{total})")));
                    }
                }
                if let Some(sheets) = file_report["sheets"].as_array() {
                    for sheet in sheets {
                        let name = sheet["シート名"].as_str().unwrap_or("");
                        let hidden = sheet["非表示"].as_bool().unwrap_or(false);
                        let sheet_id = if report.bulk || hidden { None } else { self.presentation().slides.iter().find(|s| s.name.as_deref() == Some(name)).map(|s| s.id) };
                        content = content.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(if hidden { format!("{name} ({})", text("xlsx.harunobu.hidden-sheet")) } else { name.to_string() }));
                        if let Some(areas) = sheet["評価対象エリア"].as_array() {
                            for area in areas {
                                let range = area["範囲"].as_str().unwrap_or("");
                                let confidence = area["信頼度"].as_f64().unwrap_or(0.0);
                                let headers = area["ヘッダー範囲"].as_str().unwrap_or("—");
                                content = content.child(div().text_sm().text_color(cx.theme().muted_foreground).child(format!("{}: {range} · {}: {headers} · {:.0}%", text("xlsx.harunobu.table"), text("xlsx.harunobu.header"), confidence * 100.0)));
                                if let Some(rules) = area["評価内容"].as_array() {
                                    for rule in rules {
                                        let skipped = rule["判定対象外"].as_bool().unwrap_or(false);
                                        let failed = !rule["合否"].as_bool().unwrap_or(false);
                                        let has_violations = rule["違反"].as_array().is_some_and(|v| !v.is_empty());
                                        if !skipped && !failed && !has_violations { continue; }
                                        count += 1;
                                        if count > 200 { continue; }
                                        let id = rule["ルールID"].as_str().unwrap_or("");
                                        let status = if skipped { text("xlsx.harunobu.skipped") } else if failed { text("xlsx.harunobu.issue") } else { text("xlsx.harunobu.info") };
                                        let msg = rule["メッセージ"].as_str().unwrap_or("");
                                        let mut card = v_flex().gap_1().p_2().border_1().border_color(cx.theme().border).rounded_md()
                                            .child(div().text_sm().child(format!("{name} · {id} · {status}")))
                                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(msg.to_string()));
                                        if let Some(violations) = rule["違反"].as_array() {
                                            for (vi, v) in violations.iter().take(10).enumerate() {
                                                let place = v["セル範囲"].as_str().unwrap_or("");
                                                let detail = v["説明"].as_str().unwrap_or("");
                                                let target = sheet_id.zip(super::harunobu::cell_location(place));
                                                let mut line = h_flex().gap_1().child(div().text_sm().flex_1().child(format!("{place}: {detail}")));
                                                if let Some((sheet_id, (row, col))) = target {
                                                    line = line.child(crate::shell::chrome::action_button(("harunobu-jump", count * 10 + vi)).label(text("_shell.view")).small()
                                                        .on_click(cx.listener(move |this, _, _, cx| this.jump_harunobu(sheet_id, row, col, cx))));
                                                }
                                                card = card.child(line);
                                            }
                                            if violations.len() > 10 { card = card.child(div().text_sm().child(format!("{}: {}", text("xlsx.machine-readability.omitted"), violations.len() - 10))); }
                                        }
                                        content = content.child(card);
                                    }
                                }
                            }
                        }
                    }
                }
                }
                if count > 200 { content = content.child(div().text_sm().child(format!("{}: {}", text("xlsx.machine-readability.omitted"), count - 200))); }
            }
        }
        content = content.child(candidates_view);
        let footer = crate::shell::chrome::action_bar().action(
            crate::shell::chrome::ActionDefinition::new("machine-recheck", "", crate::shell::chrome::ActionButtonKind::Normal)
                .label(text("xlsx.accessibility.recheck")).on_click(cx.listener(|this, _, _, cx| this.recheck_machine_readability(cx))),
        );
        Some(SidebarPanel::with_footer(text("xlsx.machine-readability.title"), content.into_any_element(), footer))
    }
}
