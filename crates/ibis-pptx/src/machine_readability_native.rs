// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Experimental in-process CSV/TSV rule execution. It is intentionally
//! incomplete and must not replace or be presented as a full Harunobu report.
use std::collections::BTreeMap;
use ibis_doc_model::machine_readability_csv::{diagnose, read_sheet, single_line_records, valid_quoting, Delimiter};
use ibis_doc_model::machine_readability_layout::{generate_column_headers, infer_column_schemas, merge_column_adjacent_regions, merge_form_fragments, preliminary_islands, HeaderCell, InferredColumnType, LayoutValue};
use ibis_doc_model::machine_readability_rules::{csv_data_fragmentation, file_format, irrelevant_information, one_data_per_cell, one_table_per_sheet, platform_characters, whitespace_formatting, ColumnType, DetectedTable, OutsideTableCell, RuleEvaluation, TextEncoding, WorkbookFormat};
use ibis_doc_model::sheet::{CellKind, CellValue, Sheet, SheetCell};
use crate::machine_readability_csv::{decode_csv_bytes, CsvInputEncoding, CsvInputError};

/// The checks currently missing from the Level 1 CSV/TSV implementation.
/// Excel-only rules L1-10 through L1-13 do not apply to this input.
pub const UNIMPLEMENTED_CSV_LEVEL1: &[&str] = &["L1-05", "L1-09"];
#[derive(Clone, Debug, PartialEq)]
pub struct PartialCsvAnalysis {
    pub encoding: CsvInputEncoding,
    pub table: DetectedTable,
    pub header_rows: Vec<u32>,
    pub checks: Vec<RuleEvaluation>,
    /// Always false until all applicable rules and report semantics are ported.
    pub complete: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeCsvError { Input(CsvInputError), UnknownLayout, UnsupportedTableCount(usize), TooManyColumns }

/// Execute the currently ported Level 1 rules for an explicitly supplied CSV
/// or TSV byte stream, limited to one detected table. It returns no score;
/// unknown layouts and zero/multiple tables are errors, never clean passes.
pub fn analyze_csv_partial(bytes: &[u8], delimiter: Delimiter) -> Result<PartialCsvAnalysis, NativeCsvError> {
    let (text, encoding) = decode_csv_bytes(bytes).map_err(NativeCsvError::Input)?;
    let parsed = read_sheet(&text, delimiter).map_err(|e| NativeCsvError::Input(CsvInputError::InvalidCsv(e)))?;
    let cells = parsed.layout_cells().map_err(|e| NativeCsvError::Input(CsvInputError::InvalidCsv(e)))?;
    let preliminary = preliminary_islands(&cells, &[]).ok_or(NativeCsvError::UnknownLayout)?;
    let columns = merge_column_adjacent_regions(&preliminary.candidates, &cells, &[]).ok_or(NativeCsvError::UnknownLayout)?;
    let forms = merge_form_fragments(&columns, &cells, &[]).ok_or(NativeCsvError::UnknownLayout)?;
    if forms.len() != 1 { return Err(NativeCsvError::UnsupportedTableCount(forms.len())); }
    let region = &forms[0];
    let range = region.candidate.range;
    let width = u64::from(range.last_column) - u64::from(range.first_column) + 1;
    if width > 1_000_000 { return Err(NativeCsvError::TooManyColumns); }
    let indices: Vec<_> = (range.first_column..=range.last_column).collect();
    let header_cells: Vec<_> = cells.iter().map(|c| HeaderCell { row: c.row, column: c.column, value: match &c.value { Some(LayoutValue::Text(s)) => Some(s.clone()), _ => None }, merged: false, master: None }).collect();
    let headers = generate_column_headers(&region.headers.rows, &indices, &header_cells).ok_or(NativeCsvError::UnknownLayout)?;
    let labels: BTreeMap<_, _> = headers.iter().map(|h| (h.column, h.label.as_str())).collect();
    let schema_input: Vec<_> = indices.iter().map(|&c| (c, labels.get(&c).copied())).collect();
    let schemas = infer_column_schemas(&schema_input, region.headers.body_first, region.headers.body_last, &cells).ok_or(NativeCsvError::UnknownLayout)?;
    let column_types: BTreeMap<_, _> = schemas.iter().map(|s| (s.column, if s.kind == InferredColumnType::Numeric { ColumnType::Numeric } else { ColumnType::Other })).collect();
    let mut sheet = Sheet::default();
    sheet.cells = cells.iter().filter_map(|c| match &c.value {
        Some(LayoutValue::Text(value)) => Some(SheetCell { row: c.row, column: c.column, value: CellValue::Text(value.clone()), text: value.clone(), kind: CellKind::Text, formula: None, style: 0, phonetic: None, runs: None }),
        _ => None,
    }).collect::<Vec<_>>().into();
    let table = DetectedTable { range, confidence: region.candidate.confidence };
    let others: Vec<_> = preliminary.others.iter().map(|o| OutsideTableCell { row: o.row, column: o.column }).collect();
    let format = match delimiter { Delimiter::Comma => WorkbookFormat::Csv, Delimiter::Tab => WorkbookFormat::Tsv };
    let text_encoding = match encoding { CsvInputEncoding::Utf8 | CsvInputEncoding::Utf8Bom => TextEncoding::Utf8, CsvInputEncoding::Cp932 => TextEncoding::Cp932 };
    let diagnostics = diagnose(&text, delimiter);
    let checks = vec![file_format(format), one_table_per_sheet(range, Some(&[table])), csv_data_fragmentation(&sheet, range).ok_or(NativeCsvError::UnknownLayout)?, irrelevant_information(Some(&others)),
        whitespace_formatting(&sheet, range, Some(&region.headers.rows)), one_data_per_cell(&sheet, range, Some(&column_types)), platform_characters(&sheet, range, text_encoding),
        single_line_records(Some(&diagnostics), range.first_row as usize + 1, range.last_row as usize + 1),
        valid_quoting(Some(&diagnostics), range.first_row as usize + 1, range.last_row as usize + 1)];
    Ok(PartialCsvAnalysis { encoding, table, header_rows: region.headers.rows.clone(), checks, complete: false })
}
