// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! OpenAI Tool definitions and JSON argument parsers for spreadsheet operations.

use ibis_doc_model::ai_sheet::{
    AiBorderStyle, AiCellEdit, AiCellFormat, AiCellValue, build_clear_cells_command,
    build_delete_axis_command, build_insert_axis_command, build_merge_command,
    build_set_axis_size_command, build_set_cells_command, build_set_comment_command,
    build_set_format_command, parse_cell_range, parse_hex_rgb,
};
use ibis_doc_model::command::Command;
use ibis_doc_model::flow::FlowAlign;
use ibis_doc_model::flow_command::{FlowCommand, FlowPos, ParaChange, RunChange};
use ibis_doc_model::model::SlideId;
use ibis_doc_model::sd_jwt::Es256KeyPair;
use ibis_doc_model::sd_jwt_binding::{ClaimValue, SdJwtTemplateKind};
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
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "find_cells",
                description: "Search cells on a sheet (or active sheet) by display text or formula substring.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string" },
                        "query": { "type": "string", "description": "Case-insensitive search text." },
                        "search_formulas": { "type": "boolean", "default": true }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "audit_formulas",
                description: "Scan all sheets (or a specific sheet) for formula or value errors (#REF!, #DIV/0!, #VALUE!, #NAME?, #N/A, etc.).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string", "description": "Optional sheet name (omit to scan all sheets)." }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "set_axis_size",
                description: "Set row height or column width in points for a 1-based range of rows or columns.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string" },
                        "axis": { "type": "string", "enum": ["row", "col"] },
                        "from": { "type": "integer", "minimum": 1, "description": "1-based start row or column index." },
                        "to": { "type": "integer", "minimum": 1, "description": "1-based end row or column index (defaults to 'from')." },
                        "size_pt": { "type": "number", "minimum": 4, "maximum": 1000, "description": "Size in points (e.g. 24 for row height, 90 for column width)." }
                    },
                    "required": ["axis", "from", "size_pt"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "set_cell_comment",
                description: "Add, update, or clear a note/comment on a cell.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string" },
                        "cell": { "type": "string", "description": "A1 cell reference, e.g. 'B2'." },
                        "text": { "type": ["string", "null"], "description": "Comment text, or null/empty to remove." }
                    },
                    "required": ["cell", "text"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "inspect_vc_bindings",
                description: "Inspect SD-JWT Verifiable Credential bindings (claim.*, sd.*, disclose.*) on a worksheet.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "apply_vc_template",
                description: "Populate a worksheet with a built-in SD-JWT VC template ('pid' for EUDI PID / JPKI, 'mdl' for ISO 18013-5 mDL, 'employee' for 社員証, 'enrollment' for 在学証明書, or 'completion' for 修了証明書).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string" },
                        "template": { "type": "string", "enum": ["pid", "mdl", "employee", "enrollment", "completion"] }
                    },
                    "required": ["template"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "infer_vc_schema",
                description: "Infer a portable SD-JWT VC schema JSON from the current worksheet layout, optionally applying bindings to the sheet.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string" },
                        "vct": { "type": "string", "description": "Optional Verifiable Credential Type URI." },
                        "apply": { "type": "boolean", "description": "If true, scaffold/apply the inferred bindings to the sheet." }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "bind_vc_claim",
                description: "Bind a cell or range on a worksheet as an SD-JWT VC claim (selective sd.<path> with disclose checkbox, or mandatory claim.<path>).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string" },
                        "range": { "type": "string", "description": "A1 cell or range, e.g. 'B2' or 'B10:C12'." },
                        "claim_path": { "type": "string", "description": "Claim path, e.g. 'family_name', 'address.locality', or 'vct'." },
                        "selective": { "type": "boolean", "description": "Default true (sd.<path> with disclose toggle); false for always-disclosed claim.<path>." }
                    },
                    "required": ["range", "claim_path"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "issue_vc_preview",
                description: "Issue an SD-JWT or ISO/IEC 18013-5 mDOC Verifiable Credential preview from the worksheet bindings using the local demo issuer key. Optionally override which selective claims are disclosed.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string" },
                        "format": { "type": "string", "enum": ["sd-jwt", "mdoc"], "description": "Output credential format (default 'sd-jwt')." },
                        "disclose_paths": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional list of selective claim paths (e.g. ['family_name', 'birthdate']) to disclose in the presentation; omit to use the sheet's disclose.* toggles."
                        },
                        "vct": { "type": "string", "description": "Optional fallback Verifiable Credential Type URI (or docType for mDOC)." },
                        "iss": { "type": "string", "description": "Optional fallback issuer URI (SD-JWT)." },
                        "name_space": { "type": "string", "description": "Optional mDOC namespace (default 'org.iso.18013.5.1')." }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "parse_vc_token",
                description: "Parse and inspect an SD-JWT or ISO 18013-5 mdoc token string, optionally importing its claims onto the current worksheet.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string" },
                        "token": { "type": "string", "description": "Compact SD-JWT (~ separated) or hex/base64url mdoc string." },
                        "issuer_jwk_json": { "type": "string", "description": "Optional issuer public JWK JSON for ES256 signature verification." },
                        "import_to_sheet": { "type": "boolean", "description": "If true, populate the target worksheet with the parsed claims and bindings." }
                    },
                    "required": ["token"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "prepare_oid4vp_presentation",
                description: "Parse an OID4VP Authorization Request (openid4vp:// URL or JSON with dcql_query / presentation_definition), match requested claims against the worksheet, and generate a Key Binding JWT vp_token (sd-jwt) or ISO 18013-5 DeviceResponse (mdoc) preview.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string" },
                        "request": { "type": "string", "description": "OID4VP URL (openid4vp://?...) or Authorization Request JSON." },
                        "format": { "type": "string", "enum": ["sd-jwt", "mdoc"], "description": "Optional presentation format override. Auto-detects 'mso_mdoc' from request if omitted." },
                        "name_space": { "type": "string", "description": "Optional ISO 18013-5 namespace when format is 'mdoc'." },
                        "apply_to_sheet": { "type": "boolean", "description": "If true, update the worksheet's disclose.* checkboxes to match the requested claims." }
                    },
                    "required": ["request"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "get_vc_public_keys",
                description: "Inspect the active VC issuer and holder ES256 public JWKs (never exposes private key material) so Verifiers can verify issued SD-JWTs, mDOCs, or KB-JWTs.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "which": { "type": "string", "enum": ["all", "issuer", "holder"], "description": "Which public key(s) to return (default 'all')." }
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

/// Tool definitions exposed to the OpenAI-compatible API for presentations (PPTX).
pub fn pptx_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "get_presentation_overview",
                description: "Get slide count, active slide index, and summary (1-based index, title, shape count, notes presence) of each slide.",
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
                name: "read_slide",
                description: "Read all text shapes (shape_id, kind, text) and speaker notes on a slide.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "slide_index": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "1-based slide number (defaults to the active slide)."
                        }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "set_shape_text",
                description: "Replace the text of a shape on a slide.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "slide_index": { "type": "integer", "minimum": 1, "description": "1-based slide number (defaults to active slide)." },
                        "shape_id": { "type": "integer", "minimum": 1, "description": "Shape ID returned by read_slide." },
                        "text": { "type": "string", "description": "New text content (use \\n for paragraphs)." }
                    },
                    "required": ["shape_id", "text"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "set_slide_notes",
                description: "Set the speaker notes text for a slide.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "slide_index": { "type": "integer", "minimum": 1, "description": "1-based slide number (defaults to active slide)." },
                        "notes": { "type": "string", "description": "Speaker notes text." }
                    },
                    "required": ["notes"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "add_slide",
                description: "Insert a new slide after the active slide (with optional title and body text) and switch to it.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Optional slide title text." },
                        "body": { "type": "string", "description": "Optional slide body text." }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "select_slide",
                description: "Switch the active slide in the presentation UI.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "slide_index": { "type": "integer", "minimum": 1, "description": "1-based slide number." }
                    },
                    "required": ["slide_index"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "duplicate_slide",
                description: "Duplicate a slide and insert the copy right after it.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "slide_index": { "type": "integer", "minimum": 1, "description": "1-based slide number (defaults to active slide)." }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "remove_slide",
                description: "Remove a slide from the presentation (cannot remove the last remaining slide).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "slide_index": { "type": "integer", "minimum": 1, "description": "1-based slide number to remove." }
                    },
                    "required": ["slide_index"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "move_slide",
                description: "Move a slide to a new 1-based position.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "slide_index": { "type": "integer", "minimum": 1, "description": "1-based source slide number." },
                        "to_index": { "type": "integer", "minimum": 1, "description": "1-based destination position." }
                    },
                    "required": ["slide_index", "to_index"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "insert_text_box",
                description: "Insert a new text box shape on a slide using percentage coordinates (0-100).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "slide_index": { "type": "integer", "minimum": 1, "description": "1-based slide number (defaults to active slide)." },
                        "text": { "type": "string", "description": "Text content for the box." },
                        "x_pct": { "type": "number", "default": 10.0 },
                        "y_pct": { "type": "number", "default": 20.0 },
                        "w_pct": { "type": "number", "default": 80.0 },
                        "h_pct": { "type": "number", "default": 20.0 },
                        "font_size": { "type": "number", "description": "Optional font size in points (e.g. 20)." },
                        "bold": { "type": "boolean" }
                    },
                    "required": ["text"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "set_shape_style",
                description: "Update text style (font_size, bold, italic, text_color) and/or fill color of a shape.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "slide_index": { "type": "integer", "minimum": 1 },
                        "shape_id": { "type": "integer", "minimum": 1 },
                        "font_size": { "type": "number", "description": "Font size in points." },
                        "bold": { "type": "boolean" },
                        "italic": { "type": "boolean" },
                        "text_color": { "type": "string", "description": "Hex color '#RRGGBB'." },
                        "fill_color": { "type": ["string", "null"], "description": "Hex color '#RRGGBB' or null for transparent." }
                    },
                    "required": ["shape_id"],
                    "additionalProperties": false
                }),
            },
        },
    ]
}

