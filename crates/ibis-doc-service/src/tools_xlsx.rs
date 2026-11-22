// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The `xlsx_*` tools (design 6.3). A sheet is named by its tab; a range is A1 text.

use ibis_calc::CellValue;
use ibis_doc_model::Presentation;
use ibis_doc_model::ai_sheet::parse_cell_input;
use ibis_doc_model::sheet::{CellRange, Sheet};
use ibis_formats::Kind;
use serde_json::{Value, json};

use crate::a1::{column_name, parse_range, range_text};
use crate::catalog::{Access, Tool, boolean, document_id, integer, object, string};
use crate::error::ToolError;
use crate::service::{Args, ToolDocument, Service};

/// Defaults and per-call limits supplied by the workbook tool's caller.
#[derive(Clone, Copy, Default)]
pub struct WorkbookToolContext {
    pub active_sheet: Option<SlideId>,
    pub max_read_cells: Option<usize>,
    pub max_clear_contents_cells: Option<usize>,
}

fn workbook_document<'a>(service: &'a mut Service, id: &str) -> Result<&'a mut ToolDocument, ToolError> {
    let open = service.tool_document(id)?;
    if open.kind != Kind::Workbook {
        return Err(ToolError::new("unsupported_operation", "this tool is for an xlsx workbook"));
    }
    Ok(open)
}

pub(crate) fn tools() -> Vec<Tool> {
    vec![
        GET_CELLS,
        Tool { name: "xlsx_list_sheets", description: "The sheets of the workbook: name, used range and whether it is shown.", access: Access::Read, returns_content: false, schema: || object(&[document_id()], &["document_id"]), run: list_sheets },
        Tool { name: "xlsx_get_used_range", description: "The range of a sheet that holds anything.", access: Access::Read, returns_content: false, schema: || object(&[document_id(), ("sheet", string("The sheet's name."))], &["document_id", "sheet"]), run: get_used_range },
        Tool {
            name: "xlsx_trace_formula",
            description: "Trace direct, statically resolvable formula precedents or dependents of one cell across visible worksheets in this workbook.",
            access: Access::Read,
            returns_content: true,
            schema: || object(&[
                document_id(), ("sheet", string("The worksheet containing the target cell.")),
                ("cell", string("One A1 cell address.")),
                ("direction", json!({"enum": ["precedents", "dependents"]})),
                ("offset", integer("First reference on this page (0-based).")),
                ("limit", integer("Maximum references per page (default 100, maximum 200).")),
            ], &["document_id", "sheet", "cell", "direction"]),
            run: trace_formula,
        },
        Tool {
            name: "xlsx_check_machine_readability",
            description: "Find structural review candidates such as merged cells, hidden rows or columns, formulas, and platform-dependent or private-use characters.",
            access: Access::Read,
            returns_content: true,
            schema: || object(&[
                document_id(),
                ("sheet", string("Optional worksheet name. Omit to scan all visible worksheets.")),
                ("offset", integer("First finding to return (0-based).")),
                ("limit", integer("Maximum findings per page (default 100, maximum 200).")),
            ], &["document_id"]),
            run: check_machine_readability,
        },
        Tool {
            name: "xlsx_find_text",
            description: "Find cells by their displayed text and optionally their visible formulas.",
            access: Access::Read,
            returns_content: true,
            schema: || object(&[document_id(), ("query", string("The text to find.")), ("sheet", string("Only this sheet.")), ("match_case", boolean("Match upper and lower case (default false).")), ("search_formulas", boolean("Also search visible formulas (default false).")), ("limit", integer("The most cells."))], &["document_id", "query"]),
            run: find_text,
        },
    ]
}

/// The sheets of the open workbook: `(slide index, name, sheet)`.
pub(crate) fn sheets_of(open: &ToolDocument) -> Result<Vec<(usize, String, &Sheet)>, ToolError> {
    if open.kind != Kind::Workbook {
        return Err(ToolError::new("unsupported_operation", "this tool is for an xlsx workbook"));
    }
    Ok(workbook_sheets(open.document.presentation()))
}

fn workbook_sheets(workbook: &Presentation) -> Vec<(usize, String, &Sheet)> {
    workbook.slides.iter().enumerate().filter_map(|(index, slide)| {
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(slide)?;
        Some((index, slide.name.clone().unwrap_or_else(|| format!("Sheet{}", index + 1)), &**sheet))
    }).collect()
}

