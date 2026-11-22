// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! UI-independent helpers for AI Assistant spreadsheet tools.
//!
//! Converts between A1-style references / lightweight cell snapshots and
//! [`Command`] / [`SheetCommand`] values so the AI tool adapter can be
//! tested without a window or HTTP client.

use crate::command::Command;
use crate::drawing::Rgb;
use crate::model::SlideId;
use crate::sheet::{
    Border, BorderLine, CellRange, CellValue, Sheet, SheetComment,
    Workbook, cell_name,
};
use crate::EMU_PER_POINT;
use crate::sheet_command::{Axis, CellInput, Edge, SheetCommand, StyleChange};

/// Summary of one sheet in a workbook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetSummary {
    pub name: String,
    pub slide: Option<SlideId>,
    pub active: bool,
    pub used_range: Option<String>,
    pub cell_count: usize,
}

/// Overview of a workbook for LLM context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkbookOverview {
    pub sheets: Vec<SheetSummary>,
    pub active_sheet: Option<String>,
    pub selection: Option<String>,
}

/// Lightweight snapshot of a single non-empty cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellSnapshot {
    pub cell: String,
    pub row: u32,
    pub column: u32,
    pub text: String,
    pub formula: Option<String>,
}

/// Border preset requested by `set_format`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiBorderStyle {
    All,
    Outline,
    Bottom,
    None,
}

/// Parse an A1-style cell reference (e.g., `"A1"`, `"$B$12"`) into `(row, column)` (0-based).
pub fn parse_cell_ref(reference: &str) -> Option<(u32, u32)> {
    let corner = ibis_calc::formula::parser::parse_cell(reference.trim())?;
    Some((corner.row, corner.column))
}

/// Parse an A1-style range (e.g., `"A1:C10"` or `"B2"`) into a normalized [`CellRange`].
pub fn parse_cell_range(range: &str) -> Option<CellRange> {
    let trimmed = range.trim();
    if let Some((start, end)) = trimmed.split_once(':') {
        let (r1, c1) = parse_cell_ref(start)?;
        let (r2, c2) = parse_cell_ref(end)?;
        Some(CellRange {
            first_row: r1.min(r2),
            first_column: c1.min(c2),
            last_row: r1.max(r2),
            last_column: c1.max(c2),
        })
    } else {
        let (r, c) = parse_cell_ref(trimmed)?;
        Some(CellRange {
            first_row: r,
            first_column: c,
            last_row: r,
            last_column: c,
        })
    }
}

/// Format a [`CellRange`] as A1 notation (e.g., `"A1:C10"` or `"B2"`).
pub fn format_range(range: &CellRange) -> String {
    let start = cell_name(range.first_column, range.first_row);
    if range.first_row == range.last_row && range.first_column == range.last_column {
        start
    } else {
        format!("{start}:{}", cell_name(range.last_column, range.last_row))
    }
}

/// Parse a `#RRGGBB` or `RRGGBB` hex color into [`Rgb`].
pub fn parse_hex_rgb(hex: &str) -> Result<Rgb, String> {
    let clean = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    if clean.len() != 6 || !clean.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid hex color '{hex}', expected #RRGGBB"));
    }
    let r = u8::from_str_radix(&clean[0..2], 16).map_err(|e| e.to_string())?;
    let g = u8::from_str_radix(&clean[2..4], 16).map_err(|e| e.to_string())?;
    let b = u8::from_str_radix(&clean[4..6], 16).map_err(|e| e.to_string())?;
    Ok(Rgb::from_rgb(r, g, b))
}

/// Compute the bounding box of cells that have non-empty values or formulas.
pub fn sheet_used_range(sheet: &Sheet) -> Option<CellRange> {
    let mut bounds: Option<CellRange> = None;
    for cell in &sheet.cells {
        if matches!(cell.value, CellValue::Empty) && cell.formula.is_none() && cell.text.is_empty() {
            continue;
        }
        bounds = Some(match bounds {
            None => CellRange {
                first_row: cell.row,
                first_column: cell.column,
                last_row: cell.row,
                last_column: cell.column,
            },
            Some(mut b) => {
                b.first_row = b.first_row.min(cell.row);
                b.first_column = b.first_column.min(cell.column);
                b.last_row = b.last_row.max(cell.row);
                b.last_column = b.last_column.max(cell.column);
                b
            }
        });
    }
    bounds
}

