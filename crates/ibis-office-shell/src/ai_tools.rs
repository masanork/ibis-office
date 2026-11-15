// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! OpenAI Tool definitions and JSON argument parsers for spreadsheet operations.

use ibis_doc_model::ai_sheet::{
    AiCellEdit, AiCellFormat, AiCellValue, build_clear_cells_command, build_set_cells_command,
    build_set_format_command, parse_cell_range,
};
use ibis_doc_model::command::Command;
use ibis_doc_model::model::SlideId;
use ibis_doc_model::sheet::CellRange;
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
                description: "Read non-empty cell values and formulas in an A1 range (e.g. 'A1:D20') on the active sheet. Maximum 200 cells per call.",
                parameters: json!({
                    "type": "object",
                    "properties": {
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
                description: "Write values or formulas into one or more cells on the active sheet. Formulas should start with '='.",
                parameters: json!({
                    "type": "object",
                    "properties": {
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
                description: "Apply formatting (bold, italic, underline, colors, number format) to an A1 range on the active sheet.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "range": { "type": "string", "description": "A1-style range, e.g. 'A1:D1'." },
                        "bold": { "type": "boolean" },
                        "italic": { "type": "boolean" },
                        "underline": { "type": "boolean" },
                        "text_color": { "type": "string", "description": "Hex color '#RRGGBB'." },
                        "fill_color": { "type": ["string", "null"], "description": "Hex color '#RRGGBB' or null to clear fill." },
                        "number_format": { "type": "string", "description": "Excel number format string, e.g. '#,##0' or '0.0%'." }
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
                description: "Clear cell values/formulas and/or formatting in an A1 range on the active sheet.",
                parameters: json!({
                    "type": "object",
                    "properties": {
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
                description: "Move the active selection in the spreadsheet UI to an A1 range.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "range": { "type": "string", "description": "A1-style cell or range, e.g. 'A1:C5'." }
                    },
                    "required": ["range"],
                    "additionalProperties": false
                }),
            },
        },
    ]
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

pub fn parse_set_format_command(slide: SlideId, arguments_json: &str) -> Result<Command, String> {
    let args: SetFormatArgs =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    let range =
        parse_cell_range(&args.range).ok_or_else(|| format!("invalid range '{}'", args.range))?;
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
