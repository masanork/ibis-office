// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Experimental in-process CSV/TSV rule execution. It is intentionally
//! incomplete and must not replace or be presented as a full Harunobu report.
use std::collections::BTreeMap;
use ibis_doc_model::machine_readability_csv::{diagnose, read_sheet, single_line_records, valid_quoting, Delimiter};
use ibis_doc_model::machine_readability_layout::{estimate_stub_columns, generate_column_headers, infer_column_schemas, inherit_headers_from_above, merge_column_adjacent_regions, merge_form_fragments, preliminary_islands, HeaderCell, InferredColumnType, LayoutValue, ResolvedIsland, StubCell, StubValue};
use ibis_doc_model::machine_readability_rules::{blank_vs_zero_ai_disabled, csv_choice_standardization, csv_column_headers, csv_data_units, csv_fragmentation_with_tables, csv_item_abbreviations, csv_numeric_purity, csv_other_details, csv_region_codes, csv_time_axis, csv_unique_headers, csv_vertical_format, file_format, header_start_position, irrelevant_information, one_data_per_cell, one_table_per_sheet, platform_characters, reference_skipped_level3, whitespace_formatting, ColumnType, DetectedTable, OutsideTableCell, RuleEvaluation, TextEncoding, WorkbookFormat};
use ibis_doc_model::sheet::{CellKind, CellValue, Sheet, SheetCell};
use crate::machine_readability_csv::{decode_csv_bytes, CsvInputEncoding, CsvInputError};

