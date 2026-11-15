// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! OpenAI Tool definitions and JSON argument parsers for spreadsheet operations.

use ibis_doc_model::ai_sheet::{
    AiBorderStyle, AiCellEdit, AiCellFormat, AiCellValue, build_clear_cells_command,
    build_delete_axis_command, build_insert_axis_command, build_merge_command,
    build_set_cells_command, build_set_format_command, parse_cell_range,
};
use ibis_doc_model::command::Command;
use ibis_doc_model::model::SlideId;
use ibis_doc_model::sheet::{CellAlign, CellRange, CellVerticalAlign};
use ibis_doc_model::sheet_command::Axis;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ai_client::{ToolDefinition, ToolFunctionDef};

pub const MAX_READ_CELLS: usize = 200;
pub const MAX_CLEAR_CELLS: usize = 500;

/// Tool definitions exposed to the OpenAI-compatible API for spreadsheets.
pub fn spreadsheet_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "get_workbook_overview",
                description: "Get sheet names, active sheet, used ranges, and current selection in the workbook.",
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "read_range",
                description: "Read non-empty cell values and formulas in an A1 range (e.g. 'A1:D20'). Maximum 200 cells per call.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": {
                            "type": "string",
                            "description": "Optional sheet name (defaults to the active sheet)."
                        },
                        "range": {
                            "type": "string",
                            "description": "A1-style cell or range, e.g. 'A1:D20' or 'B2'."
                        }
                    },
                    "required": ["range"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "set_cells",
                description: "Write values or formulas into one or more cells. Formulas should start with '='.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": {
                            "type": "string",
                            "description": "Optional sheet name (defaults to the active sheet)."
                        },
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "cell": { "type": "string", "description": "A1 cell reference, e.g. 'C2'." },
                                    "value": { "description": "Number, string, boolean, null (to clear), or formula string starting with '='." }
                                },
                                "required": ["cell", "value"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["edits"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "set_format",
                description: "Apply formatting (bold, italic, underline, colors, font size, alignment, wrap, borders, number format) to an A1 range.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string", "description": "Optional sheet name (defaults to the active sheet)." },
                        "range": { "type": "string", "description": "A1-style range, e.g. 'A1:D1'." },
                        "bold": { "type": "boolean" },
                        "italic": { "type": "boolean" },
                        "underline": { "type": "boolean" },
                        "text_color": { "type": "string", "description": "Hex color '#RRGGBB'." },
                        "fill_color": { "type": ["string", "null"], "description": "Hex color '#RRGGBB' or null to clear fill." },
                        "number_format": { "type": "string", "description": "Excel number format string, e.g. '#,##0' or '0.0%'." },
                        "align": { "type": "string", "enum": ["left", "center", "right"], "description": "Horizontal alignment." },
                        "vertical_align": { "type": "string", "enum": ["top", "middle", "bottom"], "description": "Vertical alignment." },
                        "wrap_text": { "type": "boolean", "description": "Enable or disable text wrapping." },
                        "font_size": { "type": "number", "description": "Font size in points (e.g. 11, 14)." },
                        "border": { "type": "string", "enum": ["all", "outline", "bottom", "none"], "description": "Border preset to apply." }
                    },
                    "required": ["range"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "clear_cells",
                description: "Clear cell values/formulas and/or formatting in an A1 range.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string", "description": "Optional sheet name (defaults to the active sheet)." },
                        "range": { "type": "string", "description": "A1-style range, e.g. 'A1:D10'." },
                        "clear_values": { "type": "boolean", "default": true },
                        "clear_formats": { "type": "boolean", "default": false }
                    },
                    "required": ["range"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "select_range",
                description: "Move the active selection in the spreadsheet UI to an A1 range (and optionally switch sheet).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string", "description": "Optional sheet name (defaults to the active sheet)." },
                        "range": { "type": "string", "description": "A1-style cell or range, e.g. 'A1:C5'." }
                    },
                    "required": ["range"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "switch_sheet",
                description: "Switch the active sheet in the workbook UI to the specified sheet name.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string", "description": "Exact name of the sheet to activate." }
                    },
                    "required": ["sheet"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "create_sheet",
                description: "Create a new sheet in the workbook and make it active.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Optional name for the new sheet." }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "insert_rows_or_cols",
                description: "Insert rows or columns at a 1-based row number or column letter (e.g. row '3' or column 'B').",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string", "description": "Optional sheet name (defaults to the active sheet)." },
                        "axis": { "type": "string", "enum": ["row", "col"] },
                        "at": { "type": "integer", "minimum": 1, "description": "1-based row or column index where insertion begins." },
                        "count": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 1 }
                    },
                    "required": ["axis", "at"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "delete_rows_or_cols",
                description: "Delete rows or columns starting at a 1-based row or column index.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string", "description": "Optional sheet name (defaults to the active sheet)." },
                        "axis": { "type": "string", "enum": ["row", "col"] },
                        "at": { "type": "integer", "minimum": 1, "description": "1-based row or column index where deletion begins." },
                        "count": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 1 }
                    },
                    "required": ["axis", "at"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "merge_cells",
                description: "Merge or unmerge a rectangular cell range.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string", "description": "Optional sheet name (defaults to the active sheet)." },
                        "range": { "type": "string", "description": "A1-style range, e.g. 'A1:D1'." },
                        "merge": { "type": "boolean", "default": true, "description": "true to merge, false to unmerge." },
                        "center": { "type": "boolean", "default": false, "description": "When merging, also center-align horizontally." }
                    },
                    "required": ["range"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "check_machine_readability",
                description: "Scan the workbook (or a specific sheet) for machine-readability candidates such as merged cells, hidden rows/columns, formulas, and platform-dependent or private-use characters.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string", "description": "Optional sheet name (omit to scan all sheets)." }
                    },
                    "additionalProperties": false
                }),
            },
        },
    ]
}

