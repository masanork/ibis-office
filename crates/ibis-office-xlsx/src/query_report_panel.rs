// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use ibis_doc_model::{CellValue, Command, Presentation};
use serde_json::{Value, json};

use crate::ext::*;
use crate::shell::chrome::{SidebarPanel, action_button, panel_message};
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::{format, text};

struct ReportChoice {
    name: String,
    sheet: String,
    sources: Vec<(String, String)>,
    columns: Vec<String>,
    selected: bool,
    metrics: BTreeSet<String>,
}

struct ReviewedRefresh {
    base: Presentation,
    result: Value,
    commands: Vec<Command>,
}

pub(crate) struct QueryReportPanel {
    reports: Vec<ReportChoice>,
    files: BTreeMap<String, PathBuf>,
    review: Option<ReviewedRefresh>,
    message: String,
    busy: bool,
    generation: u64,
}

impl QueryReportPanel {
    fn sources(&self) -> BTreeMap<String, String> {
        self.reports
            .iter()
            .filter(|report| report.selected)
            .flat_map(|report| report.sources.iter().cloned())
            .collect()
    }

    fn request(&self) -> Option<(Vec<PathBuf>, Value)> {
        let tables: Vec<_> = self
            .reports
            .iter()
            .filter(|report| report.selected)
            .map(|report| &report.name)
            .collect();
        if !(1..=12).contains(&tables.len()) {
            return None;
        }
        let mut paths = Vec::new();
        let mut bindings = BTreeMap::new();
        for file in self.sources().keys() {
            paths.push(self.files.get(file)?.clone());
            bindings.insert(file.clone(), paths.len().to_string());
        }
        if paths.is_empty() {
            return None;
        }
        let metrics: Vec<_> = self
            .reports
            .iter()
            .filter(|report| report.selected)
            .flat_map(|report| {
                report
                    .metrics
                    .iter()
                    .map(|column| json!({"table":report.name,"column":column}))
            })
            .collect();
        Some((
            paths,
            json!({"bindings":bindings,"tables":tables,"metrics":metrics}),
        ))
    }

    fn invalidate(&mut self) {
        self.review = None;
        self.message.clear();
    }
}

#[ibis_office_macros::editor_ext(QueryReportPanelExt)]
impl SlideEditor {
    pub(crate) fn open_query_report_panel(&mut self, cx: &mut Context<Self>) {
        let workbook = self.presentation();
        let Some(book) = workbook.workbook.as_ref() else {
            return;
        };
        let mut reports = Vec::new();
        for slide in &workbook.slides {
            let Some(sheet) = ibis_doc_model::sheet_calc::sheet_of_slide(slide) else {
                continue;
            };
            for table in &sheet.tables {
                let Some(definition) = book.query_reports.get(&table.name) else {
                    continue;
                };
                let sources = definition["sources"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|source| {
                        Some((
                            source["file"].as_str()?.into(),
                            source["kind"].as_str()?.into(),
                        ))
                    })
                    .collect();
                let width = table.columns.len();
                let mut numeric = vec![false; width];
                let mut nonnumeric = vec![false; width];
                for cell in sheet.cells_in_rows(table.range.first_row, table.range.last_row) {
                    if cell.row <= table.range.first_row {
                        continue;
                    }
                    let Some(index) = cell
                        .column
                        .checked_sub(table.range.first_column)
                        .map(|column| column as usize)
                        .filter(|&column| column < width)
                    else {
                        continue;
                    };
                    match cell.value {
                        CellValue::Number(number) if number.is_finite() => numeric[index] = true,
                        CellValue::Empty => {}
                        _ => nonnumeric[index] = true,
                    }
                }
                let columns = table
                    .columns
                    .iter()
                    .zip(numeric)
                    .zip(nonnumeric)
                    .filter_map(|((column, numeric), nonnumeric)| {
                        (numeric && !nonnumeric).then(|| column.clone())
                    })
                    .collect();
                reports.push(ReportChoice {
                    name: table.name.clone(),
                    sheet: slide.name.clone().unwrap_or_default(),
                    sources,
                    columns,
                    selected: false,
                    metrics: BTreeSet::new(),
                });
            }
        }
        if reports.len() <= 12 {
            reports.iter_mut().for_each(|report| report.selected = true);
        }
        static NEXT: AtomicU64 = AtomicU64::new(1);
        if let Some(panel) = self.wb_mut().panels.queries.as_mut() {
            panel.reports = Some(QueryReportPanel {
                reports,
                files: BTreeMap::new(),
                review: None,
                message: String::new(),
                busy: false,
                generation: NEXT.fetch_add(1, Ordering::Relaxed),
            });
        }
        cx.notify();
    }

    fn query_reports_mut(&mut self) -> Option<&mut QueryReportPanel> {
        self.sheet_state_mut()?
            .panels
            .queries
            .as_mut()?
            .reports
            .as_mut()
    }

