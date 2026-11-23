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

#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn refresh_report_scenario() {
    use crate::shell::scenario::{Scenario, button_probe};
    use ibis_doc_model::{Document, SheetCommand};

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
        let (_, mut commands) = export.stage(&mut draft, ibis_doc_service::DraftToolContext {
            workbook: ibis_doc_service::WorkbookToolContext { active_sheet: Some(slide), ..Default::default() },
            ..Default::default()
        }).unwrap();
        let mut definition = ibis_doc_service::query_report_definition(
            draft.presentation(), &json!({"table":"QueryResult1"}),
        ).unwrap();
        definition["checks"] = json!([{
            "name":"重複コード",
            "sql":"SELECT count(*) FROM (SELECT code FROM monthly GROUP BY code HAVING count(*) > 1)",
            "max":0,
        }]);
        commands.push(Command::Sheet(SheetCommand::SetQueryReport { table:"QueryResult1".into(), definition }));
        editor.document.execute(commands).unwrap();
        editor.open_queries_panel(cx);
    });

    let _probe = button_probe::Probe::new();
    scene.update(|_, _, cx| cx.notify());
    button_probe::click(&mut scene, "queries-report-refresh");

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
        assert!(review.commands.is_empty());
    });
    assert_eq!(amount(&mut scene), Some(CellValue::Number(24.0)));

    std::fs::write(&next, "code,amount\n001,12\n").unwrap();
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
