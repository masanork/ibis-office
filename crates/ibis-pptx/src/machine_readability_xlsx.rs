// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Conservative XLSX layout input from original workbook bytes. Formula caches
//! are read from worksheet XML, not from the editor's recalculated values.
use std::collections::{BTreeMap, BTreeSet};
use ibis_doc_model::machine_readability_xlsx::{prepare_xlsx_layout, XlsxLayoutError, XlsxLayoutInput, XlsxReadWindow};
use ibis_doc_model::sheet::{CellError, CellValue};
use ibis_doc_model::sheet_calc::sheet_of_slide;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XlsxInputError { Workbook(String), SheetUnavailable, InvalidFormulaCache, UnsupportedFormulaCache, FormulaModelMismatch, Layout(XlsxLayoutError) }

/// Read one worksheet from the original bytes. The caller supplies the exact
/// reader window; this function does not yet infer Harunobu's dimension limits.
/// Date-formatted or otherwise non-General numeric values are rejected until
/// the source reader supplies authoritative date/number typing.
pub fn read_xlsx_layout(bytes: &[u8], sheet_index: usize, window: XlsxReadWindow) -> Result<XlsxLayoutInput, XlsxInputError> {
    let (presentation, sources) = crate::reader::read_xlsx_with_sources(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let slide = presentation.slides.get(sheet_index).ok_or(XlsxInputError::SheetUnavailable)?;
    let sheet = sheet_of_slide(slide).ok_or(XlsxInputError::SheetUnavailable)?;
    let part = sources.sheet_parts.get(sheet_index).and_then(Option::as_deref).ok_or(XlsxInputError::SheetUnavailable)?;
    let package = crate::package::Package::new(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let xml = package.read_xml(part).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let doc = roxmltree::Document::parse(&xml).map_err(|_| XlsxInputError::InvalidFormulaCache)?;
    let root = doc.root_element();
    let mut caches = BTreeMap::<(u32, u32), CellValue>::new();
    if let Some(data) = root.children().find(|n| n.is_element() && n.tag_name().name() == "sheetData") {
        for row in data.children().filter(|n| n.is_element() && n.tag_name().name() == "row") {
            for cell in row.children().filter(|n| n.is_element() && n.tag_name().name() == "c") {
                if !cell.children().any(|n| n.is_element() && n.tag_name().name() == "f") { continue; }
                let address = cell.attribute("r").ok_or(XlsxInputError::InvalidFormulaCache)?;
                let letters = address.bytes().take_while(|c| c.is_ascii_uppercase()).count();
                if !(1..=3).contains(&letters) { return Err(XlsxInputError::InvalidFormulaCache); }
                let (column, row) = crate::xlsx::cell_ref(address).ok_or(XlsxInputError::InvalidFormulaCache)?;
                if row >= 1_048_576 || column >= 16_384 { return Err(XlsxInputError::InvalidFormulaCache); }
                let raw = cell.children().find(|n| n.is_element() && n.tag_name().name() == "v").and_then(|v| v.text()).unwrap_or("");
                let value = if raw.is_empty() { CellValue::Empty } else { match cell.attribute("t").unwrap_or("n") {
                    "n" => CellValue::Number(raw.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or(XlsxInputError::InvalidFormulaCache)?),
                    "b" => match raw { "0" => CellValue::Bool(false), "1" => CellValue::Bool(true), _ => return Err(XlsxInputError::InvalidFormulaCache) },
                    "str" => CellValue::Text(raw.to_string()),
                    "e" => CellValue::Error(CellError::parse(raw).ok_or(XlsxInputError::UnsupportedFormulaCache)?),
                    _ => return Err(XlsxInputError::UnsupportedFormulaCache),
                }};
                if caches.insert((row, column), value).is_some() { return Err(XlsxInputError::InvalidFormulaCache); }
            }
        }
    }
    let formula_positions: BTreeSet<_> = sheet.cells.iter().filter(|c| c.formula.is_some()).map(|c| (c.row, c.column)).collect();
    for &(row, column) in caches.keys() {
        let within = match window { XlsxReadWindow::Empty => false, XlsxReadWindow::Through { last_row, last_column } => row <= last_row && column <= last_column };
        if within && !formula_positions.contains(&(row, column)) { return Err(XlsxInputError::FormulaModelMismatch); }
    }
    prepare_xlsx_layout(sheet, window, &caches).map_err(XlsxInputError::Layout)
}
