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

fn signed_contribution(value: &Value) -> String {
    value.as_f64().map_or_else(|| "—".to_owned(), |number| {
        if number != 0.0 && !(0.000001..1e9).contains(&number.abs()) {
            format!("{number:+.6e}")
        } else {
            format!("{number:+.6}").trim_end_matches('0').trim_end_matches('.').to_owned()
        }
    })
}

fn quotient_preview(
    comparison: &Value, page: Option<&Value>, index: usize, selected: Option<usize>, generation: u64, cx: &mut Context<SlideEditor>,
) -> impl IntoElement {
    let rate = comparison["kind"] == "rate";
    let unit = if rate { text("query.report-quotient-pp").to_string() }
        else { comparison["unit"].as_str().unwrap_or_default().to_owned() };
    let heading = format("query.report-quotient-heading", &[
        ("table", &comparison["table"].as_str().unwrap_or_default()),
        ("metric", &comparison["metric"].as_str().unwrap_or_default()),
    ]);
    let mut section = v_flex().gap_1().child(div().text_sm().font_weight(FontWeight::BOLD).child(heading));
    let decomposition = &comparison["decomposition"];
    let ranking = &decomposition["ranking"];
    let Some(groups) = ranking["groups"].as_array() else {
        return section.child(div().text_sm().child(text("query.report-quotient-unavailable")));
    };
    if decomposition["state"] == "totals_mismatch" {
        section = section.child(div().text_sm().child(text("query.report-quotient-mismatch")));
    }
    section = section.child(div().text_sm().child(format("query.report-quotient-total", &[
        ("count", &ranking["group_count"]), ("total", &signed_contribution(&ranking["total_contribution"])), ("unit", &unit),
    ])));
    let remainder = &ranking["remainder"];
    let page_groups = page.and_then(|page| page["groups"].as_array()).into_iter().flatten();
    let remainder_count = remainder["count"].as_u64().unwrap_or_default();
    if remainder_count > 0 {
        section = section.child(div().text_sm().child(saved_check_text(format("query.report-quotient-top", &[
            ("count", &groups.len()), ("total", &signed_contribution(&ranking["top_contribution"])), ("unit", &unit),
        ]))));
    }
    let page_offset = page.and_then(|page| page["offset"].as_u64()).unwrap_or_default() as usize;
    for (position, group) in groups.iter().chain(page_groups).enumerate() {
        if position == groups.len() {
            section = section.child(quotient_remainder_button(remainder, true, &unit, index, generation, cx));
        }
        let row = if position < groups.len() { position } else { position + page_offset };
        let key = group["key"].as_str().unwrap_or_default();
        let rank = row + 1;
        let total = signed_contribution(&group["contribution"]);
        let label = format("query.report-quotient-row", &[("rank", &rank), ("key", &key), ("total", &total), ("unit", &unit)]);
        let id = SharedString::from(format!("query-report-quotient-{index}-{row}"));
        let button = action_button(id).small().label(label.clone()).on_click(cx.listener(move |editor, _, _, cx| {
            if let Some(review) = editor.query_reports_mut().filter(|panel| panel.generation == generation)
                .and_then(|panel| panel.review.as_mut())
            {
                if review.quotient_selection.get(&index) == Some(&row) {
                    review.quotient_selection.remove(&index);
                } else {
                    review.quotient_selection.insert(index, row);
                }
                cx.notify();
            }
        }));
        #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
        let button = crate::shell::scenario::button_probe::observe_control(div().id(SharedString::from(format!("quotient-row-probe-{index}-{row}"))).child(button), SharedString::from(format!("query-report-quotient-row:{label}")).into());
        section = section.child(button).child(div().text_sm().child(saved_check_text(format("query.report-quotient-parts", &[
            ("within", &signed_contribution(&group["within_group"])),
            ("composition", &signed_contribution(&group["composition"])),
        ]))));
        if selected == Some(row) {
            let fields = if rate { ["numerator", "denominator"] } else { ["sum", "count"] };
            for (side, side_label) in [("before", "query.report-quotient-before"), ("after", "query.report-quotient-after")] {
                let evidence = &group[side];
                section = section.child(div().text_sm().font_weight(FontWeight::BOLD).child(text(side_label)));
                let Some(sheet) = evidence["sheet"].as_str() else {
                    section = section.child(div().text_sm().child(text("query.report-quotient-absent")));
                    continue;
                };
                for field in fields {
                    let Some(cell) = evidence[field]["cell"].as_str() else { continue };
                    let name = format!("query.report-quotient-{field}");
                    let value = &evidence[field]["value"];
                    let value = if value.is_null() { "—".to_owned() } else { value.to_string() };
                    let line = format("query.report-quotient-evidence", &[
                        ("field", &text(&name)), ("sheet", &sheet), ("cell", &cell), ("value", &value),
                    ]);
                    if side == "before" {
                        let reference = format!("'{}'!{cell}", sheet.replace('\'', "''"));
                        let button = action_button(SharedString::from(format!("query-report-quotient-cell-{index}-{field}")))
                            .small().label(line.clone()).on_click(cx.listener(move |editor, _, window, cx| {
                                editor.go_to_reference(&reference, window, cx);
                            }));
                        #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
                        let button = crate::shell::scenario::button_probe::observe_control(div().id(SharedString::from(format!("quotient-evidence-probe-{index}-{field}"))).child(button), SharedString::from(format!("query-report-quotient-evidence:{line}")).into());
                        section = section.child(button);
                    } else {
                        section = section.child(div().text_sm().child(saved_check_text(line)));
                    }
                }
            }
        }
    }
    if remainder_count > 0 && page.is_none() {
        section = section.child(quotient_remainder_button(remainder, false, &unit, index, generation, cx));
    }
    if let Some(page) = page {
        let size = page["page_size"].as_u64().unwrap_or_default() as usize;
        let next = page["next_offset"].as_u64().map(|offset| offset as usize);
        for (action, offset) in [
            ("previous", (page_offset > 0).then(|| page_offset.saturating_sub(size))),
            ("next", next),
        ] {
            if let Some(offset) = offset {
                section = section.child(action_button(SharedString::from(format!("query-report-quotient-{action}-{index}")))
                    .small().label(text(&format!("query.report-quotient-{action}")))
                    .on_click(cx.listener(move |editor, _, _, cx| {
                        editor.set_quotient_remainder_page(index, generation, Some(offset), cx);
                    })));
            }
        }
    }
    section
}

