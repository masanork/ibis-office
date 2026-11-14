// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Conservative XLSX layout input from original workbook bytes. Formula caches
//! are read from worksheet XML, not from the editor's recalculated values.
use std::collections::{BTreeMap, BTreeSet};
use ibis_doc_model::machine_readability_xlsx::{prepare_xlsx_layout_for_mode, XlsxLayoutError, XlsxLayoutInput, XlsxReaderMode, XlsxReadWindow};
use ibis_doc_model::sheet::{CellError, CellKind, CellValue, Sheet};
use ibis_doc_model::sheet_calc::sheet_of_slide;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XlsxInputError { Workbook(String), SheetUnavailable, InvalidFormulaCache, UnsupportedFormulaCache, FormulaModelMismatch, InvalidWindowMetadata, FormulaInspectionLimit, Layout(XlsxLayoutError) }

/// Read one worksheet from the original bytes. The caller supplies the exact
/// reader window; this function does not yet infer Harunobu's dimension limits.
/// Out-of-range date and duration serials are rejected rather than guessed.
pub fn read_xlsx_layout(bytes: &[u8], sheet_index: usize, window: XlsxReadWindow) -> Result<XlsxLayoutInput, XlsxInputError> {
    read_xlsx_layout_impl(bytes, sheet_index, Some(window), false).map(|(_, input, _)| input)
}

/// Derive the default Harunobu reader window from worksheet XML, then prepare
/// layout input using the matching single-pass or large-file two-pass format
/// and blank-cell semantics.
pub fn read_xlsx_layout_auto(bytes: &[u8], sheet_index: usize) -> Result<(XlsxReadWindow, XlsxLayoutInput), XlsxInputError> {
    read_xlsx_layout_impl(bytes, sheet_index, None, false).map(|(window, input, _)| (window, input))
}

/// Original saved values and formulas, plus the reference reader window and
/// layout projection. Cells are restricted to the reader window; formula values
/// come from the XLSX cache. `text` remains display metadata and must not be
/// used as an authoritative saved value.
#[derive(Clone, Debug)]
pub struct XlsxSavedSheet { pub window: XlsxReadWindow, pub layout: XlsxLayoutInput, pub sheet: Sheet }

pub fn read_xlsx_saved_sheet(bytes: &[u8], sheet_index: usize) -> Result<XlsxSavedSheet, XlsxInputError> {
    let (window, layout, sheet) = read_xlsx_layout_impl(bytes, sheet_index, None, true)?;
    Ok(XlsxSavedSheet { window, layout, sheet: sheet.expect("saved-sheet input requested") })
}

