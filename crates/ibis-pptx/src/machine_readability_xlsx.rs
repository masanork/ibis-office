// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Conservative XLSX layout input from original workbook bytes. Formula caches
//! are read from worksheet XML, not from the editor's recalculated values.
use std::collections::{BTreeMap, BTreeSet};
use ibis_doc_model::machine_readability_xlsx::{prepare_xlsx_layout_for_mode, XlsxLayoutError, XlsxLayoutInput, XlsxReaderMode, XlsxReadWindow};
use ibis_doc_model::sheet::{CellError, CellKind, CellValue, Sheet};
use ibis_doc_model::sheet_calc::sheet_of_slide;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XlsxInputError { Workbook(String), SheetUnavailable, InvalidFormulaCache, UnsupportedFormulaCache, FormulaModelMismatch, InvalidWindowMetadata, FormulaInspectionLimit, UnsupportedReferenceValue, Layout(XlsxLayoutError) }

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XlsxNumericLexeme { Integer(String), Float(String) }

impl XlsxNumericLexeme {
    /// Reference-compatible `str(value)` for supported finite numeric XML.
    /// Unknown or invalid spellings are rejected rather than guessed.
    pub fn reference_text(&self) -> Option<String> {
        match self {
            Self::Integer(raw) => {
                let raw = raw.trim();
                let (negative, digits) = if let Some(v) = raw.strip_prefix('-') { (true, v) } else { (false, raw.strip_prefix('+').unwrap_or(raw)) };
                if digits.is_empty() || !digits.bytes().all(|v| v.is_ascii_digit()) { return None; }
                let value = digits.trim_start_matches('0');
                if value.is_empty() { Some("0".into()) } else if negative { Some(format!("-{value}")) } else { Some(value.to_string()) }
            }
            Self::Float(raw) => {
                let value: f64 = raw.parse().ok()?;
                if !value.is_finite() { return None; }
                let abs = value.abs();
                if value != 0.0 && (abs < 0.0001 || abs >= 1e16) {
                    let repr = format!("{value:e}");
                    let (mantissa, exponent) = repr.split_once('e')?;
                    let exponent: i32 = exponent.parse().ok()?;
                    Some(format!("{mantissa}e{}{:02}", if exponent < 0 { '-' } else { '+' }, exponent.unsigned_abs()))
                } else {
                    let mut repr = value.to_string();
                    if !repr.contains('.') { repr.push_str(".0"); }
                    Some(repr)
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct XlsxSavedSheet {
    pub window: XlsxReadWindow,
    pub layout: XlsxLayoutInput,
    pub sheet: Sheet,
    /// Raw numeric XML values preserve distinctions such as `1` versus `1.0`
    /// and integer precision that the editor's floating-point model cannot.
    pub numeric_lexemes: BTreeMap<(u32, u32), XlsxNumericLexeme>,
    pub date1904: bool,
}

pub fn read_xlsx_saved_sheet(bytes: &[u8], sheet_index: usize) -> Result<XlsxSavedSheet, XlsxInputError> {
    let (window, layout, saved) = read_xlsx_layout_impl(bytes, sheet_index, None, true)?;
    let (sheet, numeric_lexemes, date1904) = saved.expect("saved-sheet input requested");
    Ok(XlsxSavedSheet { window, layout, sheet, numeric_lexemes, date1904 })
}

fn read_xlsx_layout_impl(bytes: &[u8], sheet_index: usize, window: Option<XlsxReadWindow>, include_sheet: bool) -> Result<(XlsxReadWindow, XlsxLayoutInput, Option<(Sheet, BTreeMap<(u32, u32), XlsxNumericLexeme>, bool)>), XlsxInputError> {
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
        let mut lexemes = BTreeMap::new();
        if let Some(data) = root.children().find(|n| n.is_element() && n.tag_name().name() == "sheetData") {
            for cell in data.descendants().filter(|n| n.is_element() && n.tag_name().name() == "c") {
                if cell.attribute("t").unwrap_or("n") != "n" { continue; }
                let Some(raw) = cell.children().find(|n| n.is_element() && n.tag_name().name() == "v").and_then(|v| v.text()).filter(|v| !v.is_empty()) else { continue; };
                let position = source_cell_position(cell.attribute("r").ok_or(XlsxInputError::InvalidWindowMetadata)?)?;
                let within = match window { XlsxReadWindow::Empty => false, XlsxReadWindow::Through { last_row, last_column } => position.0 <= last_row && position.1 <= last_column };
                if !within { continue; }
                let lexeme = if raw.contains(['.', 'e', 'E']) { XlsxNumericLexeme::Float(raw.to_string()) } else { XlsxNumericLexeme::Integer(raw.to_string()) };
                if lexemes.insert(position, lexeme).is_some() { return Err(XlsxInputError::InvalidWindowMetadata); }
            }
        }
        let date1904 = presentation.workbook.as_deref().is_some_and(|w| w.date1904);
        Some((saved, lexemes, date1904))
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
            let formula_node = cell.children().find(|n| n.is_element() && n.tag_name().name() == "f");
            let formula = formula_node.is_some();
            let value = cell.children().find(|n| n.is_element() && n.tag_name().name() == "v").and_then(|n| n.text());
            if !formula && !(cell.attribute("t") == Some("e") && value == Some("#REF!")) { continue; }
            let (row, column) = source_cell_position(cell_ref)?;
            let artifact = FormulaArtifact { row, column, cell_ref: cell_ref.to_string(),
                formula_text: formula_node.and_then(|node| node.text()).map(str::to_string),
                formula_type: formula_node.and_then(|node| node.attribute("t")).map(str::to_string) };
            if formula { formula_count += 1; sheet.formulas.push(artifact.clone()); }
            if value == Some("#REF!") && (formula || cell.attribute("t") == Some("e")) { error_count += 1; sheet.errors.push(artifact); }
            if formula_count > MAX_REFS || error_count > MAX_REFS { return Err(XlsxInputError::FormulaInspectionLimit); }
        }
        output.push(sheet);
    }
    Ok(output)
}

/// Experimental structural checks for detected worksheet tables. Results have
/// no score and do not cover the common or report rules; they are not a full
/// machine-readability analysis.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialXlsxStructure {
    pub table: ibis_doc_model::machine_readability_rules::DetectedTable,
    pub header_rows: Vec<u32>,
    /// None means the reader could not faithfully stringify a source value.
    pub column_headers: Option<Vec<ibis_doc_model::machine_readability_layout::InferredColumnHeader>>,
    pub checks: Vec<ibis_doc_model::machine_readability_rules::RuleEvaluation>,
    /// Rules omitted because this adapter cannot represent a source value.
    pub unavailable: Vec<&'static str>,
    pub complete: bool,
}

pub fn analyze_xlsx_structure_partial(bytes: &[u8], sheet_index: usize) -> Result<Vec<PartialXlsxStructure>, XlsxInputError> {
    use ibis_doc_model::machine_readability_layout::{estimate_stub_columns, generate_column_headers, infer_column_schemas, inherit_headers_from_above, merge_column_adjacent_regions, merge_form_fragments, preliminary_islands, HeaderCell, InferredColumnType, LayoutValue, ResolvedIsland};
    use ibis_doc_model::machine_readability_rules::{blank_vs_zero_ai_disabled, csv_choice_standardization, csv_item_abbreviations, csv_other_details, csv_time_axis, file_format, format_based_semantics, formula_artifacts, header_start_position, hidden_rows_columns, irrelevant_information, merged_cells, object_artifacts, one_data_per_cell, one_table_per_sheet, platform_characters, reference_skipped_level3, whitespace_formatting, xlsx_column_headers, xlsx_data_fragmentation, xlsx_data_units, xlsx_numeric_purity, xlsx_region_codes, xlsx_unique_headers, xlsx_vertical_format, ColumnType, DetectedTable, OutsideTableCell, TextEncoding, UsedExtent, WorkbookFormat};
    use ibis_doc_model::sheet::CellRange;
    let saved = read_xlsx_saved_sheet(bytes, sheet_index)?;
    let text_view = xlsx_reference_text_sheet(&saved).ok();
    // Reference skips only L2-06 if package-wide formula inspection fails.
    let artifacts = inspect_xlsx_formula_artifacts(bytes).ok().and_then(|v| v.get(sheet_index).cloned());
    let objects = inspect_xlsx_object_artifacts(bytes).ok().and_then(|v| v.get(sheet_index).cloned());
    let formats = inspect_xlsx_cell_formats(bytes, sheet_index).ok();
    let stub_cells = formats.as_ref().and_then(|f| xlsx_stub_cells(&saved, f, bytes.len() > 5 * 1024 * 1024).ok());
    let preliminary = preliminary_islands(&saved.layout.cells, &saved.layout.merges).ok_or(XlsxInputError::InvalidWindowMetadata)?;
    let columns = merge_column_adjacent_regions(&preliminary.candidates, &saved.layout.cells, &saved.layout.merges).ok_or(XlsxInputError::InvalidWindowMetadata)?;
    let forms = merge_form_fragments(&columns, &saved.layout.cells, &saved.layout.merges).ok_or(XlsxInputError::InvalidWindowMetadata)?;
    let text_cells: BTreeMap<_, _> = text_view.as_ref().map(|sheet| sheet.cells.iter().map(|c| ((c.row, c.column), &c.value)).collect()).unwrap_or_default();
    let header_cells: Vec<_> = saved.layout.cells.iter().map(|c| {
        let master = saved.layout.merges.iter().find(|m| c.row >= m.first_row && c.row <= m.last_row && c.column >= m.first_column && c.column <= m.last_column).map(|m| (m.first_row, m.first_column));
        let value = if text_view.is_some() { match text_cells.get(&(c.row, c.column)) { Some(CellValue::Text(v)) => Some(v.clone()), _ => None } } else { match &c.value { Some(LayoutValue::Text(v)) => Some(v.clone()), Some(_) => Some("0".to_string()), None => None } };
        HeaderCell { row: c.row, column: c.column, value, merged: c.merged, master }
    }).collect();
    let mut resolved = Vec::new();
    let mut unsupported_stubs = BTreeSet::new();
    for region in forms {
        let range = region.candidate.range;
        let indices: Vec<_> = (range.first_column..=range.last_column).collect();
        let headers = generate_column_headers(&region.headers.rows, &indices, &header_cells).ok_or(XlsxInputError::InvalidWindowMetadata)?;
        let labels: BTreeMap<_, _> = headers.iter().map(|h| (h.column, h.label.as_str())).collect();
        let schema_input: Vec<_> = indices.iter().map(|&c| (c, labels.get(&c).copied())).collect();
        let schemas = if region.headers.rows.is_empty() { Vec::new() } else { infer_column_schemas(&schema_input, region.headers.body_first, region.headers.body_last, &saved.layout.cells).ok_or(XlsxInputError::InvalidWindowMetadata)? };
        let key = (range.first_row, range.first_column, range.last_row, range.last_column);
        let stubs = if region.headers.rows.is_empty() { Vec::new() } else if let Some((cells, uncertain)) = &stub_cells {
            let checked = (u64::from(range.last_column) - u64::from(range.first_column)).min(3) as u32;
            if schemas.iter().any(|c| c.column >= range.first_column && c.column < range.first_column + checked && c.kind == InferredColumnType::Numeric && uncertain.iter().any(|&(row, col)| col == c.column && row >= region.headers.body_first && row <= region.headers.body_last)) { unsupported_stubs.insert(key); Vec::new() }
            else { estimate_stub_columns(range.first_column, range.last_column, region.headers.body_first, region.headers.body_last, cells, &schemas).ok_or(XlsxInputError::InvalidWindowMetadata)? }
        } else { unsupported_stubs.insert(key); Vec::new() };
        let abstain = region.candidate.confidence < 0.7;
        resolved.push(ResolvedIsland { candidate: region.candidate, headers: region.headers, column_headers: headers, column_schemas: schemas, stub_columns: stubs, footer_rows: Vec::new(), abstain });
    }
    let mut regions = inherit_headers_from_above(&resolved, &saved.layout.cells).ok_or(XlsxInputError::InvalidWindowMetadata)?;
    regions.sort_by(|a, b| b.candidate.confidence.total_cmp(&a.candidate.confidence));
    let tables: Vec<_> = regions.iter().map(|r| DetectedTable { range: r.candidate.range, confidence: r.candidate.confidence }).collect();
    let others: Vec<_> = preliminary.others.iter().map(|o| OutsideTableCell { row: o.row, column: o.column }).collect();
    let mut reference_sheet = saved.sheet.clone();
    for cell in &mut reference_sheet.cells {
        let dependent = bytes.len() <= 5 * 1024 * 1024 && saved.sheet.merges.iter().any(|m| cell.row >= m.first_row && cell.row <= m.last_row && cell.column >= m.first_column && cell.column <= m.last_column && (cell.row != m.first_row || cell.column != m.first_column));
        if dependent || matches!(&cell.value, CellValue::Text(value) if value.starts_with('=')) { cell.value = CellValue::Empty; cell.kind = CellKind::Empty; }
    }
    let used = match saved.window { XlsxReadWindow::Empty => UsedExtent::Empty, XlsxReadWindow::Through { last_row, last_column } => UsedExtent::Range(CellRange { first_row: 0, first_column: 0, last_row, last_column }) };
    regions.iter().map(|region| {
        let range = region.candidate.range;
        let header_rows: BTreeSet<_> = region.headers.rows.iter().copied().collect();
        let column_types: BTreeMap<_, _> = region.column_schemas.iter().map(|c| (c.column, if c.kind == InferredColumnType::Numeric { ColumnType::Numeric } else { ColumnType::Other })).collect();
        let numeric_schemas: Vec<_> = region.column_schemas.iter().map(|c| (c.column, c.kind)).collect();
        let typed_numeric: BTreeSet<_> = saved.layout.cells.iter().filter(|c| c.value == Some(LayoutValue::Numeric)).map(|c| (c.row, c.column)).collect();
        let falsy: BTreeSet<_> = saved.sheet.cells.iter().filter(|c| typed_numeric.contains(&(c.row, c.column)) && (matches!(&c.value, CellValue::Bool(false)) || matches!(&c.value, CellValue::Number(v) if *v == 0.0))).map(|c| (c.row, c.column)).collect();
        let mut unavailable = Vec::new();
        if objects.is_none() { unavailable.push("L1-10"); }
        if formats.is_none() { unavailable.push("L1-11"); }
        if artifacts.is_none() { unavailable.push("L2-06"); }
        let mut checks = vec![file_format(WorkbookFormat::Xlsx), one_table_per_sheet(range, Some(&tables)), xlsx_data_fragmentation(&saved.sheet, range, &tables, bytes.len() > 5 * 1024 * 1024).ok_or(XlsxInputError::InvalidWindowMetadata)?, irrelevant_information(Some(&others)), xlsx_column_headers(&saved.sheet, range, &region.headers.rows, region.headers.body_first, region.headers.body_last, bytes.len() > 5 * 1024 * 1024).ok_or(XlsxInputError::InvalidWindowMetadata)?, whitespace_formatting(&reference_sheet, range, Some(&region.headers.rows))];
        if let Some(text) = &text_view { checks.push(one_data_per_cell(text, range, Some(&column_types))); } else { unavailable.push("L1-07"); }
        checks.extend([platform_characters(&reference_sheet, range, TextEncoding::Xlsx), blank_vs_zero_ai_disabled(), object_artifacts(range, region.abstain, objects.as_deref()), format_based_semantics(&reference_sheet, range, region.headers.body_first, formats.as_ref()), merged_cells(&saved.sheet, range, Some(&header_rows)), hidden_rows_columns(&saved.sheet, used)]);
        if let Some(text) = &text_view {
            if let Some(r) = xlsx_numeric_purity(text, range, region.headers.body_first, region.headers.body_last, &numeric_schemas, &typed_numeric) { checks.push(r); } else { unavailable.push("L2-01"); }
            let key = (range.first_row, range.first_column, range.last_row, range.last_column);
            if !unsupported_stubs.contains(&key) {
                if let Some(r) = csv_item_abbreviations(text, &region.stub_columns, region.headers.body_first, region.headers.body_last) { checks.push(r); } else { unavailable.push("L2-02"); }
            } else { unavailable.push("L2-02"); }
            if let Some(r) = xlsx_unique_headers(text, range, &region.headers.rows, region.headers.body_first, region.headers.body_last, &saved.layout.merges) { checks.push(r); } else { unavailable.push("L2-03"); }
            if let Some(r) = csv_choice_standardization(text, range, region.headers.body_first, region.headers.body_last, &numeric_schemas) { checks.push(r); } else { unavailable.push("L2-04"); }
            if let Some(r) = csv_other_details(text, range, region.headers.body_first, region.headers.body_last, &numeric_schemas) { checks.push(r); } else { unavailable.push("L2-05"); }
        } else { unavailable.extend(["L2-01", "L2-02", "L2-03", "L2-04", "L2-05"]); }
        checks.push(formula_artifacts(range, artifacts.as_ref().map(|a| (a.formulas.as_slice(), a.errors.as_slice()))));
        checks.push(header_start_position(range, &region.headers.rows, region.headers.body_first).ok_or(XlsxInputError::InvalidWindowMetadata)?);
        let skipped = reference_skipped_level3(); checks.extend(skipped[..2].iter().cloned());
        if let Some(text) = &text_view {
            if let Some(r) = xlsx_data_units(text, range, &region.headers.rows, region.headers.body_first, region.headers.body_last, &typed_numeric) { checks.push(r); } else { unavailable.push("L3-04"); }
            if let Some(r) = csv_time_axis(text, range, &region.headers.rows, region.headers.body_first, region.headers.body_last) { checks.push(r); } else { unavailable.push("L3-05"); }
            if let Some(r) = xlsx_region_codes(text, range, &region.headers.rows, region.headers.body_first, region.headers.body_last, &falsy) { checks.push(r); } else { unavailable.push("L3-06"); }
        } else { unavailable.extend(["L3-04", "L3-05", "L3-06"]); }
        checks.extend(skipped[2..].iter().cloned());
        if let Some(text) = &text_view {
            if let Some(r) = xlsx_vertical_format(text, range, &region.headers.rows, &falsy) { checks.push(r); } else { unavailable.push("L3-09"); }
        } else { unavailable.push("L3-09"); }
        Ok(PartialXlsxStructure { table: DetectedTable { range, confidence: region.candidate.confidence }, header_rows: region.headers.rows.clone(), column_headers: text_view.as_ref().map(|_| region.column_headers.clone()), checks, unavailable, complete: false })
    }).collect()
}

/// Inspect object anchors in all worksheets; unsupported or malformed package
/// metadata returns an error so callers can skip the corresponding rule.
pub fn inspect_xlsx_object_artifacts(bytes: &[u8]) -> Result<Vec<Vec<ibis_doc_model::machine_readability_rules::ObjectArtifact>>, XlsxInputError> {
    use ibis_doc_model::machine_readability_rules::ObjectArtifact;
    use crate::zip::Archive;
    const MAX_OBJECTS: usize = 50_000;
    const MAX_XML: usize = 50 * 1024 * 1024;
    const DRAWING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
    let (_, sources) = crate::reader::read_xlsx_with_sources(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let archive = Archive::new(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let package = crate::package::Package::new(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let mut output = Vec::with_capacity(sources.sheet_parts.len());
    let mut total = 0usize;
    for part in &sources.sheet_parts {
        let part = part.as_deref().ok_or(XlsxInputError::SheetUnavailable)?;
        let relationships = package.ordered_relationships(part).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
        let drawings: Vec<_> = relationships.iter().map(|(_, r)| r).filter(|r| r.kind == DRAWING).collect();
        if drawings.iter().any(|r| r.external) { return Err(XlsxInputError::InvalidWindowMetadata); }
        let mut sheet = Vec::new();
        for drawing in drawings {
            if !archive.contains(&drawing.target) { continue; }
            let xml = package.read_xml(&drawing.target).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
            if xml.len() > MAX_XML { return Err(XlsxInputError::FormulaInspectionLimit); }
            let doc = roxmltree::Document::parse(&xml).map_err(|_| XlsxInputError::InvalidWindowMetadata)?;
            for anchor in doc.descendants().filter(|n| n.is_element() && matches!(n.tag_name().name(), "oneCellAnchor" | "twoCellAnchor")) {
                let kind = [("pic", "image", "nvPicPr"), ("graphicFrame", "chart", "nvGraphicFramePr"), ("sp", "shape", "nvSpPr"), ("cxnSp", "connector", "nvCxnSpPr")].into_iter().find_map(|(tag, kind, nv)| anchor.children().find(|n| n.is_element() && n.tag_name().name() == tag).map(|child| (kind, nv, child)));
                let Some((kind, nv, child)) = kind else { continue; };
                let Some(from) = anchor.children().find(|n| n.is_element() && n.tag_name().name() == "from") else { continue; };
                let row = from.children().find(|n| n.is_element() && n.tag_name().name() == "row").and_then(|n| n.text()).unwrap_or("0").parse::<u32>();
                let column = from.children().find(|n| n.is_element() && n.tag_name().name() == "col").and_then(|n| n.text()).unwrap_or("0").parse::<u32>();
                let (Ok(row), Ok(column)) = (row, column) else { continue; };
                if !from.children().any(|n| n.is_element() && n.tag_name().name() == "row") || !from.children().any(|n| n.is_element() && n.tag_name().name() == "col") { continue; }
                let name = child.children().find(|n| n.is_element() && n.tag_name().name() == nv).and_then(|n| n.children().find(|c| c.is_element() && c.tag_name().name() == "cNvPr")).and_then(|n| n.attribute("name")).filter(|s| !s.is_empty()).map(str::to_string);
                sheet.push(ObjectArtifact { row, column, kind: kind.to_string(), name, drawing_part: drawing.target.clone() });
                total += 1;
                if total > MAX_OBJECTS { return Err(XlsxInputError::FormulaInspectionLimit); }
            }
        }
        output.push(sheet);
    }
    Ok(output)
}

/// Read direct RGB font/fill metadata and bold/italic flags as the reference
/// reader does. Unknown style representations are rejected, not guessed.
pub fn inspect_xlsx_cell_formats(bytes: &[u8], sheet_index: usize) -> Result<BTreeMap<(u32, u32), ibis_doc_model::machine_readability_rules::ReferenceCellFormat>, XlsxInputError> {
    use ibis_doc_model::machine_readability_rules::ReferenceCellFormat;
    let (_, sources) = crate::reader::read_xlsx_with_sources(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let package = crate::package::Package::new(bytes).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let rgb = |color: roxmltree::Node<'_, '_>| -> Result<Option<String>, XlsxInputError> {
        let Some(value) = color.attribute("rgb") else { return Ok(None); };
        if !matches!(value.len(), 6 | 8) || !value.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(XlsxInputError::InvalidWindowMetadata); }
        Ok(Some(if value.len() == 6 { format!("00{value}") } else { value.to_string() }))
    };
    let flag = |parent: roxmltree::Node<'_, '_>, name: &str| -> Result<bool, XlsxInputError> {
        let Some(node) = parent.children().find(|n| n.is_element() && n.tag_name().name() == name) else { return Ok(false); };
        match node.attribute("val") { None | Some("1") | Some("true") | Some("on") => Ok(true), Some("0") | Some("false") | Some("off") => Ok(false), _ => Err(XlsxInputError::InvalidWindowMetadata) }
    };
    let mut styles = vec![ReferenceCellFormat::default()];
    if let Some(part) = sources.styles.as_deref() {
        let xml = package.read_xml(part).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
        let doc = roxmltree::Document::parse(&xml).map_err(|_| XlsxInputError::InvalidWindowMetadata)?;
        let root = doc.root_element();
        let fonts = root.children().find(|n| n.is_element() && n.tag_name().name() == "fonts").ok_or(XlsxInputError::InvalidWindowMetadata)?;
        let fills = root.children().find(|n| n.is_element() && n.tag_name().name() == "fills").ok_or(XlsxInputError::InvalidWindowMetadata)?;
        let xfs = root.children().find(|n| n.is_element() && n.tag_name().name() == "cellXfs").ok_or(XlsxInputError::InvalidWindowMetadata)?;
        let borders = root.children().find(|n| n.is_element() && n.tag_name().name() == "borders").ok_or(XlsxInputError::InvalidWindowMetadata)?;
        let borders: Vec<_> = borders.children().filter(|n| n.is_element() && n.tag_name().name() == "border").collect();
        let fonts: Vec<_> = fonts.children().filter(|n| n.is_element() && n.tag_name().name() == "font").collect();
        let fills: Vec<_> = fills.children().filter(|n| n.is_element() && n.tag_name().name() == "fill").collect();
        styles.clear();
        for xf in xfs.children().filter(|n| n.is_element() && n.tag_name().name() == "xf") {
            let font_id = xf.attribute("fontId").unwrap_or("0").parse::<usize>().map_err(|_| XlsxInputError::InvalidWindowMetadata)?;
            let fill_id = xf.attribute("fillId").unwrap_or("0").parse::<usize>().map_err(|_| XlsxInputError::InvalidWindowMetadata)?;
            let border_id = xf.attribute("borderId").unwrap_or("0").parse::<usize>().map_err(|_| XlsxInputError::InvalidWindowMetadata)?;
            let border = *borders.get(border_id).ok_or(XlsxInputError::InvalidWindowMetadata)?;
            let edge = |name: &str| border.children().find(|n| n.is_element() && n.tag_name().name() == name).and_then(|n| n.attribute("style")).is_some_and(|v| !v.is_empty());
            let font = *fonts.get(font_id).ok_or(XlsxInputError::InvalidWindowMetadata)?;
            let fill = *fills.get(fill_id).ok_or(XlsxInputError::InvalidWindowMetadata)?;
            let color = font.children().find(|n| n.is_element() && n.tag_name().name() == "color").map(&rgb).transpose()?.flatten().filter(|v| v != "00000000");
            let pattern = fill.children().find(|n| n.is_element() && n.tag_name().name() == "patternFill");
            let bg = if let Some(pattern) = pattern.filter(|n| n.attribute("patternType").is_some_and(|v| v != "none")) {
                match pattern.children().find(|n| n.is_element() && n.tag_name().name() == "fgColor") { Some(color) => rgb(color)?, None => Some("00000000".to_string()) }
            } else { None };
            let stub_background_nonwhite = bg.as_ref().is_some_and(|v| v != "FFFFFFFF");
            let bg = bg.map(|v| v[v.len()-6..].to_ascii_uppercase()).filter(|v| v != "FFFFFF");
            styles.push(ReferenceCellFormat { font_color: color, background: bg, stub_background_nonwhite, bold: flag(font, "b")?, italic: flag(font, "i")?, border_left: edge("left"), border_right: edge("right") });
        }
    }
    let part = sources.sheet_parts.get(sheet_index).and_then(Option::as_deref).ok_or(XlsxInputError::SheetUnavailable)?;
    let xml = package.read_xml(part).map_err(|e| XlsxInputError::Workbook(e.to_string()))?;
    let doc = roxmltree::Document::parse(&xml).map_err(|_| XlsxInputError::InvalidWindowMetadata)?;
    let read_only = bytes.len() > 5 * 1024 * 1024;
    let mut cells = BTreeMap::new();
    for cell in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "c") {
        let Some(reference) = cell.attribute("r") else { continue; };
        let position = source_cell_position(reference)?;
        let index = cell.attribute("s").unwrap_or("0").parse::<usize>().map_err(|_| XlsxInputError::InvalidWindowMetadata)?;
        let mut format = styles.get(index).cloned().ok_or(XlsxInputError::InvalidWindowMetadata)?;
        if read_only { format.font_color = None; format.bold = false; format.italic = false; format.border_left = false; format.border_right = false; }
        if cells.insert(position, format).is_some() { return Err(XlsxInputError::InvalidWindowMetadata); }
    }
    Ok(cells)
}

/// Build a text view for rules that stringify reference-reader cell values.
/// The conversion is bounded to the supported source value range; this is a
/// partial adapter, not a full Excel analysis.
pub fn xlsx_reference_text_sheet(saved: &XlsxSavedSheet) -> Result<Sheet, XlsxInputError> {
    use ibis_doc_model::machine_readability_layout::LayoutValue;
    use ibis_doc_model::sheet::SheetCell;
    let original: BTreeMap<_, _> = saved.sheet.cells.iter().map(|c| ((c.row, c.column), c)).collect();
    let mut sheet = saved.sheet.clone();
    let mut cells = Vec::with_capacity(saved.layout.cells.len());
    for layout in &saved.layout.cells {
        let position = (layout.row, layout.column);
        let text = match &layout.value {
            None => None,
            Some(LayoutValue::Text(value)) => Some(value.clone()),
            Some(LayoutValue::Numeric) => match original.get(&position).map(|c| &c.value) {
                Some(CellValue::Bool(value)) => Some(if *value { "True".into() } else { "False".into() }),
                Some(CellValue::Number(_)) => Some(saved.numeric_lexemes.get(&position).and_then(XlsxNumericLexeme::reference_text).ok_or(XlsxInputError::UnsupportedReferenceValue)?),
                _ => return Err(XlsxInputError::UnsupportedReferenceValue),
            },
            Some(LayoutValue::Date) => {
                let value = match original.get(&position).map(|c| &c.value) { Some(CellValue::Number(v)) => *v, _ => return Err(XlsxInputError::UnsupportedReferenceValue) };
                Some(xlsx_date_reference_text(value, saved.date1904).ok_or(XlsxInputError::UnsupportedReferenceValue)?)
            }
            Some(LayoutValue::Other) => {
                let value = match original.get(&position).map(|c| &c.value) { Some(CellValue::Number(v)) => *v, _ => return Err(XlsxInputError::UnsupportedReferenceValue) };
                Some(xlsx_duration_reference_text(value).ok_or(XlsxInputError::UnsupportedReferenceValue)?)
            }
        };
        let value = text.as_ref().map_or(CellValue::Empty, |v| CellValue::Text(v.clone()));
        let mut cell = original.get(&position).map(|v| (*v).clone()).unwrap_or(SheetCell { row: layout.row, column: layout.column, value: CellValue::Empty, text: String::new(), kind: CellKind::Empty, formula: None, style: 0, phonetic: None, runs: None });
        cell.value = value; cell.text = text.unwrap_or_default(); cell.kind = CellKind::of(&cell.value);
        cells.push(cell);
    }
    sheet.cells = cells.into();
    // Merged-cell presence and masking are already represented in the view.
    sheet.merges.clear();
    Ok(sheet)
}

/// Render an Excel date serial as the reference reader's Python date/time text.
/// This accepts the bounded non-negative serial range supported by the adapter.
pub fn xlsx_date_reference_text(value: f64, date1904: bool) -> Option<String> {
    use ibis_calc::eval::convert::{civil_from_days, days_from_civil};
    if !value.is_finite() || !(0.0..=2_900_000.0).contains(&value) { return None; }
    let mut day = value.floor() as i64;
    let milliseconds = ((value - value.floor()) * 86_400_000.0).round_ties_even() as i64;
    let mut time = milliseconds;
    if !(0..86_400_000).contains(&time) { time -= 86_400_000; day += 1; }
    let hours = time / 3_600_000; let minutes = (time / 60_000) % 60; let seconds = (time / 1000) % 60; let millis = time % 1000;
    let suffix = if millis == 0 { format!("{hours:02}:{minutes:02}:{seconds:02}") } else { format!("{hours:02}:{minutes:02}:{seconds:02}.{:06}", millis * 1000) };
    if value < 1.0 && milliseconds < 86_400_000 { return Some(suffix); }
    if !date1904 && value > 0.0 && value < 60.0 { day += 1; }
    let epoch = if date1904 { days_from_civil(1904, 1, 1) } else { days_from_civil(1899, 12, 30) };
    let (year, month, date) = civil_from_days(epoch.checked_add(day)?);
    if !(1..=9999).contains(&year) { return None; }
    Some(format!("{year:04}-{month:02}-{date:02} {suffix}"))
}

/// Build the row-header heuristic input from reference-reader values and direct
/// formatting. Unknown numeric forms or missing metadata are rejected.
fn xlsx_stub_cells(saved: &XlsxSavedSheet, formats: &BTreeMap<(u32, u32), ibis_doc_model::machine_readability_rules::ReferenceCellFormat>, read_only: bool) -> Result<(Vec<ibis_doc_model::machine_readability_layout::StubCell>, BTreeSet<(u32, u32)>), XlsxInputError> {
    use ibis_doc_model::machine_readability_layout::{LayoutValue, StubCell, StubValue};
    use ibis_doc_model::machine_readability_rules::ReferenceCellFormat;
    let original: BTreeMap<_, _> = saved.sheet.cells.iter().map(|c| ((c.row, c.column), c)).collect();
    let mut result = Vec::with_capacity(saved.layout.cells.len());
    let mut uncertain = BTreeSet::new();
    for cell in &saved.layout.cells {
        let position = (cell.row, cell.column);
        let dependent = saved.sheet.merges.iter().any(|m| cell.row >= m.first_row && cell.row <= m.last_row && cell.column >= m.first_column && cell.column <= m.last_column && (cell.row != m.first_row || cell.column != m.first_column));
        let format = if dependent && (!read_only || cell.value.is_none()) { ReferenceCellFormat::default() } else if let Some(fmt) = formats.get(&position) { fmt.clone() } else if cell.merged && cell.value.is_none() { ReferenceCellFormat::default() } else { return Err(XlsxInputError::InvalidWindowMetadata) };
        let key = match &cell.value {
            None => None,
            Some(LayoutValue::Numeric) => {
                let source = original.get(&position).ok_or(XlsxInputError::UnsupportedReferenceValue)?;
                match &source.value {
                    CellValue::Bool(true) => Some("1".to_string()), CellValue::Bool(false) => None,
                    CellValue::Number(v) if v.is_finite() && *v > 0.0 && v.fract() == 0.0 => {
                        match saved.numeric_lexemes.get(&position) {
                            Some(XlsxNumericLexeme::Integer(_)) => saved.numeric_lexemes.get(&position).and_then(XlsxNumericLexeme::reference_text),
                            Some(XlsxNumericLexeme::Float(_)) => Some(format!("{v:.0}")),
                            None => return Err(XlsxInputError::UnsupportedReferenceValue),
                        }
                    }
                    _ => None,
                }
            }
            Some(LayoutValue::Text(value)) => {
                let value = value.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'));
                if !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()) {
                    let stripped = value.trim_start_matches('0'); if stripped.is_empty() { None } else { Some(stripped.to_string()) }
                } else if !value.is_empty() && value.chars().all(|c| c.is_numeric()) && !value.is_ascii() { uncertain.insert((cell.row, cell.column)); None }
                else { None }
            }
            Some(LayoutValue::Date | LayoutValue::Other) => None,
        };
        let value = cell.value.as_ref().map(|_| key.map_or(StubValue::Other, StubValue::PositiveInteger));
        result.push(StubCell { row: cell.row, column: cell.column, value, bold: format.bold, background_nonwhite: format.stub_background_nonwhite, border_left: format.border_left, border_right: format.border_right });
    }
    Ok((result, uncertain))
}

/// Render a bounded non-negative Excel duration as the reference reader does.
pub fn xlsx_duration_reference_text(value: f64) -> Option<String> {
    if !value.is_finite() || !(0.0..=2_900_000.0).contains(&value) { return None; }
    let whole_days = value.floor() as i64;
    let fractional = ((value - value.floor()) * 86_400_000_000.0).round_ties_even() as i64;
    let microseconds = whole_days.checked_mul(86_400_000_000)?.checked_add(fractional)?;
    let remainder = microseconds % 1_000_000;
    let mut seconds = microseconds / 1_000_000;
    let mut millis = 0i64;
    if remainder != 0 {
        seconds = ((microseconds as f64) / 1_000_000.0).floor() as i64;
        millis = ((remainder as f64) / 1000.0).round_ties_even() as i64;
        if millis >= 1000 { seconds += 1; millis -= 1000; }
    }
    let days = seconds / 86_400; let hours = (seconds / 3600) % 24; let minutes = (seconds / 60) % 60; let sec = seconds % 60;
    let time = if millis == 0 { format!("{hours}:{minutes:02}:{sec:02}") } else { format!("{hours}:{minutes:02}:{sec:02}.{:06}", millis * 1000) };
    if days == 0 { Some(time) } else { Some(format!("{days} day{}, {time}", if days == 1 { "" } else { "s" })) }
}

/// Score supplied XLSX table checks only when all required metadata was
/// available. The result is still experimental and is not a complete report.
pub fn score_xlsx_tables_partial(tables: &[PartialXlsxStructure]) -> Option<[ibis_doc_model::machine_readability_scoring::LevelScore; 3]> {
    use ibis_doc_model::machine_readability_scoring::{score_level, Level, RuleResult};
    if tables.iter().any(|t| !t.unavailable.is_empty()) { return None; }
    let collect = |prefix: &str| -> Vec<RuleResult> { tables.iter().flat_map(|t| t.checks.iter().filter(|c| c.result.rule_id.starts_with(prefix)).map(|c| c.result.clone())).collect() };
    Some([score_level(Level::One, &collect("L1-")), score_level(Level::Two, &collect("L2-")), score_level(Level::Three, &collect("L3-"))])
}

/// Header labels from the first XLSX table with extractable headers. `None`
/// means an earlier table's headers are unknown and cannot safely be skipped.
pub fn first_xlsx_headers(sheets: &[Vec<PartialXlsxStructure>]) -> Option<Vec<String>> {
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    for sheet in sheets {
        for table in sheet {
            let labels: Vec<_> = table.column_headers.as_ref()?.iter().filter(|h| !h.label.is_empty()).map(|h| h.label.trim_matches(space).to_string()).collect();
            if !labels.is_empty() { return Some(labels); }
        }
    }
    Some(Vec::new())
}
