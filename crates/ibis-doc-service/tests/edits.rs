// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! GitHub issue #159, stage A: the edit tools. Each makes one undo step, is read back, and is still there after saving and opening the file again (design 8).

use std::path::{Path, PathBuf};

use ibis_doc_service::{Limits, Service};
use serde_json::{Value, json};

fn sandbox(test: &str, files: &[&str]) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/doc-service-tests").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for f in files {
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/editing").join(f), dir.join(f)).unwrap();
    }
    dir.canonicalize().unwrap()
}

struct T {
    s: Service,
    dir: PathBuf,
}

impl T {
    fn new(test: &str, files: &[&str]) -> Self {
        let dir = sandbox(test, files);
        Self { s: Service::new(&[dir.clone()], false, Limits::default()), dir }
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.s.call(tool, &args).unwrap_or_else(|e| panic!("{tool}: {e}"))
    }

    fn err(&mut self, tool: &str, args: Value) -> String {
        self.s.call(tool, &args).unwrap_err().code.to_string()
    }

    fn open(&mut self, name: &str) -> String {
        self.call("app_open_document", json!({ "path": self.dir.join(name).to_string_lossy() }))["document_id"].as_str().unwrap().to_string()
    }

    /// Save as `copy_<name>` and open that.
    fn reopen(&mut self, id: &str, name: &str) -> String {
        let copy = format!("copy_{name}");
        self.call("app_save_document", json!({ "document_id": id, "path": self.dir.join(&copy).to_string_lossy(), "overwrite": true }));
        self.open(&copy)
    }
}

#[test]
fn mcp_docx_edits_are_one_undo_step_each_and_survive_saving() {
    let mut t = T::new("docx_edits", &["autospace-letters.docx"]);
    let d = t.open("autospace-letters.docx");
    let first = t.call("docx_get_paragraph", json!({ "document_id": d, "paragraph": 0 }))["text"].as_str().unwrap().to_string();
    assert!(first.chars().count() >= 2, "{first:?}");
    // Insert.
    let r = t.call("docx_insert_text", json!({ "document_id": d, "at": { "paragraph": 0, "offset": 0 }, "text": "ABC" }));
    assert_eq!(r["dirty"], true);
    assert_eq!(t.call("docx_get_paragraph", json!({ "document_id": d, "paragraph": 0 }))["text"], format!("ABC{first}"));
    // One undo takes it back.
    assert_eq!(t.call("app_undo_document", json!({ "document_id": d }))["undone"], 1);
    assert_eq!(t.call("docx_get_paragraph", json!({ "document_id": d, "paragraph": 0 }))["text"], first);
    t.call("app_redo_document", json!({ "document_id": d }));
    // Delete what was inserted.
    t.call("docx_delete_text", json!({ "document_id": d, "range": { "from": { "paragraph": 0, "offset": 0 }, "to": { "paragraph": 0, "offset": 3 } } }));
    assert_eq!(t.call("docx_get_paragraph", json!({ "document_id": d, "paragraph": 0 }))["text"], first);
    // Replace: every place, one undo step.
    t.call("docx_insert_text", json!({ "document_id": d, "at": { "paragraph": 0, "offset": 0 }, "text": "QQ and QQ and " }));
    let r = t.call("docx_replace_text", json!({ "document_id": d, "find": "QQ", "replace": "ZZZ" }));
    assert_eq!(r["replaced"], 2);
    assert!(t.call("docx_get_paragraph", json!({ "document_id": d, "paragraph": 0 }))["text"].as_str().unwrap().starts_with("ZZZ and ZZZ and "));
    t.call("app_undo_document", json!({ "document_id": d }));
    assert!(t.call("docx_get_paragraph", json!({ "document_id": d, "paragraph": 0 }))["text"].as_str().unwrap().starts_with("QQ and QQ and "));
    t.call("app_redo_document", json!({ "document_id": d }));
    // Format a range: bold, 14.5 pt, a colour, a typeface.
    t.call("docx_set_run_format", json!({ "document_id": d, "range": { "from": { "paragraph": 0, "offset": 0 }, "to": { "paragraph": 0, "offset": 3 } }, "bold": true, "size": 14.5, "color": "#FF8000", "font": { "latin": "Arial", "east_asian": "BIZ UDゴシック" } }));
    let runs = t.call("docx_get_paragraph", json!({ "document_id": d, "paragraph": 0 }))["runs"].clone();
    let run = &runs[0];
    assert_eq!((run["bold"].clone(), run["size"].clone(), run["color"].clone()), (json!(true), json!(14.5), json!("#FF8000")), "{runs}");
    assert_eq!(run["font"], "Arial");
    assert_eq!(run["east_asian_font"], "BIZ UDゴシック");
    // A wrong argument is refused and changes nothing.
    assert_eq!(t.err("docx_set_run_format", json!({ "document_id": d, "range": { "from": { "paragraph": 0, "offset": 0 }, "to": { "paragraph": 0, "offset": 1 } }, "color": "red" })), "invalid_argument");
    assert_eq!(t.err("docx_set_run_format", json!({ "document_id": d, "range": { "from": { "paragraph": 0, "offset": 0 }, "to": { "paragraph": 0, "offset": 1 } }, "colour": "#000000" })), "invalid_argument");
    assert_eq!(t.err("docx_insert_text", json!({ "document_id": d, "at": { "paragraph": 0, "offset": 99999 }, "text": "x" })), "out_of_range");
    assert_eq!(t.err("docx_insert_text", json!({ "document_id": d, "at": { "paragraph": 0, "offset": 0 }, "text": "a\nb" })), "invalid_argument");
    // A table with text in its cells: one undo step for all of it.
    let tables_before = t.call("docx_list_tables", json!({ "document_id": d }))["tables"].as_array().unwrap().len();
    t.call("docx_insert_table", json!({ "document_id": d, "at": { "paragraph": 0, "offset": 0 }, "rows": 2, "columns": 2, "cells": [["a", "b"], ["c", "d"]] }));
    assert_eq!(t.call("docx_list_tables", json!({ "document_id": d }))["tables"].as_array().unwrap().len(), tables_before + 1);
    // Saved, opened again: the edits are there.
    let again = t.reopen(&d, "autospace-letters.docx");
    let run = t.call("docx_get_paragraph", json!({ "document_id": again, "paragraph": 0 }));
    let text = t.call("docx_get_text", json!({ "document_id": again, "format": "plain" }))["text"].as_str().unwrap().to_string();
    assert!(text.contains("ZZZ and ZZZ and "), "{run}");
    assert!(text.contains("a\tb") && text.contains("c\td"), "the table's cells: {text:?}");
    let first_run = &run["runs"][0];
    assert_eq!(first_run["bold"], true);
    assert_eq!(first_run["color"], "#FF8000");
    t.call("app_undo_document", json!({ "document_id": d }));
    assert_eq!(t.call("docx_list_tables", json!({ "document_id": d }))["tables"].as_array().unwrap().len(), tables_before, "the table and its cells are one undo step");
}