fn workbook_sheet<'a>(
    workbook: &'a Presentation,
    sheets: &'a [(usize, String, &'a Sheet)],
    args: &Value,
    active: Option<SlideId>,
) -> Result<&'a (usize, String, &'a Sheet), ToolError> {
    if let Some(name) = args.opt_str("sheet")? {
        return find_sheet(sheets, name);
    }
    let active = active.ok_or_else(|| ToolError::invalid("sheet is required"))?;
    sheets.iter().find(|(index, _, _)| workbook.slides[*index].id == active)
        .ok_or_else(|| ToolError::new("no_such_sheet", "no active sheet"))
}

pub(crate) fn find_sheet<'a>(sheets: &'a [(usize, String, &'a Sheet)], name: &str) -> Result<&'a (usize, String, &'a Sheet), ToolError> {
    sheets.iter().find(|(_, n, _)| n == name).or_else(|| sheets.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(name))).ok_or_else(|| ToolError::new("no_such_sheet", format!("no sheet named {name}")))
}

fn used_range(sheet: &Sheet) -> Option<CellRange> {
    let mut cells = sheet.cells.iter().filter(|c| !matches!(c.value, CellValue::Empty) || c.formula.is_some());
    let first = cells.next()?;
    let mut r = CellRange { first_row: first.row, first_column: first.column, last_row: first.row, last_column: first.column };
    for c in cells {
        r.first_row = r.first_row.min(c.row);
        r.first_column = r.first_column.min(c.column);
        r.last_row = r.last_row.max(c.row);
        r.last_column = r.last_column.max(c.column);
    }
    Some(r)
}

pub(crate) fn value_json(value: &CellValue) -> Value {
    match value {
        CellValue::Empty => Value::Null,
        CellValue::Number(n) => serde_json::Number::from_f64(*n).map_or(Value::Null, Value::Number),
        CellValue::Text(t) => Value::String(t.clone()),
        CellValue::Bool(b) => Value::Bool(*b),
        CellValue::Error(e) => json!({ "error": format!("{e:?}") }),
    }
}

fn list_sheets(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let open = service.tool_document(args.str_arg("document_id")?)?;
    let sheets: Vec<Value> = sheets_of(open)?.iter().map(|(_, name, sheet)| json!({ "name": name, "used_range": used_range(sheet).map(|r| range_text(&r)), "cells": sheet.cells.len() })).collect();
    Ok(json!({ "sheets": sheets }))
}

fn get_used_range(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let active = service.workbook_tool_context().active_sheet;
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let sheets = workbook_sheets(workbook);
    let (_, name, sheet) = workbook_sheet(workbook, &sheets, args, active)?;
    Ok(json!({ "sheet": name, "used_range": used_range(sheet).map(|r| range_text(&r)) }))
}

fn get_cells(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let context = service.workbook_tool_context();
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let range = parse_range(args.str_arg("range")?)?;
    let kind = args.opt_str("kind")?.unwrap_or("value");
    if !matches!(kind, "value" | "formula" | "text" | "format") {
        return Err(ToolError::invalid(format!("kind {kind} is not value, formula, text or format")));
    }
    let sheets = workbook_sheets(workbook);
    let (_, name, sheet) = workbook_sheet(workbook, &sheets, args, context.active_sheet)?;
    let width = (range.last_column - range.first_column + 1) as usize;
    let height = (range.last_row - range.first_row + 1) as usize;
    let offset = args.opt_usize("offset")?.unwrap_or(0);
    if offset > height {
        return Err(ToolError::new("out_of_range", format!("offset {offset} is past the {height} rows")));
    }
    let limit = args.opt_usize("limit")?.unwrap_or((10_000 / width).max(1)).max(1);
    let limit = if let Some(max_cells) = context.max_read_cells {
        if width > max_cells {
            return Err(ToolError::new("out_of_range", format!("range is wider than {max_cells} cells")));
        }
        limit.min(max_cells / width)
    } else {
        limit
    };
    let end = offset.saturating_add(limit).min(height);
    let mut rows: Vec<Vec<Value>> = vec![vec![Value::Null; width]; end - offset];
    let first_row = range.first_row + offset as u32;
    let page = if end == offset {
        &[][..]
    } else {
        sheet.cells_in_rows(first_row, range.first_row + end as u32 - 1)
    };
    for cell in page {
        let (r, c) = (cell.row, cell.column);
        if c < range.first_column || c > range.last_column {
            continue;
        }
        let row = (r - range.first_row) as usize;
        rows[row - offset][(c - range.first_column) as usize] = match kind {
            "formula" => sheet.visible_formula(cell)
                .map_or_else(|| value_json(&cell.value), |f| Value::String(format!("={f}"))),
            "text" => Value::String(cell.text.clone()),
            "format" => {
                let style = sheet.styles.get(cell.style as usize).cloned().unwrap_or_default();
                json!({
                    "bold": style.bold,
                    "italic": style.italic,
                    "underline": style.underline != ibis_doc_model::CellUnderline::None,
                    "size": f64::from(style.font_size) / 100.0,
                    "font": style.font,
                    "color": format!("#{:06X}", style.color.0 & 0xFF_FFFF),
                    "fill": style.fill.map(|f| format!("#{:06X}", f.0 & 0xFF_FFFF)),
                })
            }
            _ => value_json(&cell.value),
        };
    }
    let mut result = json!({ "sheet": name, "range": range_text(&range), "kind": kind, "first_row": range.first_row + 1 + offset as u32, "first_column": column_name(range.first_column), "rows": rows });
    if end < height {
        result["next_offset"] = json!(end);
    }
    Ok(result)
}

fn trace_formula(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    use ibis_calc::recalc::formula_dependencies;
    use ibis_doc_model::sheet_calc::BookView;

    let address = args.str_arg("cell")?;
    let (row, column) = ibis_calc::formula::parser::parse_cell(address.trim())
        .filter(|cell| cell.row < 1_048_576 && cell.column < 16_384)
        .map(|cell| (cell.row, cell.column))
        .ok_or_else(|| ToolError::invalid("cell must be a single A1 cell address"))?;
    let direction = args.str_arg("direction")?;
    if !matches!(direction, "precedents" | "dependents") {
        return Err(ToolError::invalid("direction is precedents or dependents"));
    }
    let offset = args.opt_usize("offset")?.unwrap_or(0);
    let limit = args.opt_usize("limit")?.unwrap_or(100).clamp(1, 200);
    let active = service.workbook_tool_context().active_sheet;
    let open = workbook_document(service, args.str_arg("document_id")?)?;
    let workbook = open.document.presentation();
    let sheets = workbook_sheets(workbook);
    let (slide, name, sheet) = workbook_sheet(workbook, &sheets, args, active)?;
    let book = BookView::new(workbook);
    let target = book.index_of_slide(workbook.slides[*slide].id)
        .ok_or_else(|| ToolError::invalid("the target sheet is unavailable"))?;
    let visible: Vec<_> = sheets.iter().filter_map(|(slide, name, sheet)| {
        Some((book.index_of_slide(workbook.slides[*slide].id)?, name, sheet))
    }).collect();
    let mut references = Vec::new();
    let mut count = 0;
    let mut push = |sheet: &str, range: CellRange| {
        if count >= offset && references.len() < limit {
            references.push(json!({"sheet": sheet, "range": range_text(&range)}));
        }
        count += 1;
    };
    let mut volatile = false;
    let mut unresolved = false;
    if direction == "precedents" {
        let mut seen = std::collections::HashSet::new();
        if let Some(formula) = sheet.cell(row, column).and_then(|cell| sheet.visible_formula(cell)) {
            if let Some(found) = formula_dependencies(&book, (target, row, column), formula) {
                volatile = found.volatile;
                for (first, last, r0, c0, r1, c1) in found.areas {
                    for &(index, name, _) in &visible {
                        if (first..=last).contains(&index) && seen.insert((index, r0, c0, r1, c1)) {
                            push(name, CellRange { first_row: r0.min(r1), last_row: r0.max(r1), first_column: c0.min(c1), last_column: c0.max(c1) });
                        }
                    }
                }
            } else {
                unresolved = true;
            }
        }
    } else {
        for (index, name, sheet) in visible {
            for cell in &sheet.cells {
                let Some(formula) = sheet.visible_formula(cell) else { continue };
                let Some(found) = formula_dependencies(&book, (index, cell.row, cell.column), formula) else { continue };
                if found.areas.iter().any(|&(first, last, r0, c0, r1, c1)| {
                    (first..=last).contains(&target)
                        && (r0.min(r1)..=r0.max(r1)).contains(&row)
                        && (c0.min(c1)..=c0.max(c1)).contains(&column)
                }) {
                    push(name, CellRange { first_row: cell.row, last_row: cell.row, first_column: cell.column, last_column: cell.column });
                }
            }
        }
    }
    let next = offset.saturating_add(references.len());
    let mut result = json!({
        "sheet": name, "cell": ibis_doc_model::sheet::cell_name(column, row),
        "direction": direction, "count": count, "references": references,
    });
    if direction == "precedents" {
        result["volatile"] = json!(volatile);
        result["unresolved"] = json!(unresolved);
    }
    if next < count {
        result["next_offset"] = json!(next);
    }
    Ok(result)
}

fn check_machine_readability(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    use ibis_doc_model::machine_readability::{CandidateKind, candidates};

    let name = args.opt_str("sheet")?;
    let offset = args.opt_usize("offset")?.unwrap_or(0);
    let limit = args.opt_usize("limit")?.unwrap_or(100).clamp(1, 200);
    let open = workbook_document(service, args.str_arg("document_id")?)?;
    let sheets = sheets_of(open)?;
    let selected = match name {
        Some(name) => std::slice::from_ref(find_sheet(&sheets, name)?),
        None => sheets.as_slice(),
    };
    let mut findings = Vec::new();
    let mut count = 0;
    for (_, name, sheet) in selected {
        for candidate in candidates(sheet) {
            if count >= offset && findings.len() < limit {
                let kind = match candidate.kind {
                    CandidateKind::MergedCells => "merged_cells",
                    CandidateKind::HiddenRow => "hidden_row",
                    CandidateKind::HiddenColumn => "hidden_column",
                    CandidateKind::Formula => "formula",
                    CandidateKind::PlatformDependentCharacters => "platform_dependent_characters",
                    CandidateKind::PrivateUseCharacters => "private_use_characters",
                };
                findings.push(json!({
                    "sheet": name,
                    "kind": kind,
                    "location": ibis_doc_model::ai_sheet::format_range(&candidate.range),
                }));
            }
            count += 1;
        }
    }
    let next = offset.saturating_add(findings.len());
    let mut result = json!({ "count": count, "findings": findings });
    if next < count {
        result["next_offset"] = json!(next);
    }
    Ok(result)
}

fn find_text(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let query = args.str_arg("query")?;
    if query.is_empty() {
        return Err(ToolError::invalid("query is empty"));
    }
    let only = args.opt_str("sheet")?;
    let match_case = args.opt_bool("match_case")?.unwrap_or(false);
    let search_formulas = args.opt_bool("search_formulas")?.unwrap_or(false);
    let limit = args.opt_usize("limit")?.unwrap_or(100).max(1);
    let limit = limit.min(service.workbook_tool_context().max_read_cells.unwrap_or(usize::MAX));
    let open = service.tool_document(args.str_arg("document_id")?)?;
    let sheets = sheets_of(open)?;
    let fold = |s: &str| if match_case { s.to_string() } else { s.to_lowercase() };
    let needle = fold(query);
    let mut found = Vec::new();
    'outer: for (_, name, sheet) in &sheets {
        if only.is_some_and(|o| !o.eq_ignore_ascii_case(name)) {
            continue;
        }
        for cell in sheet.cells.iter() {
            let formula = search_formulas.then(|| sheet.visible_formula(cell)).flatten();
            if fold(&cell.text).contains(&needle) || formula.is_some_and(|formula| fold(formula).contains(&needle)) {
                let mut result = json!({ "sheet": name, "cell": format!("{}{}", column_name(cell.column), cell.row + 1), "text": cell.text });
                if search_formulas {
                    result["formula"] = formula.map(|text| format!("={text}")).into();
                }
                found.push(result);
                if found.len() >= limit {
                    break 'outer;
                }
            }
        }
    }
    Ok(json!({ "matches": found }))
}