/// The checks currently missing from the Level 1 CSV/TSV implementation.
/// Excel-only rules L1-10 through L1-13 do not apply to this input.
pub const UNIMPLEMENTED_CSV_LEVEL1: &[&str] = &["L1-09"];
#[derive(Clone, Debug, PartialEq)]
pub struct PartialCsvAnalysis {
    pub encoding: CsvInputEncoding,
    pub table: DetectedTable,
    pub header_rows: Vec<u32>,
    pub checks: Vec<RuleEvaluation>,
    /// Experimental Level 2 checks; no Level 2 score is produced.
    pub level2_checks: Vec<RuleEvaluation>,
    /// Experimental Level 3 checks; no Level 3 score is produced.
    pub level3_checks: Vec<RuleEvaluation>,
    /// Always false until all applicable rules and report semantics are ported.
    pub complete: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeCsvError { Input(CsvInputError), UnknownLayout, UnsupportedTableCount(usize), TooManyColumns }

/// Execute partial checks for one table. The result has no score and remains incomplete.
pub fn analyze_csv_partial(bytes: &[u8], delimiter: Delimiter) -> Result<PartialCsvAnalysis, NativeCsvError> {
    let mut tables = analyze_csv_tables_partial(bytes, delimiter)?;
    if tables.len() != 1 { return Err(NativeCsvError::UnsupportedTableCount(tables.len())); }
    Ok(tables.remove(0))
}

/// Execute partial checks for all detected tables. Unknown input is rejected;
/// neither this result nor its checks are a complete Harunobu report.
pub fn analyze_csv_tables_partial(bytes: &[u8], delimiter: Delimiter) -> Result<Vec<PartialCsvAnalysis>, NativeCsvError> {
    let (text, encoding) = decode_csv_bytes(bytes).map_err(NativeCsvError::Input)?;
    let parsed = read_sheet(&text, delimiter).map_err(|e| NativeCsvError::Input(CsvInputError::InvalidCsv(e)))?;
    let cells = parsed.layout_cells().map_err(|e| NativeCsvError::Input(CsvInputError::InvalidCsv(e)))?;
    let preliminary = preliminary_islands(&cells, &[]).ok_or(NativeCsvError::UnknownLayout)?;
    let columns = merge_column_adjacent_regions(&preliminary.candidates, &cells, &[]).ok_or(NativeCsvError::UnknownLayout)?;
    let forms = merge_form_fragments(&columns, &cells, &[]).ok_or(NativeCsvError::UnknownLayout)?;
    // Harunobu creates a cell for every CSV field, including empty fields;
    // preserve that distinction from missing fields for violation locations.
    let mut sheet = Sheet::default();
    let mut sheet_cells = Vec::new();
    for (row, record) in parsed.records.iter().enumerate() {
        for (column, value) in record.iter().enumerate() {
            let row = u32::try_from(row).map_err(|_| NativeCsvError::UnknownLayout)?;
            let column = u32::try_from(column).map_err(|_| NativeCsvError::TooManyColumns)?;
            let text = value.clone().unwrap_or_default();
            let value = value.as_ref().map_or(CellValue::Empty, |v| CellValue::Text(v.clone()));
            sheet_cells.push(SheetCell { row, column, value, text, kind: CellKind::Text, formula: None, style: 0, phonetic: None, runs: None });
        }
    }
    sheet.cells = sheet_cells.into();
    let header_cells: Vec<_> = cells.iter().map(|c| HeaderCell { row: c.row, column: c.column, value: match &c.value { Some(LayoutValue::Text(s)) => Some(s.clone()), _ => None }, merged: false, master: None }).collect();
    let stub_cells: Vec<_> = cells.iter().map(|c| StubCell { row: c.row, column: c.column, value: c.value.as_ref().map(|_| StubValue::Other), bold: false, background_nonwhite: false, border_left: false, border_right: false }).collect();
    let mut regions = Vec::new();
    for region in forms {
        let range = region.candidate.range;
        let width = u64::from(range.last_column) - u64::from(range.first_column) + 1;
        if width > 1_000_000 { return Err(NativeCsvError::TooManyColumns); }
        let indices: Vec<_> = (range.first_column..=range.last_column).collect();
        let headers = generate_column_headers(&region.headers.rows, &indices, &header_cells).ok_or(NativeCsvError::UnknownLayout)?;
        let labels: BTreeMap<_, _> = headers.iter().map(|h| (h.column, h.label.as_str())).collect();
        let schema_input: Vec<_> = indices.iter().map(|&c| (c, labels.get(&c).copied())).collect();
        // The reference returns immediately without schemas or stubs for headerless layouts.
        let schemas = if region.headers.rows.is_empty() { Vec::new() } else { infer_column_schemas(&schema_input, region.headers.body_first, region.headers.body_last, &cells).ok_or(NativeCsvError::UnknownLayout)? };
        let stubs = if region.headers.rows.is_empty() { Vec::new() } else { estimate_stub_columns(range.first_column, range.last_column, region.headers.body_first, region.headers.body_last, &stub_cells, &schemas).ok_or(NativeCsvError::UnknownLayout)? };
        let abstain = region.candidate.confidence < 0.7;
        regions.push(ResolvedIsland { candidate: region.candidate, headers: region.headers, column_headers: headers, column_schemas: schemas, stub_columns: stubs, footer_rows: Vec::new(), abstain });
    }
    let mut regions = inherit_headers_from_above(&regions, &cells).ok_or(NativeCsvError::UnknownLayout)?;
    regions.sort_by(|a, b| b.candidate.confidence.total_cmp(&a.candidate.confidence));
    let tables: Vec<_> = regions.iter().map(|r| DetectedTable { range: r.candidate.range, confidence: r.candidate.confidence }).collect();
    let others: Vec<_> = preliminary.others.iter().map(|o| OutsideTableCell { row: o.row, column: o.column }).collect();
    let format = match delimiter { Delimiter::Comma => WorkbookFormat::Csv, Delimiter::Tab => WorkbookFormat::Tsv };
    let text_encoding = match encoding { CsvInputEncoding::Utf8 | CsvInputEncoding::Utf8Bom => TextEncoding::Utf8, CsvInputEncoding::Cp932 => TextEncoding::Cp932 };
    let diagnostics = diagnose(&text, delimiter);
    let mut results = Vec::new();
    for region in &regions {
        let range = region.candidate.range;
        let table = DetectedTable { range, confidence: region.candidate.confidence };
        let schemas = &region.column_schemas;
        let stubs = &region.stub_columns;
        let column_types: BTreeMap<_, _> = schemas.iter().map(|s| (s.column, if s.kind == InferredColumnType::Numeric { ColumnType::Numeric } else { ColumnType::Other })).collect();
    let checks = vec![file_format(format), one_table_per_sheet(range, Some(&tables)), csv_fragmentation_with_tables(&sheet, range, &tables).ok_or(NativeCsvError::UnknownLayout)?, irrelevant_information(Some(&others)), csv_column_headers(&sheet, range, &region.headers.rows, region.headers.body_first, region.headers.body_last).ok_or(NativeCsvError::UnknownLayout)?,
        whitespace_formatting(&sheet, range, Some(&region.headers.rows)), one_data_per_cell(&sheet, range, Some(&column_types)), platform_characters(&sheet, range, text_encoding), blank_vs_zero_ai_disabled(),
        single_line_records(Some(&diagnostics), range.first_row as usize + 1, range.last_row as usize + 1),
        valid_quoting(Some(&diagnostics), range.first_row as usize + 1, range.last_row as usize + 1)];
    let numeric_schemas: Vec<_> = schemas.iter().map(|s| (s.column, s.kind)).collect();
    let level2_checks = vec![csv_numeric_purity(&sheet, range, region.headers.body_first, region.headers.body_last, &numeric_schemas).ok_or(NativeCsvError::UnknownLayout)?, csv_item_abbreviations(&sheet, &stubs, region.headers.body_first, region.headers.body_last).ok_or(NativeCsvError::UnknownLayout)?, csv_unique_headers(&sheet, range, &region.headers.rows, region.headers.body_first, region.headers.body_last).ok_or(NativeCsvError::UnknownLayout)?, csv_choice_standardization(&sheet, range, region.headers.body_first, region.headers.body_last, &numeric_schemas).ok_or(NativeCsvError::UnknownLayout)?, csv_other_details(&sheet, range, region.headers.body_first, region.headers.body_last, &numeric_schemas).ok_or(NativeCsvError::UnknownLayout)?];
    let mut level3_checks = vec![header_start_position(range, &region.headers.rows, region.headers.body_first).ok_or(NativeCsvError::UnknownLayout)?];
    let skipped = reference_skipped_level3();
    level3_checks.extend(skipped[..2].iter().cloned());
    level3_checks.push(csv_data_units(&sheet, range, &region.headers.rows, region.headers.body_first, region.headers.body_last).ok_or(NativeCsvError::UnknownLayout)?);
    level3_checks.push(csv_time_axis(&sheet, range, &region.headers.rows, region.headers.body_first, region.headers.body_last).ok_or(NativeCsvError::UnknownLayout)?);
    level3_checks.push(csv_region_codes(&sheet, range, &region.headers.rows, region.headers.body_first, region.headers.body_last).ok_or(NativeCsvError::UnknownLayout)?);
    level3_checks.extend(skipped[2..].iter().cloned());
    level3_checks.push(csv_vertical_format(&sheet, range, &region.headers.rows).ok_or(NativeCsvError::UnknownLayout)?);
        results.push(PartialCsvAnalysis { encoding, table, header_rows: region.headers.rows.clone(), checks, level2_checks, level3_checks, complete: false });
    }
    Ok(results)
}

/// Score the currently returned CSV table results without presenting them as a
/// complete report. The caller must disclose remaining analysis limitations.
pub fn score_csv_tables_partial(tables: &[PartialCsvAnalysis]) -> [ibis_doc_model::machine_readability_scoring::LevelScore; 3] {
    use ibis_doc_model::machine_readability_scoring::{score_level, Level};
    let level1: Vec<_> = tables.iter().flat_map(|t| t.checks.iter().map(|c| c.result.clone())).collect();
    let level2: Vec<_> = tables.iter().flat_map(|t| t.level2_checks.iter().map(|c| c.result.clone())).collect();
    let level3: Vec<_> = tables.iter().flat_map(|t| t.level3_checks.iter().map(|c| c.result.clone())).collect();
    [score_level(Level::One, &level1), score_level(Level::Two, &level2), score_level(Level::Three, &level3)]
}