#[test]
fn mcp_xlsx_shared_format_and_clear_keep_separate_undo_steps() {
    let mut t = T::new("xlsx_shared_format_clear", &[]);
    let bytes = ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap();
    std::fs::write(t.dir.join("blank.xlsx"), bytes).unwrap();
    let x = t.open("blank.xlsx");
    let sheet = t.call("xlsx_list_sheets", json!({ "document_id": x }))["sheets"][0]["name"].as_str().unwrap().to_string();
    t.call("xlsx_set_cells", json!({ "document_id": x, "sheet": sheet, "range": "B2", "values": 12 }));
    let format = t.call("xlsx_set_format", json!({ "document_id": x, "sheet": sheet, "range": "B2", "bold": true, "number_format": "0.00", "border": "outline" }));
    assert_eq!(format["document_id"], x);
    assert_eq!(format["warnings"], json!([]));
    let text = t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "B2", "kind": "text" }));
    assert_eq!(text["rows"], json!([["12.00"]]));
    t.call("xlsx_clear_cells", json!({ "document_id": x, "sheet": sheet, "range": "B2", "what": "all" }));
    let cleared = t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "B2" }));
    assert_eq!(cleared["rows"], json!([[null]]));
    assert_eq!(t.call("app_undo_document", json!({ "document_id": x }))["undone"], 1);
    let restored = t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "B2", "kind": "text" }));
    assert_eq!(restored["rows"], text["rows"]);
    assert_eq!(t.call("app_undo_document", json!({ "document_id": x }))["undone"], 1);
    let unformatted = t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "B2", "kind": "text" }));
    assert_eq!(unformatted["rows"], json!([["12"]]));
}

#[test]
fn mcp_xlsx_formula_reads_respect_sheet_protection() {
    use ibis_doc_model::sheet::{CellRange, SheetProtection};
    use ibis_doc_model::sheet_command::{CellEdit, CellInput, SheetCommand, StyleChange};
    use ibis_doc_model::{Command, Document};
    use ibis_formats::{Kind, SaveOptions};

    let bytes = ibis_formats::blank_file(Kind::Workbook).unwrap();
    let opened = ibis_formats::open(bytes).unwrap();
    let mut document = Document::new(opened.presentation);
    let sheet_id = document.presentation().slides[0].id;
    document.execute(vec![
        Command::Sheet(SheetCommand::SetCells { sheet: sheet_id, edits: vec![
            CellEdit { row: 0, column: 0, input: CellInput::Formula("B1-2".into()) },
            CellEdit { row: 0, column: 1, input: CellInput::Formula("2+3".into()) },
        ] }),
        Command::Sheet(SheetCommand::SetFormat { sheet: sheet_id, ranges: vec![
            CellRange { first_row: 0, last_row: 0, first_column: 0, last_column: 0 },
        ], change: StyleChange::FormulaHidden(true) }),
        Command::Sheet(SheetCommand::SetProtection { sheet: sheet_id, protection: Some(SheetProtection::default()) }),
    ]).unwrap();

    let mut t = T::new("xlsx_hidden_formulas", &[]);
    let bytes = opened.source.save(document.presentation(), SaveOptions::default()).unwrap();
    std::fs::write(t.dir.join("protected.xlsx"), bytes).unwrap();
    let x = t.open("protected.xlsx");
    let sheet = t.call("xlsx_list_sheets", json!({ "document_id": x }))["sheets"][0]["name"].as_str().unwrap().to_string();
    let formulas = t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "A1:B1", "kind": "formula" }));
    assert_eq!(formulas["rows"], json!([[3.0, "=2+3"]]));
    let hidden = t.call("xlsx_find_text", json!({"document_id":x,"sheet":sheet,"query":"B1-2","search_formulas":true}));
    assert_eq!(hidden["matches"], json!([]));
    let trace = t.call("xlsx_trace_formula", json!({"document_id":x,"sheet":sheet,"cell":"A1","direction":"precedents"}));
    assert_eq!(trace["references"], json!([]));
    let dependents = t.call("xlsx_trace_formula", json!({"document_id":x,"sheet":sheet,"cell":"B1","direction":"dependents"}));
    assert_eq!(dependents["references"], json!([]));
    let visible = t.call("xlsx_find_text", json!({"document_id":x,"sheet":sheet,"query":"2+3","search_formulas":true}));
    assert_eq!(visible["matches"][0]["formula"], "=2+3");
}