// ---- edits ----

use ibis_doc_model::sheet::{CellAlign, CellVerticalAlign, ThemeColor};
use ibis_doc_model::sheet_command::{CellEdit, CellInput, SheetCommand, StyleChange};
use ibis_doc_model::{Command, SlideId};

use crate::edit::{ColorArg, FontArg, edited, execute, only_known, parse_bool, parse_color, parse_font, parse_size};
use crate::tools_docx::{color_schema, font_schema};

pub(crate) fn edit_tools() -> Vec<Tool> {
    vec![
        SET_CELLS, CLEAR_CELLS, SET_FORMAT, crate::tools_chart::tool(),
        Tool {
            name: "xlsx_add_sheet",
            description: "Add a sheet (at the end, or at index, 0-based).",
            access: Access::Edit,
            returns_content: false,
            schema: || object(&[document_id(), ("name", string("The new sheet's name.")), ("index", integer("Where it goes (0-based)."))], &["document_id", "name"]),
            run: add_sheet,
        },
        Tool { name: "xlsx_delete_sheet", description: "Delete a sheet (not the last one).", access: Access::Edit, returns_content: false, schema: || object(&[document_id(), ("sheet", string("The sheet's name."))], &["document_id", "sheet"]), run: delete_sheet },
        Tool { name: "xlsx_rename_sheet", description: "Rename a sheet.", access: Access::Edit, returns_content: false, schema: || object(&[document_id(), ("sheet", string("The sheet's name.")), ("name", string("The new name."))], &["document_id", "sheet", "name"]), run: rename_sheet },
        Tool { name: "xlsx_calculate_workbook", description: "Calculate every formula of the workbook now.", access: Access::Edit, returns_content: false, schema: || object(&[document_id()], &["document_id"]), run: calculate_workbook },
    ]
}

