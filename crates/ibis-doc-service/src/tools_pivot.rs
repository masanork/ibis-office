// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::HashSet;

use ibis_doc_model::pivot::{
    PivotAxis, PivotData, PivotDataField, PivotSource, PivotTable, source_data,
};
use ibis_doc_model::sheet::{CellRange, Sheet};
use ibis_doc_model::sheet_command::SheetCommand;
use serde_json::{Value, json};

use crate::ToolError;
use crate::a1::{parse_range, range_text};
use crate::catalog::{Access, Tool, document_id, integer, object, string};
use crate::service::{Args, Service};
use crate::tools_xlsx::{sheet_edit, workbook_document, workbook_sheet, workbook_sheets};

const MAX_CELLS: usize = 100_000;
const AGGREGATIONS: &[&str] = &["sum", "count", "countNums", "average", "min", "max"];

pub(crate) const CREATE_PIVOT: Tool = Tool {
    name: "xlsx_create_pivot",
    description: "Create an Excel PivotTable at a blank location. Supply either a source range with headers or a named Excel table. Count counts nonblank values.",
    access: Access::Edit,
    returns_content: false,
    schema: || {
        object(
            &[
                document_id(),
                ("sheet", string("Source worksheet.")),
                (
                    "range",
                    string("Source A1 range including the header row. Specify range or table."),
                ),
                (
                    "table",
                    string("Name of a source Excel table on the source worksheet."),
                ),
                ("destination_sheet", string("Worksheet for the pivot.")),
                ("anchor", string("Top-left cell of the pivot in A1 form.")),
                (
                    "rows",
                    json!({"type":"array","maxItems":8,"items":{"type":"string"},"description":"Source headers for row fields."}),
                ),
                (
                    "columns",
                    json!({"type":"array","maxItems":8,"items":{"type":"string"},"description":"Source headers for column fields."}),
                ),
                (
                    "values",
                    json!({"type":"array","minItems":1,"maxItems":8,"items":object(&[
            ("field", string("Source header to aggregate.")),
            ("aggregate", json!({"enum": AGGREGATIONS})),
        ], &["field","aggregate"])}),
                ),
            ],
            &[
                "document_id",
                "sheet",
                "destination_sheet",
                "anchor",
                "values",
            ],
        )
    },
    run: create_pivot,
};

pub(crate) const LIST_PIVOTS: Tool = Tool {
    name: "xlsx_list_pivots",
    description: "List PivotTables on a worksheet with their source, current location, axes and measures.",
    access: Access::Read,
    returns_content: true,
    schema: || {
        object(
            &[
                document_id(),
                ("sheet", string("Worksheet containing the pivots.")),
                ("offset", integer("First pivot to return (0-based).")),
                (
                    "limit",
                    integer("Maximum pivots per page (default 25, maximum 50)."),
                ),
            ],
            &["document_id", "sheet"],
        )
    },
    run: list_pivots,
};

pub(crate) const REFRESH_PIVOT: Tool = Tool {
    name: "xlsx_refresh_pivot",
    description: "Rebuild an existing PivotTable from its current workbook table or range while preserving its field configuration.",
    access: Access::Edit,
    returns_content: false,
    schema: || {
        object(
            &[
                document_id(),
                ("sheet", string("Worksheet containing the pivot.")),
                (
                    "pivot",
                    string("Existing pivot name from xlsx_list_pivots."),
                ),
            ],
            &["document_id", "sheet", "pivot"],
        )
    },
    run: refresh_pivot,
};

fn anchor(args: &Value) -> Result<CellRange, ToolError> {
    let range = parse_range(args.str_arg("anchor")?)?;
    if range.first_row != range.last_row || range.first_column != range.last_column {
        return Err(ToolError::invalid("anchor must be a single cell"));
    }
    Ok(range)
}

fn overlaps(a: &CellRange, b: &CellRange) -> bool {
    a.first_row <= b.last_row
        && b.first_row <= a.last_row
        && a.first_column <= b.last_column
        && b.first_column <= a.last_column
}