#[test]
fn mcp_xlsx_cell_inputs_use_shared_rules() {
    let mut t = T::new("xlsx_cell_inputs", &[]);
    let bytes = ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap();
    std::fs::write(t.dir.join("blank.xlsx"), bytes).unwrap();
    let x = t.open("blank.xlsx");
    let sheet = t.call("xlsx_list_sheets", json!({ "document_id": x }))["sheets"][0]["name"].as_str().unwrap().to_string();
    t.call("xlsx_set_cells", json!({ "document_id": x, "sheet": sheet, "range": "AA1:AA3", "values": [["="], ["=  1+2  "], [true]] }));
    assert_eq!(t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "AA1:AA3" }))["rows"], json!([["="], [3.0], [true]]));
    assert_eq!(t.err("xlsx_set_cells", json!({ "document_id": x, "sheet": sheet, "range": "AA1:AA100001", "values": 0 })), "out_of_range");
}

#[test]
fn mcp_xlsx_edits_calculate_format_and_survive_saving() {
    let mut t = T::new("xlsx_edits", &["xlsx-sales.xlsx"]);
    let x = t.open("xlsx-sales.xlsx");
    let sheet = t.call("xlsx_list_sheets", json!({ "document_id": x }))["sheets"][0]["name"].as_str().unwrap().to_string();
    // Values, a formula that is calculated, one undo step.
    t.call("xlsx_set_cells", json!({ "document_id": x, "sheet": sheet, "range": "Z1:Z3", "values": [[1], [2.5], ["=Z1+Z2"]] }));
    let cells = t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "Z1:Z3" }));
    assert_eq!(cells["rows"], json!([[1.0], [2.5], [3.5]]));
    let formulas = t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "Z3", "kind": "formula" }));
    assert_eq!(formulas["rows"], json!([["=Z1+Z2"]]));
    // A single value fills the range; null clears.
    t.call("xlsx_set_cells", json!({ "document_id": x, "sheet": sheet, "range": "Y1:Y2", "values": "text" }));
    assert_eq!(t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "Y1:Y2" }))["rows"], json!([["text"], ["text"]]));
    assert_eq!(t.err("xlsx_set_cells", json!({ "document_id": x, "sheet": sheet, "range": "Y1:Y3", "values": [["a"], ["b"]] })), "out_of_range");
    // Format.
    t.call("xlsx_set_format", json!({ "document_id": x, "sheet": sheet, "range": "Z1:Z3", "bold": true, "size": 12, "color": "#112233", "fill": { "theme": "accent1", "brightness": 0.4 }, "number_format": "0.00", "horizontal_alignment": "center" }));
    assert_eq!(t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "Z1:Z2", "kind": "text" }))["rows"], json!([["1.00"], ["2.50"]]));
    assert_eq!(t.err("xlsx_set_format", json!({ "document_id": x, "sheet": sheet, "range": "Z1", "horizontal_alignment": "middle" })), "invalid_argument");
    assert_eq!(t.err("xlsx_set_format", json!({ "document_id": x, "sheet": sheet, "range": "Z1" })), "invalid_argument");
    // `null` takes a format back to the Normal style's: the same file's cells read as they did before the format.
    let plain = t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "Y2", "kind": "format" }))["rows"][0][0].clone();
    assert!(plain["bold"].is_boolean(), "{plain}");
    t.call("xlsx_set_format", json!({ "document_id": x, "sheet": sheet, "range": "Y1", "bold": true, "size": 18, "color": "#112233", "fill": "#FFEE00" }));
    assert_ne!(t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "Y1", "kind": "format" }))["rows"][0][0], plain);
    t.call("xlsx_set_format", json!({ "document_id": x, "sheet": sheet, "range": "Y1", "bold": null, "size": null, "color": null, "fill": null }));
    assert_eq!(t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "Y1", "kind": "format" }))["rows"][0][0], plain, "null: the Normal style's");
    assert_eq!(t.err("xlsx_set_format", json!({ "document_id": x, "sheet": sheet, "range": "Y1", "font": { "theme": "other" } })), "invalid_argument");
    // Clear.
    t.call("xlsx_clear_cells", json!({ "document_id": x, "sheet": sheet, "range": "Y1:Y2", "what": "all" }));
    assert_eq!(t.call("xlsx_get_cells", json!({ "document_id": x, "sheet": sheet, "range": "Y1:Y2" }))["rows"], json!([[null], [null]]));
    // Sheets.
    t.call("xlsx_add_sheet", json!({ "document_id": x, "name": "Extra" }));
    t.call("xlsx_rename_sheet", json!({ "document_id": x, "sheet": "Extra", "name": "Renamed" }));
    assert_eq!(t.err("xlsx_add_sheet", json!({ "document_id": x, "name": "renamed" })), "invalid_argument");
    assert_eq!(t.err("xlsx_add_sheet", json!({ "document_id": x, "name": "a/b" })), "invalid_argument");
    let names: Vec<String> = t.call("xlsx_list_sheets", json!({ "document_id": x }))["sheets"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names.last().map(String::as_str), Some("Renamed"));
    t.call("xlsx_set_cells", json!({ "document_id": x, "sheet": "Renamed", "range": "A1", "values": "x" }));
    t.call("xlsx_calculate_workbook", json!({ "document_id": x }));
    // Saved and opened again.
    let again = t.reopen(&x, "xlsx-sales.xlsx");
    assert_eq!(t.call("xlsx_get_cells", json!({ "document_id": again, "sheet": sheet, "range": "Z1:Z3" }))["rows"], json!([[1.0], [2.5], [3.5]]));
    assert_eq!(t.call("xlsx_get_cells", json!({ "document_id": again, "sheet": "Renamed", "range": "A1" }))["rows"], json!([["x"]]));
    // Undo, one step at a time, takes it all back; the last sheet is not deleted.
    assert_eq!(t.call("app_undo_document", json!({ "document_id": x, "count": 100 }))["can_undo"], false);
    let sheets = t.call("xlsx_list_sheets", json!({ "document_id": x }))["sheets"].as_array().unwrap().len();
    assert_eq!(sheets, names.len() - 1);
    for name in &names[..names.len() - 1] {
        if t.call("xlsx_list_sheets", json!({ "document_id": x }))["sheets"].as_array().unwrap().len() > 1 {
            t.call("xlsx_delete_sheet", json!({ "document_id": x, "sheet": name }));
        }
    }
    let last = t.call("xlsx_list_sheets", json!({ "document_id": x }))["sheets"][0]["name"].as_str().unwrap().to_string();
    assert_eq!(t.err("xlsx_delete_sheet", json!({ "document_id": x, "sheet": last })), "unsupported_operation");
}