fn sheet_edit(service: &mut Service, id: &str, commands: Vec<SheetCommand>, extra: Value) -> Result<Value, ToolError> {
    let open = service.tool_document(id)?;
    execute(open, commands.into_iter().map(Command::Sheet).collect())?;
    Ok(edited(id, open, extra, Vec::new()))
}

fn set_cells(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let context = service.workbook_tool_context();
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let range_arg = args.str_arg("range")?;
    let range = parse_range(range_arg)?;
    let rows = (range.last_row - range.first_row + 1) as usize;
    let columns = (range.last_column - range.first_column + 1) as usize;
    let count = rows.saturating_mul(columns);
    if count > 100_000 {
        return Err(ToolError::new("out_of_range", "more than 100000 cells in one call"));
    }
    let values = args.get("values").ok_or_else(|| ToolError::invalid("values is required"))?;
    let sheets = workbook_sheets(workbook);
    let (index, _, _) = workbook_sheet(workbook, &sheets, args, context.active_sheet)?;
    let sheet = workbook.slides[*index].id;
    let mut edits = Vec::with_capacity(count);
    match values {
        Value::Array(grid) => {
            if grid.len() != rows {
                return Err(ToolError::new("out_of_range", format!("the range {range_arg} is {rows} rows by {columns} columns; values is {} rows", grid.len())));
            }
            for (r, row) in grid.iter().enumerate() {
                let row = row.as_array().ok_or_else(|| ToolError::invalid("values is rows (arrays) of cells"))?;
                if row.len() != columns {
                    return Err(ToolError::new("out_of_range", format!("row {} must have {columns} cells", r + 1)));
                }
                for (c, value) in row.iter().enumerate() {
                    let input = parse_cell_input(value).map_err(ToolError::invalid)?;
                    edits.push(CellEdit { row: range.first_row + r as u32, column: range.first_column + c as u32, input });
                }
            }
        }
        single => {
            let input = parse_cell_input(single).map_err(ToolError::invalid)?;
            for row in range.first_row..=range.last_row {
                for column in range.first_column..=range.last_column {
                    edits.push(CellEdit { row, column, input: input.clone() });
                }
            }
        }
    }
    execute(open, vec![Command::Sheet(SheetCommand::SetCells { sheet, edits })])?;
    Ok(edited(id, open, json!({ "cells": count }), Vec::new()))
}