fn pivot_areas(pivot: &PivotTable) -> impl Iterator<Item = CellRange> {
    let location = pivot.location;
    let pages = pivot.page_fields.len() as u32;
    std::iter::once(location).chain((pages > 0).then_some(CellRange {
        first_row: location.first_row.saturating_sub(pages + 1),
        last_row: location.first_row.saturating_sub(1),
        last_column: location.first_column.saturating_add(1),
        ..location
    }))
}

fn blank_destination(sheet: &Sheet, range: &CellRange) -> Result<(), ToolError> {
    available_destination(sheet, range, None)
}

fn available_destination(
    sheet: &Sheet,
    range: &CellRange,
    own_pivot: Option<usize>,
) -> Result<(), ToolError> {
    let occupied = sheet
        .cells_in_rows(range.first_row, range.last_row)
        .iter()
        .any(|cell| {
            let inside_own = own_pivot.is_some_and(|index| {
                pivot_areas(&sheet.pivots[index]).any(|area| {
                    (area.first_row..=area.last_row).contains(&cell.row)
                        && (area.first_column..=area.last_column).contains(&cell.column)
                })
            });
            (range.first_column..=range.last_column).contains(&cell.column)
                && (!cell.value.is_empty() || cell.formula.is_some())
                && !inside_own
        });
    if occupied
        || sheet.merges.iter().any(|area| overlaps(area, range))
        || sheet
            .tables
            .iter()
            .any(|table| overlaps(&table.range, range))
        || sheet.pivots.iter().enumerate().any(|(index, pivot)| {
            Some(index) != own_pivot && pivot_areas(pivot).any(|area| overlaps(&area, range))
        })
    {
        return Err(ToolError::new(
            "destination_occupied",
            "Choose a blank destination without merged cells, tables, or pivots.",
        ));
    }
    if sheet.protection.is_some() {
        return Err(ToolError::new(
            "edit_rejected",
            "The destination worksheet is protected.",
        ));
    }
    Ok(())
}