/// Tool definitions exposed to the OpenAI-compatible API for word-processing documents (DOCX).
pub fn docx_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "get_document_overview",
                description: "Get total block/paragraph count and previews of the first 100 paragraphs in the document.",
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
                name: "read_paragraphs",
                description: "Read full text of a range of 0-based paragraphs (up to 50 paragraphs per call).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "start": { "type": "integer", "minimum": 0, "description": "0-based starting paragraph index." },
                        "count": { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 }
                    },
                    "required": ["start"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "insert_text",
                description: "Insert text at a character offset within a 0-based paragraph (defaults to end of paragraph). Use '\\n' to create new paragraphs.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "paragraph": { "type": "integer", "minimum": 0, "description": "0-based paragraph index." },
                        "offset": { "type": "integer", "minimum": 0, "description": "Optional 0-based character offset (defaults to paragraph length)." },
                        "text": { "type": "string", "description": "Text to insert." }
                    },
                    "required": ["paragraph", "text"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "replace_paragraph_text",
                description: "Replace the entire text of a 0-based paragraph.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "paragraph": { "type": "integer", "minimum": 0, "description": "0-based paragraph index." },
                        "text": { "type": "string", "description": "Replacement text." }
                    },
                    "required": ["paragraph", "text"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "set_paragraph_align",
                description: "Set horizontal alignment for a 0-based paragraph.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "paragraph": { "type": "integer", "minimum": 0, "description": "0-based paragraph index." },
                        "align": { "type": "string", "enum": ["left", "center", "right", "justify"] }
                    },
                    "required": ["paragraph", "align"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "set_run_format",
                description: "Apply character formatting (bold, italic, underline, font_size, color) to a paragraph or character span.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "paragraph": { "type": "integer", "minimum": 0, "description": "0-based paragraph index." },
                        "start": { "type": "integer", "minimum": 0, "description": "Optional 0-based start char offset (defaults to 0)." },
                        "end": { "type": "integer", "minimum": 0, "description": "Optional 0-based end char offset (defaults to paragraph length)." },
                        "bold": { "type": "boolean" },
                        "italic": { "type": "boolean" },
                        "underline": { "type": "boolean" },
                        "font_size": { "type": "number", "description": "Font size in points (e.g. 12)." },
                        "color": { "type": "string", "description": "Hex color '#RRGGBB'." }
                    },
                    "required": ["paragraph"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "set_paragraph_style",
                description: "Apply a named paragraph style (e.g. 'Heading1', 'Heading2', 'Normal', or null to reset).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "paragraph": { "type": "integer", "minimum": 0 },
                        "style": { "type": ["string", "null"] }
                    },
                    "required": ["paragraph", "style"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "insert_table",
                description: "Insert an empty table at the start of a 0-based paragraph.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "paragraph": { "type": "integer", "minimum": 0 },
                        "rows": { "type": "integer", "minimum": 1, "maximum": 20 },
                        "columns": { "type": "integer", "minimum": 1, "maximum": 12 }
                    },
                    "required": ["paragraph", "rows", "columns"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "find_text_in_document",
                description: "Search all paragraphs in the document for a substring and return matching 0-based paragraph indices and previews.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Case-insensitive search query." }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "export_active_lawtext",
                description: "Export the currently open document as Lawtext plain text and return its law title, law number, and article count.",
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
                name: "generate_lawtext_5set",
                description: "Generate the Japanese legislative 5-piece package (AmendmentIr, 新旧対照表 Markdown, 改め文 draft + verification, 参照条文 Markdown, and prompts for 要綱/理由) from old and new Lawtext or e-Gov XML. If new_lawtext is omitted, the active document is used as the new version.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "old_lawtext": { "type": "string", "description": "Current (現行) statute in Lawtext or e-Gov XML format. Optional if already loaded via the file picker." },
                        "new_lawtext": { "type": "string", "description": "Optional amended (改正後) statute in Lawtext or e-Gov XML. Defaults to the active document." },
                        "policy_memo": { "type": "string", "description": "Optional policy background memo for drafting 要綱 and 理由." },
                        "old_on_left": { "type": "boolean", "description": "Put 現行 on the left column of 新旧対照表 (default false: 改正後 | 現行)." }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "verify_lawtext_aratame",
                description: "Verify a 改め文 (amendment text) by applying it to old_lawtext (or the loaded baseline file) and comparing the simulated result against expected_new_lawtext (defaults to the active document).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "old_lawtext": { "type": "string", "description": "Current (現行) statute in Lawtext or e-Gov XML. Optional if already loaded via the file picker." },
                        "aratame_text": { "type": "string", "description": "Amendment text (改め文) to verify." },
                        "expected_new_lawtext": { "type": "string", "description": "Optional expected amended statute in Lawtext or e-Gov XML. Defaults to the active document." }
                    },
                    "required": ["aratame_text"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "lint_lawtext",
                description: "Run static legislative drafting lint checks (dangling internal references, renumbered/deleted ripple references, conjunction hierarchy 及び/並びに・又は/若しくは, duplicate abbreviations, and article numbering) on the active document or provided Lawtext/XML.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "new_lawtext": { "type": "string", "description": "Optional Lawtext or e-Gov XML to lint. Defaults to the active document." },
                        "old_lawtext": { "type": "string", "description": "Optional baseline (現行) Lawtext or e-Gov XML to enable ripple-effect checks across deleted/renumbered articles." }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "fetch_egov_law",
                description: "Fetch an official Japanese statute XML from e-Gov 法令 API v2 by law_title, law_num, or law_id, convert it to Lawtext, and optionally set it as the baseline (現行法令) for 5-set generation.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "law_title": { "type": "string", "description": "Statute title (e.g. '行政手続法')." },
                        "law_num": { "type": "string", "description": "Optional statute number (e.g. '平成五年法律第八十八号')." },
                        "law_id": { "type": "string", "description": "Optional e-Gov law_id." },
                        "set_as_baseline": { "type": "boolean", "description": "Store as the baseline (現行法令) for subsequent generate_lawtext_5set / lint_lawtext calls (default true)." }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "open_markdown_document",
                description: "Open a generated Markdown document (such as 新旧対照表, 改め文, 参照条文, 法律案要綱, or 理由) in a new ibisOffice DOCX window.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Document window/file title (e.g. '新旧対照表.md')." },
                        "markdown": { "type": "string", "description": "Full Markdown content to open as a DOCX document." }
                    },
                    "required": ["title", "markdown"],
                    "additionalProperties": false
                }),
            },
        },
    ]
}

