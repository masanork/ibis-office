// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::{CellRange, Command, ShapeKind, SlideId};
use ibis_doc_model::ai_sheet::parse_cell_range;
use ibis_doc_model::sheet_calc::sheet_of_slide;
use ibis_formats::Kind;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::catalog::{Access, Tool};
use crate::edit::{edited, execute};
use crate::{Service, ToolError};

pub(crate) fn tool() -> Tool {
    Tool {
        name: "xlsx_create_chart",
        description: "Create an editable chart linked to worksheet cells. Supply a table with headers or explicit series, and a destination worksheet and anchor cell.",
        access: Access::Edit,
        returns_content: false,
        schema: || json!({
            "type": "object",
            "properties": {
                "document_id": { "type": "string", "description": "The open workbook handle." },
                "sheet": { "type": "string", "description": "The worksheet containing the data." },
                "destination_sheet": { "type": "string", "description": "The worksheet to place the chart on." },
                "anchor": { "type": "string", "description": "Top-left chart cell on the destination sheet, e.g. F2." },
                "kind": { "type": "string", "enum": ["column", "stacked_column", "bar", "stacked_bar", "line", "area", "pie", "doughnut", "scatter"] },
                "range": { "type": "string", "description": "Contiguous source table in A1 notation, including headers. Supply range or series, never both." },
                "domain_range": { "type": "string", "description": "Optional shared categories, or numeric X values for scatter. Only for explicit series." },
                "series": {
                    "type": "array", "minItems": 1, "maxItems": 8,
                    "items": {
                        "type": "object",
                        "properties": {
                            "values_range": { "type": "string", "description": "One row or column of numeric values, without headers." },
                            "domain_range": { "type": "string", "description": "Categories or scatter X values for this series. Overrides the shared domain; must match the number of values." },
                            "name_cell": { "type": "string", "description": "Optional series label cell on the data sheet, e.g. B1." }
                        },
                        "required": ["values_range"],
                        "additionalProperties": false
                    }
                },
                "title": { "type": "string", "description": "Chart title." },
                "x_title": { "type": "string", "description": "Optional horizontal axis title with unit." },
                "y_title": { "type": "string", "description": "Optional vertical axis title with unit." }
            },
            "required": ["document_id", "sheet", "destination_sheet", "kind", "anchor", "title"],
            "additionalProperties": false
        }),
        run: create_chart,
    }
}

