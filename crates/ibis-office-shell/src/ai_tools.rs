// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! OpenAI Tool definitions and JSON argument parsers for spreadsheet operations.

use ibis_doc_model::ai_sheet::{
    build_delete_axis_command, build_insert_axis_command, build_merge_command,
    build_set_axis_size_command, build_set_comment_command,
    parse_cell_range,
};
use ibis_doc_model::command::Command;
use ibis_doc_model::model::SlideId;
use ibis_doc_model::sd_jwt::Es256KeyPair;
use ibis_doc_model::sd_jwt_binding::{ClaimValue, SdJwtTemplateKind};
use ibis_doc_model::sheet::CellRange;
use ibis_doc_model::sheet_command::Axis;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ai_client::{ToolDefinition, ToolFunctionDef};

pub const MAX_READ_CELLS: usize = 200;

/// Tool definitions exposed to the OpenAI-compatible API for spreadsheets.
#[cfg(not(target_family = "wasm"))]
pub fn spreadsheet_tool_definitions() -> Vec<ToolDefinition> {
    let mut tools = vec![
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "create_chart",
                description: "Create an editable chart linked to worksheet cells. Use range for a table with headers, or explicit series for nonadjacent cells. Pick an empty area or a separate sheet for placement.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sheet": { "type": "string", "description": "Data sheet (defaults to the active sheet)." },
                        "destination_sheet": { "type": "string", "description": "Chart sheet (defaults to the active sheet)." },
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
                    "required": ["kind", "anchor", "title"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "get_workbook_context",
                description: "Get the active worksheet and current selection. Use xlsx_list_sheets for workbook contents.",
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
    ];
    tools.extend(ibis_doc_service::document_tools(ibis_formats::Kind::Workbook)
        .map(|tool| editor_tool_definition(tool, tool.name != "xlsx_find_text")));
    tools
}

#[cfg(not(target_family = "wasm"))]
fn editor_tool_definition(descriptor: &ibis_doc_service::Tool, active_sheet: bool) -> ToolDefinition {
    let mut parameters = (descriptor.schema)();
    if let Some(properties) = parameters["properties"].as_object_mut() {
        properties.remove("document_id");
        if active_sheet && let Some(sheet) = properties.get_mut("sheet") {
            sheet["description"] = json!("The sheet's name. Omit to use the active sheet.");
        }
    }
    if let Some(required) = parameters["required"].as_array_mut() {
        required.retain(|name| name != "document_id" && (!active_sheet || name != "sheet"));
    }
    ToolDefinition {
        kind: "function",
        function: ToolFunctionDef {
            name: descriptor.name,
            description: descriptor.description,
            parameters,
        },
    }
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

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChartArgs {
    pub(crate) sheet: Option<String>,
    pub(crate) destination_sheet: Option<String>,
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
    pub(crate) fn frame(&self, sheet: &ibis_doc_model::Sheet, origin: ibis_doc_model::EmuRect) -> Result<ibis_doc_model::EmuRect, String> {
        let (row, column) = ibis_doc_model::ai_sheet::parse_cell_ref(&self.anchor)
            .filter(|&(row, column)| row < 1_048_576 && column < 16_384)
            .ok_or("anchor must be a single A1 cell")?;
        let cell = sheet.cell_bounds_emu(column, row);
        Ok(ibis_doc_model::EmuRect::new(origin.x + cell.x, origin.y + cell.y, 5_486_400, 3_429_000))
    }

    pub(crate) fn kind(&self) -> ibis_formats::NewChartKind {
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

    pub(crate) fn refs(&self, workbook: &ibis_doc_model::Presentation, name: &str, sheet: &ibis_doc_model::Sheet) -> Result<Vec<ibis_formats::SeriesRefs>, String> {
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

    pub(crate) fn label(&self, chart: &mut ibis_doc_model::Chart) {
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

pub fn parse_range_arg(arguments_json: &str) -> Result<CellRange, String> {
    let arg: RangeArg =
        serde_json::from_str(arguments_json).map_err(|e| format!("invalid arguments: {e}"))?;
    parse_cell_range(&arg.range).ok_or_else(|| format!("invalid range '{}'", arg.range))
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

/// Tool definitions exposed to the OpenAI-compatible API for presentations (PPTX).
/// Source representations the document or presentation editor can edit.
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditableSourceKind {
    Markdown,
    Mermaid,
}

pub struct EditableSource {
    pub target: String,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadEditableSource {
    pub kind: EditableSourceKind,
    pub start_line: Option<usize>,
    pub line_count: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplaceSourceText {
    pub kind: EditableSourceKind,
    pub target: String,
    pub expected: String,
    pub replacement: String,
}

fn source_tool_definitions(markdown: bool) -> Vec<ToolDefinition> {
    let kinds = if markdown { vec!["markdown", "mermaid"] } else { vec!["mermaid"] };
    vec![
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "read_editable_source",
                description: "Read an active Markdown document or the source of the selected Mermaid image. Returns a target token for a subsequent edit.",
                parameters: json!({
                    "type": "object", "required": ["kind"], "additionalProperties": false,
                    "properties": {
                        "kind": { "type": "string", "enum": kinds },
                        "start_line": { "type": "integer", "minimum": 1, "description": "First line, defaults to 1." },
                        "line_count": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Maximum lines, defaults to 100." }
                    }
                }),
            },
        },
        ToolDefinition {
            kind: "function",
            function: ToolFunctionDef {
                name: "replace_source_text",
                description: "Replace one unique, exact fragment of editable source. First read it with read_editable_source and pass its target token. This applies one undoable edit immediately.",
                parameters: json!({
                    "type": "object", "required": ["kind", "target", "expected", "replacement"], "additionalProperties": false,
                    "properties": {
                        "kind": { "type": "string", "enum": kinds },
                        "target": { "type": "string", "description": "Target token from read_editable_source." },
                        "expected": { "type": "string", "description": "Nonempty literal source fragment to replace, including original whitespace." },
                        "replacement": { "type": "string", "description": "Replacement source text. Empty removes the fragment." }
                    }
                }),
            },
        },
    ]
}

pub(crate) fn replace_unique_source(text: &str, expected: &str, replacement: &str) -> Result<String, &'static str> {
    if expected.is_empty() || expected.len() > 32_768 || replacement.len() > 65_536 {
        return Err("expected must be nonempty; the source fragment or replacement exceeds its size limit");
    }
    let start = text.find(expected).ok_or("expected source was not found; read the current source again")?;
    let next = start + expected.chars().next().unwrap().len_utf8();
    if text[next..].contains(expected) {
        return Err("expected source occurs more than once; include more surrounding text");
    }
    let mut result = text.to_owned();
    result.replace_range(start..start + expected.len(), replacement);
    Ok(result)
}

pub fn pptx_tool_definitions() -> Vec<ToolDefinition> {
    let mut tools = vec![
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
                name: "set_shape_fill",
                description: "Set the fill color of a shape, or clear the fill with null.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "slide_index": { "type": "integer", "minimum": 1 },
                        "shape_id": { "type": "integer" },
                        "fill_color": { "type": ["string", "null"] }
                    },
                    "required": ["shape_id", "fill_color"],
                    "additionalProperties": false
                }),
            },
        },
    ];
    #[cfg(not(target_family = "wasm"))]
    tools.extend(ibis_doc_service::document_tools(ibis_formats::Kind::Presentation)
        .map(|tool| editor_tool_definition(tool, false)));
    tools.extend(source_tool_definitions(false));
    tools
}

/// Tool definitions exposed to the OpenAI-compatible API for word-processing documents (DOCX).
pub fn docx_tool_definitions() -> Vec<ToolDefinition> {
    let mut tools = vec![
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
                description: "Generate five Japanese legislative Markdown drafts (要綱, 法律案, 理由, 新旧対照表, 参照条文) with AmendmentIr, reverse-application verification, lint findings, and optional AI drafting prompts from old and new Lawtext or e-Gov XML. If new_lawtext is omitted, the active document is used as the new version.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "old_lawtext": { "type": "string", "description": "Current (現行) statute in Lawtext or e-Gov XML format. Optional if already loaded via the file picker." },
                        "new_lawtext": { "type": "string", "description": "Optional amended (改正後) statute in Lawtext or e-Gov XML. Defaults to the active document." },
                        "policy_memo": { "type": "string", "description": "Optional policy background memo for drafting 要綱 and 理由." },
                        "reference_laws": { "type": "array", "items": { "type": "string" }, "description": "Optional external law snapshots in Lawtext or e-Gov XML for 参照条文." },
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
    ];
    #[cfg(not(target_family = "wasm"))]
    tools.extend(ibis_doc_service::document_tools(ibis_formats::Kind::Document)
        .map(|tool| editor_tool_definition(tool, false)));
    tools.extend(source_tool_definitions(true));
    tools
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
    pub reference_laws: Option<Vec<String>>,
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

fn default_twenty() -> usize {
    20
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
pub struct SetShapeFillArgs {
    pub slide_index: Option<usize>,
    pub shape_id: u64,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub fill_color: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use ibis_doc_model::sheet_command::SheetCommand;

    #[test]
    fn source_patch_is_literal_and_requires_a_unique_match() {
        assert_eq!(replace_unique_source("前置き
A --> B
", "A --> B", "A --> C").unwrap(), "前置き
A --> C
");
        assert!(replace_unique_source("aaaa", "aaa", "x").is_err());
        assert!(replace_unique_source("本文", "", "x").is_err());
        assert!(replace_unique_source("本文", "なし", "x").is_err());
    }

    #[cfg(not(target_family = "wasm"))]
    fn set_chart_cells(document: &mut ibis_doc_model::Document, sheet: SlideId, range: &str, values: Value) {
        let context = ibis_doc_service::DraftToolContext {
            workbook: ibis_doc_service::WorkbookToolContext { active_sheet: Some(sheet), ..Default::default() },
            ..Default::default()
        };
        ibis_doc_service::run_draft_tool(
            ibis_formats::Kind::Workbook, document, context, "xlsx_set_cells",
            json!({ "range": range, "values": values }),
        ).unwrap();
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn ai_create_chart_infers_categories_and_values_from_a_table() {
        use ibis_doc_model::{ChartType, Document, ShapeKind, sheet_calc::sheet_of_slide};
        use ibis_formats::{Kind, SaveOptions};

        let opened = ibis_formats::open(ibis_formats::blank_file(Kind::Workbook).unwrap()).unwrap();
        let mut doc = Document::new(opened.presentation.clone());
        let slide = doc.presentation().slides[0].id;
        set_chart_cells(&mut doc, slide, "A1:B3", json!([["年度", "費用"], ["R8", 10], ["R9", 12]]));
        let args: ChartArgs = serde_json::from_value(json!({
            "kind":"line", "anchor":"D2", "range":"A1:B3", "title":"費用の推移"
        })).unwrap();
        let model = &doc.presentation().slides[0];
        let sheet = sheet_of_slide(model).unwrap();
        let name = model.name.as_deref().unwrap();
        let refs = args.refs(doc.presentation(), name, sheet).unwrap();
        assert_eq!(refs.len(), 1);
        assert!(refs[0].categories.as_deref().unwrap().ends_with("!$A$2:$A$3"));
        assert!(refs[0].values.as_deref().unwrap().ends_with("!$B$2:$B$3"));

        let frame = args.frame(sheet, model.shapes[0].frame).unwrap();
        let id = doc.ids().new_shape_id();
        let mut shape = opened.source.new_sheet_chart(doc.presentation(), slide, id, frame, args.kind(), refs).unwrap();
        let ShapeKind::Chart(chart) = &mut shape.kind else { panic!("expected chart") };
        args.label(chart);
        doc.execute(vec![Command::InsertShape { slide, shapes: vec![(1, shape)] }]).unwrap();
        let saved = opened.source.save(doc.presentation(), SaveOptions::default()).unwrap();
        let reopened = ibis_formats::read(&saved).unwrap();
        let ShapeKind::Chart(chart) = &reopened.slides[0].shapes[1].kind else { panic!("expected chart") };
        assert!(matches!(chart.groups[0].kind, ChartType::Line));
        assert_eq!(chart.groups[0].series[0].categories, ["R8", "R9"]);
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn ai_scatter_chart_round_trips_sparse_series() {
        use ibis_doc_model::{Document, ShapeKind, sheet_calc::sheet_of_slide};
        use ibis_formats::{Kind, NewChartKind, SaveOptions};
        let opened = ibis_formats::open(ibis_formats::blank_file(Kind::Workbook).unwrap()).unwrap();
        let mut doc = Document::new(opened.presentation.clone());
        let slide = doc.presentation().slides[0].id;
        set_chart_cells(&mut doc, slide, "A1:C4", json!([[null, "同区分", "選択団体"], [195.6, null, 105.81], [23.7, 254.38, null], [5.1, "=NA()", null]]));
        let args_json = json!({
            "kind":"scatter", "anchor":"E2", "domain_range":"A2:A4",
            "series":[{"values_range":"B2:B4","name_cell":"B1"},{"values_range":"C2:C4","name_cell":"C1"}],
            "title":"人口と運用経費の増減率", "x_title":"人口（万人）", "y_title":"増減率（%）"
        });
        let args: ChartArgs = serde_json::from_value(args_json.clone()).unwrap();
        let model = &doc.presentation().slides[0];
        let sheet = sheet_of_slide(model).unwrap();
        let refs = args.refs(doc.presentation(), model.name.as_deref().unwrap(), sheet).unwrap();
        let frame = args.frame(sheet, model.shapes[0].frame).unwrap();
        let mut invalid = args_json;
        invalid["series"][0]["values_range"] = json!("B2:B3");
        let invalid: ChartArgs = serde_json::from_value(invalid).unwrap();
        assert!(invalid.refs(doc.presentation(), model.name.as_deref().unwrap(), sheet).is_err());

        let id = doc.ids().new_shape_id();
        let mut shape = opened.source.new_sheet_chart(doc.presentation(), slide, id, frame, NewChartKind::Scatter, refs.clone()).unwrap();
        let ShapeKind::Chart(chart) = &mut shape.kind else { panic!("expected chart") };
        args.label(chart);
        doc.execute(vec![Command::InsertShape { slide, shapes: vec![(1, shape)] }]).unwrap();
        let saved = opened.source.save(doc.presentation(), SaveOptions::default()).unwrap();
        let reopened = ibis_formats::read(&saved).unwrap();
        let ShapeKind::Chart(chart) = &reopened.slides[0].shapes[1].kind else { panic!("expected chart") };
        assert_eq!(chart.title.as_ref().unwrap().text, "人口と運用経費の増減率");
        assert!(chart.axes.iter().any(|axis| axis.title.as_ref().is_some_and(|title| title.text == "増減率（%）")));
        assert_eq!(chart.groups[0].series[0].name, "同区分");
        assert_eq!(chart.groups[0].series[0].values.iter().map(|n| n.map(|n| n.0)).collect::<Vec<_>>(), [None, Some(254.38), None]);
        assert_eq!(chart.cell_refs[1].values, refs[1].values);
        assert!(doc.undo());
        assert_eq!(doc.presentation().slides[0].shapes.len(), 1);
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn assistant_document_tool_schemas_come_from_the_shared_catalog() {
        use ibis_formats::Kind;
        for (kind, tools) in [
            (Kind::Workbook, spreadsheet_tool_definitions()),
            (Kind::Document, docx_tool_definitions()),
            (Kind::Presentation, pptx_tool_definitions()),
        ] {
            for shared in ibis_doc_service::document_tools(kind) {
                let entries: Vec<_> = tools.iter().filter(|tool| tool.function.name == shared.name).collect();
                assert_eq!(entries.len(), 1, "{}", shared.name);
                let mut canonical = (shared.schema)();
                let schema = &entries[0].function.parameters;
                assert!(schema["properties"].get("document_id").is_none());
                canonical["properties"].as_object_mut().unwrap().remove("document_id");
                if kind == Kind::Workbook && shared.name != "xlsx_find_text"
                    && let Some(sheet) = canonical["properties"].as_object_mut().unwrap().get_mut("sheet")
                {
                    *sheet = schema["properties"]["sheet"].clone();
                    assert!(!schema["required"].as_array().unwrap().contains(&json!("sheet")));
                }
                assert_eq!(schema["properties"], canonical["properties"], "{}", shared.name);
            }
            assert!(tools.iter().all(|tool| !tool.function.name.starts_with("app_")));
        }
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
    fn parses_pptx_and_xlsx_tool_arg_structs() {
        let box_args: InsertTextBoxArgs = serde_json::from_str(
            r#"{"text":"Hello","x_pct":10.0,"y_pct":20.0,"w_pct":80.0,"h_pct":15.0,"font_size":20.0,"bold":true}"#,
        )
        .unwrap();
        assert_eq!(box_args.text, "Hello");
        assert_eq!(box_args.bold, Some(true));

        let fill: SetShapeFillArgs = serde_json::from_str(r##"{"shape_id":42,"fill_color":"#FFFFFF"}"##).unwrap();
        assert_eq!(fill.shape_id, 42);
        assert_eq!(fill.fill_color.as_deref(), Some("#FFFFFF"));
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