fn validate_headers(headers: &[String]) -> Result<(), ToolError> {
    let mut seen = HashSet::new();
    if headers.is_empty()
        || headers.iter().any(|header| {
            header.trim().is_empty()
                || header.encode_utf16().count() > 255
                || !seen.insert(header.to_lowercase())
        })
    {
        return Err(ToolError::invalid(
            "Headers must be distinct, nonblank, and at most 255 UTF-16 units. Use SQL aliases if needed.",
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn write_table(
    service: &mut Service,
    args: &Value,
    result: ibis_doc_model::query::SqliteRows,
    definition: Value,
) -> Result<Value, ToolError> {
    use ibis_doc_model::sheet::SheetTable;

    if result.truncated || result.rows.is_empty() {
        return Err(ToolError::invalid(
            "A table needs a complete result with at least one data row.",
        ));
    }
    validate_headers(&result.columns)?;
    let id = args.str_arg("document_id")?;
    let at = anchor(args)?;
    let active = service.workbook_tool_context().active_sheet;
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let sheets = workbook_sheets(workbook);
    let (index, sheet_name, sheet) = workbook_sheet(
        workbook,
        &sheets,
        &json!({"sheet": args["destination_sheet"]}),
        active,
    )?;
    let slide = workbook.slides[*index].id;
    let range = CellRange {
        last_row: at.first_row + result.rows.len() as u32,
        last_column: at.first_column + result.columns.len() as u32 - 1,
        ..at
    };
    if range.last_row >= 1_048_576 || range.last_column >= 16_384 {
        return Err(ToolError::new(
            "out_of_range",
            "The result does not fit in the worksheet.",
        ));
    }
    blank_destination(sheet, &range)?;
    if workbook.workbook.as_ref().is_some_and(|book| {
        book.query_reports
            .values()
            .map(|definition| definition.to_string().len())
            .sum::<usize>()
            .saturating_add(definition.to_string().len())
            > 512 * 1024
    }) {
        return Err(ToolError::invalid(
            "The workbook's saved query definitions exceed 512 KiB.",
        ));
    }
    let name = (1..)
        .map(|n| format!("QueryResult{n}"))
        .find(|name| {
            !sheets.iter().any(|(_, _, sheet)| {
                sheet
                    .tables
                    .iter()
                    .any(|table| table.name.eq_ignore_ascii_case(name))
            }) && !workbook.workbook.as_ref().is_some_and(|book| {
                book.query_reports.contains_key(name)
                    || book
                        .names
                        .iter()
                        .any(|defined| defined.name.eq_ignore_ascii_case(name))
            })
        })
        .unwrap();
    let cells = query_cells(&result, at, false)?;
    let table = SheetTable {
        name: name.clone(),
        range,
        header_rows: 1,
        totals_rows: 0,
        columns: result.columns,
        style: None,
        filter: None,
        totals: Vec::new(),
    };
    let command = SheetCommand::SetTable {
        sheet: slide,
        index: sheet.tables.len(),
        table: Some(table),
        cells,
    };
    let summary = json!({"sheet":sheet_name,"range":range_text(&range),"table":name,"rows":range.last_row-at.first_row});
    let save_definition = SheetCommand::SetQueryReport {
        table: name,
        definition,
    };
    sheet_edit(service, id, vec![command, save_definition], summary)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn query_cells(
    result: &ibis_doc_model::query::SqliteRows,
    at: CellRange,
    clear_null: bool,
) -> Result<Vec<ibis_doc_model::sheet_command::PutCell>, ToolError> {
    use ibis_doc_model::sheet::CellValue;
    use ibis_doc_model::sheet_command::{CellInput, PutCell};
    let mut cells = Vec::with_capacity(result.columns.len() * (result.rows.len() + 1));
    for (column, text) in result.columns.iter().enumerate() {
        cells.push(PutCell {
            row: at.first_row,
            column: at.first_column + column as u32,
            input: CellInput::Value(CellValue::Text(text.clone())),
            style: None,
        });
    }
    for (row, values) in result.rows.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            let value = match value {
                Value::Null if clear_null => {
                    cells.push(PutCell {
                        row: at.first_row + 1 + row as u32,
                        column: at.first_column + column as u32,
                        input: CellInput::Clear,
                        style: None,
                    });
                    continue;
                }
                Value::Null => continue,
                Value::String(value) if value.encode_utf16().count() <= 32_767 => {
                    CellValue::Text(value.clone())
                }
                Value::Number(value)
                    if value.as_i64().is_some_and(|n| n.unsigned_abs() >= 1 << 53) =>
                {
                    return Err(ToolError::invalid(
                        "An integer exceeds worksheet precision. Cast it to TEXT in SQL.",
                    ));
                }
                Value::Number(value) => CellValue::Number(value.as_f64().unwrap()),
                _ => {
                    return Err(ToolError::invalid(
                        "A SQL value does not fit in a worksheet cell.",
                    ));
                }
            };
            cells.push(PutCell {
                row: at.first_row + 1 + row as u32,
                column: at.first_column + column as u32,
                input: CellInput::Value(value),
                style: None,
            });
        }
    }
    Ok(cells)
}

fn field_index(data: &PivotData, field: &str) -> Result<usize, ToolError> {
    data.headings
        .iter()
        .position(|name| name == field)
        .ok_or_else(|| ToolError::invalid(format!("No source field named {field}")))
}

fn axis_fields(args: &Value, key: &str, data: &PivotData) -> Result<Vec<usize>, ToolError> {
    match args.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(fields)) if fields.len() <= 8 => fields
            .iter()
            .map(|field| {
                let field = field
                    .as_str()
                    .ok_or_else(|| ToolError::invalid(format!("{key} must contain field names")))?;
                field_index(data, field)
            })
            .collect(),
        _ => Err(ToolError::invalid(format!(
            "{key} must be an array of at most eight field names"
        ))),
    }
}

fn axis_size(data: &PivotData, fields: &[usize]) -> usize {
    if fields.is_empty() {
        return 1;
    }
    let groups: HashSet<_> = data
        .records
        .iter()
        .map(|record| fields.iter().map(|&i| &record[i]).collect::<Vec<_>>())
        .collect();
    groups.len() * fields.len() + 1
}

fn create_pivot(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let at = anchor(args)?;
    let active = service.workbook_tool_context().active_sheet;
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let sheets = workbook_sheets(workbook);
    let (source_index, source_name, source_sheet) =
        workbook_sheet(workbook, &sheets, args, active)?;
    let (source_range, source_table) = match (args.opt_str("range")?, args.opt_str("table")?) {
        (Some(range), None) => (parse_range(range)?, None),
        (None, Some(name)) => {
            let table = source_sheet
                .tables
                .iter()
                .find(|table| table.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| ToolError::invalid(format!("No source table named {name}")))?;
            if table.header_rows != 1 {
                return Err(ToolError::invalid(
                    "The source table must have one header row.",
                ));
            }
            (
                CellRange {
                    last_row: table.range.last_row.saturating_sub(table.totals_rows),
                    ..table.range
                },
                Some(table.name.clone()),
            )
        }
        _ => return Err(ToolError::invalid("Specify exactly one of range or table.")),
    };
    let height = (source_range.last_row - source_range.first_row + 1) as usize;
    let width = (source_range.last_column - source_range.first_column + 1) as usize;
    if height < 2 || height.saturating_mul(width) > MAX_CELLS {
        return Err(ToolError::invalid(
            "The pivot source must have headers, data, and at most 100000 cells.",
        ));
    }
    let data = source_data(source_sheet, source_range);
    validate_headers(&data.headings)?;
    let rows = axis_fields(args, "rows", &data)?;
    let columns = axis_fields(args, "columns", &data)?;
    let mut axes = HashSet::new();
    if rows
        .iter()
        .chain(&columns)
        .any(|field| !axes.insert(*field))
    {
        return Err(ToolError::invalid(
            "A field can appear only once across the row and column axes.",
        ));
    }
    let values = args["values"]
        .as_array()
        .filter(|values| (1..=8).contains(&values.len()))
        .ok_or_else(|| ToolError::invalid("Specify one to eight value fields."))?;
    let mut entries = Vec::new();
    let mut captions = HashSet::new();
    for entry in values {
        let field = entry.str_arg("field")?;
        let function = entry.str_arg("aggregate")?;
        if !AGGREGATIONS.contains(&function) || !captions.insert((field, function)) {
            return Err(ToolError::invalid(
                "Each value field must use a supported aggregation and be unique.",
            ));
        }
        entries.push(PivotDataField {
            name: format!("{function} of {field}"),
            field: field_index(&data, field)? as u32,
            function: function.into(),
            ..Default::default()
        });
    }
    let (index, dest_name, dest_sheet) = workbook_sheet(
        workbook,
        &sheets,
        &json!({"sheet":args["destination_sheet"]}),
        active,
    )?;
    let slide = workbook.slides[*index].id;
    check_output_size(&data, &rows, &columns, entries.len(), false)?;
    let name = (1..)
        .map(|n| format!("PivotTable{n}"))
        .find(|name| {
            !sheets.iter().any(|(_, _, sheet)| {
                sheet
                    .pivots
                    .iter()
                    .any(|pivot| pivot.name.eq_ignore_ascii_case(name))
            })
        })
        .unwrap();
    let source = PivotSource {
        sheet: Some(source_name.clone()),
        range: Some(source_range),
        name: source_table,
    };
    let mut pivot = PivotTable::new(&name, source, &data, at.first_row, at.first_column);
    for &field in &rows {
        pivot.row_fields.push(field as i32);
        pivot.fields[field].axis = Some(PivotAxis::Row);
    }
    for &field in &columns {
        pivot.col_fields.push(field as i32);
        pivot.fields[field].axis = Some(PivotAxis::Column);
    }
    pivot.data_fields = entries;
    if pivot.data_fields.len() > 1 {
        pivot.col_fields.push(-2);
    }
    let pivot_index = dest_sheet.pivots.len();
    let replacement = ibis_formats::rebuilt_pivot(workbook, slide, pivot_index, Some(pivot))
        .ok_or_else(|| ToolError::invalid("Could not build the pivot from this source."))?;
    let range = replacement.pivots[pivot_index].location;
    if range.last_row >= 1_048_576 || range.last_column >= 16_384 {
        return Err(ToolError::new(
            "out_of_range",
            "The pivot does not fit in the worksheet.",
        ));
    }
    if slide == workbook.slides[*source_index].id && overlaps(&source_range, &range) {
        return Err(ToolError::new(
            "destination_occupied",
            "The pivot overlaps its source range.",
        ));
    }
    blank_destination(dest_sheet, &range)?;
    let summary = json!({"sheet":dest_name,"range":range_text(&range),"pivot":name});
    sheet_edit(
        service,
        id,
        vec![SheetCommand::ReplaceSheet {
            sheet: slide,
            replacement: Box::new(replacement),
        }],
        summary,
    )
}

fn list_pivots(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let offset = args.opt_usize("offset")?.unwrap_or(0);
    let limit = args.opt_usize("limit")?.unwrap_or(25).clamp(1, 50);
    let active = service.workbook_tool_context().active_sheet;
    let open = workbook_document(service, args.str_arg("document_id")?)?;
    let workbook = open.document.presentation();
    let sheets = workbook_sheets(workbook);
    let (_, name, sheet) = workbook_sheet(workbook, &sheets, args, active)?;
    let pivots: Vec<_> = sheet.pivots.iter().skip(offset).take(limit).map(|pivot| {
        let heading = |field: i32| {
            usize::try_from(field).ok().and_then(|i| pivot.fields.get(i)).map(|field| &field.name)
        };
        let source = ibis_formats::pivot_source_location(workbook, pivot).map(|(slide, range)| {
            let source_sheet = slide.and_then(|id| workbook.slide(id)).and_then(|slide| slide.name.as_deref()).unwrap_or(name);
            json!({"sheet": source_sheet, "range": range_text(&range), "name": pivot.source.name})
        });
        let values: Vec<_> = pivot.data_fields.iter().map(|value| json!({
            "name": value.name, "field": heading(value.field as i32), "aggregate": value.function,
        })).collect();
        json!({
            "name": pivot.name, "range": range_text(&pivot.location), "source": source,
            "rows": pivot.row_fields.iter().filter_map(|&i| heading(i)).collect::<Vec<_>>(),
            "columns": pivot.col_fields.iter().filter_map(|&i| heading(i)).collect::<Vec<_>>(),
            "filters": pivot.page_fields.iter().filter_map(|field| heading(field.field as i32)).collect::<Vec<_>>(),
            "values": values,
        })
    }).collect();
    let count = sheet.pivots.len();
    let next = offset.saturating_add(pivots.len());
    let mut result = json!({"sheet":name,"count":count,"pivots":pivots});
    if next < count {
        result["next_offset"] = json!(next);
    }
    Ok(result)
}

fn check_output_size(
    data: &PivotData,
    rows: &[usize],
    columns: &[usize],
    values: usize,
    data_on_rows: bool,
) -> Result<(), ToolError> {
    let (row_values, col_values) = if data_on_rows {
        (values.max(1), 1)
    } else {
        (1, values.max(1))
    };
    let rows_bound = axis_size(data, rows).saturating_mul(row_values) + columns.len() + 4;
    let cols_bound = axis_size(data, columns).saturating_mul(col_values) + rows.len() + 2;
    if rows_bound.saturating_mul(cols_bound) > MAX_CELLS {
        return Err(ToolError::new(
            "out_of_range",
            "The pivot output would be too large.",
        ));
    }
    Ok(())
}

fn refresh_pivot(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let id = args.str_arg("document_id")?;
    let name = args.str_arg("pivot")?;
    let active = service.workbook_tool_context().active_sheet;
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let sheets = workbook_sheets(workbook);
    let (slide_index, sheet_name, sheet) = workbook_sheet(workbook, &sheets, args, active)?;
    let slide = workbook.slides[*slide_index].id;
    let (index, _) = sheet
        .pivots
        .iter()
        .enumerate()
        .find(|(_, pivot)| pivot.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| ToolError::new("no_such_pivot", format!("No pivot named {name}")))?;
    let (replacement, chart_commands) = rebuild_pivot(workbook, slide, sheet, index)?;
    let refreshed = &replacement.pivots[index];
    let summary = json!({
        "sheet": sheet_name, "pivot": refreshed.name, "range": range_text(&refreshed.location),
        "updated_charts": chart_commands.len(),
    });
    let mut commands = vec![ibis_doc_model::Command::Sheet(SheetCommand::ReplaceSheet {
        sheet: slide,
        replacement: Box::new(replacement),
    })];
    commands.extend(chart_commands);
    crate::edit::execute(open, commands)?;
    Ok(crate::edit::edited(id, open, summary, Vec::new()))
}

fn rebuild_pivot(
    workbook: &ibis_doc_model::Presentation,
    slide: ibis_doc_model::SlideId,
    sheet: &Sheet,
    index: usize,
) -> Result<(Sheet, Vec<ibis_doc_model::Command>), ToolError> {
    let pivot = &sheet.pivots[index];
    let (source_slide, source_range) = ibis_formats::pivot_source_location(workbook, pivot)
        .ok_or_else(|| ToolError::invalid("The pivot source is unavailable."))?;
    let height = (source_range.last_row - source_range.first_row + 1) as usize;
    let width = (source_range.last_column - source_range.first_column + 1) as usize;
    if height.saturating_mul(width) > MAX_CELLS {
        return Err(ToolError::new(
            "out_of_range",
            "The pivot source exceeds 100000 cells.",
        ));
    }
    let data = ibis_formats::pivot_data(workbook, slide, index)
        .ok_or_else(|| ToolError::invalid("The pivot source is unavailable."))?;
    let axis = |fields: &[i32]| -> Vec<usize> {
        fields
            .iter()
            .filter_map(|&i| usize::try_from(i).ok())
            .filter(|&i| i < width)
            .collect()
    };
    check_output_size(
        &data,
        &axis(&pivot.row_fields),
        &axis(&pivot.col_fields),
        pivot.data_fields.len(),
        pivot.data_on_rows,
    )?;
    let replacement = ibis_formats::refreshed_pivot(workbook, slide, index)
        .ok_or_else(|| ToolError::invalid("Could not refresh the pivot."))?;
    let refreshed = &replacement.pivots[index];
    for area in pivot_areas(refreshed) {
        let size = ((area.last_row - area.first_row + 1) as usize)
            .saturating_mul((area.last_column - area.first_column + 1) as usize);
        if area.last_row >= 1_048_576 || area.last_column >= 16_384 || size > MAX_CELLS {
            return Err(ToolError::new(
                "out_of_range",
                "The refreshed pivot does not fit the output limits.",
            ));
        }
        if source_slide.unwrap_or(slide) == slide && overlaps(&source_range, &area) {
            return Err(ToolError::new(
                "destination_occupied",
                "The refreshed pivot overlaps its source.",
            ));
        }
        available_destination(sheet, &area, Some(index))?;
    }
    let charts = ibis_formats::pivot_chart_commands(workbook, slide, &replacement, refreshed);
    Ok((replacement, charts))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn refresh_query_table(
    service: &mut Service,
    args: &Value,
    result: ibis_doc_model::query::SqliteRows,
    definition: &Value,
) -> Result<Value, ToolError> {
    use ibis_doc_model::sheet_command::{CellInput, PutCell};
    use ibis_doc_model::{Command, Document};

    let id = args.str_arg("document_id")?;
    let name = args.str_arg("table")?;
    let open = workbook_document(service, id)?;
    let workbook = open.document.presentation();
    let sheets = workbook_sheets(workbook);
    let (slide_index, sheet_name, sheet, index, table) = sheets
        .iter()
        .find_map(|(slide_index, name_on_tab, sheet)| {
            let (index, table) = sheet
                .tables
                .iter()
                .enumerate()
                .find(|(_, table)| table.name.eq_ignore_ascii_case(name))?;
            Some((*slide_index, name_on_tab, *sheet, index, table))
        })
        .ok_or_else(|| ToolError::invalid("The report table is missing."))?;
    let current = crate::tools_sqlite::query_report_definition(workbook, args)?;
    if current != *definition {
        return Err(ToolError::new(
            "report_changed",
            "The report definition changed. Retry the refresh.",
        ));
    }
    if result.truncated || result.rows.is_empty() {
        return Err(ToolError::invalid(
            "Refresh needs a complete result with at least one data row.",
        ));
    }
    if table.header_rows != 1 || table.totals_rows != 0 || table.columns != result.columns {
        return Err(ToolError::new(
            "schema_changed",
            "Report refresh requires the same column headings and a table without a totals row.",
        ));
    }
    if sheet.protection.is_some() {
        return Err(ToolError::new(
            "edit_rejected",
            "The destination worksheet is protected.",
        ));
    }
    let old = table.range;
    let range = CellRange {
        last_row: old.first_row + result.rows.len() as u32,
        ..old
    };
    if range.last_row >= 1_048_576 {
        return Err(ToolError::new(
            "out_of_range",
            "The result does not fit in the worksheet.",
        ));
    }
    if range.last_row > old.last_row {
        blank_destination(
            sheet,
            &CellRange {
                first_row: old.last_row + 1,
                ..range
            },
        )?;
    }
    let mut cells = query_cells(&result, range, true)?;
    for row in range.last_row + 1..=old.last_row {
        for column in old.first_column..=old.last_column {
            cells.push(PutCell {
                row,
                column,
                input: CellInput::Clear,
                style: None,
            });
        }
    }
    let slide = workbook.slides[slide_index].id;
    let mut replacement = table.clone();
    replacement.range = range;
    let command = Command::Sheet(SheetCommand::SetTable {
        sheet: slide,
        index,
        table: Some(replacement),
        cells,
    });
    let mut commands = vec![command.clone()];
    let mut scratch = Document::new(workbook.clone());
    let apply = |scratch: &mut Document, commands| {
        scratch
            .execute(commands)
            .map_err(|error| ToolError::new("edit_rejected", format!("{error:?}")))
    };
    apply(&mut scratch, vec![command])?;

    let mut updated_pivots = 0;
    let mut updated_charts = 0;
    // Follow named-table sources, which also follow a change in the number of records.
    for (pivot_slide_index, _, candidate) in &sheets {
        let pivot_slide = workbook.slides[*pivot_slide_index].id;
        for (pivot_index, pivot) in candidate.pivots.iter().enumerate() {
            if !pivot
                .source
                .name
                .as_ref()
                .is_some_and(|source| source.eq_ignore_ascii_case(name))
            {
                continue;
            }
            let view = scratch.presentation();
            let sheet = view
                .slide(pivot_slide)
                .and_then(ibis_doc_model::sheet_calc::sheet_of_slide)
                .ok_or_else(|| ToolError::invalid("The pivot worksheet is missing."))?;
            let (updated, charts) = rebuild_pivot(view, pivot_slide, sheet, pivot_index)?;
            updated_pivots += 1;
            updated_charts += charts.len();
            let mut update = vec![Command::Sheet(SheetCommand::ReplaceSheet {
                sheet: pivot_slide,
                replacement: Box::new(updated),
            })];
            update.extend(charts);
            apply(&mut scratch, update.clone())?;
            commands.extend(update);
        }
    }
    let summary = json!({"sheet":sheet_name,"table":table.name,"range":range_text(&range),
        "rows":result.rows.len(),"updated_pivots":updated_pivots,"updated_charts":updated_charts});
    crate::edit::execute(open, commands)?;
    Ok(crate::edit::edited(id, open, summary, Vec::new()))
}
