// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Experimental in-process CSV/TSV rule execution. It is intentionally
//! incomplete and must not replace or be presented as a full Harunobu report.
use std::collections::BTreeMap;
use ibis_doc_model::machine_readability_csv::{diagnose, diagnostics_report, read_sheet, Delimiter};
use ibis_doc_model::machine_readability_layout::{estimate_stub_columns, generate_column_headers, infer_column_schemas, inherit_headers_from_above, merge_column_adjacent_regions, merge_form_fragments, preliminary_islands, HeaderCell, InferredColumnSchema, InferredColumnType, LayoutValue, ResolvedIsland, StubCell, StubValue};
use ibis_doc_model::machine_readability_rules::{blank_vs_zero_ai_disabled_report, csv_choice_standardization_report, csv_column_headers_report, csv_fragmentation_report, csv_item_abbreviations_report, csv_other_details_report, csv_unique_headers_report, data_units_report, file_format_report, header_start_position_report, irrelevant_information_report, numeric_purity_report, one_data_per_cell_report, one_table_per_sheet_report, platform_characters_report, reference_skipped_level3_reports, region_codes_report, time_axis_report, vertical_format_report, whitespace_formatting_report, ColumnType, DetectedTable, OutsideCellText, OutsideTableCell, RuleDiagnostics, RuleEvaluation, TextEncoding};
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
    pub column_headers: Vec<ibis_doc_model::machine_readability_layout::InferredColumnHeader>,
    pub column_schemas: Vec<InferredColumnSchema>,
    pub diagnostics: [Vec<RuleDiagnostics>; 3],
    pub checks: Vec<RuleEvaluation>,
    /// Experimental Level 2 checks; no Level 2 score is produced.
    pub level2_checks: Vec<RuleEvaluation>,
    /// Experimental Level 3 checks; no Level 3 score is produced.
    pub level3_checks: Vec<RuleEvaluation>,
    /// Always false until all applicable rules and report semantics are ported.
    pub complete: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeCsvError { Input(CsvInputError), UnknownLayout, UnsupportedTableCount(usize), TooManyColumns, InvalidReportInput }

/// Execute partial checks for one table. The result has no score and remains incomplete.
pub fn analyze_csv_partial(bytes: &[u8], delimiter: Delimiter) -> Result<PartialCsvAnalysis, NativeCsvError> {
    let mut tables = analyze_csv_tables_partial(bytes, delimiter)?;
    if tables.len() != 1 { return Err(NativeCsvError::UnsupportedTableCount(tables.len())); }
    Ok(tables.remove(0))
}

/// Execute partial checks for all detected tables. Unknown input is rejected;
/// neither this result nor its checks are a complete Harunobu report.
pub fn analyze_csv_tables_partial(bytes: &[u8], delimiter: Delimiter) -> Result<Vec<PartialCsvAnalysis>, NativeCsvError> {
    Ok(prepare_csv(bytes, delimiter, "").map(|p|p.tables)?)
}

struct PreparedCsv {
    tables: Vec<PartialCsvAnalysis>,
    others: Vec<(u32,u32,String,Option<String>)>,
    rows: u32,
    columns: u32,
}