#[derive(Deserialize)]
pub struct LintLawtextArgs {
    pub new_lawtext: Option<String>,
    pub old_lawtext: Option<String>,
}

#[derive(Deserialize)]
pub struct FetchEgovLawArgs {
    pub law_title: Option<String>,
    pub law_num: Option<String>,
    pub law_id: Option<String>,
    pub set_as_baseline: Option<bool>,
}

#[derive(Deserialize)]
pub struct OpenMarkdownDocumentArgs {
    pub title: String,
    pub markdown: String,
}

#[derive(Deserialize)]
pub struct GenerateLawtext5SetArgs {
    pub old_lawtext: Option<String>,
    pub new_lawtext: Option<String>,
    pub policy_memo: Option<String>,
    pub old_on_left: Option<bool>,
}

#[derive(Deserialize)]
pub struct VerifyLawtextAratameArgs {
    pub old_lawtext: Option<String>,
    pub aratame_text: String,
    pub expected_new_lawtext: Option<String>,
}

#[derive(Deserialize)]
pub struct SlideIndexOptionalArg {
    pub slide_index: Option<usize>,
}

#[derive(Deserialize)]
pub struct SlideIndexRequiredArg {
    pub slide_index: usize,
}

#[derive(Deserialize)]
pub struct SetShapeTextArgs {
    pub slide_index: Option<usize>,
    pub shape_id: u64,
    pub text: String,
}