#[test]
fn mcp_pptx_edits_are_one_undo_step_each_and_survive_saving() {
    let mut t = T::new("pptx_edits", &["pptx-plan.pptx"]);
    let p = t.open("pptx-plan.pptx");
    let slide = t.call("pptx_get_slide", json!({ "document_id": p, "slide": 1 }));
    let with_text = slide["shapes"].as_array().unwrap().iter().find(|s| s["text"].is_string()).expect("a shape with text").clone();
    let shape = with_text["shape_id"].as_u64().unwrap();
    // Text, one undo step.
    t.call("pptx_set_shape_text", json!({ "document_id": p, "slide": 1, "shape": shape, "text": "QQ first\nQQ second" }));
    assert_eq!(t.call("pptx_get_shape_text", json!({ "document_id": p, "slide": 1, "shape": shape }))["paragraphs"].as_array().unwrap().len(), 2);
    let r = t.call("pptx_replace_text", json!({ "document_id": p, "find": "QQ", "replace": "Z" }));
    assert_eq!(r["replaced"], 2);
    let shown = t.call("pptx_get_slide", json!({ "document_id": p, "slide": 1 }));
    let text = shown["shapes"].as_array().unwrap().iter().find(|s| s["shape_id"] == shape).unwrap()["text"].clone();
    assert_eq!(text, "Z first\nZ second");
    t.call("app_undo_document", json!({ "document_id": p }));
    let undone = t.call("pptx_get_slide", json!({ "document_id": p, "slide": 1 }));
    assert_eq!(undone["shapes"].as_array().unwrap().iter().find(|s| s["shape_id"] == shape).unwrap()["text"], "QQ first\nQQ second");
    // Format the characters of the first paragraph.
    t.call("pptx_set_run_format", json!({ "document_id": p, "slide": 1, "shape": shape, "range": { "paragraph": 0, "from": 0, "to": 2 }, "bold": true, "size": 20, "color": "#336699" }));
    assert_eq!(t.err("pptx_set_run_format", json!({ "document_id": p, "slide": 1, "shape": shape, "bold": "yes" })), "invalid_argument");
    // `null` takes a format back to what the styles give; a theme typeface is written as the theme's (with a warning); null for a typeface is the theme's.
    let run = |t: &mut T| t.call("pptx_get_shape_text", json!({ "document_id": p, "slide": 1, "shape": shape }))["paragraphs"][0]["runs"][0].clone();
    let before = run(&mut t);
    assert_eq!((before["bold"].clone(), before["size"].clone(), before["color"].clone()), (json!(true), json!(20.0), json!("#336699")));
    t.call("pptx_set_run_format", json!({ "document_id": p, "slide": 1, "shape": shape, "range": { "paragraph": 0, "from": 0, "to": 2 }, "bold": null, "size": null, "color": null }));
    let after = run(&mut t);
    assert_eq!((after["bold"].clone(), after["size"].clone(), after["color"].clone()), (Value::Null, Value::Null, Value::Null), "{after}");
    let themed = t.call("pptx_set_run_format", json!({ "document_id": p, "slide": 1, "shape": shape, "range": { "paragraph": 0, "from": 0, "to": 2 }, "font": { "theme": "major" } }));
    let warned = themed["warnings"].as_array().unwrap().iter().any(|w| w["code"] == "theme_font_resolved");
    let named = run(&mut t)["font"].is_string();
    assert!(warned && named || t.err("pptx_set_run_format", json!({ "document_id": p, "slide": 1, "shape": shape, "font": { "theme": "major" } })) == "unsupported_operation", "a theme typeface is written as its name, or refused when the file has none");
    t.call("pptx_set_run_format", json!({ "document_id": p, "slide": 1, "shape": shape, "range": { "paragraph": 0, "from": 0, "to": 2 }, "font": null }));
    assert_eq!(run(&mut t)["font"], Value::Null, "null: the theme's typeface again");
    assert_eq!(t.err("pptx_set_run_format", json!({ "document_id": p, "slide": 1, "shape": shape, "font": { "theme": "other" } })), "invalid_argument");
    // Frame in points, notes.
    t.call("pptx_set_frame", json!({ "document_id": p, "slide": 1, "frames": [{ "shape": shape, "x": 36, "y": 72, "width": 200, "height": 50.5 }] }));
    let moved = t.call("pptx_get_slide", json!({ "document_id": p, "slide": 1 }));
    let frame = moved["shapes"].as_array().unwrap().iter().find(|s| s["shape_id"] == shape).unwrap().clone();
    assert_eq!((frame["x"].clone(), frame["y"].clone(), frame["width"].clone(), frame["height"].clone()), (json!(36.0), json!(72.0), json!(200.0), json!(50.5)));
    t.call("pptx_set_notes", json!({ "document_id": p, "slide": 1, "text": "memo" }));
    // Slides.
    let count = t.call("pptx_list_slides", json!({ "document_id": p }))["slides"].as_array().unwrap().len();
    let layouts = t.call("pptx_list_layouts", json!({ "document_id": p }));
    assert!(!layouts["layouts"].as_array().unwrap().is_empty());
    t.call("pptx_add_slide", json!({ "document_id": p, "index": 1 }));
    assert_eq!(t.call("pptx_list_slides", json!({ "document_id": p }))["slides"].as_array().unwrap().len(), count + 1);
    t.call("pptx_move_slide", json!({ "document_id": p, "slide": 2, "index": 0 }));
    assert_eq!(t.err("pptx_move_slide", json!({ "document_id": p, "slide": 2, "index": 999 })), "out_of_range");
    t.call("pptx_move_slide", json!({ "document_id": p, "slide": 1, "index": 1 }));
    t.call("pptx_delete_shape", json!({ "document_id": p, "slide": 1, "shapes": [shape] }));
    assert_eq!(t.err("pptx_get_shape_text", json!({ "document_id": p, "slide": 1, "shape": shape })), "no_such_shape");
    t.call("app_undo_document", json!({ "document_id": p }));
    // Saved and opened again: the new slide, the notes, the frame.
    let again = t.reopen(&p, "pptx-plan.pptx");
    assert_eq!(t.call("pptx_list_slides", json!({ "document_id": again }))["slides"].as_array().unwrap().len(), count + 1);
    assert_eq!(t.call("pptx_get_notes", json!({ "document_id": again, "slide": 1 }))["notes"], "memo");
    let saved = t.call("pptx_get_slide", json!({ "document_id": again, "slide": 1 }));
    let frame = saved["shapes"].as_array().unwrap().iter().find(|s| s["x"] == 36.0 && s["y"] == 72.0 && s["width"] == 200.0);
    assert!(frame.is_some(), "the moved shape is where it was put: {saved}");
    t.call("pptx_delete_slide", json!({ "document_id": p, "slide": 1 }));
    assert_eq!(t.call("pptx_list_slides", json!({ "document_id": p }))["slides"].as_array().unwrap().len(), count);
}