/// Build an overview of the workbook and the active sheet's selection.
pub fn workbook_overview<'a>(
    workbook: &'a Workbook,
    active_slide: SlideId,
    sheet_for_slide: &dyn Fn(SlideId) -> Option<&'a Sheet>,
    selection: Option<CellRange>,
) -> WorkbookOverview {
    let mut sheets = Vec::with_capacity(workbook.sheets.len());
    let mut active_sheet = None;
    for entry in &workbook.sheets {
        let active = entry.slide == Some(active_slide);
        if active {
            active_sheet = Some(entry.name.clone());
        }
        let sheet = entry
            .slide
            .and_then(|id| sheet_for_slide(id))
            .or(entry.hidden.as_deref());
        let (used_range, cell_count) = match sheet {
            Some(s) => (
                sheet_used_range(s).map(|r| format_range(&r)),
                s.cells.iter().filter(|c| !matches!(c.value, CellValue::Empty) || c.formula.is_some()).count(),
            ),
            None => (None, 0),
        };
        sheets.push(SheetSummary {
            name: entry.name.clone(),
            slide: entry.slide,
            active,
            used_range,
            cell_count,
        });
    }
    WorkbookOverview {
        sheets,
        active_sheet,
        selection: selection.map(|r| format_range(&r)),
    }
}

/// Convert an Assistant or MCP JSON value into a worksheet cell input.
pub fn parse_cell_input(value: &serde_json::Value) -> Result<CellInput, &'static str> {
    use serde_json::Value;

    match value {
        Value::Null => Ok(CellInput::Clear),
        Value::Bool(value) => Ok(CellInput::Value(CellValue::Bool(*value))),
        Value::Number(value) => value.as_f64()
            .map(|number| CellInput::Value(CellValue::Number(number)))
            .ok_or("number is out of range"),
        Value::String(text) => match text.strip_prefix('=').map(str::trim) {
            Some(formula) if !formula.is_empty() => Ok(CellInput::Formula(formula.to_owned())),
            _ => Ok(CellInput::Value(CellValue::Text(text.clone()))),
        },
        _ => Err("expected a number, a string, a boolean or null"),
    }
}

/// Build the worksheet commands for a border preset.
pub fn build_border_commands(slide: SlideId, range: CellRange, preset: AiBorderStyle) -> Vec<Command> {
    let thin = Some(Border { line: BorderLine::Thin, color: Rgb(0) });
    let edges = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];
    let outer = [
        (Edge::Top, CellRange { last_row: range.first_row, ..range }),
        (Edge::Bottom, CellRange { first_row: range.last_row, ..range }),
        (Edge::Left, CellRange { last_column: range.first_column, ..range }),
        (Edge::Right, CellRange { first_column: range.last_column, ..range }),
    ];
    let targets = match preset {
        AiBorderStyle::All | AiBorderStyle::None => edges.map(|edge| (edge, range)).to_vec(),
        AiBorderStyle::Bottom => vec![outer[1]],
        AiBorderStyle::Outline => outer.to_vec(),
    };
    let border = if preset == AiBorderStyle::None { None } else { thin };
    targets.into_iter().map(|(edge, range)| Command::Sheet(SheetCommand::SetFormat {
        sheet: slide,
        ranges: vec![range],
        change: StyleChange::Border(edge, border),
    })).collect()
}

/// Resolve an optional sheet name to its [`SlideId`], defaulting to `active_slide`.
pub fn resolve_sheet_slide(
    workbook: &Workbook,
    name: Option<&str>,
    active_slide: SlideId,
) -> Result<SlideId, String> {
    let Some(name) = name.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(active_slide);
    };
    workbook
        .sheets
        .iter()
        .find(|s| s.name == name)
        .and_then(|s| s.slide)
        .ok_or_else(|| format!("sheet '{name}' not found"))
}

