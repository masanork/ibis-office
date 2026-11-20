// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Queries over the cells of a workbook: reading a source table out of the
//! sheets, and finding the other tables a merge or an append names.
use ibis_calc::CellValue;

use super::{table_of_cells, Parameter, QColumn, QTable, QType, Query, Resolver, Source, Step};
use crate::model::Presentation;
use crate::sheet::Sheet;
use crate::sheet_calc::sheet_of_slide;

/// The sheet named `name` (hidden ones too).
fn sheet_named<'a>(p: &'a Presentation, name: &str) -> Option<&'a Sheet> {
    let wb = p.workbook.as_deref()?;
    let i = wb.sheet_index(name)?;
    let ws = &wb.sheets[i];
    match ws.slide {
        Some(id) => p.slide(id).and_then(sheet_of_slide).map(|s| &**s),
        None => ws.hidden.as_deref(),
    }
}

/// The cells of a block of a sheet: rows then columns.
fn block(sheet: &Sheet, first_row: u32, first_column: u32, last_row: u32, last_column: u32) -> Vec<Vec<CellValue>> {
    (first_row..=last_row).map(|r| (first_column..=last_column).map(|c| sheet.cell(r, c).map_or(CellValue::Empty, |x| x.value.clone())).collect()).collect()
}

/// Type guessed for a column of values: numbers (whole or not), text, or mixed.
fn guess_type(values: &[&CellValue]) -> QType {
    let mut seen_number = false;
    let mut all_whole = true;
    for v in values {
        match v {
            CellValue::Empty => {}
            CellValue::Number(n) => {
                seen_number = true;
                all_whole &= n.fract() == 0.0;
            }
            CellValue::Text(_) => return QType::Text,
            CellValue::Bool(_) => return QType::Logical,
            CellValue::Error(_) => return QType::Any,
        }
    }
    if seen_number {
        if all_whole { QType::Whole } else { QType::Decimal }
    } else {
        QType::Any
    }
}

/// A source as a table: a table of the workbook (its headings the column names, its columns typed from
/// their values), a defined name's range, or a range of a sheet (the columns `Column1`, `Column2`…).
pub fn read_source(p: &Presentation, source: &Source) -> Result<QTable, String> {
    match source {
        Source::Parquet { .. } => Err("Select the Parquet source file".into()),
        Source::Blank => Ok(QTable {
            columns: vec![QColumn {
                name: "Column1".to_string(),
                ty: QType::Any,
            }],
            rows: Vec::new(),
        }),
        Source::Range { sheet, range } => {
            let s = sheet_named(p, sheet).ok_or_else(|| format!("シート {sheet} がありません"))?;
            let mut ends = range.split(':');
            let a = ibis_calc::formula::parser::parse_cell(ends.next().unwrap_or("").trim()).ok_or_else(|| format!("範囲 {range} を読めません"))?;
            let b = match ends.next() {
                Some(e) => ibis_calc::formula::parser::parse_cell(e.trim()).ok_or_else(|| format!("範囲 {range} を読めません"))?,
                None => a.clone(),
            };
            let (r0, r1) = (a.row.min(b.row), a.row.max(b.row));
            let (c0, c1) = (a.column.min(b.column), a.column.max(b.column));
            Ok(table_of_cells(block(s, r0, c0, r1, c1), (c1 - c0 + 1) as usize))
        }
        Source::Table(name) => {
            // A table of the workbook.
            for slide in &p.slides {
                let Some(sheet) = sheet_of_slide(slide) else { continue };
                if let Some(t) = sheet.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name)) {
                    let r = &t.range;
                    let header_rows = t.header_rows.max(1);
                    let data_first = r.first_row + header_rows;
                    let data_last = if t.totals_rows > 0 { r.last_row.saturating_sub(t.totals_rows) } else { r.last_row };
                    let rows = if data_first <= data_last { block(sheet, data_first, r.first_column, data_last, r.last_column) } else { Vec::new() };
                    let columns = (0..=(r.last_column - r.first_column))
                        .map(|i| {
                            let name = t.columns.get(i as usize).cloned().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Column{}", i + 1));
                            let values: Vec<&CellValue> = rows.iter().map(|row| &row[i as usize]).collect();
                            QColumn { name, ty: guess_type(&values) }
                        })
                        .collect();
                    return Ok(QTable { columns, rows });
                }
            }
            // A defined name of a range (`Sheet1!$A$1:$C$5`): the first row the headings.
            let wb = p.workbook.as_deref().ok_or("ブックではありません")?;
            let n = wb.names.iter().find(|n| n.name.eq_ignore_ascii_case(name)).ok_or_else(|| format!("テーブル {name} がありません"))?;
            let (sheet, range) = n.formula.rsplit_once('!').ok_or_else(|| format!("名前 {name} は範囲ではありません"))?;
            let sheet = sheet.trim_matches('\'').replace("''", "'");
            let table = read_source(p, &Source::Range { sheet, range: range.replace('$', "") })?;
            Step::PromoteHeaders.apply(&table, &[], &NoTables)
        }
    }
}

struct NoTables;
impl Resolver for NoTables {
    fn table(&self, _: &str) -> Option<QTable> {
        None
    }
}

/// Finds the tables other steps name: another query of the workbook (its result), or a table of the workbook.
pub struct BookResolver<'a> {
    pub presentation: &'a Presentation,
    pub queries: &'a [Query],
    pub parameters: &'a [Parameter],
    /// The query being worked out, which cannot be its own source.
    pub current: &'a str,
    pub depth: usize,
}

impl Resolver for BookResolver<'_> {
    fn table(&self, name: &str) -> Option<QTable> {
        if let Some(q) = self.queries.iter().find(|q| q.name == name && q.name != self.current) {
            if self.depth > 8 {
                return None;
            }
            return result_of(self.presentation, q, self.queries, self.parameters, self.depth + 1).ok();
        }
        read_source(self.presentation, &Source::Table(name.to_string())).ok()
    }
}

/// The result of a query: its source worked through every step.
pub fn result_of(p: &Presentation, query: &Query, queries: &[Query], parameters: &[Parameter], depth: usize) -> Result<QTable, String> {
    let source = read_source(p, &query.source)?;
    result_from_source(p, query, queries, parameters, depth, source)
}

pub fn result_from_source(p: &Presentation, query: &Query, queries: &[Query], parameters: &[Parameter], depth: usize, source: QTable) -> Result<QTable, String> {
    let resolver = BookResolver { presentation: p, queries, parameters, current: &query.name, depth };
    let (tables, error) = query.run(source, parameters, &resolver);
    match error {
        Some((_, e)) => Err(e),
        None => tables.into_iter().last().ok_or_else(|| "結果がありません".to_string()),
    }
}