/// #159 stage B: in a document `null` takes a direct format away (the styles decide again) and survives saving; a theme typeface is written as the theme's name with a warning.
#[test]
fn mcp_docx_null_takes_a_format_back_and_a_theme_typeface_is_written() {
    let mut t = T::new("docx_null", &["autospace-letters.docx"]);
    let d = t.open("autospace-letters.docx");
    let range = json!({ "from": { "paragraph": 0, "offset": 0 }, "to": { "paragraph": 0, "offset": 3 } });
    let run = |t: &mut T, d: &str| t.call("docx_get_paragraph", json!({ "document_id": d, "paragraph": 0 }))["runs"][0].clone();
    let before = run(&mut t, &d);
    t.call("docx_set_run_format", json!({ "document_id": d, "range": range, "bold": true, "size": 31, "color": "#FF8000", "font": "Arial" }));
    let direct = run(&mut t, &d);
    assert_eq!((direct["bold"].clone(), direct["size"].clone(), direct["color"].clone()), (json!(true), json!(31.0), json!("#FF8000")));
    t.call("docx_set_run_format", json!({ "document_id": d, "range": range, "bold": null, "size": null, "color": null, "font": null }));
    let reset = run(&mut t, &d);
    for key in ["bold", "size", "color", "font"] {
        assert_eq!(reset[key], before[key], "{key} is what the styles give again: {before} {reset}");
    }
    let saved = t.reopen(&d, "autospace-letters.docx");
    assert_eq!(run(&mut t, &saved)["size"], before["size"], "and the file has no direct size");
    let themed = t.call("docx_set_run_format", json!({ "document_id": d, "range": range, "font": { "theme": "minor" } }));
    assert!(themed["warnings"].as_array().unwrap().iter().any(|w| w["code"] == "theme_font_resolved"), "{themed}");
    assert_eq!(t.err("docx_set_run_format", json!({ "document_id": d, "range": range, "font": { "theme": "x" } })), "invalid_argument");
}

#[test]
fn canonical_docx_edits_replay_as_one_editor_draft() {
    use ibis_formats::Kind;
    use ibis_doc_model::Document;

    let mut t = T::new("shared_docx_draft", &[]);
    let bytes = ibis_formats::blank_file(Kind::Document).unwrap();
    let base = ibis_formats::read(&bytes).unwrap();
    std::fs::write(t.dir.join("blank.docx"), bytes).unwrap();
    let id = t.open("blank.docx");
    let mut live = Document::new(base.clone());
    let mut draft = Document::new(base.clone());
    let mut commands = Vec::new();

    for (name, args) in [
        ("docx_insert_text", json!({ "at": {"paragraph":0,"offset":0}, "text": "draft" })),
        ("docx_set_paragraph_text", json!({ "paragraph":0, "text": "revised" })),
        ("docx_set_paragraph_format", json!({ "paragraph":0, "align": "center" })),
        ("docx_insert_table", json!({ "at":{"paragraph":0,"offset":7}, "rows":1,"columns":1,"cells":[["table text"]] })),
    ] {
        let (planned, edits) = ibis_doc_service::run_draft_tool(Kind::Document, &mut draft, Default::default(), name, args.clone()).unwrap();
        assert!(planned.get("document_id").is_none());
        let mut file_args = args;
        file_args["document_id"] = json!(id);
        let mut actual = t.call(name, file_args);
        actual.as_object_mut().unwrap().remove("document_id");
        assert_eq!(planned, actual);
        commands.extend(edits);
    }
    for read in ["docx_get_text", "docx_list_tables"] {
        let (planned, edits) = ibis_doc_service::run_draft_tool(Kind::Document, &mut draft, Default::default(), read, json!({})).unwrap();
        assert!(edits.is_empty());
        assert_eq!(planned, t.call(read, json!({ "document_id": id })));
    }
    assert_eq!(live.presentation(), &base);
    live.execute(commands).unwrap();
    assert_eq!(live.presentation(), draft.presentation());
    assert_eq!(live.undo_len(), 1);
    assert!(live.undo());
    assert_eq!(live.presentation(), &base);

    let rejected = ibis_doc_service::run_draft_tool(Kind::Document, &mut draft, Default::default(), "app_save_document", json!({})).unwrap_err();
    assert_eq!(rejected.code, "unsupported_operation");
}