/// Build a [`Command`] to insert `count` rows or columns at 0-based index `at`.
pub fn build_insert_axis_command(
    slide: SlideId,
    axis: Axis,
    at: u32,
    count: u32,
) -> Result<Command, String> {
    if count == 0 || count > 1000 {
        return Err(format!("count must be between 1 and 1000 (got {count})"));
    }
    Ok(Command::Sheet(SheetCommand::Insert {
        sheet: slide,
        axis,
        at,
        count,
    }))
}

/// Build a [`Command`] to delete `count` rows or columns starting at 0-based index `at`.
pub fn build_delete_axis_command(
    slide: SlideId,
    axis: Axis,
    at: u32,
    count: u32,
) -> Result<Command, String> {
    if count == 0 || count > 1000 {
        return Err(format!("count must be between 1 and 1000 (got {count})"));
    }
    Ok(Command::Sheet(SheetCommand::Delete {
        sheet: slide,
        axis,
        at,
        count,
    }))
}

/// Build a [`Command`] to merge or unmerge `range`.
pub fn build_merge_command(
    slide: SlideId,
    range: CellRange,
    merge: bool,
    center: bool,
) -> Command {
    if merge {
        Command::Sheet(SheetCommand::Merge {
            sheet: slide,
            range,
            center,
        })
    } else {
        Command::Sheet(SheetCommand::Unmerge { sheet: slide, range })
    }
}

/// Find non-empty cells whose display text (or formula) matches `query` (case-insensitive).
pub fn find_sheet_cells(
    sheet: &Sheet,
    query: &str,
    search_formulas: bool,
    max_results: usize,
) -> Vec<CellSnapshot> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for cell in sheet.cells.iter() {
        let text_match = cell.text.to_ascii_lowercase().contains(&q);
        let formula = sheet.visible_formula(cell);
        let formula_match = search_formulas
            && formula.is_some_and(|f| f.to_ascii_lowercase().contains(&q));
        if text_match || formula_match {
            out.push(CellSnapshot {
                cell: cell_name(cell.row, cell.column),
                row: cell.row,
                column: cell.column,
                text: cell.text.clone(),
                formula: formula.map(str::to_owned),
            });
            if out.len() >= max_results {
                break;
            }
        }
    }
    out
}

/// Find cells with formula or value errors (`#REF!`, `#DIV/0!`, etc.) on `sheet`.
pub fn audit_sheet_errors(sheet: &Sheet, max_results: usize) -> Vec<CellSnapshot> {
    let mut out = Vec::new();
    for cell in sheet.cells.iter() {
        let is_err = matches!(cell.value, CellValue::Error(_))
            || (cell.formula.is_some() && cell.text.starts_with('#'));
        if is_err {
            out.push(CellSnapshot {
                cell: cell_name(cell.row, cell.column),
                row: cell.row,
                column: cell.column,
                text: cell.text.clone(),
                formula: sheet.visible_formula(cell).map(str::to_owned),
            });
            if out.len() >= max_results {
                break;
            }
        }
    }
    out
}

/// Build a [`Command`] to set row height or column width in points for 0-based `first..=last`.
pub fn build_set_axis_size_command(
    slide: SlideId,
    axis: Axis,
    first: u32,
    last: u32,
    points: f32,
) -> Result<Command, String> {
    if first > last {
        return Err("start index must be <= end index".to_string());
    }
    if !points.is_finite() || points < 4.0 || points > 1000.0 {
        return Err(format!("size_pt must be between 4 and 1000 (got {points})"));
    }
    let size = (points * EMU_PER_POINT as f32).round() as i64;
    Ok(Command::Sheet(SheetCommand::SetSize {
        sheet: slide,
        axis,
        first,
        last,
        size,
    }))
}