#[derive(Deserialize)]
struct OptionalSheetArg {
    sheet: Option<String>,
}

#[derive(Deserialize)]
struct SwitchSheetArg {
    sheet: String,
}

#[derive(Deserialize)]
struct CreateSheetArg {
    name: Option<String>,
}

#[derive(Deserialize)]
struct AxisOpArgs {
    axis: String,
    at: u32,
    #[serde(default = "default_one")]
    count: u32,
}

fn default_one() -> u32 {
    1
}

#[derive(Deserialize)]
struct MergeCellsArgs {
    range: String,
    #[serde(default = "default_true")]
    merge: bool,
    #[serde(default)]
    center: bool,
}

#[derive(Deserialize)]
pub struct RangeArg {
    pub range: String,
}

#[derive(Deserialize)]
struct SetCellsArgs {
    edits: Vec<SetCellItem>,
}

#[derive(Deserialize)]
struct SetCellItem {
    cell: String,
    value: Value,
}

#[derive(Deserialize)]
struct SetFormatArgs {
    range: String,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    text_color: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_nullable")]
    fill_color: Option<Option<String>>,
    number_format: Option<String>,
    align: Option<String>,
    vertical_align: Option<String>,
    wrap_text: Option<bool>,
    font_size: Option<f32>,
    border: Option<String>,
}

fn deserialize_optional_nullable<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(deserializer)?))
}

#[derive(Deserialize)]
struct ClearCellsArgs {
    range: String,
    #[serde(default = "default_true")]
    clear_values: bool,
    #[serde(default)]
    clear_formats: bool,
}

fn default_true() -> bool {
    true
}

pub fn parse_range_arg(arguments_json: &str) -> Result<CellRange, String> {
    let arg: RangeArg =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    parse_cell_range(&arg.range).ok_or_else(|| format!("invalid range '{}'", arg.range))
}

pub fn parse_set_cells_command(slide: SlideId, arguments_json: &str) -> Result<Command, String> {
    let args: SetCellsArgs =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    let edits = args
        .edits
        .into_iter()
        .map(|item| {
            let value = match item.value {
                Value::Null => AiCellValue::Empty,
                Value::Bool(b) => AiCellValue::Bool(b),
                Value::Number(n) => AiCellValue::Number(
                    n.as_f64()
                        .ok_or_else(|| format!("number out of f64 range for '{}'", item.cell))?,
                ),
                Value::String(s) => {
                    if let Some(formula) = s.strip_prefix('=') {
                        AiCellValue::Formula(formula.to_string())
                    } else {
                        AiCellValue::Text(s)
                    }
                }
                other => {
                    return Err(format!(
                        "unsupported cell value type for '{}': {other}",
                        item.cell
                    ));
                }
            };
            Ok(AiCellEdit {
                cell: item.cell,
                value,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    build_set_cells_command(slide, edits)
}

pub fn parse_sheet_arg(arguments_json: &str) -> Result<Option<String>, String> {
    if arguments_json.trim().is_empty() {
        return Ok(None);
    }
    let arg: OptionalSheetArg =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    Ok(arg.sheet.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    }))
}

pub fn parse_switch_sheet_arg(arguments_json: &str) -> Result<String, String> {
    let arg: SwitchSheetArg =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    let name = arg.sheet.trim().to_string();
    if name.is_empty() {
        return Err("sheet name must not be empty".to_string());
    }
    Ok(name)
}

pub fn parse_create_sheet_arg(arguments_json: &str) -> Result<Option<String>, String> {
    if arguments_json.trim().is_empty() {
        return Ok(None);
    }
    let arg: CreateSheetArg =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    Ok(arg.name.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    }))
}

