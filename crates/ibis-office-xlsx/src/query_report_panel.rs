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
use ibis_doc_service::{ReportParameterControl, ReportParameterKind, report_parameter_controls};
use serde_json::{Value, json};

use crate::ext::*;
use crate::shell::chrome::{Dropdown, DropdownItem, SidebarPanel, action_button, panel_message};
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::{format, text};

fn saved_check_text(value: String) -> AnyElement {
    #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
    {
        let id = SharedString::from(format!("query-report-check-text:{value}"));
        return crate::shell::scenario::button_probe::observe_control(
            div().id(id.clone()).whitespace_normal().child(value),
            id.into(),
        ).into_any_element();
    }
    #[cfg(not(all(feature = "scenarios", any(target_os = "macos", target_os = "linux"))))]
    div().whitespace_normal().child(value).into_any_element()
}

fn parameter_choices(workbook: &Presentation, control: &ReportParameterControl) -> Vec<(Value, String)> {
    let Some(sheet) = workbook.slides.iter()
        .find(|slide| slide.name.as_deref() == Some(&control.sheet))
        .and_then(ibis_doc_model::sheet_calc::sheet_of_slide) else { return Vec::new() };
    let Some(range) = ibis_doc_model::ai_sheet::parse_cell_range(&control.range) else { return Vec::new() };
    (range.first_row..=range.last_row).filter_map(|row| {
        let value = match (&sheet.cell(row, range.first_column)?.value, control.kind) {
            (CellValue::Text(text), ReportParameterKind::Text) => json!(text),
            (CellValue::Number(v), ReportParameterKind::Integer) if v.fract() == 0.0 && v.abs() <= (1_u64 << 53) as f64 => json!(*v as i64),
            (CellValue::Number(v), ReportParameterKind::Number) => json!(v),
            _ => return None,
        };
        if !control.kind.accepts(&value) {
            return None;
        }
        let shown = value.as_str().map_or_else(|| value.to_string(), str::to_owned);
        let label = match &sheet.cell(row, range.last_column)?.value {
            CellValue::Text(name) if name == &shown => name.clone(),
            CellValue::Text(name) if !name.trim().is_empty() => format!("{shown} · {name}"),
            _ => shown,
        };
        Some((value, label))
    }).collect()
}

struct ReportChoice {
    name: String,
    sheet: String,
    sources: Vec<(String, String)>,
    columns: Vec<String>,
    selected: bool,
    metrics: BTreeSet<String>,
    check_details_open: bool,
    controls: Option<Vec<(ReportParameterControl, Value)>>,
}

struct ReviewedRefresh {
    base: Presentation,
    result: Value,
    commands: Vec<Command>,
}

pub(crate) struct QueryReportPanel {
    reports: Vec<ReportChoice>,
    files: BTreeMap<String, PathBuf>,
    parameter_values: BTreeMap<String, Value>,
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