/// Build a [`Command`] to set or clear a note/comment on a cell.
pub fn build_set_comment_command(
    slide: SlideId,
    cell: &str,
    text: Option<String>,
) -> Result<Command, String> {
    let (row, column) =
        parse_cell_ref(cell).ok_or_else(|| format!("invalid cell reference '{cell}'"))?;
    let comment = text.and_then(|t| {
        let trimmed = t.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(SheetComment {
                row,
                column,
                author: "Assistant".to_string(),
                text: trimmed,
                runs: Vec::new(),
                rect: None,
            })
        }
    });
    Ok(Command::Sheet(SheetCommand::SetComment {
        sheet: slide,
        row,
        column,
        comment,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::{CellKind, SheetCell};

    #[test]
    fn parses_cell_refs_and_ranges() {
        assert_eq!(parse_cell_ref("A1"), Some((0, 0)));
        assert_eq!(parse_cell_ref("$C$12"), Some((11, 2)));
        assert_eq!(parse_cell_ref("bad"), None);

        let range = parse_cell_range("C10:A2").expect("normalized range");
        assert_eq!(
            range,
            CellRange {
                first_row: 1,
                first_column: 0,
                last_row: 9,
                last_column: 2,
            }
        );
        assert_eq!(format_range(&range), "A2:C10");
        assert_eq!(
            format_range(&parse_cell_range("B3").expect("single cell")),
            "B3"
        );
    }

    #[test]
    fn find_and_audit_respect_formula_visibility() {
        let mut sheet = Sheet::default();
        sheet.cells.push(SheetCell {
            row: 0,
            column: 0,
            value: CellValue::Number(10.0),
            text: "10".to_string(),
            kind: CellKind::Number,
            formula: None,
            style: 0,
            phonetic: None,
            runs: None,
        });
        sheet.cells.push(SheetCell {
            row: 0,
            column: 1,
            value: CellValue::Number(20.0),
            text: "20".to_string(),
            kind: CellKind::Number,
            formula: Some("A1*2".to_string()),
            style: 0,
            phonetic: None,
            runs: None,
        });

        sheet.styles.push(crate::sheet::CellStyle { formula_hidden: true, ..Default::default() });
        assert_eq!(find_sheet_cells(&sheet, "A1*2", true, 10)[0].formula.as_deref(), Some("A1*2"));
        sheet.protection = Some(crate::sheet::SheetProtection::default());
        assert!(find_sheet_cells(&sheet, "A1*2", true, 10).is_empty());
        assert_eq!(find_sheet_cells(&sheet, "20", false, 10)[0].formula, None);
        sheet.cells[1].text = "#REF!".into();
        assert_eq!(audit_sheet_errors(&sheet, 10)[0].formula, None);
        sheet.protection = None;
        assert_eq!(find_sheet_cells(&sheet, "A1*2", true, 10)[0].formula.as_deref(), Some("A1*2"));
    }

    #[test]
    fn parses_json_cell_inputs() {
        use serde_json::json;

        for (value, expected) in [
            (json!(null), CellInput::Clear),
            (json!(true), CellInput::Value(CellValue::Bool(true))),
            (json!(2.5), CellInput::Value(CellValue::Number(2.5))),
            (json!("  text  "), CellInput::Value(CellValue::Text("  text  ".into()))),
            (json!("="), CellInput::Value(CellValue::Text("=".into()))),
            (json!("=  A1*2  "), CellInput::Formula("A1*2".into())),
        ] {
            assert_eq!(parse_cell_input(&value), Ok(expected));
        }
        assert!(parse_cell_input(&json!({})).is_err());
        assert!(parse_cell_input(&json!([])).is_err());
    }

    #[test]
    fn builds_outline_border_commands() {
        let range = parse_cell_range("A1:B3").unwrap();
        let commands = build_border_commands(SlideId(1), range, AiBorderStyle::Outline);
        assert_eq!(commands.len(), 4);
        assert!(matches!(&commands[0], Command::Sheet(SheetCommand::SetFormat {
            ranges, change: StyleChange::Border(Edge::Top, Some(_)), ..
        }) if ranges[0].last_row == range.first_row));
    }
}