#[test]
fn canonical_pptx_edits_share_targets_and_keep_mcp_undo_per_call() {
    use ibis_formats::Kind;
    use ibis_doc_model::Document;

    let mut t = T::new("shared_pptx_draft", &[]);
    let bytes = ibis_formats::blank_file(Kind::Presentation).unwrap();
    let base = ibis_formats::read(&bytes).unwrap();
    std::fs::write(t.dir.join("blank.pptx"), bytes).unwrap();
    let id = t.open("blank.pptx");
    let mut live = Document::new(base.clone());
    let mut draft = Document::new(base);
    let mut commands = Vec::new();
    for (name, args) in [
        ("pptx_add_slide", json!({})),
        ("pptx_set_notes", json!({ "slide":2, "text": "speaker notes" })),
        ("pptx_move_slide", json!({ "slide":2, "index":0 })),
    ] {
        let (_, edits) = ibis_doc_service::run_draft_tool(Kind::Presentation, &mut draft, Default::default(), name, args.clone()).unwrap();
        assert!(!edits.is_empty());
        let mut file_args = args;
        file_args["document_id"] = json!(id);
        t.call(name, file_args);
        commands.extend(edits);
    }
    let (notes, _) = ibis_doc_service::run_draft_tool(Kind::Presentation, &mut draft, Default::default(), "pptx_get_notes", json!({"slide":1})).unwrap();
    assert_eq!(notes, t.call("pptx_get_notes", json!({"document_id":id,"slide":1})));
    live.execute(commands).unwrap();
    live.reserve_ids_from(&draft);
    assert_eq!(live.presentation(), draft.presentation());
    assert!(live.undo());
    assert_eq!(live.presentation().slides.len(), 1);
    t.call("app_undo_document", json!({"document_id":id}));
    let notes = t.call("pptx_get_notes", json!({"document_id":id,"slide":2}));
    assert_eq!(notes["notes"], "speaker notes");
}

#[test]
fn workbook_active_sheet_is_a_draft_default_only() {
    use ibis_doc_model::Document;
    use ibis_formats::Kind;

    let mut t = T::new("workbook_draft_sheet_default", &[]);
    let bytes = ibis_formats::blank_file(Kind::Workbook).unwrap();
    let mut draft = Document::new(ibis_formats::read(&bytes).unwrap());
    std::fs::write(t.dir.join("blank.xlsx"), bytes).unwrap();
    let id = t.open("blank.xlsx");
    let active = draft.presentation().slides[0].id;

    assert_eq!(t.err("xlsx_get_used_range", json!({"document_id": id})), "invalid_argument");
    for (name, args) in [
        ("xlsx_rename_sheet", json!({"name": "Draft name"})),
        ("xlsx_get_used_range", json!({})),
    ] {
        let context = ibis_doc_service::DraftToolContext {
            workbook: ibis_doc_service::WorkbookToolContext { active_sheet: Some(active), ..Default::default() },
            ..Default::default()
        };
        let (result, _) = ibis_doc_service::run_draft_tool(Kind::Workbook, &mut draft, context, name, args).unwrap();
        assert_eq!(result["sheet"], "Draft name");
    }
}

#[test]
fn mcp_chart_links_to_another_sheet_after_saving() {
    use ibis_doc_model::ShapeKind;
    use ibis_formats::Kind;

    let mut t = T::new("shared_chart", &[]);
    let bytes = ibis_formats::blank_file(Kind::Workbook).unwrap();
    std::fs::write(t.dir.join("blank.xlsx"), bytes).unwrap();
    let id = t.open("blank.xlsx");
    let data = t.call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].as_str().unwrap().to_owned();
    t.call("xlsx_add_sheet", json!({"document_id":id,"name":"Report"}));
    t.call("xlsx_set_cells", json!({"document_id":id,"sheet":data,"range":"A1:B3","values":[["Year","Cost"],["2025",10],["2026",12]]}));
    let args = json!({
        "document_id":id,"sheet":data,"destination_sheet":"Report","range":"A1:B3",
        "anchor":"F2","kind":"line","title":"Expenses",
    });
    let mut missing = args.clone();
    missing.as_object_mut().unwrap().remove("destination_sheet");
    assert_eq!(t.err("xlsx_create_chart", missing), "invalid_argument");
    let result = t.call("xlsx_create_chart", args);
    assert_eq!(result["destination_sheet"], "Report");
    assert!(result["shape_id"].is_number());

    t.call("app_save_document", json!({"document_id":id,"path":t.dir.join("chart.xlsx")}));
    let reopened = ibis_formats::read(&std::fs::read(t.dir.join("chart.xlsx")).unwrap()).unwrap();
    let report = reopened.slides.iter().find(|slide| slide.name.as_deref() == Some("Report")).unwrap();
    let chart = report.shapes.iter().find_map(|shape| match &shape.kind {
        ShapeKind::Chart(chart) => Some(chart),
        _ => None,
    }).unwrap();
    let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(report).unwrap();
    let origin = report.shapes.iter().find(|shape| matches!(shape.kind, ShapeKind::Sheet(_))).unwrap().frame;
    let cell = sheet.cell_bounds_emu(5, 1);
    let figure = report.shapes.iter().find(|shape| matches!(shape.kind, ShapeKind::Chart(_))).unwrap();
    assert_eq!((figure.frame.x, figure.frame.y), (origin.x + cell.x, origin.y + cell.y));
    assert_eq!(chart.title.as_ref().unwrap().text, "Expenses");
    let values = chart.cell_refs[0].values.as_deref().unwrap();
    let (sheet_name, range) = ibis_formats::parse_ref(values).unwrap();
    assert_eq!(sheet_name, data);
    assert_eq!((range.first_row, range.last_row, range.first_column), (1, 2, 1));
    assert_eq!(t.call("app_undo_document", json!({"document_id":id}))["undone"], 1);
}