fn clear_cells(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let context = service.workbook_tool_context();
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let range = parse_range(args.str_arg("range")?)?;
    let what = args.str_arg("what")?;
    if !matches!(what, "contents" | "formats" | "all") {
        return Err(ToolError::invalid("what is contents, formats or all"));
    }
    if what != "formats"
        && let Some(max_cells) = context.max_clear_contents_cells
    {
        let rows = (range.last_row - range.first_row + 1) as usize;
        let columns = (range.last_column - range.first_column + 1) as usize;
        if rows.saturating_mul(columns) > max_cells {
            return Err(ToolError::new("out_of_range", format!("cannot clear contents of more than {max_cells} cells at once")));
        }
    }
    let sheets = workbook_sheets(workbook);
    let (index, _, current_sheet) = workbook_sheet(workbook, &sheets, args, context.active_sheet)?;
    let sheet = workbook.slides[*index].id;
    let mut commands = Vec::new();
    if what != "formats" {
        let mut edits = Vec::new();
        for cell in current_sheet.cells_in_rows(range.first_row, range.last_row) {
            if (range.first_column..=range.last_column).contains(&cell.column) {
                edits.push(CellEdit { row: cell.row, column: cell.column, input: CellInput::Clear });
            }
        }
        if !edits.is_empty() {
            commands.push(SheetCommand::SetCells { sheet, edits });
        }
    }
    if what != "contents" {
        commands.push(SheetCommand::ClearFormats { sheet, ranges: vec![range] });
    }
    if commands.is_empty() {
        return Ok(edited(id, open, json!({ "cleared": 0 }), Vec::new()));
    }
    execute(open, commands.into_iter().map(Command::Sheet).collect())?;
    Ok(edited(id, open, json!({}), Vec::new()))
}

