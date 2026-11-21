// SPDX-License-Identifier: AGPL-3.0-only
//! Review and apply explicit transfers from structured data into named cells.

use std::collections::BTreeSet;

use ibis_calc::formula::parser::{Expr, parse};
use ibis_doc_model::sheet_command::{CellEdit, CellInput, SheetCommand};
use ibis_doc_model::{
    CellValue, Command, Presentation, SlideId, sheet::cell_name, sheet_calc::sheet_of_slide,
};
use ibis_formats::structured_preview;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::a1::parse_range;
use crate::catalog::{Access, Tool, document_id, object, string};
use crate::edit::{edited, execute};
use crate::service::Args;
use crate::tools_xlsx::{find_sheet, sheets_of, value_json};
use crate::{Service, ToolError};

pub(crate) fn tool() -> Tool {
    Tool {
        name: "xlsx_preview_prefill",
        description: "Preview explicit field-to-named-cell mappings from structured data. Returns existing and proposed values without editing the workbook.",
        access: Access::Read,
        returns_content: true,
        schema: || schema(false),
        run: preview,
    }
}

pub(crate) fn apply_tool() -> Tool {
    Tool {
        name: "xlsx_apply_prefill",
        description: "Apply explicitly selected ready fields from an unchanged prefill preview as one undo step in the open workbook. Does not save.",
        access: Access::Edit,
        returns_content: false,
        schema: || schema(true),
        run: apply,
    }
}

fn schema(apply: bool) -> Value {
    let mut schema = object(
        &[
            document_id(),
            ("sheet", string("Name of the destination sheet.")),
            ("format", json!({"enum":["json", "xml", "sd-jwt", "mdoc"]})),
            (
                "source",
                string("UTF-8 structured data. For mdoc, supply hex or base64url text."),
            ),
            (
                "mappings",
                json!({"type":"array", "minItems":1, "maxItems":100, "items":{
                    "type":"object", "additionalProperties":false, "required":["target"],
                    "properties":{
                        "target":{"type":"string"},
                                "label":{"type":"string"},
                        "source_path":{"type":["string","null"]}
                    }
                }}),
            ),
        ],
        &["document_id", "sheet", "format", "source", "mappings"],
    );
    if apply {
        schema["properties"]["preview_id"] = string("The ID returned by xlsx_preview_prefill.");
        schema["properties"]["targets"] = json!({
            "type":"array", "items":{"type":"string"}, "minItems":1, "maxItems":100, "uniqueItems":true
        });
        schema["required"]
            .as_array_mut()
            .unwrap()
            .extend([json!("preview_id"), json!("targets")]);
    }
    schema
}

fn preview(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    plan(service, args).map(|(report, _)| report)
}

fn plan(service: &mut Service, args: &Value) -> Result<(Value, SlideId), ToolError> {
    let source = args.str_arg("source")?;
    let open = service.open_document(args.str_arg("document_id")?)?;
    let sheets = sheets_of(open)?;
    let (index, _, _) = find_sheet(&sheets, args.str_arg("sheet")?)?;
    let presentation = open.document.presentation();
    let slide = presentation.slides[*index].id;
    let mut report = review_prefill(
        presentation,
        slide,
        open.signed,
        args.str_arg("format")?,
        source,
        &args["mappings"],
    )?;
    let mut checksum = Sha256::new();
    checksum.update(b"ibis-prefill-v1");
    checksum.update(open.fingerprint.sha256);
    checksum.update(Sha256::digest(source.as_bytes()));
    checksum.update(report.to_string().as_bytes());
    report["preview_id"] = json!(format!("{:x}", checksum.finalize()));
    Ok((report, slide))
}