#[test]
fn draft_chart_defaults_to_active_sheet_and_keeps_sparse_series() {
    use ibis_doc_model::{Document, ShapeKind};
    use ibis_formats::{Kind, SaveOptions};
    use std::sync::Arc;

    let opened = ibis_formats::open(ibis_formats::blank_file(Kind::Workbook).unwrap()).unwrap();
    let live_source = opened.source;
    let chart_source = Arc::new(live_source.with_private_chart_cache());
    let mut draft = Document::new(opened.presentation);
    let active = draft.presentation().slides[0].id;
    let context = || ibis_doc_service::DraftToolContext {
        source: Some(chart_source.clone()),
        workbook: ibis_doc_service::WorkbookToolContext { active_sheet: Some(active), ..Default::default() },
    };
    let run = ibis_doc_service::run_draft_tool;
    run(Kind::Workbook, &mut draft, context(), "xlsx_set_cells", json!({
        "range":"A1:C4","values":[[null,"Group1","Group2"],[195.6,null,105.81],[23.7,254.38,null],[5.1,"=NA()",null]]
    })).unwrap();
    let args = json!({
        "kind":"scatter","anchor":"E2","domain_range":"A2:A4",
        "series":[{"values_range":"B2:B4","name_cell":"B1"},{"values_range":"C2:C4","name_cell":"C1"}],
        "title":"Comparison","x_title":"Population","y_title":"Cost"
    });
    let mut invalid = args.clone();
    invalid["series"][0]["values_range"] = json!("B2:B3");
    assert_eq!(run(Kind::Workbook, &mut draft, context(), "xlsx_create_chart", invalid).unwrap_err().code, "invalid_argument");
    let (_, commands) = run(Kind::Workbook, &mut draft, context(), "xlsx_create_chart", args).unwrap();
    assert_eq!(commands.len(), 1);
    let shape = draft.presentation().slides[0].shapes.last().unwrap();
    assert!(chart_source.has_new_chart(shape.id));
    assert!(!live_source.has_new_chart(shape.id));
    let saved = chart_source.save(draft.presentation(), SaveOptions::default()).unwrap();
    let reopened = ibis_formats::read(&saved).unwrap();
    let ShapeKind::Chart(chart) = &reopened.slides[0].shapes.last().unwrap().kind else { panic!("expected chart") };
    assert_eq!(chart.title.as_ref().unwrap().text, "Comparison");
    assert!(chart.axes.iter().any(|axis| axis.title.as_ref().is_some_and(|title| title.text == "Cost")));
    assert_eq!(chart.groups[0].series[0].values.iter().map(|n| n.map(|n| n.0)).collect::<Vec<_>>(), [None, Some(254.38), None]);
    assert!(draft.undo());
    assert_eq!(draft.presentation().slides[0].shapes.len(), 1);
}

#[test]
fn mcp_machine_readability_pages_candidates_on_a_read_only_workbook() {
    let mut t = T::new("shared_readability", &[]);
    let bytes = ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap();
    std::fs::write(t.dir.join("blank.xlsx"), bytes).unwrap();
    let id = t.open("blank.xlsx");
    let sheet = t.call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].as_str().unwrap().to_owned();
    t.call("xlsx_set_cells", json!({"document_id":id,"sheet":sheet,"range":"A1:A2","values":[["①"],["=1+1"]]}));
    t.call("xlsx_add_sheet", json!({"document_id":id,"name":"Other"}));
    t.call("xlsx_set_cells", json!({"document_id":id,"sheet":"Other","range":"B1","values":"\u{e000}"}));
    let path = t.dir.join("candidates.xlsx");
    t.call("app_save_document", json!({"document_id":id,"path":path}));
    t.s = Service::new(&[t.dir.clone()], true, Limits::default());
    let id = t.open("candidates.xlsx");
    let name = "xlsx_check_machine_readability";
    let page = t.call(name, json!({"document_id":id,"limit":1}));
    assert_eq!(page["count"], 3);
    assert_eq!(page["next_offset"], 1);
    assert_eq!(page["findings"], json!([{"sheet":sheet,"kind":"platform_dependent_characters","location":"A1"}]));
    let next = t.call(name, json!({"document_id":id,"offset":page["next_offset"]}));
    assert!(next.get("next_offset").is_none());
    assert_eq!(next["findings"], json!([
        {"sheet":sheet,"kind":"formula","location":"A2"},
        {"sheet":"Other","kind":"private_use_characters","location":"B1"}
    ]));
    assert_eq!(t.call(name, json!({"document_id":id,"sheet":"other"}))["count"], 1);
    assert_eq!(t.err(name, json!({"document_id":id,"sheet":"Missing"})), "no_such_sheet");
}

#[test]
fn xlsx_formula_trace_follows_cross_sheet_ranges_and_pages_dependents() {
    let mut t = T::new("shared_formula_trace", &[]);
    let bytes = ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap();
    std::fs::write(t.dir.join("blank.xlsx"), bytes).unwrap();
    let id = t.open("blank.xlsx");
    let data = t.call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].as_str().unwrap().to_owned();
    t.call("xlsx_add_sheet", json!({"document_id":id,"name":"Report"}));
    t.call("xlsx_set_cells", json!({"document_id":id,"sheet":data,"range":"A1:A3","values":[[1],[2],[3]]}));
    t.call("xlsx_set_cells", json!({"document_id":id,"sheet":data,"range":"B1","values":"=A2*2"}));
    t.call("xlsx_set_cells", json!({"document_id":id,"sheet":"Report","range":"A1:B1","values":[[format!("=SUM({}!$A$1:$A$3)+B1", ibis_formats::quote_sheet(&data)), 8]]}));
    let tool = "xlsx_trace_formula";
    let read = t.call(tool, json!({"document_id":id,"sheet":"Report","cell":"$A$1","direction":"precedents"}));
    assert_eq!(read["cell"], "A1");
    assert_eq!(read["references"], json!([{"sheet":data,"range":"A1:A3"},{"sheet":"Report","range":"B1"}]));
    let page = t.call(tool, json!({"document_id":id,"sheet":data,"cell":"A2","direction":"dependents","limit":1}));
    assert_eq!(page["count"], 2);
    assert_eq!(page["references"], json!([{"sheet":data,"range":"B1"}]));
    let next = t.call(tool, json!({"document_id":id,"sheet":data,"cell":"A2","direction":"dependents","offset":page["next_offset"]}));
    assert_eq!(next["references"], json!([{"sheet":"Report","range":"A1"}]));
    assert!(next.get("next_offset").is_none());
    assert_eq!(t.err(tool, json!({"document_id":id,"sheet":data,"cell":"A1:A2","direction":"precedents"})), "invalid_argument");
}