fn create_chart(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let args: ChartArgs = serde_json::from_value(args.clone()).map_err(|error| ToolError::invalid(error.to_string()))?;
    let id = &args.document_id;
    let active = service.workbook_tool_context().active_sheet;
    let source = service.tool_source(id)?;
    let open = service.tool_document(id)?;
    if open.kind != Kind::Workbook {
        return Err(ToolError::new("unsupported_operation", "this tool is for an xlsx workbook"));
    }
    let workbook = open.document.presentation();
    let resolve = |name: Option<&str>, key| -> Result<SlideId, ToolError> {
        match name {
            Some(name) => {
                let sheets = workbook.workbook.as_ref().ok_or_else(|| ToolError::invalid("no workbook"))?;
                sheets.sheets.iter().find(|sheet| sheet.name.eq_ignore_ascii_case(name))
                    .and_then(|sheet| sheet.slide)
                    .ok_or_else(|| ToolError::new("no_such_sheet", format!("no worksheet named {name}")))
            }
            None => active.ok_or_else(|| ToolError::invalid(format!("{key} is required"))),
        }
    };
    let data_id = resolve(args.sheet.as_deref(), "sheet")?;
    let destination = resolve(args.destination_sheet.as_deref(), "destination_sheet")?;
    let data_slide = workbook.slide(data_id).ok_or_else(|| ToolError::invalid("data sheet not found"))?;
    let name = data_slide.name.as_deref().ok_or_else(|| ToolError::invalid("data sheet has no name"))?;
    let data = sheet_of_slide(data_slide).ok_or_else(|| ToolError::invalid("data sheet not found"))?;
    let refs = args.refs(workbook, name, data).map_err(ToolError::invalid)?;
    let destination_slide = workbook.slide(destination).ok_or_else(|| ToolError::invalid("destination sheet not found"))?;
    let (sheet, origin) = destination_slide.shapes.iter().find_map(|shape| {
        if let ShapeKind::Sheet(sheet) = &shape.kind { Some((sheet, shape.frame)) } else { None }
    }).ok_or_else(|| ToolError::invalid("destination sheet not found"))?;
    let frame = args.frame(sheet, origin).map_err(ToolError::invalid)?;
    let index = destination_slide.shapes.len();
    let destination_name = destination_slide.name.clone();
    let shape_id = open.document.ids().new_shape_id();
    let mut shape = source.new_sheet_chart(open.document.presentation(), destination, shape_id, frame, args.kind(), refs)
        .map_err(|error| ToolError::new("edit_rejected", error.to_string()))?;
    if let ShapeKind::Chart(chart) = &mut shape.kind {
        args.label(chart);
    }
    execute(open, vec![Command::InsertShape { slide: destination, shapes: vec![(index, shape)] }])?;
    Ok(edited(id, open, json!({ "shape_id": shape_id.0, "destination_sheet": destination_name, "anchor": args.anchor }), Vec::new()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChartArgs {
    document_id: String,
    sheet: Option<String>,
    destination_sheet: Option<String>,
    anchor: String,
    kind: ChartKindArg,
    range: Option<String>,
    domain_range: Option<String>,
    series: Option<Vec<ChartSeriesArgs>>,
    title: String,
    x_title: Option<String>,
    y_title: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChartSeriesArgs {
    values_range: String,
    domain_range: Option<String>,
    name_cell: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChartKindArg {
    Column,
    StackedColumn,
    Bar,
    StackedBar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
}

impl ChartArgs {
    fn frame(&self, sheet: &ibis_doc_model::Sheet, origin: ibis_doc_model::EmuRect) -> Result<ibis_doc_model::EmuRect, String> {
        let (row, column) = ibis_doc_model::ai_sheet::parse_cell_ref(&self.anchor)
            .filter(|&(row, column)| row < 1_048_576 && column < 16_384)
            .ok_or("anchor must be a single A1 cell")?;
        let cell = sheet.cell_bounds_emu(column, row);
        Ok(ibis_doc_model::EmuRect::new(origin.x + cell.x, origin.y + cell.y, 5_486_400, 3_429_000))
    }

    fn kind(&self) -> ibis_formats::NewChartKind {
        use ibis_formats::NewChartKind as Kind;
        match self.kind {
            ChartKindArg::Column => Kind::ClusteredColumn,
            ChartKindArg::StackedColumn => Kind::StackedColumn,
            ChartKindArg::Bar => Kind::ClusteredBar,
            ChartKindArg::StackedBar => Kind::StackedBar,
            ChartKindArg::Line => Kind::Line,
            ChartKindArg::Area => Kind::Area,
            ChartKindArg::Pie => Kind::Pie,
            ChartKindArg::Doughnut => Kind::Doughnut,
            ChartKindArg::Scatter => Kind::Scatter,
        }
    }

    fn refs(&self, workbook: &ibis_doc_model::Presentation, name: &str, sheet: &ibis_doc_model::Sheet) -> Result<Vec<ibis_formats::SeriesRefs>, String> {
        let bounded = |address: &str| {
            parse_cell_range(address).filter(|r| {
                let (rows, cols) = (r.last_row - r.first_row + 1, r.last_column - r.first_column + 1);
                r.last_column < 16_384 && r.last_row < 1_048_576
                    && rows <= 10_000 && cols <= 10_000 && u64::from(rows) * u64::from(cols) <= 90_000
            }).ok_or_else(|| format!("invalid or oversized A1 range: {address}"))
        };
        let vector = |address: &str| {
            let range = bounded(address)?;
            if range.first_row != range.last_row && range.first_column != range.last_column {
                return Err(format!("expected one row or column: {address}"));
            }
            Ok(range)
        };
        let length = |r: CellRange| (r.last_row - r.first_row).max(r.last_column - r.first_column) + 1;
        if self.title.trim().is_empty() || [self.title.as_str(), self.x_title.as_deref().unwrap_or(""), self.y_title.as_deref().unwrap_or("")]
            .iter().any(|title| title.chars().count() > 200)
        {
            return Err("title is required and each title must be at most 200 characters".into());
        }
        let kind = self.kind();
        let scatter = matches!(kind, ibis_formats::NewChartKind::Scatter);
        let pie = matches!(kind, ibis_formats::NewChartKind::Pie);
        let quoted = ibis_formats::quote_sheet(name);
        let reference = |range| ibis_formats::format_ref(&quoted, range);
        let refs = match (&self.range, &self.series) {
            (Some(range), None) if self.domain_range.is_none() => {
                let mut refs = ibis_formats::refs_for_new_chart(workbook, &quoted, bounded(range)?, kind);
                if pie {
                    refs.truncate(1);
                }
                refs
            }
            (None, Some(series)) if (1..=8).contains(&series.len()) && (!pie || series.len() == 1) => {
                series.iter().map(|series| {
                    let values = vector(&series.values_range)?;
                    let domain = series.domain_range.as_deref().or(self.domain_range.as_deref())
                        .map(vector).transpose()?;
                    if domain.is_some_and(|range| length(range) != length(values)) {
                        return Err("each domain range must have as many cells as its values range".into());
                    }
                    let name = series.name_cell.as_deref().map(|cell| {
                        ibis_doc_model::ai_sheet::parse_cell_ref(cell)
                            .and_then(|_| bounded(cell).ok())
                            .map(reference)
                            .ok_or("name_cell must be a single A1 cell")
                    }).transpose()?;
                    Ok(ibis_formats::SeriesRefs { name, categories: domain.map(reference), values: Some(reference(values)), sizes: None })
                }).collect::<Result<Vec<_>, String>>()?
            }
            _ => return Err("provide a source range or one to eight explicit series (one for pie)".into()),
        };
        if refs.is_empty() || refs.len() > 8 {
            return Err("the source must yield one to eight series".into());
        }
        let numeric = |range: CellRange, index: u32| {
            let row = range.first_row + if range.first_row == range.last_row { 0 } else { index };
            let col = range.first_column + if range.first_column == range.last_column { 0 } else { index };
            sheet.cell(row, col).is_some_and(|cell| {
                matches!(cell.value, ibis_doc_model::CellValue::Number(n) if n.is_finite())
            })
        };
        let parse = |value: &Option<String>| value.as_deref()
            .and_then(ibis_formats::parse_ref).map(|(_, range)| range);
        if scatter && refs.iter().any(|series| parse(&series.categories).is_none()) {
            return Err("scatter charts need numeric X values; supply domain_range if the range cannot be inferred".into());
        }
        let has_point = refs.iter().any(|series| {
            let Some(values) = parse(&series.values) else { return false };
            let domain = parse(&series.categories);
            (0..length(values)).any(|index| numeric(values, index)
                && (!scatter || domain.is_some_and(|x| numeric(x, index))))
        });
        if !has_point {
            return Err("the chart source contains no plottable values".into());
        }
        Ok(refs)
    }

    fn label(&self, chart: &mut ibis_doc_model::Chart) {
        use ibis_doc_model::chart::{ChartText, ChartTitle, Side};
        let title = |text: &str, size| ChartTitle {
            text: text.to_owned(),
            style: ChartText { size, ..Default::default() },
            at: None,
        };
        chart.title = Some(title(&self.title, 1400));
        for axis in &mut chart.axes {
            let label = match axis.side {
                Side::Bottom | Side::Top => self.x_title.as_deref(),
                Side::Left | Side::Right => self.y_title.as_deref(),
            };
            if let Some(label) = label {
                axis.title = Some(title(label, 1100));
            }
        }
    }
}