/// Inspect explicit mappings against a snapshot of an open sheet without editing it.
pub fn review_prefill(
    presentation: &Presentation,
    slide: SlideId,
    signed: bool,
    format: &str,
    source: &str,
    mappings: &Value,
) -> Result<Value, ToolError> {
    let source = structured_preview::read(format, source.as_bytes()).map_err(ToolError::invalid)?;
    let mappings = mappings
        .as_array()
        .filter(|mappings| (1..=100).contains(&mappings.len()))
        .ok_or_else(|| ToolError::invalid("mappings must contain 1–100 entries"))?;
    let selected = presentation
        .slide(slide)
        .ok_or_else(|| ToolError::invalid("missing destination sheet"))?;
    let sheet =
        sheet_of_slide(selected).ok_or_else(|| ToolError::invalid("missing destination sheet"))?;
    let workbook = presentation
        .workbook
        .as_ref()
        .ok_or_else(|| ToolError::invalid("missing workbook names"))?;
    let local_index = workbook
        .sheets
        .iter()
        .position(|sheet| sheet.slide == Some(slide));
    let mut destinations = BTreeSet::new();
    let mut rows = Vec::with_capacity(mappings.len());
    for mapping in mappings {
        let target = mapping.str_arg("target")?;
        let path = mapping.opt_str("source_path")?;
        let mut names = workbook
            .names
            .iter()
            .filter(|name| name.name.eq_ignore_ascii_case(target));
        let chosen = names
            .clone()
            .find(|name| name.sheet.is_some() && name.sheet == local_index)
            .or_else(|| names.find(|name| name.sheet.is_none()))
            .filter(|name| !name.hidden)
            .ok_or_else(|| {
                ToolError::invalid("target is not a visible defined name in this sheet")
            })?;
        let direct = || ToolError::invalid("target must directly reference one cell on this sheet");
        let Expr::Ref(reference) =
            parse(chosen.formula.trim().trim_start_matches('=')).map_err(|_| direct())?
        else {
            return Err(direct());
        };
        let same_sheet = match &reference.sheet {
            Some(owner) => {
                owner.last.is_none()
                    && owner.book.is_none()
                    && owner.path.is_none()
                    && workbook
                        .sheet_index(&owner.first)
                        .is_some_and(|index| Some(index) == local_index)
            }
            None => chosen.sheet.is_some(),
        };
        let (row, col) = (reference.start.row, reference.start.column);
        if !reference.is_cell() || !same_sheet || row >= 1_048_576 || col >= 16_384 {
            return Err(direct());
        }
        if !destinations.insert((row, col)) {
            return Err(ToolError::invalid("two mappings target the same cell"));
        }
        let current = sheet.cell(row, col);
        let before = current.map_or(Value::Null, |cell| value_json(&cell.value));
        let proposed = match path {
            Some(path) => {
                let mut fields = source.fields.iter().filter(|field| field.path == path);
                let value = fields
                    .next()
                    .map(|field| {
                        if source.format == "XML" {
                            Ok(Value::String(field.value.clone()))
                        } else {
                            serde_json::from_str(&field.value).map_err(|_| {
                                ToolError::invalid("source value cannot be represented")
                            })
                        }
                    })
                    .transpose()?;
                if fields.next().is_some() {
                    return Err(ToolError::invalid("source path is ambiguous"));
                }
                value
            }
            None => None,
        };
        let status = if signed
            || sheet.protection.is_some()
            || sheet.spills.values().any(|range| {
                (range.first_row..=range.last_row).contains(&row)
                    && (range.first_column..=range.last_column).contains(&col)
            })
            || sheet.hidden_rows.contains_key(&row)
            || sheet.hidden_columns.contains_key(&col)
            || sheet.merge_at(row, col).is_some()
            || current.is_some_and(|cell| cell.formula.is_some())
        {
            "blocked_target"
        } else {
            match (&proposed, path) {
                (_, None) => "manual",
                (None, _) => "not_in_source",
                (Some(value), _) if literal_cell_value(value).is_none() => "unsupported_value",
                (Some(_), _)
                    if current.is_some_and(|cell| !matches!(cell.value, CellValue::Empty)) =>
                {
                    "occupied"
                }
                _ => "ready",
            }
        };
        rows.push(json!({
            "target": chosen.name, "target_label": mapping.opt_str("label")?,
            "cell": cell_name(col, row), "source_path": path,
            "current_value": before, "proposed_value": proposed, "status": status,
        }));
    }
    let sheet_name = local_index
        .and_then(|index| workbook.sheets.get(index))
        .map(|entry| entry.name.as_str())
        .or(selected.name.as_deref())
        .unwrap_or("");
    Ok(json!({
        "sheet":sheet_name, "source_format":source.format, "changes":rows,
        "credential_trust": if source.credential.is_some() { "not_checked" } else { "not_applicable" },
    }))
}

fn literal_cell_value(value: &Value) -> Option<CellValue> {
    match value {
        Value::String(text) => Some(CellValue::Text(text.clone())),
        Value::Bool(value) => Some(CellValue::Bool(*value)),
        Value::Number(value) => value
            .as_i64()
            .filter(|value| value.unsigned_abs() <= 999_999_999_999_999)
            .map(|value| CellValue::Number(value as f64)),
        _ => None,
    }
}

fn apply(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let expected = args.str_arg("preview_id")?;
    let (report, sheet) = plan(service, args)?;
    let stale = || {
        ToolError::new(
            "stale_preview",
            "the prefill plan changed; preview it again",
        )
    };
    if report["preview_id"] != expected {
        return Err(stale());
    }
    let changes = report["changes"].as_array().ok_or_else(stale)?;
    let targets = args["targets"]
        .as_array()
        .filter(|targets| (1..=100).contains(&targets.len()))
        .ok_or_else(|| ToolError::invalid("select one or more ready targets"))?;
    let mut seen = BTreeSet::new();
    let mut edits = Vec::with_capacity(targets.len());
    for target in targets {
        let name = target
            .as_str()
            .ok_or_else(|| ToolError::invalid("targets must be names"))?;
        if !seen.insert(name) {
            return Err(ToolError::invalid("duplicate selected target"));
        }
        let change = changes
            .iter()
            .find(|change| change["target"] == name && change["status"] == "ready")
            .ok_or_else(|| ToolError::invalid("selected target is not ready in the preview"))?;
        let cell = parse_range(change["cell"].as_str().ok_or_else(stale)?)?;
        let value = literal_cell_value(&change["proposed_value"]).ok_or_else(stale)?;
        edits.push(CellEdit {
            row: cell.first_row,
            column: cell.first_column,
            input: CellInput::Value(value),
        });
    }
    let id = args.str_arg("document_id")?;
    let open = service.open_document(id)?;
    if !open.fingerprint.still(&open.path) {
        return Err(stale());
    }
    execute(
        open,
        vec![Command::Sheet(SheetCommand::SetCells { sheet, edits })],
    )?;
    Ok(edited(
        id,
        open,
        json!({"applied_targets":targets}),
        Vec::new(),
    ))
}