fn prepare_csv(bytes: &[u8], delimiter: Delimiter, sheet_name: &str) -> Result<PreparedCsv, NativeCsvError> {
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
    let format = match delimiter { Delimiter::Comma => "csv", Delimiter::Tab => "tsv" };
    let other_values: Vec<_> = preliminary.others.iter().map(|o| {
        let kind = match o.kind { ibis_doc_model::machine_readability_layout::OtherRegion::Title=>"title", ibis_doc_model::machine_readability_layout::OtherRegion::Total=>"total", ibis_doc_model::machine_readability_layout::OtherRegion::Note=>"note", ibis_doc_model::machine_readability_layout::OtherRegion::Source=>"source", ibis_doc_model::machine_readability_layout::OtherRegion::Isolated=>"isolated" };
        let value = sheet.cells.iter().find(|c|c.row == o.row && c.column == o.column).and_then(|c|match &c.value { CellValue::Text(v)=>Some(v.clone()), _=>None });
        (o.row,o.column,kind.to_string(),value)
    }).collect();
    let outside: Vec<_> = other_values.iter().map(|(row,col,kind,value)|OutsideCellText { cell:OutsideTableCell { row:*row,column:*col },region_type:kind,value:value.as_deref() }).collect();
    let text_encoding = match encoding { CsvInputEncoding::Utf8 | CsvInputEncoding::Utf8Bom => TextEncoding::Utf8, CsvInputEncoding::Cp932 => TextEncoding::Cp932 };
    let diagnostics = diagnose(&text, delimiter);
    let mut results = Vec::new();
    for region in &regions {
        let range = region.candidate.range;
        let table = DetectedTable { range, confidence: region.candidate.confidence };
        let schemas = &region.column_schemas;
        let stubs = &region.stub_columns;
        let column_types: BTreeMap<_, _> = schemas.iter().map(|s| (s.column, if s.kind == InferredColumnType::Numeric { ColumnType::Numeric } else { ColumnType::Other })).collect();
        let numeric_schemas: Vec<_> = schemas.iter().map(|s| (s.column, s.kind)).collect();
        let no_positions = std::collections::BTreeSet::new();
        let level1 = vec![file_format_report("",format), one_table_per_sheet_report(sheet_name,range,Some(&tables)).ok_or(NativeCsvError::UnknownLayout)?, csv_fragmentation_report(&sheet,range,&tables).ok_or(NativeCsvError::UnknownLayout)?, irrelevant_information_report(Some(&outside)).ok_or(NativeCsvError::UnknownLayout)?, csv_column_headers_report(&sheet,range,&region.headers.rows,region.headers.body_first,region.headers.body_last).ok_or(NativeCsvError::UnknownLayout)?, whitespace_formatting_report(&sheet,range,Some(&region.headers.rows)).ok_or(NativeCsvError::UnknownLayout)?, one_data_per_cell_report(&sheet,range,Some(&column_types)).ok_or(NativeCsvError::UnknownLayout)?, platform_characters_report(&sheet,range,text_encoding).ok_or(NativeCsvError::UnknownLayout)?, blank_vs_zero_ai_disabled_report(), diagnostics_report(Some(&diagnostics),range.first_row as usize+1,range.last_row as usize+1,delimiter,false).ok_or(NativeCsvError::UnknownLayout)?, diagnostics_report(Some(&diagnostics),range.first_row as usize+1,range.last_row as usize+1,delimiter,true).ok_or(NativeCsvError::UnknownLayout)?];
        let level2 = vec![numeric_purity_report(&sheet,range,region.headers.body_first,region.headers.body_last,&numeric_schemas,&no_positions).ok_or(NativeCsvError::UnknownLayout)?, csv_item_abbreviations_report(&sheet,stubs,region.headers.body_first,region.headers.body_last).ok_or(NativeCsvError::UnknownLayout)?, csv_unique_headers_report(&sheet,range,&region.headers.rows,region.headers.body_first,region.headers.body_last).ok_or(NativeCsvError::UnknownLayout)?, csv_choice_standardization_report(&sheet,range,region.headers.body_first,region.headers.body_last,&numeric_schemas).ok_or(NativeCsvError::UnknownLayout)?, csv_other_details_report(&sheet,range,region.headers.body_first,region.headers.body_last,&numeric_schemas).ok_or(NativeCsvError::UnknownLayout)?];
        let skipped = reference_skipped_level3_reports();
        let mut level3 = vec![header_start_position_report(range,&region.headers.rows,region.headers.body_first).ok_or(NativeCsvError::UnknownLayout)?];
        level3.extend(skipped[..2].iter().cloned());
        level3.push(data_units_report(&sheet,range,&region.headers.rows,region.headers.body_first,region.headers.body_last,&no_positions).ok_or(NativeCsvError::UnknownLayout)?);
        level3.push(time_axis_report(&sheet,range,&region.headers.rows,region.headers.body_first,region.headers.body_last).ok_or(NativeCsvError::UnknownLayout)?);
        level3.push(region_codes_report(&sheet,range,&region.headers.rows,region.headers.body_first,region.headers.body_last,&no_positions).ok_or(NativeCsvError::UnknownLayout)?);
        level3.extend(skipped[2..].iter().cloned());
        level3.push(vertical_format_report(&sheet,range,&region.headers.rows,&no_positions).ok_or(NativeCsvError::UnknownLayout)?);
        let checks=level1.iter().map(|d|d.evaluation.clone()).collect();
        let level2_checks=level2.iter().map(|d|d.evaluation.clone()).collect();
        let level3_checks=level3.iter().map(|d|d.evaluation.clone()).collect();
        results.push(PartialCsvAnalysis { encoding, table, header_rows:region.headers.rows.clone(), column_headers:region.column_headers.clone(), column_schemas:schemas.clone(), diagnostics:[level1,level2,level3], checks, level2_checks, level3_checks, complete:false });
    }
    Ok(PreparedCsv { tables:results, others:other_values, rows:u32::try_from(parsed.records.len()).map_err(|_|NativeCsvError::UnknownLayout)?, columns:u32::try_from(parsed.max_columns).map_err(|_|NativeCsvError::TooManyColumns)? })
}