#[derive(Deserialize)]
pub struct SetSlideNotesArgs {
    pub slide_index: Option<usize>,
    pub notes: String,
}

#[derive(Deserialize)]
pub struct AddSlideArgs {
    pub title: Option<String>,
    pub body: Option<String>,
}

#[derive(Deserialize)]
pub struct ReadParagraphsArgs {
    pub start: usize,
    #[serde(default = "default_twenty")]
    pub count: usize,
}

fn default_twenty() -> usize {
    20
}

#[derive(Deserialize)]
pub struct InsertTextArgs {
    pub paragraph: u32,
    pub offset: Option<usize>,
    pub text: String,
}

#[derive(Deserialize)]
pub struct ReplaceParagraphArgs {
    pub paragraph: u32,
    pub text: String,
}

#[derive(Deserialize)]
pub struct SetParagraphAlignArgs {
    pub paragraph: u32,
    pub align: String,
}

#[derive(Deserialize)]
pub struct MoveSlideArgs {
    pub slide_index: usize,
    pub to_index: usize,
}

#[derive(Deserialize)]
pub struct InsertTextBoxArgs {
    pub slide_index: Option<usize>,
    pub text: String,
    #[serde(default = "default_ten")]
    pub x_pct: f32,
    #[serde(default = "default_twenty_f32")]
    pub y_pct: f32,
    #[serde(default = "default_eighty")]
    pub w_pct: f32,
    #[serde(default = "default_twenty_f32")]
    pub h_pct: f32,
    pub font_size: Option<f32>,
    pub bold: Option<bool>,
}

fn default_ten() -> f32 {
    10.0
}
fn default_twenty_f32() -> f32 {
    20.0
}
fn default_eighty() -> f32 {
    80.0
}