fn quotient_remainder_button(
    remainder: &Value, expanded: bool, unit: &str, index: usize, generation: u64, cx: &mut Context<SlideEditor>,
) -> impl IntoElement {
    let label = format(if expanded { "query.report-quotient-remainder-expanded" } else { "query.report-quotient-remainder" }, &[
        ("count", &remainder["count"]), ("total", &signed_contribution(&remainder["contribution"])), ("unit", &unit),
    ]);
    let button = action_button(SharedString::from(format!("query-report-quotient-remainder-{index}")))
        .small().label(label.clone()).on_click(cx.listener(move |editor, _, _, cx| {
            editor.set_quotient_remainder_page(index, generation, (!expanded).then_some(0), cx);
        }));
    #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
    let button = crate::shell::scenario::button_probe::observe_control(
        div().id(SharedString::from(format!("quotient-remainder-probe-{index}"))).child(button),
        SharedString::from(format!("query-report-quotient-remainder:{label}")).into(),
    );
    button
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
    candidate: Option<Presentation>,
    chart_diffs: Vec<Value>,
    chart_diffs_truncated: bool,
    quotient_previews: Vec<Value>,
    quotient_selection: BTreeMap<usize, usize>,
    quotient_pages: BTreeMap<usize, Value>,
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
    fn set_quotient_remainder_page(&mut self, index: usize, generation: u64, offset: Option<usize>, cx: &mut Context<Self>) {
        let page = offset.and_then(|offset| {
            let panel = self.wb().panels.queries.as_ref()?.reports.as_ref()?;
            let review = panel.review.as_ref()?;
            if panel.generation != generation || self.presentation() != &review.base {
                return None;
            }
            let comparison = review.quotient_previews.get(index)?;
            let table = comparison["table"].as_str()?;
            let metric = comparison["metric"].as_str()?;
            let candidate = review.candidate.as_ref()?;
            let result = crate::report_quotient::compare_with_remainder(&review.base, candidate, table, metric, None, Some(offset));
            result["decomposition"]["ranking"]["remainder"].get("page").cloned()
        });
        if let Some(review) = self.query_reports_mut().filter(|panel| panel.generation == generation)
            .and_then(|panel| panel.review.as_mut())
        {
            review.quotient_selection.remove(&index);
            if let Some(page) = page {
                review.quotient_pages.insert(index, page);
            } else {
                review.quotient_pages.remove(&index);
            }
            cx.notify();
        }
    }

    pub(crate) fn live_report_preview_diff(&self, args: &Value) -> Value {
        let contributor_key = match args.get("contributor_key") {
            None => None,
            Some(key) => match key.as_str().filter(|key| !key.trim().is_empty()) {
                Some(key) => Some(key),
                None => return json!({"error":"Supply a nonempty contributor key"}),
            },
        };
        let report = match (args.get("table"), args.get("metric")) {
            (None, None) => None,
            (Some(table), Some(metric)) => {
                let (Some(table), Some(metric)) = (table.as_str(), metric.as_str()) else {
                    return json!({"error":"Supply a report table and metric"});
                };
                if [table, metric].iter().any(|value| value.trim().is_empty())
                    || ["shape_id", "series", "category"].iter().any(|key| args.get(key).is_some())
                {
                    return json!({"error":"Supply a report table and metric without chart arguments"});
                }
                Some((table, metric))
            }
            _ => return json!({"error":"Supply a report table and metric together"}),
        };
        let remainder_offset = match args.get("remainder_offset") {
            None => None,
            Some(offset) => {
                let Some(offset) = offset.as_u64().and_then(|offset| usize::try_from(offset).ok()) else {
                    return json!({"error":"Supply a non-negative integer remainder_offset"});
                };
                if report.is_none() || args["applied"] == true {
                    return json!({"error":"remainder_offset requires a rate or average report preview"});
                }
                Some(offset)
            }
        };
        if let Some(applied) = args.get("applied") {
            let (Some(applied), Some((table, metric))) = (applied.as_bool(), report) else {
                return json!({"error":"applied must be a boolean and requires a report table and metric"});
            };
            if applied {
                let Some(key) = contributor_key else {
                    return json!({"error":"Applied report lookup requires a contributor key"});
                };
                return crate::chart_cohort::applied_report_row(self.presentation(), table, metric, key);
            }
        }
        let Some(panel) = self.wb().panels.queries.as_ref().and_then(|panel| panel.reports.as_ref()) else {
            return json!({"state":"idle"});
        };
        if panel.busy {
            return json!({"state":"previewing"});
        }
        let Some(review) = &panel.review else { return json!({"state":"idle"}) };
        if self.presentation() != &review.base {
            return json!({"state":"stale"});
        }
        let Some(candidate) = &review.candidate else { return json!({"state":"blocked"}) };
        if let Some((table, metric)) = report {
            if !review.result["reports"].as_array().into_iter().flatten().any(|report| {
                report["table"].as_str().is_some_and(|name| name.eq_ignore_ascii_case(table))
            }) {
                return json!({"state":"report_not_in_preview"});
            }
            if remainder_offset.is_some() {
                if !crate::chart_cohort::report_comparison_definition(&review.base, table).is_ok_and(crate::report_quotient::supports) {
                    return json!({"error":"remainder_offset requires a rate or average report preview"});
                }
                return crate::report_quotient::compare_with_remainder(&review.base, candidate, table, metric, contributor_key, remainder_offset);
            }
            return crate::chart_cohort::compare_report(&review.base, candidate, table, metric, contributor_key);
        }
        match (args.get("shape_id"), args.get("series"), args.get("category")) {
            (None, None, None) if contributor_key.is_none() => json!({
                "state":"review","charts":review.chart_diffs,"truncated":review.chart_diffs_truncated
            }),
            (Some(shape), Some(series), Some(category)) => {
                let (Some(shape), Some(series), Some(category)) = (shape.as_u64(), series.as_str(), category.as_str()) else {
                    return json!({"error":"Supply a chart shape ID, series label and category label"});
                };
                crate::chart_preview_diff::detail(&review.base, candidate, ibis_doc_model::ShapeId(shape), (series, category), contributor_key)
            }
            _ => json!({"error":"Supply a chart shape ID, series label and category label together"}),
        }
    }

    pub(crate) fn live_report_analysis(&mut self, request: Option<&Value>, cx: &mut Context<Self>) -> Value {
        let Some(book) = self.presentation().workbook.as_ref() else {
            return json!({"error":"No workbook is open"});
        };
        let applied: BTreeMap<_, BTreeMap<_, _>> = book.query_reports.iter().map(|(table, definition)| {
            let values = report_parameter_controls(definition).unwrap_or_default().into_iter()
                .map(|control| (control.name, definition["params"][control.slot - 1].clone()))
                .collect();
            (table.clone(), values)
        }).collect();

        if let Some(request) = request {
            let Some(values) = request.as_object().filter(|v| (1..=12).contains(&v.len())) else {
                return json!({"error":"Supply 1 to 12 named parameter values"});
            };
            let Some(panel) = self.wb().panels.queries.as_ref().and_then(|p| p.reports.as_ref()) else {
                return json!({"error":"Open SQL Report Refresh in the workbook and assign the source files"});
            };
            if panel.busy || panel.review.is_some() {
                return json!({"error":"Finish or close the current report preview first"});
            }
            if panel.sources().keys().any(|name| !panel.files.contains_key(name)) {
                return json!({"error":"Assign all selected source files in SQL Report Refresh"});
            }
            let Some(controls) = panel.controls() else {
                return json!({"error":"The selected reports have incompatible parameters"});
            };
            for (name, value) in values {
                if !controls.get(name.as_str()).is_some_and(|(control, _)| {
                    control.kind.accepts(value)
                        && parameter_choices(self.presentation(), control).iter().any(|(candidate, _)| {
                            candidate == value || (control.kind == ReportParameterKind::Number && candidate.as_f64() == value.as_f64())
                        })
                }) {
                    return json!({"error":format!("Unknown parameter or invalid choice: {name}")});
                }
            }

            let panel = self.query_reports_mut().expect("report panel is open");
            for report in &mut panel.reports {
                if let Some(controls) = &mut report.controls {
                    for (control, saved) in controls {
                        if let Some(value) = applied.get(&report.name).and_then(|p| p.get(&control.name)) {
                            *saved = value.clone();
                        }
                    }
                }
            }
            let old = std::mem::replace(&mut panel.parameter_values, values.clone().into_iter().collect());
            if panel.request().is_none() {
                panel.parameter_values = old;
                return json!({"error":"Select compatible reports with an unambiguous applied value for every other parameter"});
            }
            self.preview_query_reports(cx);
            return json!({"state":"previewing"});
        }

        let selected_bar = (|| {
            let (file, slide, shape, series, point) = self.selected_chart_bar?;
            if file != self.file_id || slide != self.current_slide || self.selection != Some(shape) {
                return None;
            }
            let ibis_doc_model::ShapeKind::Chart(chart) = &self.slide().shape(shape)?.kind else { return None };
            let data = chart.groups.iter().flat_map(|g| &g.series).nth(series)?;
            let refs = chart.cell_refs.get(series)?;
            let cell = |reference: &str| {
                let reference = crate::sheet_ui::parse_reference(reference)?;
                let (slide, range) = self.resolve_reference(&reference)?;
                let (column, row) = crate::event_hooks::chart_source_cell(range, point)?;
                Some(json!({"sheet":self.presentation().slide(slide)?.name,"cell":ibis_doc_model::cell_name(column,row)}))
            };
            Some(json!({
                "shape_id":shape.0,"series_index":series,"point_index":point,
                "title":chart.title.as_ref().map(|title| &title.text),
                "series":data.name,"category":data.categories.get(point),
                "category_cell":refs.categories.as_deref().and_then(cell),
                "value_cell":refs.values.as_deref().and_then(cell),
            }))
        })();
        let preview = self.wb().panels.queries.as_ref().and_then(|p| p.reports.as_ref()).map(|panel| {
            if panel.busy {
                json!({"state":"previewing"})
            } else if let Some(review) = &panel.review {
                if self.presentation() != &review.base {
                    json!({"state":"stale"})
                } else {
                    json!({
                        "state":if review.result["blocked"] == true {"blocked"} else {"review"},
                        "charts":review.result["charts"],
                        "reports":review.result["reports"].as_array().into_iter().flatten().map(|r| {
                            let comparison = r["table"].as_str().and_then(|table| book.query_reports.get(table)).and_then(|definition| definition.get("comparison"));
                            json!({"table":r["table"],"sheet":r["sheet"],"previous_rows":r["previous_rows"],"rows":r["rows"],"comparison":comparison})
                        }).collect::<Vec<_>>(),
                    })
                }
            } else {
                json!({"state":"idle","message":panel.message})
            }
        });
        json!({"sheet":self.slide().name,"selected_bar":selected_bar,"applied_parameters":applied,"report_preview":preview})
    }

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
                            let (result, commands, candidate) = batch.review_with_candidate(&base)?;
                            let (chart_diffs, chart_diffs_truncated) = candidate.as_ref().map(|candidate| {
                                crate::chart_preview_diff::summary(&base, candidate, &result["charts"])
                            }).unwrap_or_default();
                            let quotient_previews = candidate.as_ref().map(|candidate| {
                                result["reports"].as_array().into_iter().flatten().filter_map(|report| {
                                    let table = report["table"].as_str()?;
                                    let definition = crate::chart_cohort::report_comparison_definition(&base, table).ok()?;
                                    if !crate::report_quotient::supports(definition) {
                                        return None;
                                    }
                                    let metric = definition["metric"].as_str()?;
                                    let mut preview = crate::report_quotient::compare(&base, candidate, table, metric, None);
                                    preview["table"] = json!(table);
                                    preview["metric"] = json!(metric);
                                    Some(preview)
                                }).collect()
                            }).unwrap_or_default();
                            Ok(ReviewedRefresh { base, result, commands, candidate, chart_diffs, chart_diffs_truncated,
                                quotient_previews, quotient_selection: BTreeMap::new(), quotient_pages: BTreeMap::new() })
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
            for chart in review.result["charts"].as_array().into_iter().flatten() {
                let label = if chart["changed"] == true { "query.report-chart-changed" } else { "query.report-chart-unchanged" };
                body = body.child(div().text_sm().child(format(label, &[
                    ("chart", &chart["title"].as_str().unwrap_or_default()),
                ])));
            }
            if self.presentation() == &review.base {
                for (index, comparison) in review.quotient_previews.iter().enumerate() {
                    body = body.child(quotient_preview(comparison, review.quotient_pages.get(&index), index,
                        review.quotient_selection.get(&index).copied(), generation, cx));
                }
                for chart in &review.chart_diffs {
                    for change in chart["top"].as_array().into_iter().flatten() {
                        body = body.child(div().text_sm().child(format("query.report-chart-delta", &[
                            ("chart", &chart["title"].as_str().unwrap_or_default()),
                            ("category", &change["category"].as_str().unwrap_or_default()),
                            ("series", &change["series"].as_str().unwrap_or_default()),
                            ("before", &change["amounts"]["before"]),
                            ("after", &change["amounts"]["after"]),
                            ("delta", &change["amounts"]["delta"]),
                        ])));
                        if change["cohort"]["state"] == "ready" {
                            let cohort = &change["cohort"];
                            body = body.child(div().text_sm().child(format("query.report-cohort-delta", &[
                                ("retained_count", &cohort["retained"]["count"]),
                                ("retained_delta", &cohort["retained"]["delta"]),
                                ("added_count", &cohort["added"]["count"]),
                                ("added_delta", &cohort["added"]["delta"]),
                                ("removed_count", &cohort["removed"]["count"]),
                                ("removed_delta", &cohort["removed"]["delta"]),
                            ])));
                            for (key, label) in [
                                ("increases", "query.report-contributors-increase"),
                                ("decreases", "query.report-contributors-decrease"),
                            ] {
                                let Some(entries) = cohort[key].as_array().filter(|entries| !entries.is_empty()) else { continue };
                                body = body.child(div().text_sm().font_weight(FontWeight::BOLD).child(text(label)));
                                for entry in entries {
                                    let shown = |side: &str| {
                                        let evidence = &entry[side];
                                        evidence["amount_cell"].as_str().map_or_else(|| "—".to_owned(), |cell| {
                                            format!("{} ({}!{cell})", evidence["amount"], evidence["sheet"].as_str().unwrap_or_default())
                                        })
                                    };
                                    body = body.child(div().text_sm().child(format("query.report-contributor", &[
                                        ("key", &entry["key"].as_str().unwrap_or_default()),
                                        ("before", &shown("before")), ("after", &shown("after")),
                                        ("delta", &entry["delta"]),
                                    ])));
                                }
                            }
                            body = body.child(div().text_sm().child(format("query.report-contributors-remainder", &[
                                ("count", &cohort["remainder"]["count"]), ("delta", &cohort["remainder"]["delta"]),
                            ])));
                        } else if change.get("cohort").is_some() {
                            body = body.child(div().text_sm().child(text("query.report-cohort-unavailable")));
                        }
                    }
                }
            }
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

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn report_comparison_scenario() {
    use crate::shell::scenario::{Scenario, button_probe};
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("sales.csv");
    let new = dir.path().join("next.csv");
    let blank = dir.path().join("blank.xlsx");
    let workbook = dir.path().join("products.xlsx");
    std::fs::write(&old, "sku,label,revenue\nP01,Notebook,10\nP02,Pen,10\nP03,Paper,0\nP04,Folder,9\n").unwrap();
    std::fs::write(&new, "sku,label,revenue\nP01,Notebook,19\nP02,Pen,19\nP03,Paper,0\nP05,Stapler,5\n").unwrap();
    std::fs::write(&blank, ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();

    let mut service = ibis_doc_service::Service::new(&[dir.path().to_owned()], false, ibis_doc_service::Limits::default());
    let id = service.call("app_open_document", &json!({"path":blank})).unwrap()["document_id"].clone();
    let sheet = service.call("xlsx_list_sheets", &json!({"document_id":id})).unwrap()["sheets"][0]["name"].clone();
    service.call("sqlite_query_to_table", &json!({
        "document_id":id,"destination_sheet":sheet,"anchor":"C4",
        "sql":"SELECT sku, label, CAST(revenue AS INTEGER) AS revenue FROM sales ORDER BY sku",
        "datasets":[{"source":old,"alias":"sales"}],
    })).unwrap();
    service.call("xlsx_set_query_report_comparison", &json!({
        "document_id":id,"table":"QueryResult1",
        "comparison":{"key_column":"sku","metric_column":"revenue","unit":"USD"},
    })).unwrap();
    service.call("app_save_document", &json!({"document_id":id,"path":workbook})).unwrap();
    let mut scene = Scenario::open(workbook.to_str().unwrap());
    let _probe = button_probe::Probe::new();
    scene.update(|editor, _, cx| {
        editor.open_queries_panel(cx);
        editor.open_query_report_panel(cx);
        editor.query_reports_mut().unwrap().files.insert("sales.csv".into(), new.clone());
        cx.notify();
    });
    button_probe::click(&mut scene, "query-reports-preview");
    let args = json!({"table":"QueryResult1","metric":"revenue","contributor_key":"P05"});
    let mut result = Value::Null;
    for _ in 0..250 {
        result = scene.with(|editor, _| editor.live_report_preview_diff(&args));
        if result["state"] == "review" { break; }
        assert_eq!(result["state"], "previewing");
        std::thread::sleep(Duration::from_millis(20));
        scene.wait(20);
    }
    assert_eq!(result["state"], "review");
    assert_eq!(result["unit"], "USD");
    assert_eq!(result["before"], 29.0);
    assert_eq!(result["after"], 43.0);
    let comparison = &result["comparison"];
    assert_eq!(comparison["state"], "ready");
    assert_eq!(comparison["delta"], 14.0);
    assert_eq!(comparison["retained"]["count"], 3);
    assert_eq!(comparison["increases"][0]["key"], "P01");
    assert_eq!(comparison["increases"][1]["key"], "P02");
    assert_eq!(comparison["decreases"][0]["key"], "P04");
    assert_eq!(comparison["remainder"]["count"], 1);
    assert_eq!(comparison["remainder"]["delta"], 0.0);
    assert!(comparison["contributor"]["before"].is_null());
    assert_eq!(scene.with(|editor, _| editor.live_report_preview_diff(&json!({
        "table":"OtherReport","metric":"revenue"
    })))["state"], "report_not_in_preview");
    let expected = &comparison["contributor"]["after"];
    assert_eq!(expected["key_cell"], "C8");
    assert_eq!(expected["amount_cell"], "E8");
    assert_eq!(expected["amount"], 5.0);
    let applied_args = json!({"table":"QueryResult1","metric":"revenue","contributor_key":"P05","applied":true});
    assert_eq!(scene.with(|editor, _| editor.live_report_preview_diff(&applied_args))["state"], "absent");
    button_probe::click(&mut scene, "query-reports-apply");
    let applied = scene.with(|editor, _| editor.live_report_preview_diff(&applied_args));
    assert_eq!(applied["state"], "ready");
    assert_eq!(&applied["evidence"], expected);
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn report_precision_scenario() {
    use crate::shell::scenario::{Scenario, button_probe};
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("values.csv");
    let new = dir.path().join("next.csv");
    let blank = dir.path().join("blank.xlsx");
    let workbook = dir.path().join("precision.xlsx");
    std::fs::write(&old, "id,currency,metric,reference,offset\na,0.05,0.125,0.25,0.25\nb,0.15,0.125,0.25,0.25\n").unwrap();
    std::fs::write(&new, "id,currency,metric,reference,offset\na,0.10,0.1,0.3,0.31\nb,0.20,0.2,0.3,0.31\n").unwrap();
    std::fs::write(&blank, ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
    let mut service = ibis_doc_service::Service::new(&[dir.path().to_owned()], false, ibis_doc_service::Limits::default());
    let id = service.call("app_open_document", &json!({"path":blank})).unwrap()["document_id"].clone();
    let sheet = service.call("xlsx_list_sheets", &json!({"document_id":id})).unwrap()["sheets"][0]["name"].clone();
    for (anchor, sql) in [
        ("A1", "SELECT 'All' AS category, SUM(CAST(ROUND(currency * 100) AS INTEGER)) AS minor, MAX(CAST(reference AS REAL)) AS reference, MAX(CAST(offset AS REAL)) AS offset FROM src"),
        ("F1", "SELECT id AS key, CAST(ROUND(currency * 100) AS INTEGER) AS amount FROM src ORDER BY id"),
        ("I1", "SELECT id AS key, CAST(metric AS REAL) AS amount FROM src ORDER BY id"),
    ] {
        service.call("sqlite_query_to_table", &json!({
            "document_id":id,"destination_sheet":sheet,"anchor":anchor,"sql":sql,
            "datasets":[{"source":old,"alias":"src"}],
        })).unwrap();
    }
    for (column, anchor, title) in [("B", "A6", "Minor units"), ("C", "H6", "Fractional"), ("D", "A22", "Mismatch")] {
        service.call("xlsx_create_chart", &json!({
            "document_id":id,"sheet":sheet,"destination_sheet":sheet,"anchor":anchor,"kind":"column","title":title,
            "domain_range":"A2:A2","series":[{"values_range":format!("{column}2:{column}2"),"name_cell":format!("{column}1")}],
        })).unwrap();
    }
    service.call("app_save_document", &json!({"document_id":id,"path":workbook})).unwrap();
    let mut scene = Scenario::open(workbook.to_str().unwrap());
    let _probe = button_probe::Probe::new();
    scene.update(|editor, _, cx| {
        for (column, anchor) in [(1, "#F1"), (2, "#I1"), (3, "#I1")] {
            editor.set_cell(column, 1);
            editor.insert_cell_link_full(anchor, None, Some(crate::event_hooks::COHORT_LINK_MARKER), cx);
        }
        editor.open_queries_panel(cx);
        editor.open_query_report_panel(cx);
        editor.query_reports_mut().unwrap().files.insert("values.csv".into(), new.clone());
        cx.notify();
    });
    button_probe::click(&mut scene, "query-reports-preview");
    let mut summary = Value::Null;
    for _ in 0..250 {
        summary = scene.with(|editor, _| editor.live_report_preview_diff(&json!({})));
        if summary["state"] == "review" { break; }
        assert_eq!(summary["state"], "previewing");
        std::thread::sleep(Duration::from_millis(20));
        scene.wait(20);
    }
    assert_eq!(summary["state"], "review");
    assert_report_precision(&summary);
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
fn assert_report_precision(summary: &Value) {
    let point = |title: &str| {
        &summary["charts"].as_array().unwrap().iter().find(|chart| chart["title"] == title).unwrap()["top"][0]
    };
    let minor = point("Minor units");
    assert_eq!(minor["amounts"]["before"], 20.);
    assert_eq!(minor["amounts"]["after"], 30.);
    let cohort = &minor["cohort"];
    assert_eq!(cohort["state"], "ready");
    assert_eq!(cohort["precision"]["mode"], "exact_integer");
    for check in cohort["precision"]["checks"].as_object().unwrap().values() {
        assert_eq!(check["residual"], 0.);
        assert_eq!(check["tolerance"], 0.);
        assert_eq!(check["relative_to_total"], 0.);
    }
    let close = &point("Fractional")["cohort"];
    let mismatch = &point("Mismatch")["cohort"];
    assert_eq!(close["state"], "ready");
    assert_eq!(mismatch["state"], "totals_mismatch");
    for (comparison, total, accepted) in [(close, 0.3, true), (mismatch, 0.31, false)] {
        assert_eq!(comparison["precision"]["mode"], "binary64");
        let check = &comparison["precision"]["checks"]["after_total"];
        let residual = check["residual"].as_f64().unwrap().abs();
        let tolerance = check["tolerance"].as_f64().unwrap();
        let relative = check["relative_to_total"].as_f64().unwrap();
        assert!(residual > 0. && tolerance > 0.);
        assert_eq!(residual <= tolerance, accepted);
        assert!((relative - residual / total).abs() < 1e-17);
    }
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn report_rate_scenario() {
    use crate::shell::scenario::{Scenario, button_probe};
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("traffic.csv");
    let new = dir.path().join("next.csv");
    let blank = dir.path().join("blank.xlsx");
    let workbook = dir.path().join("traffic.xlsx");
    std::fs::write(&old, "sku,orders,visits\nA,1,4\nB,9,10\nM,,3\nZ,7,0\nX,2,-1\n").unwrap();
    std::fs::write(&new, "sku,orders,visits\nA,2,4\nB,10,20\nM,3,\nZ,9,0\nX,2,-1\n").unwrap();
    std::fs::write(&blank, ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
    let mut service = ibis_doc_service::Service::new(&[dir.path().to_owned()], false, ibis_doc_service::Limits::default());
    let id = service.call("app_open_document", &json!({"path":blank})).unwrap()["document_id"].clone();
    let sheet = service.call("xlsx_list_sheets", &json!({"document_id":id})).unwrap()["sheets"][0]["name"].clone();
    let sql = "SELECT sku, CAST(NULLIF(orders,'') AS INTEGER) AS orders, CAST(NULLIF(visits,'') AS INTEGER) AS visits FROM src";
    for (anchor, query) in [("C4", format!("{sql} ORDER BY sku")), ("I4", format!("{sql} WHERE sku IN ('M','Z') ORDER BY sku"))] {
        let result = service.call("sqlite_query_to_table", &json!({
            "document_id":id,"destination_sheet":sheet,"anchor":anchor,"sql":query,
            "datasets":[{"source":old,"alias":"src"}],
        })).unwrap();
        service.call("xlsx_set_query_report_comparison", &json!({
            "document_id":id,"table":result["table"],
            "comparison":{"kind":"rate","key_column":"sku","metric":"conversion","numerator_column":"orders","denominator_column":"visits"},
        })).unwrap();
    }
    service.call("app_save_document", &json!({"document_id":id,"path":workbook})).unwrap();
    let mut scene = Scenario::open(workbook.to_str().unwrap());
    let _probe = button_probe::Probe::new();
    scene.update(|editor, _, cx| {
        editor.open_queries_panel(cx);
        editor.open_query_report_panel(cx);
        editor.query_reports_mut().unwrap().files.insert("traffic.csv".into(), new.clone());
        cx.notify();
    });
    button_probe::click(&mut scene, "query-reports-preview");
    let args = |table, key| json!({"table":table,"metric":"conversion","contributor_key":key});
    let mut summary = Value::Null;
    for _ in 0..250 {
        summary = scene.with(|editor, _| editor.live_report_preview_diff(&args("QueryResult1", "A")));
        if summary["state"] == "review" { break; }
        assert_eq!(summary["state"], "previewing");
        std::thread::sleep(Duration::from_millis(20));
        scene.wait(20);
    }
    let zero = scene.with(|editor, _| editor.live_report_preview_diff(&args("QueryResult1", "Z")));
    let missing = scene.with(|editor, _| editor.live_report_preview_diff(&args("QueryResult1", "M")));
    let none = scene.with(|editor, _| editor.live_report_preview_diff(&args("QueryResult2", "Z")));
    assert_rate_evidence(&summary, &zero, &missing, &none);
    assert_quotient_preview_ui(&mut scene, &summary, "conversion");
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
fn assert_rate_evidence(summary: &Value, zero: &Value, missing: &Value, none: &Value) {
    assert_eq!(summary["state"], "review");
    for (side, numerator, denominator) in [("before", 10., 14.), ("after", 12., 24.)] {
        assert_eq!(summary[side]["numerator"], numerator);
        assert_eq!(summary[side]["denominator"], denominator);
        assert!((summary[side]["rate"].as_f64().unwrap() - numerator / denominator).abs() < 1e-12);
        assert_eq!(summary[side]["counts"], json!({"included":2,"missing":1,"zero_denominator":1,"invalid":1}));
    }
    let pp = summary["percentage_point_delta"].as_f64().unwrap();
    assert!((pp - (0.5 - 10. / 14.) * 100.).abs() < 1e-12);
    assert_quotient_effects(summary, "percentage_point_delta", [-2125. / 84., 325. / 84.], [475. / 84., -125. / 28., 2. / 7., 1. / 6.]);
    assert_eq!(none["decomposition"]["state"], "unavailable");
    let a = &summary["contributor"];
    assert_eq!(a["before"]["rate"], 0.25);
    assert_eq!(a["after"]["rate"], 0.5);
    assert_eq!(a["percentage_point_delta"], 25.);
    assert_eq!(a["after"]["key_cell"], "C5");
    assert_eq!(a["after"]["numerator"], json!({"cell":"D5","value":2.}));
    assert_eq!(a["after"]["denominator"], json!({"cell":"E5","value":4.}));
    assert_eq!(zero["contributor"]["after"]["state"], "zero_denominator");
    assert_eq!(zero["contributor"]["after"]["denominator"]["value"], 0.);
    assert!(zero["contributor"]["after"]["rate"].is_null());
    assert_eq!(missing["contributor"]["before"]["state"], "missing");
    assert!(missing["contributor"]["before"]["numerator"]["value"].is_null());
    assert_eq!(missing["contributor"]["after"]["state"], "missing");
    assert!(missing["contributor"]["after"]["denominator"]["value"].is_null());
    assert_eq!(none["after"]["state"], "no_eligible_rows");
    assert!(none["after"]["rate"].is_null());
    assert!(none["percentage_point_delta"].is_null());
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn report_average_scenario() {
    use crate::shell::scenario::{Scenario, button_probe};
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("observations.csv");
    let new = dir.path().join("next.csv");
    let blank = dir.path().join("blank.xlsx");
    let workbook = dir.path().join("observations.xlsx");
    std::fs::write(&old, "group_id,total,samples\nA,10,2\nB,90,3\nM,,3\nZ,7,0\nX,2,1.5\n").unwrap();
    std::fs::write(&new, "group_id,total,samples\nA,36,3\nB,104,8\nM,3,\nZ,9,0\nX,2,-1\n").unwrap();
    std::fs::write(&blank, ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
    let mut service = ibis_doc_service::Service::new(&[dir.path().to_owned()], false, ibis_doc_service::Limits::default());
    let id = service.call("app_open_document", &json!({"path":blank})).unwrap()["document_id"].clone();
    let sheet = service.call("xlsx_list_sheets", &json!({"document_id":id})).unwrap()["sheets"][0]["name"].clone();
    let sql = "SELECT group_id, CAST(NULLIF(total,'') AS REAL) AS total, CAST(NULLIF(samples,'') AS REAL) AS samples FROM src";
    for (anchor, query) in [("C4", format!("{sql} ORDER BY group_id")), ("I4", format!("{sql} WHERE group_id IN ('M','Z') ORDER BY group_id"))] {
        let result = service.call("sqlite_query_to_table", &json!({
            "document_id":id,"destination_sheet":sheet,"anchor":anchor,"sql":query,
            "datasets":[{"source":old,"alias":"src"}],
        })).unwrap();
        service.call("xlsx_set_query_report_comparison", &json!({
            "document_id":id,"table":result["table"],
            "comparison":{"kind":"average","key_column":"group_id","metric":"mean","sum_column":"total","count_column":"samples","unit":"points"},
        })).unwrap();
    }
    service.call("app_save_document", &json!({"document_id":id,"path":workbook})).unwrap();
    let mut scene = Scenario::open(workbook.to_str().unwrap());
    let _probe = button_probe::Probe::new();
    scene.update(|editor, _, cx| {
        editor.open_queries_panel(cx);
        editor.open_query_report_panel(cx);
        editor.query_reports_mut().unwrap().files.insert("observations.csv".into(), new.clone());
        cx.notify();
    });
    button_probe::click(&mut scene, "query-reports-preview");
    let args = |table, key| json!({"table":table,"metric":"mean","contributor_key":key});
    let mut summary = Value::Null;
    for _ in 0..250 {
        summary = scene.with(|editor, _| editor.live_report_preview_diff(&args("QueryResult1", "A")));
        if summary["state"] == "review" { break; }
        assert_eq!(summary["state"], "previewing");
        std::thread::sleep(Duration::from_millis(20));
        scene.wait(20);
    }
    let zero = scene.with(|editor, _| editor.live_report_preview_diff(&args("QueryResult1", "Z")));
    let missing = scene.with(|editor, _| editor.live_report_preview_diff(&args("QueryResult1", "M")));
    let invalid = scene.with(|editor, _| editor.live_report_preview_diff(&args("QueryResult1", "X")));
    let none = scene.with(|editor, _| editor.live_report_preview_diff(&args("QueryResult2", "Z")));
    assert_average_evidence(&summary, &zero, &missing, &invalid, &none);
    assert_quotient_preview_ui(&mut scene, &summary, "mean");
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
fn assert_average_evidence(summary: &Value, zero: &Value, missing: &Value, invalid: &Value, none: &Value) {
    assert_eq!(summary["state"], "review");
    assert_eq!(summary["kind"], "average");
    assert_eq!(summary["unit"], "points");
    for (side, sum, count) in [("before", 100., 5), ("after", 140., 11)] {
        assert_eq!(summary[side]["sum"], sum);
        assert_eq!(summary[side]["count"], count);
        assert!((summary[side]["average"].as_f64().unwrap() - sum / count as f64).abs() < 1e-12);
        assert_eq!(summary[side]["counts"], json!({"included":2,"missing":1,"zero_count":1,"invalid":1}));
    }
    assert!((summary["delta"].as_f64().unwrap() - (140. / 11. - 20.)).abs() < 1e-12);
    assert_quotient_effects(summary, "delta", [-491. / 55., 91. / 55.], [259. / 110., -119. / 110., 2. / 5., 3. / 11.]);
    assert_eq!(none["decomposition"]["state"], "unavailable");
    let a = &summary["contributor"];
    assert_eq!(a["before"]["average"], 5.);
    assert_eq!(a["after"]["average"], 12.);
    assert_eq!(a["delta"], 7.);
    assert_eq!(a["after"]["key_cell"], "C5");
    assert_eq!(a["after"]["sum"], json!({"cell":"D5","value":36.}));
    assert_eq!(a["after"]["count"], json!({"cell":"E5","value":3.}));
    assert_eq!(zero["contributor"]["after"]["state"], "zero_count");
    assert_eq!(zero["contributor"]["after"]["count"]["value"], 0.);
    assert!(zero["contributor"]["after"]["average"].is_null());
    assert_eq!(missing["contributor"]["before"]["state"], "missing");
    assert!(missing["contributor"]["before"]["sum"]["value"].is_null());
    assert_eq!(missing["contributor"]["after"]["state"], "missing");
    assert!(missing["contributor"]["after"]["count"]["value"].is_null());
    assert_eq!(invalid["contributor"]["before"]["state"], "invalid");
    assert_eq!(invalid["contributor"]["before"]["count"]["value"], 1.5);
    assert!(invalid["contributor"]["before"]["average"].is_null());
    assert_eq!(none["after"]["state"], "no_eligible_rows");
    assert!(none["after"]["average"].is_null());
    assert!(none["delta"].is_null());
}


#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
fn assert_quotient_effects(summary: &Value, delta_key: &str, effects: [f64; 2], contributor: [f64; 4]) {
    let breakdown = &summary["decomposition"];
    assert_eq!(breakdown["state"], "ready");
    assert_eq!(breakdown["counts"], json!({"continued":2,"entered":0,"exited":0}));
    let ranking = &breakdown["ranking"];
    let groups = ranking["groups"].as_array().unwrap();
    assert_eq!(ranking["group_count"], 2);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["key"], "B");
    assert_eq!(groups[1]["key"], "A");
    let sum: f64 = groups.iter().map(|row| row["contribution"].as_f64().unwrap()).sum();
    let tolerance = breakdown["tolerance"].as_f64().unwrap();
    assert!((sum - ranking["total_contribution"].as_f64().unwrap()).abs() <= tolerance);
    assert!((sum - summary[delta_key].as_f64().unwrap()).abs() <= tolerance);
    assert!(ranking["residual"].as_f64().unwrap().abs() <= tolerance);
    let within = breakdown["within_group"].as_f64().unwrap();
    let composition = breakdown["composition"].as_f64().unwrap();
    assert!((within - effects[0]).abs() < 1e-12);
    assert!((composition - effects[1]).abs() < 1e-12);
    assert!((within + composition - summary[delta_key].as_f64().unwrap()).abs()
        <= breakdown["tolerance"].as_f64().unwrap());
    assert!(breakdown["residual"].as_f64().unwrap().abs() <= breakdown["tolerance"].as_f64().unwrap());
    for (field, expected) in ["within_group","composition","before_share","after_share"].into_iter().zip(contributor) {
        assert!((breakdown["contributor"][field].as_f64().unwrap() - expected).abs() < 1e-12, "{field}");
    }
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
fn assert_quotient_preview_ui(scene: &mut crate::shell::scenario::Scenario, summary: &Value, metric: &str) {
    use crate::shell::scenario::button_probe;
    let rate = metric == "conversion";
    let unit = if rate { text("query.report-quotient-pp").to_string() }
        else { summary["unit"].as_str().unwrap().to_owned() };
    let ranking = summary["decomposition"]["ranking"]["groups"].as_array().unwrap();
    scene.size(1280., 1200.);
    scene.wait(100);
    for (row, group) in ranking.iter().enumerate() {
        let label = format("query.report-quotient-row", &[
            ("rank", &(row + 1)), ("key", &group["key"].as_str().unwrap()),
            ("total", &signed_contribution(&group["contribution"])), ("unit", &unit),
        ]);
        assert!(button_probe::bounds(&format!("query-report-quotient-row:{label}")).is_some(), "{label}");
        let parts = format("query.report-quotient-parts", &[
            ("within", &signed_contribution(&group["within_group"])),
            ("composition", &signed_contribution(&group["composition"])),
        ]);
        assert!(button_probe::bounds(&format!("query-report-check-text:{parts}")).is_some(), "{parts}");
    }
    let first = button_probe::bounds("query-report-quotient-0-0").unwrap();
    let second = button_probe::bounds("query-report-quotient-0-1").unwrap();
    assert!(first.origin.y < second.origin.y);
    button_probe::click(scene, "query-report-quotient-0-0");
    let fields = if rate { ["numerator", "denominator"] } else { ["sum", "count"] };
    for (side, prefix) in [("before", "query-report-quotient-evidence"), ("after", "query-report-check-text")] {
        let evidence = &ranking[0][side];
        for field in fields {
            let label = text(&format!("query.report-quotient-{field}"));
            let line = format("query.report-quotient-evidence", &[
                ("field", &label), ("sheet", &evidence["sheet"].as_str().unwrap()),
                ("cell", &evidence[field]["cell"].as_str().unwrap()), ("value", &evidence[field]["value"]),
            ]);
            assert!(button_probe::bounds(&format!("{prefix}:{line}")).is_some(), "{side}: {line}");
        }
    }
    let first_field = fields[0];
    button_probe::click(scene, &format!("query-report-quotient-cell-0-{first_field}"));
    assert_eq!(scene.with(|editor, _| editor.current_cell()), Some((3, 5)));
}

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn report_quotient_remainder_scenario(rate: bool) {
    use crate::shell::scenario::{Scenario, button_probe};
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("base.csv");
    let new = dir.path().join("next.csv");
    let blank = dir.path().join("blank.xlsx");
    let workbook = dir.path().join("remainder.xlsx");
    for (path, refresh) in [(&old, false), (&new, true)] {
        let mut csv = String::from("sku,amount,weight\n");
        for index in 1..=23 {
            let change = if index % 2 == 0 { index } else { -index };
            let amount = if refresh { 30 + change } else { 30 };
            csv.push_str(&format!("G{index:02},{amount},2\n"));
        }
        std::fs::write(path, csv).unwrap();
    }
    std::fs::write(&blank, ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
    let mut service = ibis_doc_service::Service::new(&[dir.path().to_owned()], false, ibis_doc_service::Limits::default());
    let id = service.call("app_open_document", &json!({"path":blank})).unwrap()["document_id"].clone();
    let sheet = service.call("xlsx_list_sheets", &json!({"document_id":id})).unwrap()["sheets"][0]["name"].clone();
    let result = service.call("sqlite_query_to_table", &json!({
        "document_id":id,"destination_sheet":sheet,"anchor":"C4",
        "sql":"SELECT sku, CAST(amount AS INTEGER) AS amount, CAST(weight AS INTEGER) AS weight FROM src ORDER BY sku",
        "datasets":[{"source":old,"alias":"src"}],
    })).unwrap();
    let (metric, comparison) = if rate {
        ("conversion", json!({"kind":"rate","key_column":"sku","metric":"conversion","numerator_column":"amount","denominator_column":"weight"}))
    } else {
        ("mean", json!({"kind":"average","key_column":"sku","metric":"mean","sum_column":"amount","count_column":"weight","unit":"points"}))
    };
    service.call("xlsx_set_query_report_comparison", &json!({"document_id":id,"table":result["table"],"comparison":comparison})).unwrap();
    service.call("app_save_document", &json!({"document_id":id,"path":workbook})).unwrap();
    let mut scene = Scenario::open(workbook.to_str().unwrap());
    let _probe = button_probe::Probe::new();
    scene.size(1280., 1800.);
    scene.update(|editor, _, cx| {
        editor.open_queries_panel(cx);
        editor.open_query_report_panel(cx);
        editor.query_reports_mut().unwrap().files.insert("base.csv".into(), new.clone());
        cx.notify();
    });
    button_probe::click(&mut scene, "query-reports-preview");
    let mut summary = Value::Null;
    let args = |offset| json!({"table":"QueryResult1","metric":metric,"remainder_offset":offset});
    for _ in 0..250 {
        summary = scene.with(|editor, _| editor.live_report_preview_diff(&args(0)));
        if summary["state"] == "review" { break; }
        assert_eq!(summary["state"], "previewing");
        std::thread::sleep(Duration::from_millis(20));
        scene.wait(20);
    }
    let ranking = &summary["decomposition"]["ranking"];
    let remainder = &ranking["remainder"];
    let scale = if rate { 100. } else { 1. };
    let delta = summary[if rate { "percentage_point_delta" } else { "delta" }].as_f64().unwrap();
    let tolerance = summary["decomposition"]["tolerance"].as_f64().unwrap();
    let top = ranking["top_contribution"].as_f64().unwrap();
    let other = remainder["contribution"].as_f64().unwrap();
    assert_eq!(remainder["count"], 13);
    assert_eq!(ranking["groups"].as_array().unwrap().len(), 10);
    assert!((top - (-5. / 46. * scale)).abs() <= tolerance);
    assert!((other - (-7. / 46. * scale)).abs() <= tolerance);
    assert!((top + other - delta).abs() <= tolerance);
    assert!(ranking["partition_residual"].as_f64().unwrap().abs() <= tolerance);
    let page = &remainder["page"];
    assert_eq!(page["next_offset"], 10);
    assert_eq!(page["groups"][0]["key"], "G13");
    assert_eq!(page["groups"][9]["key"], "G04");
    let second = scene.with(|editor, _| editor.live_report_preview_diff(&args(10)));
    let second_page = &second["decomposition"]["ranking"]["remainder"]["page"];
    assert_eq!(second_page["groups"][0]["key"], "G03");
    assert_eq!(second_page["groups"].as_array().unwrap().len(), 3);
    assert!(second_page["next_offset"].is_null());
    let sum: f64 = page["groups"].as_array().unwrap().iter()
        .chain(second_page["groups"].as_array().unwrap()).map(|entry| entry["contribution"].as_f64().unwrap()).sum();
    assert!((sum - other).abs() <= tolerance);

    let unit = if rate { text("query.report-quotient-pp").to_string() } else { "points".to_owned() };
    let label = format("query.report-quotient-remainder", &[
        ("count", &remainder["count"]), ("total", &signed_contribution(&remainder["contribution"])), ("unit", &unit),
    ]);
    scene.wait(100);
    let top_label = format("query.report-quotient-top", &[
        ("count", &10), ("total", &signed_contribution(&ranking["top_contribution"])), ("unit", &unit),
    ]);
    assert!(button_probe::bounds(&format!("query-report-check-text:{top_label}")).is_some());
    assert!(button_probe::bounds(&format!("query-report-quotient-remainder:{label}")).is_some());
    assert!(button_probe::bounds("query-report-quotient-0-10").is_none());
    button_probe::click(&mut scene, "query-report-quotient-remainder-0");
    let group = &page["groups"][0];
    let label = format("query.report-quotient-row", &[
        ("rank", &11), ("key", &"G13"), ("total", &signed_contribution(&group["contribution"])), ("unit", &unit),
    ]);
    assert!(button_probe::bounds(&format!("query-report-quotient-row:{label}")).is_some());
    button_probe::click(&mut scene, "query-report-quotient-0-10");
    let field = if rate { "numerator" } else { "sum" };
    for (side, prefix, value) in [("before", "query-report-quotient-evidence", 30), ("after", "query-report-check-text", 17)] {
        let evidence = &group[side];
        assert_eq!(evidence[field]["cell"], "D17");
        assert_eq!(evidence[field]["value"].as_f64().unwrap(), f64::from(value));
        let line = format("query.report-quotient-evidence", &[
            ("field", &text(&format!("query.report-quotient-{field}"))),
            ("sheet", &evidence["sheet"].as_str().unwrap()), ("cell", &"D17"), ("value", &evidence[field]["value"]),
        ]);
        assert!(button_probe::bounds(&format!("{prefix}:{line}")).is_some(), "{line}");
    }
    button_probe::click(&mut scene, &format!("query-report-quotient-cell-0-{field}"));
    assert_eq!(scene.with(|editor, _| editor.current_cell()), Some((3, 16)));
    button_probe::click(&mut scene, "query-report-quotient-next-0");
    let group = &second_page["groups"][0];
    let label = format("query.report-quotient-row", &[
        ("rank", &21), ("key", &"G03"), ("total", &signed_contribution(&group["contribution"])), ("unit", &unit),
    ]);
    assert!(button_probe::bounds(&format!("query-report-quotient-row:{label}")).is_some());
    button_probe::click(&mut scene, "query-report-quotient-previous-0");
    button_probe::click(&mut scene, "query-report-quotient-remainder-0");
    let review = scene.with(|editor, _| editor.wb().panels.queries.as_ref().unwrap().reports.as_ref()
        .unwrap().review.as_ref().unwrap().quotient_pages.is_empty());
    assert!(review);
}