fn theme_of(name: &str) -> Result<u8, ToolError> {
    // SpreadsheetML's order: 0 lt1, 1 dk1, 2 lt2, 3 dk2, 4-9 accent1-6.
    Ok(match name {
        "background1" => 0,
        "text1" => 1,
        "background2" => 2,
        "text2" => 3,
        "accent1" => 4,
        "accent2" => 5,
        "accent3" => 6,
        "accent4" => 7,
        "accent5" => 8,
        "accent6" => 9,
        _ => return Err(ToolError::new("unsupported_operation", "the hyperlink theme colours are not supported yet")),
    })
}

/// The workbook's Normal style: what a cell goes back to when a format is taken away (`null`, design 6.6).
fn normal_style(workbook: &Presentation) -> ibis_doc_model::CellStyle {
    workbook.workbook.as_ref().and_then(|w| w.default_style.clone()).unwrap_or_default()
}

fn color_change(workbook: &Presentation, arg: ColorArg, fill: bool) -> Result<StyleChange, ToolError> {
    Ok(match arg {
        ColorArg::Rgb(rgb) => {
            if fill {
                StyleChange::Fill(Some(rgb))
            } else {
                StyleChange::Color(rgb)
            }
        }
        ColorArg::Inherit => {
            // No fill; the text in the Normal style's colour.
            if fill {
                StyleChange::Fill(None)
            } else {
                StyleChange::Color(normal_style(workbook).color)
            }
        }
        ColorArg::Theme { name, brightness } => {
            // The tint is in millionths (800000: 80 % lighter, -250000: 25 % darker).
            let theme = ThemeColor { index: theme_of(&name)?, tint: (brightness * 1_000_000.0).round() as i32 };
            let rgb = theme.resolve(&workbook.theme_colors).unwrap_or(ibis_doc_model::Rgb::BLACK);
            if fill {
                StyleChange::FillTheme(rgb, theme)
            } else {
                StyleChange::ColorTheme(rgb, theme)
            }
        }
    })
}