#[derive(Deserialize)]
pub struct SetShapeStyleArgs {
    pub slide_index: Option<usize>,
    pub shape_id: u64,
    pub font_size: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub text_color: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_nullable")]
    pub fill_color: Option<Option<String>>,
}

#[derive(Deserialize)]
pub struct SetRunFormatArgs {
    pub paragraph: u32,
    pub start: Option<usize>,
    pub end: Option<usize>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub font_size: Option<f32>,
    pub color: Option<String>,
}

#[derive(Deserialize)]
pub struct SetParagraphStyleArgs {
    pub paragraph: u32,
    pub style: Option<String>,
}

#[derive(Deserialize)]
pub struct InsertTableArgs {
    pub paragraph: u32,
    pub rows: usize,
    pub columns: usize,
}

#[derive(Deserialize)]
pub struct FindTextArgs {
    pub query: String,
}

pub fn build_set_run_format_command(
    args: SetRunFormatArgs,
    para_len: usize,
) -> Result<Command, String> {
    let start = args.start.unwrap_or(0).min(para_len);
    let end = args.end.unwrap_or(para_len).min(para_len);
    if start >= end {
        return Err("empty character span (start >= end)".to_string());
    }
    let from = FlowPos::new(args.paragraph, start);
    let to = FlowPos::new(args.paragraph, end);
    let mut cmds = Vec::new();
    if let Some(b) = args.bold {
        cmds.push(Command::Flow(FlowCommand::SetRunFormat {
            from,
            to,
            change: RunChange::Bold(b),
        }));
    }
    if let Some(i) = args.italic {
        cmds.push(Command::Flow(FlowCommand::SetRunFormat {
            from,
            to,
            change: RunChange::Italic(i),
        }));
    }
    if let Some(u) = args.underline {
        cmds.push(Command::Flow(FlowCommand::SetRunFormat {
            from,
            to,
            change: RunChange::Underline(u),
        }));
    }
    if let Some(pt) = args.font_size {
        if !pt.is_finite() || pt <= 0.0 || pt > 409.0 {
            return Err(format!("invalid font_size '{pt}'"));
        }
        cmds.push(Command::Flow(FlowCommand::SetRunFormat {
            from,
            to,
            change: RunChange::Size((pt * 100.0).round() as u32),
        }));
    }
    if let Some(hex) = args.color {
        let rgb = parse_hex_rgb(&hex)?;
        cmds.push(Command::Flow(FlowCommand::SetRunFormat {
            from,
            to,
            change: RunChange::Color(rgb),
        }));
    }
    if cmds.is_empty() {
        return Err("at least one formatting property must be specified".to_string());
    }
    if cmds.len() == 1 {
        Ok(cmds.pop().expect("single"))
    } else {
        Ok(Command::Group(cmds))
    }
}

#[derive(Deserialize)]
pub struct FindCellsArgs {
    pub query: String,
    #[serde(default = "default_true")]
    pub search_formulas: bool,
}

#[derive(Deserialize)]
struct SetAxisSizeArgs {
    axis: String,
    from: u32,
    to: Option<u32>,
    size_pt: f32,
}

#[derive(Deserialize)]
struct SetCellCommentArgs {
    cell: String,
    text: Option<String>,
}

pub fn parse_set_axis_size_command(slide: SlideId, arguments_json: &str) -> Result<Command, String> {
    let args: SetAxisSizeArgs =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    let axis = match args.axis.trim().to_ascii_lowercase().as_str() {
        "row" | "rows" => Axis::Rows,
        "col" | "cols" | "column" | "columns" => Axis::Columns,
        other => return Err(format!("invalid axis '{other}'")),
    };
    if args.from == 0 {
        return Err("from must be >= 1".to_string());
    }
    let to = args.to.unwrap_or(args.from);
    if to == 0 {
        return Err("to must be >= 1".to_string());
    }
    build_set_axis_size_command(slide, axis, args.from - 1, to - 1, args.size_pt)
}

pub fn parse_set_cell_comment_command(
    slide: SlideId,
    arguments_json: &str,
) -> Result<Command, String> {
    let args: SetCellCommentArgs =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    build_set_comment_command(slide, &args.cell, args.text)
}

#[derive(Deserialize)]
pub struct ApplyVcTemplateArgs {
    pub template: String,
}

pub fn parse_vc_template_kind(raw: &str) -> Result<SdJwtTemplateKind, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pid" => Ok(SdJwtTemplateKind::Pid),
        "mdl" | "iso18013" | "iso18013_mdl" => Ok(SdJwtTemplateKind::Iso18013Mdl),
        "employee" | "employee_id" => Ok(SdJwtTemplateKind::EmployeeId),
        "enrollment" | "enrollment_cert" => Ok(SdJwtTemplateKind::EnrollmentCert),
        "completion" | "completion_cert" => Ok(SdJwtTemplateKind::CompletionCert),
        other => Err(format!(
            "unknown VC template '{other}' (expected 'pid', 'mdl', 'employee', 'enrollment', or 'completion')"
        )),
    }
}