/// Experimental end-to-end CSV/TSV JSON report. Input content validation and
/// broader reference compatibility are not yet established; do not substitute
/// this for the application's existing backend.
pub fn analyze_csv_report_experimental(bytes: &[u8], file_name: &str, mode: &str) -> Result<serde_json::Value, NativeCsvError> {
    use crate::machine_readability_report::{file_report_json, FileReport, OtherCell, SheetProperty, SheetReport, TableReport};
    let path = std::path::Path::new(file_name);
    if path.file_name().and_then(|n|n.to_str()) != Some(file_name) { return Err(NativeCsvError::InvalidReportInput); }
    let (delimiter,format) = match path.extension().and_then(|s|s.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("csv") => (Delimiter::Comma,"csv"), Some("tsv") => (Delimiter::Tab,"tsv"), _ => return Err(NativeCsvError::InvalidReportInput),
    };
    let levels = match mode { "lite"=>1,"standard"=>2,"thorough"=>3,_=>return Err(NativeCsvError::InvalidReportInput) };
    let sheet_name = path.file_stem().and_then(|s|s.to_str()).filter(|s|!s.is_empty()).ok_or(NativeCsvError::InvalidReportInput)?;
    let prepared = prepare_csv(bytes,delimiter,sheet_name)?;
    let tables:Vec<_> = prepared.tables.iter().map(|table|TableReport {
        range:table.table.range, confidence:table.table.confidence, header_rows:&table.header_rows, column_headers:&table.column_headers, columns:&table.column_schemas,
        checks:[&table.diagnostics[0],if levels >= 2 { &table.diagnostics[1] } else { &[] },if levels >= 3 { &table.diagnostics[2] } else { &[] }],
    }).collect();
    let others:Vec<_> = prepared.others.iter().map(|(row,column,kind,value)|OtherCell { row:*row,column:*column,region_type:kind,value:serde_json::json!(value) }).collect();
    let used_range = if prepared.rows > 0 && prepared.columns > 0 {
        let mut col = u64::from(prepared.columns); let mut letters = Vec::new(); while col > 0 { col -= 1; letters.push((b'A'+(col%26) as u8) as char); col /= 26; }
        format!("A1:{}{}",letters.into_iter().rev().collect::<String>(),prepared.rows)
    } else { String::new() };
    let sheets = [SheetReport { name:sheet_name,used_range:&used_range,hidden:false,property:Some(SheetProperty { name:sheet_name,max_row:prepared.rows,max_col:prepared.columns,hidden:false }),others:&others,tables:&tables }];
    file_report_json(&FileReport { name:file_name,size:bytes.len() as u64,format,sheet_count:1,sheets:&sheets }).ok_or(NativeCsvError::InvalidReportInput)
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

/// Header labels from the first CSV table with extractable headers, in the
/// same order used by the cross-file format check.
pub fn first_csv_headers(tables: &[PartialCsvAnalysis]) -> Vec<String> {
    let space = |c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}');
    for table in tables {
        let labels: Vec<_> = table.column_headers.iter().filter(|h| !h.label.is_empty()).map(|h| h.label.trim_matches(space).to_string()).collect();
        if !labels.is_empty() { return labels; }
    }
    Vec::new()
}