fn set_format(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let context = service.workbook_tool_context();
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    only_known(args, &["document_id", "sheet", "range", "bold", "italic", "underline", "size", "color", "font", "fill", "number_format", "horizontal_alignment", "vertical_alignment", "wrap_text", "border"])?;
    let range = parse_range(args.str_arg("range")?)?;
    let sheets = workbook_sheets(workbook);
    let (index, _, _) = workbook_sheet(workbook, &sheets, args, context.active_sheet)?;
    let sheet = workbook.slides[*index].id;
    let normal = normal_style(workbook);
    let theme_fonts = workbook.workbook.as_ref().and_then(|w| w.theme_fonts.clone());
    let mut changes = Vec::new();
    let mut warnings = Vec::new();
    // `null` takes a format back to the Normal style's.
    for (key, make, base) in [("bold", StyleChange::Bold as fn(bool) -> StyleChange, normal.bold), ("italic", StyleChange::Italic, normal.italic), ("underline", StyleChange::Underline, normal.underline != ibis_doc_model::CellUnderline::None), ("wrap_text", StyleChange::WrapText, normal.wrap)] {
        if let Some(v) = args.get(key) {
            changes.push(match v {
                Value::Null => make(base),
                v => make(parse_bool(key, v)?),
            });
        }
    }
    if let Some(v) = args.get("size") {
        changes.push(match v {
            Value::Null => StyleChange::FontSize(normal.font_size),
            v => StyleChange::FontSize(parse_size("size", v)?),
        });
    }
    if let Some(v) = args.get("color") {
        changes.push(color_change(workbook, parse_color("color", v)?, false)?);
    }
    if let Some(v) = args.get("fill") {
        changes.push(color_change(workbook, parse_color("fill", v)?, true)?);
    }
    if let Some(v) = args.get("font") {
        match parse_font("font", v)? {
            FontArg::All(name) => changes.push(StyleChange::Font(Some(name))),
            FontArg::Split { latin, east_asian, .. } => match latin.or(east_asian) {
                Some(name) => changes.push(StyleChange::Font(Some(name))),
                None => return Err(ToolError::invalid("font has no typeface")),
            },
            FontArg::Inherit => changes.push(StyleChange::Font(normal.font.clone())),
            FontArg::Theme { major } => {
                // A cell keeps the typeface's name, not a reference to the theme: the theme's typeface now is written.
                let fonts = theme_fonts.clone().ok_or_else(|| ToolError::new("unsupported_operation", "this workbook has no theme typefaces"))?;
                let set = if major { fonts.major } else { fonts.minor };
                let name = set.latin.or(set.east_asian).ok_or_else(|| ToolError::new("unsupported_operation", "the theme has no typeface of that kind"))?;
                warnings.push(json!({ "code": "theme_font_resolved", "message": format!("the {} theme typeface was written as {name}; the cells do not follow a later change of the theme", if major { "heading" } else { "body" }) }));
                changes.push(StyleChange::Font(Some(name)));
            }
        }
    }
    if let Some(v) = args.get("number_format") {
        changes.push(StyleChange::NumberFormat(v.as_str().filter(|s| !s.is_empty()).ok_or_else(|| ToolError::invalid("number_format is a format code"))?.to_string()));
    }
    if let Some(v) = args.get("horizontal_alignment") {
        changes.push(StyleChange::HorizontalAlignment(match v.as_str() {
            Some("general") => CellAlign::General,
            Some("left") => CellAlign::Left,
            Some("center") => CellAlign::Center,
            Some("right") => CellAlign::Right,
            Some("justify") => CellAlign::Justify,
            _ => return Err(ToolError::invalid("horizontal_alignment is general, left, center, right or justify")),
        }));
    }
    if let Some(v) = args.get("vertical_alignment") {
        changes.push(StyleChange::VerticalAlignment(match v.as_str() {
            Some("top") => CellVerticalAlign::Top,
            Some("center") => CellVerticalAlign::Center,
            Some("bottom") => CellVerticalAlign::Bottom,
            Some("justify") => CellVerticalAlign::Justify,
            _ => return Err(ToolError::invalid("vertical_alignment is top, center, bottom or justify")),
        }));
    }
    let mut commands: Vec<Command> = changes.into_iter().map(|change| Command::Sheet(SheetCommand::SetFormat {
        sheet, ranges: vec![range], change,
    })).collect();
    if let Some(border) = args.get("border") {
        use ibis_doc_model::ai_sheet::{AiBorderStyle, build_border_commands};
        let border = match border.as_str() {
            Some("all") => AiBorderStyle::All,
            Some("outline") => AiBorderStyle::Outline,
            Some("bottom") => AiBorderStyle::Bottom,
            Some("none") => AiBorderStyle::None,
            _ => return Err(ToolError::invalid("border is all, outline, bottom or none")),
        };
        commands.extend(build_border_commands(sheet, range, border));
    }
    if commands.is_empty() {
        return Err(ToolError::invalid("no format is given"));
    }
    execute(open, commands)?;
    Ok(edited(id, open, json!({}), warnings))
}

fn add_sheet(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let name = args.str_arg("name")?.trim().to_string();
    if name.is_empty() || name.chars().count() > 31 || name.contains(['\\', '/', '?', '*', '[', ']', ':']) {
        return Err(ToolError::invalid("a sheet's name is 1 to 31 characters, without \\ / ? * [ ] :"));
    }
    let open = service.tool_document(id)?;
    let count = sheets_of(open)?.len();
    if sheets_of(open)?.iter().any(|(_, n, _)| n.eq_ignore_ascii_case(&name)) {
        return Err(ToolError::invalid(format!("a sheet named {name} is there already")));
    }
    let index = args.opt_usize("index")?.unwrap_or(count);
    if index > count {
        return Err(ToolError::new("out_of_range", format!("index {index}; the workbook has {count} sheets")));
    }
    let (slide, shape) = {
        let ids = open.document.ids();
        (ids.new_slide_id(), ids.new_shape_id())
    };
    sheet_edit(service, id, vec![SheetCommand::AddSheet { index, name: name.clone(), sheet: slide, shape }], json!({ "sheet": name, "index": index }))
}

fn delete_sheet(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let active = service.workbook_tool_context().active_sheet;
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let sheets = workbook_sheets(workbook);
    let (index, name, _) = workbook_sheet(workbook, &sheets, args, active)?;
    let sheet = workbook.slides[*index].id;
    if sheets.len() <= 1 {
        return Err(ToolError::new("unsupported_operation", "the last sheet cannot be deleted"));
    }
    let result = json!({ "deleted": name });
    execute(open, vec![Command::Sheet(SheetCommand::DeleteSheet { sheet })])?;
    Ok(edited(id, open, result, Vec::new()))
}