    fn pick_report_source(
        &mut self,
        file: String,
        kind: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let file_id = self.file_id;
        let Some(panel) = self.query_reports_mut().filter(|panel| !panel.busy) else {
            return;
        };
        let generation = panel.generation;
        panel.busy = true;
        let extensions: &[&str] = match kind {
            "csv" => &["csv"],
            "parquet" => &["parquet"],
            "sqlite" => &["sqlite", "sqlite3", "db"],
            _ => ibis_doc_service::DATASET_EXTENSIONS,
        };
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            extensions: extensions.iter().map(ToString::to_string).collect(),
            prompt: Some(format("query.report-pick-file", &[("file", &file)]).into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let chosen = picked
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .and_then(|paths| paths.into_iter().next());
            let _ = this.update(cx, |editor, cx| {
                if editor.file_id != file_id {
                    return;
                }
                let Some(panel) = editor
                    .query_reports_mut()
                    .filter(|panel| panel.generation == generation)
                else {
                    return;
                };
                panel.busy = false;
                if let Some(path) = chosen {
                    panel.files.insert(file, path);
                    panel.invalidate();
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn preview_query_reports(&mut self, cx: &mut Context<Self>) {
        self.commit_cell_edit(cx);
        let base = self.presentation().clone();
        let file_id = self.file_id;
        let Some(panel) = self.query_reports_mut().filter(|panel| !panel.busy) else {
            return;
        };
        let Some((paths, args)) = panel.request() else {
            return;
        };
        let generation = panel.generation;
        panel.invalidate();
        panel.busy = true;
        panel.message = text("query.report-reading").to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let definitions =
                        ibis_doc_service::batch_query_report_definitions(&base, &args)?;
                    let batch =
                        ibis_doc_service::prepare_selected_report_batch(&paths, args, definitions)?;
                    let (result, commands) = batch.review(&base)?;
                    Ok::<_, ibis_doc_service::ToolError>(ReviewedRefresh {
                        base,
                        result,
                        commands,
                    })
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                if editor.file_id != file_id {
                    return;
                }
                let Some(panel) = editor
                    .query_reports_mut()
                    .filter(|panel| panel.generation == generation)
                else {
                    return;
                };
                panel.busy = false;
                match result {
                    Ok(review) => {
                        panel.message = if review.result["blocked"] == true {
                            text("query.report-blocked").to_string()
                        } else {
                            String::new()
                        };
                        panel.review = Some(review);
                    }
                    Err(error) => panel.message = error.message,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_query_report_review(&mut self, cx: &mut Context<Self>) {
        if self.is_read_only() {
            return;
        }
        self.commit_cell_edit(cx);
        let Some(review) = self
            .query_reports_mut()
            .filter(|panel| !panel.busy)
            .and_then(|panel| panel.review.take())
        else {
            return;
        };
        if review.commands.is_empty() {
            return;
        }
        let message = if self.presentation() != &review.base {
            text("query.report-changed").to_string()
        } else {
            if let Some(state) = self.sheet_state_mut() {
                state.last_fill = None;
            }
            crate::sheet_view::release_layouts();
            match self.document.execute(review.commands) {
                Ok(()) => {
                    self.sync_peers(cx);
                    let count = review.result["reports"].as_array().map_or(0, Vec::len);
                    format("query.report-applied", &[("count", &count)])
                }
                Err(error) => error.to_string(),
            }
        };
        if let Some(panel) = self.query_reports_mut() {
            panel.message = message;
        }
        cx.notify();
    }

    pub(crate) fn query_report_panel(&mut self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        let read_only = self.is_read_only();
        let panel = self.query_reports_mut()?;
        let generation = panel.generation;
        let busy = panel.busy;
        let can_preview = panel.request().is_some() && !busy;
        let can_apply = !read_only
            && !busy
            && panel
                .review
                .as_ref()
                .is_some_and(|review| !review.commands.is_empty());
        let mut body = v_flex().gap_2().px(gpui_kit::px(18.)).py(gpui_kit::px(8.));
        if panel.reports.is_empty() {
            body = body.child(text("query.report-empty"));
        } else {
            body = body.child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(text("query.report-tables")),
            );
        }
        for (index, report) in panel.reports.iter().enumerate() {
            body = body.child(
                Checkbox::new(("query-report-table", index))
                    .label(format!("{} · {}", report.name, report.sheet))
                    .checked(report.selected)
                    .disabled(busy)
                    .on_change(cx.listener(move |editor, checked, _, cx| {
                        if let Some(panel) = editor
                            .query_reports_mut()
                            .filter(|panel| !panel.busy && panel.generation == generation)
                        {
                            panel.reports[index].selected = *checked;
                            panel.invalidate();
                            cx.notify();
                        }
                    })),
            );
            if report.selected && !report.columns.is_empty() {
                let mut metrics = h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(text("query.report-metrics"));
                for (column_index, column) in report.columns.iter().enumerate() {
                    let key =
                        SharedString::from(format!("query-report-metric-{index}-{column_index}"));
                    let column = column.clone();
                    metrics = metrics.child(
                        Checkbox::new(key)
                            .label(column.clone())
                            .checked(report.metrics.contains(&column))
                            .disabled(busy)
                            .on_change(cx.listener(move |editor, checked, _, cx| {
                                if let Some(panel) = editor
                                    .query_reports_mut()
                                    .filter(|panel| !panel.busy && panel.generation == generation)
                                {
                                    let metrics = &mut panel.reports[index].metrics;
                                    if *checked {
                                        metrics.insert(column.clone());
                                    } else {
                                        metrics.remove(&column);
                                    }
                                    panel.invalidate();
                                    cx.notify();
                                }
                            })),
                    );
                }
                body = body.child(metrics);
            }
        }
        if panel
            .reports
            .iter()
            .filter(|report| report.selected)
            .count()
            > 12
        {
            body = body.child(text("query.report-limit"));
        }
        let sources = panel.sources();
        if !sources.is_empty() {
            body = body.child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(text("query.report-files")),
            );
        }
        for (index, (file, kind)) in sources.into_iter().enumerate() {
            let label = panel
                .files
                .get(&file)
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| text("query.report-choose-file").to_string());
            body = body.child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child(file.clone()))
                    .child(
                        action_button(("query-report-source", index))
                            .small()
                            .label(label)
                            .disabled(busy)
                            .on_click(cx.listener(move |editor, _, window, cx| {
                                editor.pick_report_source(file.clone(), &kind, window, cx)
                            })),
                    ),
            );
        }
        if let Some(review) = &panel.review {
            body = body.child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(text("query.report-preview")),
            );
            for report in review.result["reports"].as_array().into_iter().flatten() {
                let name = report["table"].as_str().unwrap_or_default();
                body = body.child(div().text_sm().child(format(
                    "query.report-rows",
                    &[
                        ("table", &name),
                        ("previous", &report["previous_rows"]),
                        ("current", &report["rows"]),
                    ],
                )));
                for metric in report["metrics"].as_array().into_iter().flatten() {
                    let name = metric["column"].as_str().unwrap_or_default();
                    let fields: [(&str, &dyn std::fmt::Display); 3] = [
                        ("column", &name),
                        ("previous", &metric["previous"]),
                        ("current", &metric["current"]),
                    ];
                    let mut fields = fields.to_vec();
                    fields.push(("delta", &metric["delta"]));
                    let rate = metric["percent_change"]
                        .as_f64()
                        .map(|rate| format!("{rate:.2}"));
                    let key = if let Some(rate) = &rate {
                        fields.push(("rate", rate));
                        "query.report-metric-rate"
                    } else {
                        "query.report-metric"
                    };
                    body = body.child(div().text_sm().child(format(key, &fields)));
                }
                for check in report["checks"].as_array().into_iter().flatten() {
                    let name = check["name"].as_str().unwrap_or_default();
                    let line = if check["max"].is_null() {
                        format(
                            "query.report-check",
                            &[("name", &name), ("count", &check["count"])],
                        )
                    } else {
                        format(
                            "query.report-check-max",
                            &[
                                ("name", &name),
                                ("count", &check["count"]),
                                ("max", &check["max"]),
                            ],
                        )
                    };
                    body = body.child(div().text_sm().child(line));
                }
            }
        }
        body = body.children(panel_message(&panel.message));
        use crate::shell::chrome::{ActionButtonKind, ActionDefinition, action_bar};
        let footer = action_bar()
            .action(
                ActionDefinition::new("query-reports-back", "", ActionButtonKind::Normal)
                    .label(text("query.report-back"))
                    .on_click(cx.listener(|editor, _, _, cx| {
                        if let Some(panel) = editor.wb_mut().panels.queries.as_mut() {
                            panel.reports = None;
                        }
                        cx.notify();
                    })),
            )
            .action(
                ActionDefinition::new("query-reports-preview", "", ActionButtonKind::Normal)
                    .label(text("query.report-preview"))
                    .disabled(!can_preview)
                    .on_click(cx.listener(|editor, _, _, cx| editor.preview_query_reports(cx))),
            )
            .action(
                ActionDefinition::new("query-reports-apply", "", ActionButtonKind::Primary)
                    .label(text("query.refresh"))
                    .disabled(!can_apply)
                    .on_click(cx.listener(|editor, _, _, cx| editor.apply_query_report_review(cx))),
            );
        Some(SidebarPanel::with_footer(
            text("query.report-refresh-title"),
            body.into_any_element(),
            footer,
        ))
    }
}