fn parse_axis_args(arguments_json: &str) -> Result<(Axis, u32, u32), String> {
    let args: AxisOpArgs =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    let axis = match args.axis.trim().to_ascii_lowercase().as_str() {
        "row" | "rows" => Axis::Rows,
        "col" | "cols" | "column" | "columns" => Axis::Columns,
        other => return Err(format!("invalid axis '{other}' (expected 'row' or 'col')")),
    };
    if args.at == 0 {
        return Err("at must be >= 1 (1-based index)".to_string());
    }
    Ok((axis, args.at - 1, args.count))
}

pub fn parse_insert_axis_command(slide: SlideId, arguments_json: &str) -> Result<Command, String> {
    let (axis, at, count) = parse_axis_args(arguments_json)?;
    build_insert_axis_command(slide, axis, at, count)
}

pub fn parse_delete_axis_command(slide: SlideId, arguments_json: &str) -> Result<Command, String> {
    let (axis, at, count) = parse_axis_args(arguments_json)?;
    build_delete_axis_command(slide, axis, at, count)
}

pub fn parse_merge_cells_command(slide: SlideId, arguments_json: &str) -> Result<Command, String> {
    let args: MergeCellsArgs =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    let range =
        parse_cell_range(&args.range).ok_or_else(|| format!("invalid range '{}'", args.range))?;
    Ok(build_merge_command(slide, range, args.merge, args.center))
}

pub fn parse_set_format_command(slide: SlideId, arguments_json: &str) -> Result<Command, String> {
    let args: SetFormatArgs =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    let range =
        parse_cell_range(&args.range).ok_or_else(|| format!("invalid range '{}'", args.range))?;
    let align = match args.align.as_deref() {
        None => None,
        Some("left") => Some(CellAlign::Left),
        Some("center") => Some(CellAlign::Center),
        Some("right") => Some(CellAlign::Right),
        Some(other) => return Err(format!("invalid align '{other}'")),
    };
    let vertical_align = match args.vertical_align.as_deref() {
        None => None,
        Some("top") => Some(CellVerticalAlign::Top),
        Some("middle") => Some(CellVerticalAlign::Center),
        Some("bottom") => Some(CellVerticalAlign::Bottom),
        Some(other) => return Err(format!("invalid vertical_align '{other}'")),
    };
    let border = match args.border.as_deref() {
        None => None,
        Some("all") => Some(AiBorderStyle::All),
        Some("outline") => Some(AiBorderStyle::Outline),
        Some("bottom") => Some(AiBorderStyle::Bottom),
        Some("none") => Some(AiBorderStyle::None),
        Some(other) => return Err(format!("invalid border '{other}'")),
    };
    build_set_format_command(
        slide,
        vec![range],
        AiCellFormat {
            bold: args.bold,
            italic: args.italic,
            underline: args.underline,
            text_color: args.text_color,
            fill_color: args.fill_color,
            number_format: args.number_format,
            align,
            vertical_align,
            wrap_text: args.wrap_text,
            font_size_pt: args.font_size,
            border,
        },
    )
}

pub fn parse_clear_cells_command(slide: SlideId, arguments_json: &str) -> Result<Command, String> {
    let args: ClearCellsArgs =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    let range =
        parse_cell_range(&args.range).ok_or_else(|| format!("invalid range '{}'", args.range))?;
    build_clear_cells_command(
        slide,
        vec![range],
        args.clear_values,
        args.clear_formats,
        MAX_CLEAR_CELLS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibis_doc_model::sheet_command::{CellInput, SheetCommand};

    #[test]
    fn parses_set_cells_json() {
        let cmd = parse_set_cells_command(
            SlideId(1),
            r#"{"edits":[{"cell":"A1","value":"Total"},{"cell":"B1","value":120},{"cell":"C1","value":"=B1*1.1"}]}"#,
        )
        .unwrap();
        match cmd {
            Command::Sheet(SheetCommand::SetCells { edits, .. }) => {
                assert_eq!(edits.len(), 3);
                assert_eq!(edits[2].input, CellInput::Formula("B1*1.1".to_string()));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_set_format_json() {
        let cmd = parse_set_format_command(
            SlideId(1),
            r##"{"range":"A1:C1","bold":true,"fill_color":"#E8F0FE","number_format":"#,##0"}"##,
        )
        .unwrap();
        assert!(matches!(cmd, Command::Group(v) if v.len() == 3));
    }
}