fn rename_sheet(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let new_name = args.str_arg("name")?.trim().to_string();
    if new_name.is_empty() || new_name.chars().count() > 31 || new_name.contains(['\\', '/', '?', '*', '[', ']', ':']) {
        return Err(ToolError::invalid("a sheet's name is 1 to 31 characters, without \\ / ? * [ ] :"));
    }
    let active = service.workbook_tool_context().active_sheet;
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let sheets = workbook_sheets(workbook);
    let (index, old, _) = workbook_sheet(workbook, &sheets, args, active)?;
    let sheet = workbook.slides[*index].id;
    if !old.eq_ignore_ascii_case(&new_name) && sheets.iter().any(|(_, n, _)| n.eq_ignore_ascii_case(&new_name)) {
        return Err(ToolError::invalid(format!("a sheet named {new_name} is there already")));
    }
    sheet_edit(service, id, vec![SheetCommand::RenameSheet { sheet, name: new_name.clone() }], json!({ "sheet": new_name }))
}

fn calculate_workbook(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let open = service.tool_document(id)?;
    sheets_of(open)?;
    sheet_edit(service, id, vec![SheetCommand::CalculateNow], json!({}))
}

const GET_CELLS: Tool = Tool {
    name: "xlsx_get_cells",
    description: "The cells of a range, row by row: value (calculated), formula, or text (as shown). Formula mode returns the calculated value when sheet protection hides the formula. Paged by rows.",
    access: Access::Read,
    returns_content: true,
    schema: || {
        object(
            &[
                document_id(),
                ("sheet", string("The sheet's name.")),
                ("range", string("A1 range, e.g. B2:D10.")),
                ("kind", json!({ "enum": ["value", "formula", "text"], "description": "value (default), formula or text (as shown)." })),
                ("offset", integer("The first row of the page (0-based, from the range's first row).")),
                ("limit", integer("The most rows.")),
            ],
            &["document_id", "sheet", "range"],
        )
    },
    run: get_cells,
};

const SET_CELLS: Tool = Tool {
    name: "xlsx_set_cells",
    description: "Put values in a range: rows of numbers, strings, true/false or null (clears the cell). A string starting with = followed by formula text is a formula. A single value fills the whole range; otherwise the rows must be as large as the range. What depends on them is calculated again.",
    access: Access::Edit,
    returns_content: false,
    schema: || {
        object(
            &[document_id(), ("sheet", string("The sheet's name.")), ("range", string("A1 range; its first cell is where the values start.")), ("values", json!({ "description": "A value, or rows of values.", "oneOf": [{ "type": "array", "items": { "type": "array" } }, { "type": ["string", "number", "boolean", "null"] }] }))],
            &["document_id", "sheet", "range", "values"],
        )
    },
    run: set_cells,
};

const CLEAR_CELLS: Tool = Tool {
    name: "xlsx_clear_cells",
    description: "Clear what is in a range: contents, formats, or all.",
    access: Access::Edit,
    returns_content: false,
    schema: || object(&[document_id(), ("sheet", string("The sheet's name.")), ("range", string("A1 range.")), ("what", json!({ "enum": ["contents", "formats", "all"] }))], &["document_id", "sheet", "range", "what"]),
    run: clear_cells,
};

fn resettable(schema: Value) -> Value {
    json!({ "anyOf": [schema, { "type": "null" }] })
}

const SET_FORMAT: Tool = Tool {
    name: "xlsx_set_format",
    description: "Set the format of a range: bold, italic, underline, size (pt), color, font, fill, number_format (Excel's format code), horizontal_alignment, vertical_alignment, wrap_text, border. What is not given is not changed.",
    access: Access::Edit,
    returns_content: false,
    schema: || {
        object(
            &[
                document_id(),
                ("sheet", string("The sheet's name.")),
                ("range", string("A1 range.")),
                ("bold", resettable(boolean("Bold."))),
                ("italic", resettable(boolean("Italic."))),
                ("underline", resettable(boolean("Underline."))),
                ("size", resettable(json!({ "type": "number", "description": "Points, 1 to 409.5 in steps of 0.5." }))),
                ("color", resettable(color_schema())),
                ("font", resettable(font_schema())),
                ("fill", resettable(color_schema())),
                ("number_format", string("Excel's number format code, e.g. 0.00 or yyyy/m/d.")),
                ("horizontal_alignment", json!({ "enum": ["general", "left", "center", "right", "justify"] })),
                ("vertical_alignment", json!({ "enum": ["top", "center", "bottom", "justify"] })),
                ("wrap_text", resettable(boolean("Wrap the text in the cell."))),
                ("border", json!({ "enum": ["all", "outline", "bottom", "none"] })),
            ],
            &["document_id", "sheet", "range"],
        )
    },
    run: set_format,
};