#[derive(Deserialize)]
pub struct InferVcSchemaArgs {
    pub vct: Option<String>,
    #[serde(default)]
    pub apply: bool,
}

#[derive(Deserialize)]
pub struct BindVcClaimArgs {
    pub range: String,
    pub claim_path: String,
    #[serde(default = "default_true")]
    pub selective: bool,
}

#[derive(Deserialize)]
pub struct IssueVcPreviewArgs {
    pub format: Option<String>,
    pub disclose_paths: Option<Vec<String>>,
    pub vct: Option<String>,
    pub iss: Option<String>,
    pub name_space: Option<String>,
}

#[derive(Deserialize)]
pub struct ParseVcTokenArgs {
    pub token: String,
    pub issuer_jwk_json: Option<String>,
    #[serde(default)]
    pub import_to_sheet: bool,
}

#[derive(Deserialize)]
pub struct PrepareOid4vpArgs {
    pub request: String,
    pub format: Option<String>,
    pub name_space: Option<String>,
    #[serde(default)]
    pub apply_to_sheet: bool,
}

#[derive(Deserialize)]
pub struct GetVcPublicKeysArgs {
    pub which: Option<String>,
}

fn resolve_vc_key(setting_key: &str, demo: fn() -> Es256KeyPair) -> (Es256KeyPair, &'static str) {
    if let Some(raw) = crate::settings::get(setting_key) {
        let trimmed = raw.trim();
        if !trimmed.is_empty()
            && let Some(kp) = Es256KeyPair::from_jwk_json(trimmed)
        {
            return (kp, "settings");
        }
    }
    (demo(), "demo")
}

pub fn resolve_vc_issuer_key() -> (Es256KeyPair, &'static str) {
    resolve_vc_key("vc.issuer_jwk", Es256KeyPair::demo_issuer_key)
}

pub fn resolve_vc_holder_key() -> (Es256KeyPair, &'static str) {
    resolve_vc_key("vc.holder_jwk", Es256KeyPair::demo_holder_key)
}

pub(crate) fn claim_value_to_json(val: &ClaimValue) -> Value {
    match val {
        ClaimValue::Null => Value::Null,
        ClaimValue::Bool(b) => Value::Bool(*b),
        ClaimValue::Int(i) => json!(*i),
        ClaimValue::Float(f) => json!(*f),
        ClaimValue::String(s) => Value::String(s.clone()),
        ClaimValue::Array(items) => Value::Array(items.iter().map(claim_value_to_json).collect()),
        ClaimValue::Object(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                obj.insert(k.clone(), claim_value_to_json(v));
            }
            Value::Object(obj)
        }
    }
}

pub fn vc_public_keys_json(which: &str) -> Result<Value, String> {
    let mode = which.trim().to_ascii_lowercase();
    let include_issuer = matches!(mode.as_str(), "" | "all" | "issuer");
    let include_holder = matches!(mode.as_str(), "" | "all" | "holder");
    if !include_issuer && !include_holder {
        return Err(format!(
            "unknown key selector '{which}' (expected 'all', 'issuer', or 'holder')"
        ));
    }

    let mut out = serde_json::Map::new();
    out.insert("ok".to_string(), json!(true));
    if include_issuer {
        let (kp, src) = resolve_vc_issuer_key();
        out.insert("issuer_source".to_string(), json!(src));
        out.insert("issuer_public_jwk".to_string(), claim_value_to_json(&kp.to_public_jwk()));
    }
    if include_holder {
        let (kp, src) = resolve_vc_holder_key();
        out.insert("holder_source".to_string(), json!(src));
        out.insert("holder_public_jwk".to_string(), claim_value_to_json(&kp.to_public_jwk()));
    }
    Ok(Value::Object(out))
}

pub fn parse_set_paragraph_align_command(arguments_json: &str) -> Result<Command, String> {
    let args: SetParagraphAlignArgs =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    let align = match args.align.as_str() {
        "left" => FlowAlign::Left,
        "center" => FlowAlign::Center,
        "right" => FlowAlign::Right,
        "justify" => FlowAlign::Justify,
        other => return Err(format!("invalid align '{other}'")),
    };
    Ok(Command::Flow(FlowCommand::SetParagraphFormat {
        first: args.paragraph,
        last: args.paragraph,
        change: ParaChange::Align(align),
    }))
}