    fn controls(&self) -> Option<BTreeMap<&str, (&ReportParameterControl, Option<&Value>)>> {
        let mut controls: BTreeMap<_, (&ReportParameterControl, Option<&Value>)> = BTreeMap::new();
        for report in self.reports.iter().filter(|report| report.selected) {
            for (control, saved) in report.controls.as_ref()? {
                let supplied = self.parameter_values.get(&control.name);
                let (first, value) = controls.entry(control.name.as_str())
                    .or_insert((control, Some(supplied.unwrap_or(saved))));
                if first.kind != control.kind || first.sheet != control.sheet || first.range != control.range {
                    return None;
                }
                if supplied.is_none() && *value != Some(saved) {
                    *value = None;
                }
            }
        }
        Some(controls)
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
        let parameter_values = self.controls()?.into_iter()
            .map(|(name, (_, value))| Some((name, value?.clone())))
            .collect::<Option<BTreeMap<_, _>>>()?;
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
            json!({"bindings":bindings,"tables":tables,"metrics":metrics,"parameter_values":parameter_values,"preview":true}),
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
                    check_details_open: false,
                    controls: report_parameter_controls(definition).ok().map(|controls| {
                        controls.into_iter().map(|control| {
                            let value = definition["params"][control.slot - 1].clone();
                            (control, value)
                        }).collect()
                    }),
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
                parameter_values: BTreeMap::new(),
                review: None,
                message: String::new(),
                busy: false,
                generation: NEXT.fetch_add(1, Ordering::Relaxed),
            });
        }
        cx.notify();
    }

    pub(crate) fn open_assistant_report_refresh(
        &mut self, table: &str, files: BTreeMap<String, PathBuf>, cx: &mut Context<Self>,
    ) {
        self.close_ai_assistant_panel(cx);
        self.open_queries_panel(cx);
        self.open_query_report_panel(cx);
        let Some(panel) = self.query_reports_mut() else { return };
        for report in &mut panel.reports {
            report.selected = report.name == table;
        }
        panel.files = files;
        self.preview_query_reports(cx);
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
            "arrow_ipc" => &["arrow", "ipc", "feather"],
            "json_lines" => &["jsonl", "ndjson"],
            "avro" => &["avro"],
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
                    // Sheet commands can exceed the macOS dispatch-worker stack,
                    // particularly in debug builds. Keep this work off the UI thread.
                    std::thread::Builder::new()
                        .name("query-report-review".into())
                        .stack_size(8 * 1024 * 1024)
                        .spawn(move || -> Result<ReviewedRefresh, ibis_doc_service::ToolError> {
                            let definitions =
                                ibis_doc_service::batch_query_report_definitions(&base, &args)?;
                            let batch = ibis_doc_service::prepare_selected_report_batch(
                                &paths, args, definitions,
                            )?;
                            let (result, commands) = batch.review(&base)?;
                            Ok(ReviewedRefresh { base, result, commands })
                        })
                        .map_err(|error| ibis_doc_service::ToolError::new("internal", error.to_string()))?
                        .join()
                        .map_err(|_| ibis_doc_service::ToolError::new("internal", "Query report review worker failed"))?
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
        let panel = self.wb().panels.queries.as_ref()?.reports.as_ref()?;
        let workbook = self.presentation().workbook.as_ref()?;
        let generation = panel.generation;
        let busy = panel.busy;
        let mut can_preview = panel.request().is_some() && !busy;
        let can_apply = !read_only
            && !busy
            && panel
                .review
                .as_ref()
                .is_some_and(|review| !review.commands.is_empty());
        let mut body = v_flex().gap_2().px(gpui_kit::px(18.)).py(gpui_kit::px(8.));
        if let Some(controls) = panel.controls() {
            let owner = cx.entity().downgrade();
            for (index, (name, (control, selected))) in controls.into_iter().enumerate() {
                let choices = parameter_choices(self.presentation(), control);
                let mut label = None;
                let entries = choices.into_iter().map(|(value, display)| {
                    let checked = selected.is_some_and(|selected| {
                        if control.kind == ReportParameterKind::Number {
                            selected.as_f64() == value.as_f64()
                        } else {
                            selected == &value
                        }
                    });
                    if checked {
                        label = Some(display.clone());
                    }
                    let name = name.to_owned();
                    DropdownItem::new(display).checked(checked).on_select(std::rc::Rc::new(move |editor: &mut SlideEditor, _, cx| {
                        if let Some(panel) = editor.query_reports_mut()
                            .filter(|panel| !panel.busy && panel.generation == generation)
                        {
                            panel.parameter_values.insert(name.clone(), value.clone());
                            panel.invalidate();
                            cx.notify();
                        }
                    })).menu().into()
                }).collect::<Vec<_>>();
                can_preview &= label.is_some();
                if entries.is_empty() {
                    body = body.child(format!("{} · {}", control.label, text("query.report-parameter-error")));
                    continue;
                }
                body = body.child(control.label.clone()).child(
                    Dropdown::from_entries(
                        "query-report-parameter",
                        label.unwrap_or_else(|| text("query.report-parameter-choose").to_string()),
                        entries, true, index, &owner,
                    )
                        .fill_width().disabled(busy),
                );
            }
        } else {
            body = body.child(text("query.report-parameter-error"));
        }
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
            if let Some(checks) = workbook
                .query_reports
                .get(&report.name)
                .and_then(|definition| definition["checks"].as_array())
                .filter(|checks| !checks.is_empty())
            {
                let id = SharedString::from(format!("query-report-check-details-{index}"));
                let mut list = v_flex()
                    .gap_1()
                    .pl_4()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child(text("query.report-saved-checks")),
                    )
                    .child(
                        action_button(id)
                            .small()
                            .label(text(if report.check_details_open {
                                "query.report-hide-check-sql"
                            } else {
                                "query.report-show-check-sql"
                            }))
                            .on_click(cx.listener(move |editor, _, _, cx| {
                                if let Some(panel) = editor
                                    .query_reports_mut()
                                    .filter(|panel| panel.generation == generation)
                                    && let Some(report) = panel.reports.get_mut(index)
                                {
                                    report.check_details_open = !report.check_details_open;
                                    cx.notify();
                                }
                            })),
                    );
                for check in checks {
                    let Some(name) = check["name"].as_str() else {
                        continue;
                    };
                    let diagnostic = check["diagnostic"]["sql"].as_str();
                    let diagnostic_label = text(if diagnostic.is_some() {
                        "query.report-has-diagnostic"
                    } else {
                        "query.report-no-diagnostic"
                    });
                    let limit = check["max"].as_i64().map_or_else(
                        || text("query.report-check-informational").to_string(),
                        |max| format("query.report-check-limit", &[("max", &max)]),
                    );
                    list = list.child(div().text_sm().child(saved_check_text(format(
                        "query.report-saved-check",
                        &[
                            ("name", &name),
                            ("limit", &limit),
                            ("diagnostic", &diagnostic_label)
                        ],
                    ))));
                    if report.check_details_open {
                        for (label, sql) in [
                            ("query.report-count-sql", check["sql"].as_str()),
                            ("query.report-diagnostic-sql", diagnostic),
                        ] {
                            if let Some(sql) = sql {
                                list = list.child(
                                    v_flex()
                                        .pl_4()
                                        .text_sm()
                                        .child(
                                            div()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(text(label)),
                                        )
                                        .child(saved_check_text(sql.to_owned())),
                                );
                            }
                        }
                    }
                }
                body = body.child(list);
            }
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
                        action_button(SharedString::from(format!("query-report-source-{index}")))
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
                for change in report["schema_changes"].as_array().into_iter().flatten() {
                    let source = change["source"].as_str().unwrap_or_default();
                    let column = change["column"].as_str().unwrap_or_default();
                    let previous = change["previous"].as_str();
                    let current = change["current"].as_str();
                    let line = match (previous, current) {
                        (Some(previous), Some(current)) => format(
                            "query.report-schema-type",
                            &[
                                ("source", &source),
                                ("column", &column),
                                ("previous", &previous),
                                ("current", &current),
                            ],
                        ),
                        (None, Some(current)) => format(
                            "query.report-schema-added",
                            &[
                                ("source", &source),
                                ("column", &column),
                                ("current", &current),
                            ],
                        ),
                        (Some(previous), None) => format(
                            "query.report-schema-removed",
                            &[
                                ("source", &source),
                                ("column", &column),
                                ("previous", &previous),
                            ],
                        ),
                        (None, None) => continue,
                    };
                    body = body.child(div().text_sm().child(line));
                }
                let change_count = report["schema_change_count"].as_u64().unwrap_or(0);
                let shown = report["schema_changes"].as_array().map_or(0, Vec::len) as u64;
                if change_count > shown {
                    body = body.child(div().text_sm().child(format(
                        "query.report-schema-more",
                        &[("count", &(change_count - shown))],
                    )));
                }
                if report["input_schema_status"] == "unavailable" {
                    body = body.child(
                        div()
                            .text_sm()
                            .child(text("query.report-schema-unavailable")),
                    );
                }
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
                    let diagnostic = &check["diagnostic"];
                    if let Some(message) = diagnostic["error"]["message"].as_str() {
                        body = body.child(div().text_sm().child(format(
                            "query.report-diagnostic-error", &[("message", &message)],
                        )));
                    }
                    if let Some(rows) = diagnostic["rows"].as_array().filter(|rows| !rows.is_empty()) {
                        body = body.child(div().text_sm().child(diagnostic["columns"].to_string()));
                        for row in rows {
                            body = body.child(div().text_sm().child(row.to_string()));
                        }
                    }
                    if diagnostic["truncated"] == true {
                        body = body.child(div().text_sm().child(text("query.report-diagnostic-truncated")));
                    }
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

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn refresh_report_scenario() {
    use crate::shell::scenario::{Scenario, button_probe};
    use ibis_doc_model::Document;

    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("monthly.csv");
    let next = dir.path().join("next.csv");
    std::fs::write(&source, "code,amount\n001,24\n").unwrap();
    let mut scene = Scenario::new_document(ibis_formats::Kind::Workbook);
    scene.update(|editor, _, cx| {
        let slide = editor.current_slide;
        let export = ibis_doc_service::prepare_selected_datasets_export(
            std::slice::from_ref(&source),
            json!({
                "sql":"SELECT code, CAST(amount AS INTEGER) AS amount FROM monthly",
                "datasets":[{"source":"1","alias":"monthly"}],"anchor":"A1",
            }),
        ).unwrap();
        let mut draft = Document::new(editor.presentation().clone());
        let (_, commands) = export.stage(&mut draft, ibis_doc_service::DraftToolContext {
            workbook: ibis_doc_service::WorkbookToolContext { active_sheet: Some(slide), ..Default::default() },
            ..Default::default()
        }).unwrap();
        editor.document.execute(commands).unwrap();
    });

    let _probe = button_probe::Probe::new();
    let report = json!({"table":"QueryResult1"});
    let saved_checks = |scene: &mut Scenario| scene.with(|editor, _| {
        ibis_doc_service::query_report_definition(editor.presentation(), &report).unwrap()["checks"].clone()
    });
    let informational = json!([
        {"name":"行数","sql":"SELECT count(*) FROM monthly"},
        {"name":"重複コード","sql":"SELECT 0","max":1},
    ]);
    let blocking = json!([{
        "name":"重複コード",
        "sql":"SELECT count(*) FROM (SELECT code FROM monthly GROUP BY code HAVING count(*) > 1)",
        "max":0,
        "diagnostic":{"sql":"SELECT code, count(*) FROM monthly GROUP BY code HAVING count(*) > 1"},
    }]);
    let stage = |scene: &mut Scenario, checks: &Value| {
        scene.update(|editor, window, cx| {
            editor.stage_scenario_workbook_tool(
                "xlsx_set_query_report_checks",
                json!({"table":"QueryResult1","checks":checks}),
                window,
                cx,
            );
        });
    };
    let open_report = |scene: &mut Scenario| {
        scene.update(|editor, _, cx| {
            editor.close_ai_assistant_panel(cx);
            editor.open_queries_panel(cx);
        });
        button_probe::click(scene, "queries-report-refresh");
    };

    let visible = |value: &str| {
        button_probe::bounds(&format!("query-report-check-text:{value}")).is_some()
    };
    let initial_sql = informational[0]["sql"].as_str().unwrap();
    let initial_summary = format("query.report-saved-check", &[
        ("name", &"行数"),
        ("limit", &text("query.report-check-informational")),
        ("diagnostic", &text("query.report-no-diagnostic")),
    ]);
    let review_text = |fragment: &str| {
        button_probe::all().iter().any(|id| {
            id.starts_with("assistant-report-check:") && id.contains(fragment)
        })
    };
    stage(&mut scene, &informational);
    assert!(saved_checks(&mut scene).is_null());
    assert!(review_text("行数　追加"));
    button_probe::click(&mut scene, "ai-workbook-apply");
    assert_eq!(saved_checks(&mut scene), informational);
    open_report(&mut scene);
    assert!(visible(&initial_summary));
    assert!(!visible(initial_sql));
    button_probe::click(&mut scene, "query-report-check-details-0");
    assert!(visible(initial_sql));
    button_probe::click(&mut scene, "query-report-check-details-0");
    button_probe::click(&mut scene, "query-reports-back");
    button_probe::click(&mut scene, "queries-close");

    stage(&mut scene, &blocking);
    assert_eq!(saved_checks(&mut scene), informational);
    assert!(review_text("行数　削除"));
    assert!(review_text("重複コード　変更　許容上限 1 件・診断なし → 許容上限 0 件・診断あり"));
    std::fs::write(&next, "code,amount\n001,12\n001,12\n").unwrap();
    let bind_trial = |scene: &mut Scenario| {
        button_probe::click(scene, "assistant-check-binding-0-0");
        scene.answer_path_prompt(|options| {
            assert!(!options.multiple);
            Some(vec![next.clone()])
        });
    };
    bind_trial(&mut scene);
    let check_trial = |scene: &mut Scenario| {
        button_probe::clear();
        scene.update(|_, _, cx| cx.notify());
        button_probe::click(scene, "assistant-check-trial-0");
        for _ in 0..100 {
            if review_text("重複コード · 1 件 · 上限 0 件を超過") {
                assert!(review_text(r#"["001",2]"#));
                assert!(review_text("next.csv"));
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            scene.wait(20);
        }
        panic!("check trial result missing: {:?}", button_probe::all());
    };
    check_trial(&mut scene);
    assert_eq!(saved_checks(&mut scene), informational);
    let before_sql = "変更前\n件数チェックのSQL\nSELECT 0";
    assert!(!review_text(before_sql));
    button_probe::click(&mut scene, "assistant-report-check-sql");
    assert!(review_text(before_sql));
    assert!(review_text(blocking[0]["sql"].as_str().unwrap()));
    assert!(review_text(blocking[0]["diagnostic"]["sql"].as_str().unwrap()));
    button_probe::click(&mut scene, "assistant-report-check-sql");
    button_probe::click(&mut scene, "ai-workbook-discard");
    assert_eq!(saved_checks(&mut scene), informational);
    assert!(button_probe::bounds("assistant-report-refresh-handoff").is_none());

    let wait_for_review = |scene: &mut Scenario| {
        for _ in 0..100 {
            let (ready, busy, message) = scene.with(|editor, _| {
                let panel = editor
                    .wb()
                    .panels
                    .queries
                    .as_ref()
                    .unwrap()
                    .reports
                    .as_ref()
                    .unwrap();
                (panel.review.is_some(), panel.busy, panel.message.clone())
            });
            if ready {
                return;
            }
            assert!(busy, "report preview failed: {message}");
            std::thread::sleep(std::time::Duration::from_millis(20));
            scene.wait(20);
        }
        panic!("report preview did not complete");
    };
    stage(&mut scene, &blocking);
    bind_trial(&mut scene);
    check_trial(&mut scene);
    button_probe::click(&mut scene, "ai-workbook-apply");
    assert_eq!(saved_checks(&mut scene), blocking);

    // The trial failed on duplicated rows. The handoff rereads the same path
    // after the source is repaired and starts a new preview without editing.
    std::fs::write(&next, "code,amount\n001,18\n").unwrap();
    button_probe::clear();
    scene.update(|_, _, cx| cx.notify());
    button_probe::click(&mut scene, "assistant-report-refresh-handoff");
    wait_for_review(&mut scene);
    scene.update(|editor, _, _| {
        let panel = editor.query_reports_mut().unwrap();
        assert_eq!(panel.files.get("monthly.csv"), Some(&next));
        let review = panel.review.as_ref().unwrap();
        assert_eq!(review.result["blocked"], false);
        assert_eq!(review.result["reports"][0]["checks"][0]["count"], 0);
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(
            editor.presentation().slide(editor.current_slide).unwrap()
        ).unwrap();
        assert_eq!(sheet.cell(1, 1).unwrap().value, CellValue::Number(24.0));
    });
    button_probe::click(&mut scene, "query-reports-back");
    button_probe::click(&mut scene, "queries-report-refresh");
    let updated_sql = blocking[0]["sql"].as_str().unwrap();
    let diagnostic_sql = blocking[0]["diagnostic"]["sql"].as_str().unwrap();
    let updated_summary = format("query.report-saved-check", &[
        ("name", &"重複コード"),
        ("limit", &format("query.report-check-limit", &[("max", &0)])),
        ("diagnostic", &text("query.report-has-diagnostic")),
    ]);
    assert!(visible(&updated_summary));
    assert!(!visible(&initial_summary));
    assert!(!visible(updated_sql));
    assert!(!visible(diagnostic_sql));

    let details_open = |scene: &mut Scenario| {
        scene.with(|editor, _| {
            editor
                .wb()
                .panels
                .queries
                .as_ref()
                .unwrap()
                .reports
                .as_ref()
                .unwrap()
                .reports[0]
                .check_details_open
        })
    };
    assert!(!details_open(&mut scene));
    button_probe::click(&mut scene, "query-report-check-details-0");
    assert!(details_open(&mut scene));
    assert!(visible(updated_sql));
    assert!(visible(diagnostic_sql));
    assert!(!visible(initial_sql));
    button_probe::click(&mut scene, "query-report-check-details-0");
    assert!(!details_open(&mut scene));

    button_probe::click(&mut scene, "query-report-source-0");
    scene.answer_path_prompt(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        assert_eq!(options.extensions, ["csv"]);
        None
    });
    scene.with(|editor, _| {
        let panel = editor
            .wb()
            .panels
            .queries
            .as_ref()
            .unwrap()
            .reports
            .as_ref()
            .unwrap();
        assert!(!panel.busy);
        assert!(panel.files.is_empty());
    });
    button_probe::click(&mut scene, "query-report-source-0");
    scene.answer_path_prompt(|_| Some(vec![next.clone()]));
    scene.update(|editor, _, cx| {
        let panel = editor.query_reports_mut().unwrap();
        assert_eq!(panel.files.get("monthly.csv"), Some(&next));
        panel.reports[0].metrics.insert("amount".into());
        cx.notify();
    });
    let amount = |scene: &mut Scenario| {
        scene.with(|editor, _| {
            let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(
                editor.presentation().slide(editor.current_slide).unwrap(),
            )
            .unwrap();
            sheet.cell(1, 1).map(|cell| cell.value.clone())
        })
    };
    std::fs::write(&next, "code,amount\n001,12\n001,12\n").unwrap();
    button_probe::click(&mut scene, "query-reports-preview");
    wait_for_review(&mut scene);
    scene.with(|editor, _| {
        let panel = editor
            .wb()
            .panels
            .queries
            .as_ref()
            .unwrap()
            .reports
            .as_ref()
            .unwrap();
        let review = panel.review.as_ref().unwrap();
        assert_eq!(review.result["blocked"], true);
        assert_eq!(review.result["reports"][0]["checks"][0]["count"], 1);
        assert_eq!(review.result["reports"][0]["checks"][0]["diagnostic"]["rows"], json!([["001",2]]));
        assert!(review.commands.is_empty());
    });
    assert_eq!(amount(&mut scene), Some(CellValue::Number(24.0)));

    std::fs::write(&next, "code,amount,region\n001,12,north\n").unwrap();
    button_probe::click(&mut scene, "query-reports-preview");
    wait_for_review(&mut scene);
    scene.with(|editor, _| {
        let panel = editor
            .wb()
            .panels
            .queries
            .as_ref()
            .unwrap()
            .reports
            .as_ref()
            .unwrap();
        let review = panel.review.as_ref().unwrap();
        assert_eq!(review.result["blocked"], false);
        assert_eq!(review.result["reports"][0]["metrics"][0]["delta"], -12.0);
        assert_eq!(
            review.result["reports"][0]["schema_changes"],
            json!([{"source":"monthly","column":"region","previous":null,"current":"TEXT"}]),
        );
    });
    // Apply what appeared in the review even if the source is replaced meanwhile.
    std::fs::write(&next, "code,amount\n001,6\n").unwrap();
    button_probe::click(&mut scene, "query-reports-apply");
    assert_eq!(amount(&mut scene), Some(CellValue::Number(12.0)));
    button_probe::click(&mut scene, "query-reports-back");
    button_probe::click(&mut scene, "queries-close");
    let cell = scene.with(|editor, _| editor.cell_center(3, 0).unwrap());
    scene.click((f32::from(cell.x), f32::from(cell.y)));
    scene.key(&format!("{}-z", crate::shell::app::secondary_modifier()));
    assert_eq!(amount(&mut scene), Some(CellValue::Number(24.0)));
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn arrow_report_snapshot_scenario() {
    use crate::shell::scenario::{Scenario, button_probe};
    use ibis_doc_model::Document;

    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("lookup.ipc");
    let small = include_bytes!("../../ibis-doc-service/tests/fixtures/query-lookup.ipc");
    let large = include_bytes!("../../ibis-doc-service/tests/fixtures/query-large-lookup.ipc");
    std::fs::write(&source, small).unwrap();

    let mut scene = Scenario::new_document(ibis_formats::Kind::Workbook);
    scene.update(|editor, _, cx| {
        for (anchor, sql) in [
            ("A1", "SELECT count(*) AS n FROM lookup"),
            ("D1", "SELECT count(code) AS n FROM lookup"),
        ] {
            let mut draft = Document::new(editor.presentation().clone());
            let export = ibis_doc_service::prepare_selected_datasets_export(
                std::slice::from_ref(&source),
                json!({"sql":sql,"anchor":anchor,"datasets":[
                    {"source":"1","alias":"lookup","columns":["code","labels"]},
                ]}),
            ).unwrap();
            let (_, commands) = export.stage(&mut draft, ibis_doc_service::DraftToolContext {
                workbook: ibis_doc_service::WorkbookToolContext {
                    active_sheet: Some(editor.current_slide), ..Default::default()
                },
                ..Default::default()
            }).unwrap();
            editor.document.execute(commands).unwrap();
        }
        editor.open_queries_panel(cx);
    });

    let _probe = button_probe::Probe::new();
    scene.update(|_, _, cx| cx.notify());
    button_probe::click(&mut scene, "queries-report-refresh");
    scene.update(|editor, _, cx| {
        let panel = editor.query_reports_mut().unwrap();
        panel.files.insert("lookup.ipc".into(), source.clone());
        for report in &mut panel.reports {
            report.metrics.insert("n".into());
        }
        cx.notify();
    });

    let preview = |scene: &mut Scenario, expected: u32| {
        button_probe::click(scene, "query-reports-preview");
        for _ in 0..500 {
            let (result, busy, message) = scene.with(|editor, _| {
                let panel = editor.wb().panels.queries.as_ref().unwrap().reports.as_ref().unwrap();
                (panel.review.as_ref().map(|review| review.result.clone()), panel.busy, panel.message.clone())
            });
            if let Some(result) = result {
                let reports = result["reports"].as_array().unwrap();
                assert_eq!(reports.len(), 2);
                for report in reports {
                    assert_eq!(report["metrics"][0]["current"].as_f64(), Some(f64::from(expected)));
                }
                return;
            }
            assert!(busy, "report preview failed: {message}");
            std::thread::sleep(std::time::Duration::from_millis(20));
            scene.wait(20);
        }
        panic!("Arrow report preview did not complete");
    };

    preview(&mut scene, 4);
    std::fs::write(&source, large).unwrap();
    preview(&mut scene, 50_001);
    std::fs::write(&source, small).unwrap();
    button_probe::click(&mut scene, "query-reports-apply");
    scene.with(|editor, _| {
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(
            editor.presentation().slide(editor.current_slide).unwrap(),
        ).unwrap();
        for column in [0, 3] {
            assert_eq!(sheet.cell(1, column).map(|cell| &cell.value), Some(&CellValue::Number(50_001.0)));
        }
    });
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn json_avro_report_snapshot_scenario() {
    use crate::shell::scenario::{Scenario, button_probe};
    use ibis_doc_model::{Document, query::{DATASET_MEMORY_LIMIT, DiskDataset}};

    let dir = tempfile::tempdir().unwrap();
    let original = [
        dir.path().join("labels.jsonl"),
        dir.path().join("observations.avro"),
    ];
    let bindings = [
        dir.path().join("selected-labels.ndjson"),
        dir.path().join("selected-observations.avro"),
    ];
    let small_json = b"{\"code\":\"001\",\"labels\":[\"a\",null]}\n{\"code\":\"002\",\"labels\":[]}\n";
    let small_avro = include_bytes!("../../ibis-doc-service/tests/fixtures/query-observations.avro");
    let large_avro =
        include_bytes!("../../ibis-doc-service/tests/fixtures/query-large-observations.avro");
    for (file, contents) in original.iter().zip([&small_json[..], &small_avro[..]]) {
        std::fs::write(file, contents).unwrap();
    }
    std::fs::write(&bindings[0], small_json).unwrap();
    std::fs::write(&bindings[1], small_avro).unwrap();

    let mut scene = Scenario::new_document(ibis_formats::Kind::Workbook);
    scene.update(|editor, _, cx| {
        for (anchor, sql) in [
            ("A1", "SELECT count(*) AS n FROM labels"),
            ("D1", "SELECT sum(amount) AS n FROM facts"),
        ] {
            let mut draft = Document::new(editor.presentation().clone());
            let export = ibis_doc_service::prepare_selected_datasets_export(
                &original,
                json!({"sql":sql,"anchor":anchor,"datasets":[
                    {"source":"1","alias":"labels","columns":["code","labels"]},
                    {"source":"2","alias":"facts","columns":["code","amount"]},
                ]}),
            ).unwrap();
            let (_, commands) = export.stage(&mut draft, ibis_doc_service::DraftToolContext {
                workbook: ibis_doc_service::WorkbookToolContext {
                    active_sheet: Some(editor.current_slide), ..Default::default()
                },
                ..Default::default()
            }).unwrap();
            editor.document.execute(commands).unwrap();
        }
        editor.open_queries_panel(cx);
    });

    let _probe = button_probe::Probe::new();
    scene.update(|_, _, cx| cx.notify());
    button_probe::click(&mut scene, "queries-report-refresh");
    for (index, (extensions, file)) in [
        (&["jsonl", "ndjson"][..], &bindings[0]),
        (&["avro"][..], &bindings[1]),
    ].into_iter().enumerate() {
        button_probe::click(&mut scene, &format!("query-report-source-{index}"));
        scene.answer_path_prompt(|options| {
            assert!(options.files && !options.directories && !options.multiple);
            assert!(options.extensions.iter().map(String::as_str).eq(extensions.iter().copied()));
            Some(vec![file.clone()])
        });
    }
    scene.update(|editor, _, cx| {
        let panel = editor.query_reports_mut().unwrap();
        assert_eq!(panel.files.get("labels.jsonl"), Some(&bindings[0]));
        assert_eq!(panel.files.get("observations.avro"), Some(&bindings[1]));
        for report in &mut panel.reports {
            report.metrics.insert("n".into());
        }
        cx.notify();
    });

    let preview = |scene: &mut Scenario, expected: [f64; 2]| {
        button_probe::click(scene, "query-reports-preview");
        for _ in 0..500 {
            let (result, busy, message) = scene.with(|editor, _| {
                let panel = editor.wb().panels.queries.as_ref().unwrap().reports.as_ref().unwrap();
                (panel.review.as_ref().map(|review| review.result.clone()), panel.busy, panel.message.clone())
            });
            if let Some(result) = result {
                assert_eq!(result["blocked"], false);
                let reports = result["reports"].as_array().unwrap();
                assert_eq!(reports.len(), expected.len());
                for (report, expected) in reports.iter().zip(expected) {
                    assert_eq!(report["metrics"][0]["current"].as_f64(), Some(expected));
                }
                return;
            }
            assert!(busy, "report preview failed: {message}");
            std::thread::sleep(std::time::Duration::from_millis(20));
            scene.wait(20);
        }
        panic!("JSON Lines and Avro report preview did not complete");
    };
    let values = |scene: &mut Scenario| {
        scene.with(|editor, _| {
            let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(
                editor.presentation().slide(editor.current_slide).unwrap(),
            ).unwrap();
            [0, 3].map(|column| sheet.cell(1, column).unwrap().value.clone())
        })
    };

    preview(&mut scene, [2., 5.]);
    // Each source has 50,001 rows times two selected columns. Either source
    // independently exceeds the 100,000-cell in-memory input budget.
    let large_json = b"{\"code\":\"001\",\"labels\":[]}\n".repeat(50_001);
    std::fs::write(&bindings[0], &large_json).unwrap();
    let json_memory = DiskDataset::JsonLines(bindings[0].clone(), vec!["code".into(), "labels".into()]);
    assert_eq!(json_memory.collect_bounded(100_000, 8 * 1024 * 1024).unwrap_err(), DATASET_MEMORY_LIMIT);
    preview(&mut scene, [50_001., 5.]);
    std::fs::write(&bindings[0], small_json).unwrap();
    std::fs::write(&bindings[1], large_avro).unwrap();
    let avro_memory = DiskDataset::Avro(bindings[1].clone(), vec!["code".into(), "amount".into()]);
    assert_eq!(avro_memory.collect_bounded(100_000, 8 * 1024 * 1024).unwrap_err(), DATASET_MEMORY_LIMIT);
    preview(&mut scene, [2., 250_005.]);
    std::fs::write(&bindings[0], &large_json).unwrap();
    preview(&mut scene, [50_001., 250_005.]);
    assert_eq!(values(&mut scene), [CellValue::Number(2.), CellValue::Number(5.)]);
    scene.screenshot("json-avro-preview");

    std::fs::write(&bindings[0], small_json).unwrap();
    std::fs::write(&bindings[1], small_avro).unwrap();
    button_probe::click(&mut scene, "query-reports-apply");
    assert_eq!(values(&mut scene), [CellValue::Number(50_001.), CellValue::Number(250_005.)]);
    scene.screenshot("json-avro-applied");
    button_probe::click(&mut scene, "query-reports-back");
    button_probe::click(&mut scene, "queries-close");
    let cell = scene.with(|editor, _| editor.cell_center(3, 0).unwrap());
    scene.click((f32::from(cell.x), f32::from(cell.y)));
    scene.key(&format!("{}-z", crate::shell::app::secondary_modifier()));
    assert_eq!(values(&mut scene), [CellValue::Number(2.), CellValue::Number(5.)]);
    scene.screenshot("json-avro-undone");
}