#[test]
fn xlsx_formula_trace_in_a_draft_resolves_names_and_marks_volatile_formulas() {
    use ibis_doc_model::{Command, Document, SheetCommand};
    use ibis_formats::Kind;

    let base = ibis_formats::read(&ibis_formats::blank_file(Kind::Workbook).unwrap()).unwrap();
    let active = base.slides[0].id;
    let sheet = base.slides[0].name.clone().unwrap();
    let mut draft = Document::new(base);
    draft.execute(vec![Command::Sheet(SheetCommand::SetName {
        name: "Totals".into(),
        sheet: None,
        formula: Some(format!("{}!$B$2:$B$3", ibis_formats::quote_sheet(&sheet))),
    })]).unwrap();
    let context = || ibis_doc_service::DraftToolContext {
        workbook: ibis_doc_service::WorkbookToolContext { active_sheet: Some(active), ..Default::default() },
        ..Default::default()
    };
    let run = ibis_doc_service::run_draft_tool;
    run(Kind::Workbook, &mut draft, context(), "xlsx_set_cells",
        json!({"range":"A1:A2","values":[["=SUM(Totals)"],["=INDIRECT(\"B2\")"]]})).unwrap();
    let (named, edits) = run(Kind::Workbook, &mut draft, context(), "xlsx_trace_formula",
        json!({"cell":"A1","direction":"precedents"})).unwrap();
    assert!(edits.is_empty());
    assert_eq!(named["references"], json!([{"sheet":sheet,"range":"B2:B3"}]));
    let (dynamic, _) = run(Kind::Workbook, &mut draft, context(), "xlsx_trace_formula",
        json!({"cell":"A2","direction":"precedents"})).unwrap();
    assert_eq!(dynamic["volatile"], true);
    assert_eq!(dynamic["references"], json!([]));
}

#[test]
fn xlsx_sheet_rules_filter_and_page_in_file_and_draft() {
    use ibis_doc_model::sheet::{CellRange, CfKind, CfRule, ConditionalFormat, Validation};
    use ibis_doc_model::{Command, Document, SheetCommand};
    use ibis_formats::{Kind, SaveOptions};

    let opened = ibis_formats::open(ibis_formats::blank_file(Kind::Workbook).unwrap()).unwrap();
    let active = opened.presentation.slides[0].id;
    let sheet = opened.presentation.slides[0].name.clone().unwrap();
    let mut draft = Document::new(opened.presentation);
    let column = |c| CellRange { first_row: 0, first_column: c, last_row: 3, last_column: c };
    let ranges = vec![column(0), column(2)];
    draft.execute(vec![
        Command::Sheet(SheetCommand::SetValidation {
            sheet: active,
            ranges: ranges.clone(),
            rule: Some(Validation {
                ranges,
                kind: "list".into(),
                formula1: Some("\"Open,Closed\"".into()),
                dropdown: true,
                ..Default::default()
            }),
        }),
        Command::Sheet(SheetCommand::AddConditional {
            sheet: active,
            format: ConditionalFormat {
                ranges: vec![column(2)],
                rules: vec![CfRule {
                    kind: CfKind::Expression("C1>0".into()),
                    priority: 1,
                    stop: true,
                    style: None,
                }],
                ..Default::default()
            },
        }),
    ]).unwrap();

    let tool = "xlsx_list_sheet_rules";
    let context = ibis_doc_service::DraftToolContext {
        workbook: ibis_doc_service::WorkbookToolContext { active_sheet: Some(active), ..Default::default() },
        ..Default::default()
    };
    let (page, edits) = ibis_doc_service::run_draft_tool(
        Kind::Workbook, &mut draft, context, tool, json!({"range":"C4:D5","limit":1}),
    ).unwrap();
    assert!(edits.is_empty());
    assert_eq!(page["sheet"], sheet);
    assert_eq!(page["count"], 2);
    assert_eq!(page["rules"][0]["ranges"], json!(["A1:A4", "C1:C4"]));
    assert_eq!(page["rules"][0]["formula1"], "\"Open,Closed\"");
    assert_eq!(page["next_offset"], 1);

    let mut t = T::new("xlsx_sheet_rules", &[]);
    let bytes = opened.source.save(draft.presentation(), SaveOptions::default()).unwrap();
    std::fs::write(t.dir.join("rules.xlsx"), bytes).unwrap();
    let id = t.open("rules.xlsx");
    let next = t.call(tool, json!({"document_id":id,"sheet":sheet,"range":"C4:D5","offset":page["next_offset"]}));
    assert_eq!(next["count"], 2);
    assert_eq!(next["rules"][0]["condition"], json!({"type":"expression","formula":"C1>0"}));
    assert_eq!(next["rules"][0]["stop_if_true"], true);
    assert!(next.get("next_offset").is_none());
    let empty = t.call(tool, json!({"document_id":id,"sheet":sheet,"range":"A1","kind":"conditional_formatting"}));
    assert_eq!(empty["count"], 0);
    assert_eq!(empty["rules"], json!([]));
    assert_eq!(t.err(tool, json!({"document_id":id,"sheet":sheet,"kind":"unknown"})), "invalid_argument");
}