pub fn build_replace_paragraph_command(paragraph: u32, old_len: usize, text: String) -> Command {
    if old_len == 0 {
        Command::Flow(FlowCommand::InsertText {
            at: FlowPos::new(paragraph, 0),
            text,
            format: None,
        })
    } else if text.is_empty() {
        Command::Flow(FlowCommand::Delete {
            from: FlowPos::new(paragraph, 0),
            to: FlowPos::new(paragraph, old_len),
        })
    } else {
        Command::Group(vec![
            Command::Flow(FlowCommand::Delete {
                from: FlowPos::new(paragraph, 0),
                to: FlowPos::new(paragraph, old_len),
            }),
            Command::Flow(FlowCommand::InsertText {
                at: FlowPos::new(paragraph, 0),
                text,
                format: None,
            }),
        ])
    }
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

    #[test]
    fn parses_set_axis_size_json() {
        let cmd = parse_set_axis_size_command(
            SlideId(1),
            r#"{"axis":"column","from":1,"to":3,"size_pt":120.0}"#,
        )
        .unwrap();
        assert!(matches!(cmd, Command::Sheet(SheetCommand::SetSize { .. })));
        assert!(parse_set_axis_size_command(
            SlideId(1),
            r#"{"axis":"row","from":0,"to":1,"size_pt":24.0}"#
        )
        .is_err());
    }

    #[test]
    fn parses_set_cell_comment_json() {
        let set_cmd = parse_set_cell_comment_command(
            SlideId(1),
            r#"{"cell":"B2","text":"Check formula"}"#,
        )
        .unwrap();
        assert!(matches!(
            set_cmd,
            Command::Sheet(SheetCommand::SetComment { .. })
        ));

        let clear_cmd = parse_set_cell_comment_command(SlideId(1), r#"{"cell":"B2"}"#).unwrap();
        assert!(matches!(
            clear_cmd,
            Command::Sheet(SheetCommand::SetComment { comment: None, .. })
        ));
    }

    #[test]
    fn builds_set_run_format_commands() {
        let args = SetRunFormatArgs {
            paragraph: 0,
            start: Some(0),
            end: Some(5),
            bold: Some(true),
            italic: Some(false),
            underline: None,
            font_size: Some(14.0),
            color: Some("#1A73E8".to_string()),
        };
        let cmd = build_set_run_format_command(args, 10).unwrap();
        assert!(matches!(cmd, Command::Group(v) if v.len() == 4));
    }

    #[test]
    fn parses_set_paragraph_align_json() {
        let cmd = parse_set_paragraph_align_command(r#"{"paragraph":2,"align":"center"}"#).unwrap();
        assert!(matches!(
            cmd,
            Command::Flow(FlowCommand::SetParagraphFormat { .. })
        ));
        assert!(parse_set_paragraph_align_command(r#"{"paragraph":2,"align":"diagonal"}"#).is_err());
    }

    #[test]
    fn parses_pptx_and_xlsx_tool_arg_structs() {
        let box_args: InsertTextBoxArgs = serde_json::from_str(
            r#"{"text":"Hello","x_pct":10.0,"y_pct":20.0,"w_pct":80.0,"h_pct":15.0,"font_size":20.0,"bold":true}"#,
        )
        .unwrap();
        assert_eq!(box_args.text, "Hello");
        assert_eq!(box_args.bold, Some(true));

        let style_args: SetShapeStyleArgs = serde_json::from_str(
            r##"{"shape_id":42,"fill_color":"#FFFFFF","text_color":"#000000","font_size":16.0}"##,
        )
        .unwrap();
        assert_eq!(style_args.shape_id, 42);
        assert_eq!(style_args.fill_color.as_ref().map(Option::as_deref), Some(Some("#FFFFFF")));

        let find_args: FindCellsArgs =
            serde_json::from_str(r#"{"query":"SUM","search_formulas":true}"#).unwrap();
        assert_eq!(find_args.query, "SUM");
        assert!(find_args.search_formulas);
    }

    #[test]
    fn parses_vc_tool_args_and_templates() {
        assert_eq!(parse_vc_template_kind("pid").unwrap(), SdJwtTemplateKind::Pid);
        assert_eq!(
            parse_vc_template_kind("mdl").unwrap(),
            SdJwtTemplateKind::Iso18013Mdl
        );
        assert_eq!(
            parse_vc_template_kind("employee").unwrap(),
            SdJwtTemplateKind::EmployeeId
        );
        assert_eq!(
            parse_vc_template_kind("enrollment_cert").unwrap(),
            SdJwtTemplateKind::EnrollmentCert
        );
        assert_eq!(
            parse_vc_template_kind("completion").unwrap(),
            SdJwtTemplateKind::CompletionCert
        );
        assert!(parse_vc_template_kind("unknown").is_err());

        let bind_default: BindVcClaimArgs =
            serde_json::from_str(r#"{"range":"B2","claim_path":"family_name"}"#).unwrap();
        assert!(bind_default.selective);
        assert_eq!(bind_default.claim_path, "family_name");

        let bind_mandatory: BindVcClaimArgs = serde_json::from_str(
            r#"{"range":"B1","claim_path":"vct","selective":false}"#,
        )
        .unwrap();
        assert!(!bind_mandatory.selective);

        let infer_args: InferVcSchemaArgs =
            serde_json::from_str(r#"{"vct":"https://example.com/cred"}"#).unwrap();
        assert!(!infer_args.apply);
        assert_eq!(infer_args.vct.as_deref(), Some("https://example.com/cred"));

        let issue_args: IssueVcPreviewArgs = serde_json::from_str(
            r#"{"format":"mdoc","disclose_paths":["family_name","birthdate"],"iss":"https://issuer.example","name_space":"org.iso.18013.5.1"}"#,
        )
        .unwrap();
        assert_eq!(issue_args.format.as_deref(), Some("mdoc"));
        assert_eq!(
            issue_args.disclose_paths.as_deref(),
            Some(&["family_name".to_string(), "birthdate".to_string()][..])
        );

        let parse_args: ParseVcTokenArgs =
            serde_json::from_str(r#"{"token":"eyJ...~disc~","import_to_sheet":true}"#).unwrap();
        assert!(parse_args.import_to_sheet);

        let vp_args: PrepareOid4vpArgs = serde_json::from_str(
            r#"{"request":"openid4vp://?client_id=https://verifier.example&nonce=abc","format":"mdoc","name_space":"org.iso.18013.5.1","apply_to_sheet":true}"#,
        )
        .unwrap();
        assert!(vp_args.apply_to_sheet);
        assert_eq!(vp_args.format.as_deref(), Some("mdoc"));
        assert_eq!(vp_args.name_space.as_deref(), Some("org.iso.18013.5.1"));
        assert!(vp_args.request.starts_with("openid4vp://"));

        let keys_args: GetVcPublicKeysArgs = serde_json::from_str(r#"{"which":"issuer"}"#).unwrap();
        assert_eq!(keys_args.which.as_deref(), Some("issuer"));

        let val = vc_public_keys_json("all").unwrap();
        assert_eq!(val["ok"], true);
        let issuer = val["issuer_public_jwk"].as_object().unwrap();
        let holder = val["holder_public_jwk"].as_object().unwrap();
        assert!(issuer.get("d").is_none(), "issuer public JWK must not contain private 'd'");
        assert!(holder.get("d").is_none(), "holder public JWK must not contain private 'd'");
        assert_eq!(issuer.get("kty").and_then(|v| v.as_str()), Some("EC"));
        assert_eq!(holder.get("crv").and_then(|v| v.as_str()), Some("P-256"));
        assert!(vc_public_keys_json("invalid").is_err());
    }

    #[test]
    fn exposes_and_parses_lawtext_docx_tools() {
        let defs = docx_tool_definitions();
        for name in [
            "export_active_lawtext",
            "generate_lawtext_5set",
            "verify_lawtext_aratame",
            "lint_lawtext",
            "fetch_egov_law",
            "open_markdown_document",
        ] {
            assert!(
                defs.iter().any(|d| d.function.name == name),
                "missing docx tool definition: {name}"
            );
        }

        let gen_args: GenerateLawtext5SetArgs = serde_json::from_str(
            r#"{"old_lawtext":"テスト法（令和七年法律第一号）\n第一条 旧。","policy_memo":"改正背景","old_on_left":true}"#,
        )
        .unwrap();
        assert_eq!(gen_args.old_on_left, Some(true));
        assert_eq!(gen_args.policy_memo.as_deref(), Some("改正背景"));

        let verify_args: VerifyLawtextAratameArgs = serde_json::from_str(
            r#"{"old_lawtext":"old","aratame_text":"第一条中「旧」を「新」に改める。"}"#,
        )
        .unwrap();
        assert!(verify_args.expected_new_lawtext.is_none());

        let lint_args: LintLawtextArgs =
            serde_json::from_str(r#"{"old_lawtext":"old","new_lawtext":"new"}"#).unwrap();
        assert_eq!(lint_args.old_lawtext.as_deref(), Some("old"));

        let open_args: OpenMarkdownDocumentArgs =
            serde_json::from_str(r#"{"title":"新旧対照表.md","markdown":"| 改正後 | 現行 |"}"#)
                .unwrap();
        assert_eq!(open_args.title, "新旧対照表.md");

        let old_lt = "テスト法（令和七年法律第一号）\n第一条 この法律は、処分に関する手続を定める。";
        let new_lt = "テスト法（令和七年法律第一号）\n第一条 この法律は、処分及び行政指導に関する手続を定める。";
        let old_doc = crate::ai_assistant::parse_law_input_str(old_lt).unwrap();
        let new_doc = crate::ai_assistant::parse_law_input_str(new_lt).unwrap();
        let pkg = ibis_formats::lawtext::build_5set_package(
            &old_doc,
            &new_doc,
            "行政指導を追加",
            &ibis_formats::lawtext::ShinkyuOptions::default(),
        );
        assert!(pkg.aratame_verification.is_exact_match);
        let md = ibis_formats::lawtext::to_markdown(&new_doc);
        assert_eq!(crate::ai_assistant::parse_law_input_str(&md).unwrap().law_title, "テスト法");
        let xml = ibis_formats::lawtext::to_law_xml(&new_doc);
        assert_eq!(crate::ai_assistant::parse_law_input_str(&xml).unwrap().law_num, "令和七年法律第一号");
    }
}
