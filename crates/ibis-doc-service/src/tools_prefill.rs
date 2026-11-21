// SPDX-License-Identifier: AGPL-3.0-only
//! Plan, but do not apply, transfers from structured data into named cells.

use std::collections::BTreeSet;

use ibis_calc::formula::parser::{Expr, parse};
use ibis_doc_model::{CellValue, sheet::cell_name};
use ibis_formats::structured_preview;
use serde_json::{Value, json};

use crate::catalog::{Access, Tool, document_id, object, string};
use crate::service::Args;
use crate::tools_xlsx::{find_sheet, sheets_of, value_json};
use crate::{Service, ToolError};

pub(crate) fn tool() -> Tool {
    Tool {
        name: "xlsx_preview_prefill",
        description: "Preview explicit field-to-named-cell mappings from structured data. Returns existing and proposed values without editing the workbook.",
        access: Access::Read,
        returns_content: true,
        schema: || {
            object(
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
                                "source_path":{"type":["string","null"]}
                            }
                        }}),
                    ),
                ],
                &["document_id", "sheet", "format", "source", "mappings"],
            )
        },
        run: preview,
    }
}

fn preview(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    let format = args.str_arg("format")?;
    let source = structured_preview::read(format, args.str_arg("source")?.as_bytes())
        .map_err(ToolError::invalid)?;
    let mappings = args["mappings"]
        .as_array()
        .filter(|mappings| (1..=100).contains(&mappings.len()))
        .ok_or_else(|| ToolError::invalid("mappings must contain 1–100 entries"))?;
    let open = service.open_document(args.str_arg("document_id")?)?;
    let sheets = sheets_of(open)?;
    let (index, sheet_name, sheet) = find_sheet(&sheets, args.str_arg("sheet")?)?;
    let presentation = open.document.presentation();
    let slide = presentation.slides[*index].id;
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
        let status = if sheet.protection.is_some()
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
                (Some(Value::Null | Value::Array(_) | Value::Object(_)), _) => "unsupported_value",
                (Some(_), _)
                    if current.is_some_and(|cell| !matches!(cell.value, CellValue::Empty)) =>
                {
                    "occupied"
                }
                _ => "ready",
            }
        };
        rows.push(json!({
            "target": chosen.name, "cell": cell_name(col, row), "source_path": path,
            "current_value": before, "proposed_value": proposed, "status": status,
        }));
    }
    Ok(json!({
        "sheet":sheet_name, "source_format":source.format, "changes":rows,
        "credential_trust": if source.credential.is_some() { "not_checked" } else { "not_applicable" },
    }))
}
