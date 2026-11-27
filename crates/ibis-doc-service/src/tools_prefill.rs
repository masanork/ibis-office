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
        schema: &|| schema(false),
        run: preview,
    }
}

pub(crate) fn apply_tool() -> Tool {
    Tool {
        name: "xlsx_apply_prefill",
        description: "Apply explicitly selected ready fields from an unchanged prefill preview as one undo step in the open workbook. Does not save.",
        access: Access::Edit,
        returns_content: false,
        schema: &|| schema(true),
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
    let sheets = sheets_of(&open.model)?;
    let (index, _, _) = find_sheet(&sheets, args.str_arg("sheet")?)?;
    let presentation = open.model.document.presentation();
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
            || sheet
                .merge_at(row, col)
                .is_some_and(|range| range.first_row != row || range.first_column != col)
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
    let command = reviewed_prefill_command(&report, sheet, &args["targets"])?;
    let targets = &args["targets"];
    let id = args.str_arg("document_id")?;
    let open = service.open_document(id)?;
    if !open.fingerprint.still(&open.path) {
        return Err(stale());
    }
    execute(&mut open.model, vec![command])?;
    Ok(edited(
        id,
        &open.model,
        json!({"applied_targets":targets}),
        Vec::new(),
    ))
}

/// Build the single undoable edit for explicit selections from a freshly checked preview.
pub fn reviewed_prefill_command(
    report: &Value,
    sheet: SlideId,
    targets: &Value,
) -> Result<Command, ToolError> {
    reviewed_prefill_command_with_options(report, sheet, targets, false)
}

/// Build the single undoable edit for explicit selections from a freshly checked preview.
/// When `allow_overwrite` is `true`, targets with status `"occupied"` may also be selected;
/// `"blocked_target"`, `"manual"`, `"not_in_source"`, and `"unsupported_value"` remain refused.
pub fn reviewed_prefill_command_with_options(
    report: &Value,
    sheet: SlideId,
    targets: &Value,
    allow_overwrite: bool,
) -> Result<Command, ToolError> {
    let invalid = || ToolError::invalid("invalid prefill plan");
    let changes = report["changes"].as_array().ok_or_else(invalid)?;
    let targets = targets
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
            .find(|change| {
                change["target"] == name
                    && (change["status"] == "ready"
                        || (allow_overwrite && change["status"] == "occupied"))
            })
            .ok_or_else(|| ToolError::invalid("selected target is not ready in the preview"))?;
        let cell = parse_range(change["cell"].as_str().ok_or_else(invalid)?)?;
        let value = literal_cell_value(&change["proposed_value"]).ok_or_else(invalid)?;
        edits.push(CellEdit {
            row: cell.first_row,
            column: cell.first_column,
            input: CellInput::Value(value),
        });
    }
    Ok(Command::Sheet(SheetCommand::SetCells { sheet, edits }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use ibis_doc_model::sheet::{SheetCell, SheetState, WorkbookSheet};
    use ibis_doc_model::{
        CellRange, DefinedName, EmuRect, Shape, ShapeId, ShapeKind, Sheet, Slide, Workbook,
    };

    fn workbook_for_prefill_tests() -> (Presentation, SlideId) {
        let slide_id = SlideId(1);
        let mut sheet = Sheet::default();
        // Merge B4:D4 (0-based: row 3, col 1..=3)
        sheet.merges.push(CellRange {
            first_row: 3,
            first_column: 1,
            last_row: 3,
            last_column: 3,
        });
        // Occupied cell at B5 (0-based: row 4, col 1)
        sheet.cells.push(SheetCell {
            row: 4,
            column: 1,
            value: CellValue::Text("OldValue".into()),
            text: "OldValue".into(),
            kind: ibis_doc_model::sheet::CellKind::Text,
            formula: None,
            style: 0,
            phonetic: None,
            runs: None,
        });
        // Formula cell at B6 (0-based: row 5, col 1)
        sheet.cells.push(SheetCell {
            row: 5,
            column: 1,
            value: CellValue::Number(2.0),
            text: "2".into(),
            kind: ibis_doc_model::sheet::CellKind::Number,
            formula: Some("1+1".into()),
            style: 0,
            phonetic: None,
            runs: None,
        });

        let mut slide = Slide::new(slide_id);
        slide.name = Some("Form".into());
        slide.shapes.push(Shape::new(
            ShapeId(1),
            EmuRect::default(),
            ShapeKind::Sheet(Arc::new(sheet)),
        ));
        let workbook = Workbook {
            sheets: vec![WorkbookSheet {
                name: "Form".into(),
                state: SheetState::Visible,
                slide: Some(slide_id),
                hidden: None,
                other: false,
            }],
            names: vec![
                DefinedName {
                    name: "merged_anchor".into(),
                    formula: "Form!$B$4".into(),
                    sheet: None,
                    hidden: false,
                },
                DefinedName {
                    name: "merged_interior".into(),
                    formula: "Form!$C$4".into(),
                    sheet: None,
                    hidden: false,
                },
                DefinedName {
                    name: "existing_cell".into(),
                    formula: "Form!$B$5".into(),
                    sheet: None,
                    hidden: false,
                },
                DefinedName {
                    name: "formula_cell".into(),
                    formula: "Form!$B$6".into(),
                    sheet: None,
                    hidden: false,
                },
            ],
            ..Workbook::default()
        };
        let mut presentation = Presentation::default();
        presentation.slides = vec![slide];
        presentation.workbook = Some(Arc::new(workbook));
        (presentation, slide_id)
    }

    #[test]
    fn merged_top_left_anchor_is_ready_and_interior_is_blocked() {
        let (presentation, slide_id) = workbook_for_prefill_tests();
        let source = r#"{"company":"株式会社サンプル","extra":"ignored","formula_val":"x"}"#;
        let mappings = json!([
            {"target": "merged_anchor", "source_path": "/company"},
            {"target": "merged_interior", "source_path": "/extra"},
            {"target": "existing_cell", "source_path": "/company"},
            {"target": "formula_cell", "source_path": "/formula_val"}
        ]);

        let report = review_prefill(&presentation, slide_id, false, "json", source, &mappings)
            .expect("review_prefill succeeds");
        let changes = report["changes"].as_array().unwrap();
        assert_eq!(changes[0]["status"], "ready");
        assert_eq!(changes[0]["cell"], "B4");
        assert_eq!(changes[1]["status"], "blocked_target");
        assert_eq!(changes[1]["cell"], "C4");
        assert_eq!(changes[2]["status"], "occupied");
        assert_eq!(changes[3]["status"], "blocked_target");

        // Top-left merged anchor succeeds in default command builder.
        let cmd = reviewed_prefill_command(&report, slide_id, &json!(["merged_anchor"]))
            .expect("merged anchor command succeeds");
        match cmd {
            Command::Sheet(SheetCommand::SetCells { sheet, edits }) => {
                assert_eq!(sheet, slide_id);
                assert_eq!(edits.len(), 1);
                assert_eq!((edits[0].row, edits[0].column), (3, 1));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        // Occupied cell is refused by default, allowed with explicit allow_overwrite = true.
        assert!(reviewed_prefill_command(&report, slide_id, &json!(["existing_cell"])).is_err());
        let overwrite_cmd = reviewed_prefill_command_with_options(
            &report,
            slide_id,
            &json!(["merged_anchor", "existing_cell"]),
            true,
        )
        .expect("explicit overwrite succeeds");
        match overwrite_cmd {
            Command::Sheet(SheetCommand::SetCells { edits, .. }) => {
                assert_eq!(edits.len(), 2);
                assert_eq!((edits[1].row, edits[1].column), (4, 1));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        // Blocked targets (merged interior & formula) stay refused even with allow_overwrite = true.
        assert!(
            reviewed_prefill_command_with_options(
                &report,
                slide_id,
                &json!(["merged_interior"]),
                true,
            )
            .is_err()
        );
        assert!(
            reviewed_prefill_command_with_options(
                &report,
                slide_id,
                &json!(["formula_cell"]),
                true,
            )
            .is_err()
        );
    }
}