fn read_xlsx_layout_impl(bytes: &[u8], sheet_index: usize, window: Option<XlsxReadWindow>, include_sheet: bool) -> Result<(XlsxReadWindow, XlsxLayoutInput, Option<Sheet>), XlsxInputError> {
    let mode = if bytes.len() > 5 * 1024 * 1024 { XlsxReaderMode::TwoPass } else { XlsxReaderMode::SinglePass };
    let (presentation, sources) = crate::reader::read_xlsx_with_sources(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let slide = presentation.slides.get(sheet_index).ok_or(XlsxInputError::SheetUnavailable)?;
    let sheet = sheet_of_slide(slide).ok_or(XlsxInputError::SheetUnavailable)?;
    let part = sources.sheet_parts.get(sheet_index).and_then(Option::as_deref).ok_or(XlsxInputError::SheetUnavailable)?;
    let package = crate::package::Package::new(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let xml = package.read_xml(part).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let doc = roxmltree::Document::parse(&xml).map_err(|_| XlsxInputError::InvalidFormulaCache)?;
    let root = doc.root_element();
    let shared_empty = if let Some(part) = sources.shared_strings.as_deref() {
        let text = package.read_xml(part).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
        let shared = roxmltree::Document::parse(&text).map_err(|_| XlsxInputError::InvalidWindowMetadata)?;
        Some(shared.root_element().children().filter(|n| n.is_element() && n.tag_name().name() == "si").map(|si| {
            !si.children().any(|n| n.is_element() && ((n.tag_name().name() == "t" && n.text().is_some_and(|t| !t.is_empty())) || (n.tag_name().name() == "r" && n.children().any(|t| t.is_element() && t.tag_name().name() == "t" && t.text().is_some_and(|s| !s.is_empty())))))
        }).collect::<Vec<_>>())
    } else { None };
    let derived = derive_window(root, &sheet.merges, shared_empty.as_deref(), mode)?;
    let window = window.unwrap_or(derived);
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
    let input = prepare_xlsx_layout_for_mode(sheet, window, &caches, mode).map_err(XlsxInputError::Layout)?;
    let saved = if include_sheet {
        let mut saved = (**sheet).clone();
        saved.cells.retain(|cell| match window { XlsxReadWindow::Empty => false, XlsxReadWindow::Through { last_row, last_column } => cell.row <= last_row && cell.column <= last_column });
        for cell in &mut saved.cells {
            if cell.formula.is_some() {
                cell.value = caches.get(&(cell.row, cell.column)).ok_or(XlsxInputError::FormulaModelMismatch)?.clone();
                cell.kind = CellKind::of(&cell.value);
            }
        }
        Some(saved)
    } else { None };
    Ok((window, input, saved))
}

fn source_cell_position(address: &str) -> Result<(u32, u32), XlsxInputError> {
    let letters = address.bytes().take_while(|c| c.is_ascii_uppercase()).count();
    if !(1..=3).contains(&letters) { return Err(XlsxInputError::InvalidWindowMetadata); }
    let (column, row) = crate::xlsx::cell_ref(address).ok_or(XlsxInputError::InvalidWindowMetadata)?;
    if row >= 1_048_576 || column >= 16_384 { return Err(XlsxInputError::InvalidWindowMetadata); }
    Ok((row, column))
}

/// Reproduce the reader's max-row/max-column limits and, for the single-pass
/// path only, its wide-sheet sample. Coordinates and merge ranges come from original XML;
/// formula occupancy comes from saved cached values, not recalculation.
fn derive_window(root: roxmltree::Node<'_, '_>, merges: &[ibis_doc_model::sheet::CellRange], shared_empty: Option<&[bool]>, mode: XlsxReaderMode) -> Result<XlsxReadWindow, XlsxInputError> {
    let mut max_row = 0u32; // openpyxl reports 1x1 even for an empty sheet
    let mut max_column = 0u32;
    let mut sample_column = 0u32;
    let mut seen = BTreeSet::new();
    for merge in merges {
        if merge.first_row > merge.last_row || merge.first_column > merge.last_column || merge.last_row >= 1_048_576 || merge.last_column >= 16_384 { return Err(XlsxInputError::InvalidWindowMetadata); }
        max_row = max_row.max(merge.last_row);
        max_column = max_column.max(merge.last_column);
    }
    if let Some(data) = root.children().find(|n| n.is_element() && n.tag_name().name() == "sheetData") {
        for row_node in data.children().filter(|n| n.is_element() && n.tag_name().name() == "row") {
            for cell in row_node.children().filter(|n| n.is_element() && n.tag_name().name() == "c") {
                let (row, column) = source_cell_position(cell.attribute("r").ok_or(XlsxInputError::InvalidWindowMetadata)?)?;
                if !seen.insert((row, column)) { return Err(XlsxInputError::InvalidWindowMetadata); }
                max_row = max_row.max(row);
                max_column = max_column.max(column);
                let kind = cell.attribute("t").unwrap_or("n");
                let value = cell.children().find(|n| n.is_element() && n.tag_name().name() == "v").and_then(|v| v.text()).unwrap_or("");
                if kind == "s" && !value.is_empty() {
                    let index = value.parse::<usize>().map_err(|_| XlsxInputError::InvalidWindowMetadata)?;
                    if *shared_empty.and_then(|s| s.get(index)).ok_or(XlsxInputError::InvalidWindowMetadata)? { return Err(XlsxInputError::InvalidWindowMetadata); }
                }
                if kind == "e" && !value.is_empty() && !CellError::parse(value).is_some_and(|e| e.as_str() == value) { return Err(XlsxInputError::InvalidWindowMetadata); }
                if !matches!(kind, "n" | "b" | "e" | "str" | "s" | "inlineStr") { return Err(XlsxInputError::InvalidWindowMetadata); }
                if row >= 100 { continue; }
                // Non-master cells in merged ranges are replaced by empty
                // MergedCell objects by openpyxl before sampling.
                if merges.iter().any(|m| row >= m.first_row && row <= m.last_row && column >= m.first_column && column <= m.last_column && (row != m.first_row || column != m.first_column)) { continue; }
                let occupied = match kind {
                    "n" | "b" | "e" | "str" | "s" => !value.is_empty(),
                    "inlineStr" => cell.children().find(|n| n.is_element() && n.tag_name().name() == "is").is_some_and(|is| {
                        is.children().any(|n| n.is_element() && ((n.tag_name().name() == "t" && n.text().is_some_and(|t| !t.is_empty())) || (n.tag_name().name() == "r" && n.children().any(|t| t.is_element() && t.tag_name().name() == "t" && t.text().is_some_and(|s| !s.is_empty())))))
                    }),
                    // Other cell representations need reader-specific semantics.
                    _ => return Err(XlsxInputError::InvalidWindowMetadata),
                };
                if occupied { sample_column = sample_column.max(column); }
            }
        }
    }
    let last_row = max_row.min(9_999);
    let last_column = if mode == XlsxReaderMode::SinglePass && max_column >= 255 { sample_column.saturating_add(5).min(max_column).min(499) } else { max_column.min(499) };
    Ok(XlsxReadWindow::Through { last_row, last_column })
}

/// Formula references from the original package, in worksheet XML order.
/// Both lists cover every worksheet or this function returns an error; this
/// matters because the reference skips L2-06 when package inspection fails.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XlsxFormulaArtifacts {
    pub formulas: Vec<ibis_doc_model::machine_readability_rules::FormulaArtifact>,
    pub errors: Vec<ibis_doc_model::machine_readability_rules::FormulaArtifact>,
}

pub fn inspect_xlsx_formula_artifacts(bytes: &[u8]) -> Result<Vec<XlsxFormulaArtifacts>, XlsxInputError> {
    use ibis_doc_model::machine_readability_rules::FormulaArtifact;
    const MAX_REFS: usize = 50_000;
    const MAX_XML_BYTES: usize = 100 * 1024 * 1024;
    let (_, sources) = crate::reader::read_xlsx_with_sources(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let package = crate::package::Package::new(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let mut output = Vec::with_capacity(sources.sheet_parts.len());
    let (mut formula_count, mut error_count) = (0usize, 0usize);
    for part in &sources.sheet_parts {
        let part = part.as_deref().ok_or(XlsxInputError::SheetUnavailable)?;
        let xml = package.read_xml(part).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
        if xml.len() > MAX_XML_BYTES { return Err(XlsxInputError::FormulaInspectionLimit); }
        let doc = roxmltree::Document::parse(&xml).map_err(|_| XlsxInputError::InvalidFormulaCache)?;
        let mut sheet = XlsxFormulaArtifacts::default();
        for cell in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "c") {
            let Some(cell_ref) = cell.attribute("r") else { continue; };
            let formula = cell.children().any(|n| n.is_element() && n.tag_name().name() == "f");
            let value = cell.children().find(|n| n.is_element() && n.tag_name().name() == "v").and_then(|n| n.text());
            if !formula && !(cell.attribute("t") == Some("e") && value == Some("#REF!")) { continue; }
            let (row, column) = source_cell_position(cell_ref)?;
            let artifact = FormulaArtifact { row, column, cell_ref: cell_ref.to_string() };
            if formula { formula_count += 1; sheet.formulas.push(artifact.clone()); }
            if value == Some("#REF!") && (formula || cell.attribute("t") == Some("e")) { error_count += 1; sheet.errors.push(artifact); }
            if formula_count > MAX_REFS || error_count > MAX_REFS { return Err(XlsxInputError::FormulaInspectionLimit); }
        }
        output.push(sheet);
    }
    Ok(output)
}

/// Experimental Excel-only checks for detected worksheet tables. Results have
/// no score and do not cover the common or report rules; they are not a full
/// machine-readability analysis.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialXlsxStructure {
    pub table: ibis_doc_model::machine_readability_rules::DetectedTable,
    pub header_rows: Vec<u32>,
    pub checks: Vec<ibis_doc_model::machine_readability_rules::RuleEvaluation>,
    pub complete: bool,
}

pub fn analyze_xlsx_structure_partial(bytes: &[u8], sheet_index: usize) -> Result<Vec<PartialXlsxStructure>, XlsxInputError> {
    use ibis_doc_model::machine_readability_layout::{merge_column_adjacent_regions, merge_form_fragments, preliminary_islands};
    use ibis_doc_model::machine_readability_rules::{file_format, formula_artifacts, hidden_rows_columns, irrelevant_information, merged_cells, one_table_per_sheet, xlsx_data_fragmentation, DetectedTable, OutsideTableCell, UsedExtent, WorkbookFormat};
    use ibis_doc_model::sheet::CellRange;
    let saved = read_xlsx_saved_sheet(bytes, sheet_index)?;
    // Reference skips only L2-06 if package-wide formula inspection fails.
    let artifacts = inspect_xlsx_formula_artifacts(bytes).ok().and_then(|v| v.get(sheet_index).cloned());
    let preliminary = preliminary_islands(&saved.layout.cells, &saved.layout.merges).ok_or(XlsxInputError::InvalidWindowMetadata)?;
    let columns = merge_column_adjacent_regions(&preliminary.candidates, &saved.layout.cells, &saved.layout.merges).ok_or(XlsxInputError::InvalidWindowMetadata)?;
    let mut regions = merge_form_fragments(&columns, &saved.layout.cells, &saved.layout.merges).ok_or(XlsxInputError::InvalidWindowMetadata)?;
    regions.sort_by(|a, b| b.candidate.confidence.total_cmp(&a.candidate.confidence));
    let tables: Vec<_> = regions.iter().map(|r| DetectedTable { range: r.candidate.range, confidence: r.candidate.confidence }).collect();
    let others: Vec<_> = preliminary.others.iter().map(|o| OutsideTableCell { row: o.row, column: o.column }).collect();
    let used = match saved.window { XlsxReadWindow::Empty => UsedExtent::Empty, XlsxReadWindow::Through { last_row, last_column } => UsedExtent::Range(CellRange { first_row: 0, first_column: 0, last_row, last_column }) };
    regions.iter().map(|region| {
        let range = region.candidate.range;
        let header_rows: BTreeSet<_> = region.headers.rows.iter().copied().collect();
        let checks = vec![file_format(WorkbookFormat::Xlsx), one_table_per_sheet(range, Some(&tables)), xlsx_data_fragmentation(&saved.sheet, range, &tables, bytes.len() > 5 * 1024 * 1024).ok_or(XlsxInputError::InvalidWindowMetadata)?, irrelevant_information(Some(&others)), merged_cells(&saved.sheet, range, Some(&header_rows)), hidden_rows_columns(&saved.sheet, used), formula_artifacts(range, artifacts.as_ref().map(|a| (a.formulas.as_slice(), a.errors.as_slice())))];
        Ok(PartialXlsxStructure { table: DetectedTable { range, confidence: region.candidate.confidence }, header_rows: region.headers.rows.clone(), checks, complete: false })
    }).collect()
}
