// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! CSV/TSV raw-text diagnostics for the Rust machine-readability checker.
//!
//! Based on Harunobu 0.2.2 `core/reader.py`, copyright (c) 2026 Digital
//! Agency of Japan. See ../HARUNOBU-MIT.txt. This accepts already-decoded
//! text; callers must separately determine and preserve the input encoding.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delimiter { Comma, Tab }
impl Delimiter { fn character(self) -> char { match self { Self::Comma => ',', Self::Tab => '\t' } } }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind { Multiline, UnclosedQuote, UnquotedQuote, FieldCountMismatch }
impl IssueKind { fn index(self) -> usize { match self { Self::Multiline => 0, Self::UnclosedQuote => 1, Self::UnquotedQuote => 2, Self::FieldCountMismatch => 3 } } }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub kind: IssueKind,
    /// Logical row and optional column, both one-based.
    pub row: usize,
    pub column: Option<usize>,
    pub physical_start: usize,
    pub physical_end: usize,
    pub fields: Option<usize>,
    pub expected_fields: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostics {
    /// Counts are ordered as Multiline, UnclosedQuote, UnquotedQuote, FieldCountMismatch.
    pub counts: [usize; 4],
    pub samples: Vec<Issue>,
    pub expected_fields: Option<usize>,
    pub total_records: usize,
    pub truncated: bool,
    pub confidence: f64,
}

impl Diagnostics {
    pub fn count(&self, kind: IssueKind) -> usize { self.counts[kind.index()] }
}

const MAX_SAMPLES: usize = 100;
const MAX_SAMPLES_PER_TYPE: usize = 50;
const MAX_ISSUES: usize = 1_000;

struct Scanner {
    out: Diagnostics,
    sample_counts: [usize; 4],
    issue_total: usize,
    row: usize,
    physical_line: usize,
    record_start: usize,
    fields: usize,
    field_start: bool,
    in_quotes: bool,
    after_quote: bool,
    field_raw: bool,
    field_unquoted_quote: bool,
    record_raw: bool,
    record_ends_newline: bool,
    unquoted_count: usize,
    unquoted_columns: Vec<usize>,
}

impl Scanner {
    fn new() -> Self {
        Self { out: Diagnostics { counts: [0; 4], samples: Vec::new(), expected_fields: None, total_records: 0, truncated: false, confidence: 1.0 },
            sample_counts: [0; 4], issue_total: 0, row: 1, physical_line: 1, record_start: 1, fields: 0,
            field_start: true, in_quotes: false, after_quote: false, field_raw: false, field_unquoted_quote: false,
            record_raw: false, record_ends_newline: false, unquoted_count: 0, unquoted_columns: Vec::new() }
    }
    fn finish_field(&mut self) {
        self.fields += 1;
        if self.field_unquoted_quote {
            self.unquoted_count += 1;
            if self.unquoted_columns.len() < MAX_SAMPLES_PER_TYPE { self.unquoted_columns.push(self.fields); }
        }
        self.field_start = true;
        self.after_quote = false;
        self.field_raw = false;
        self.field_unquoted_quote = false;
    }
    fn add_issue(&mut self, issue: Issue) {
        let index = issue.kind.index();
        self.out.counts[index] += 1;
        self.issue_total += 1;
        if self.out.samples.len() < MAX_SAMPLES && self.sample_counts[index] < MAX_SAMPLES_PER_TYPE {
            self.out.samples.push(issue);
            self.sample_counts[index] += 1;
        }
        if self.issue_total > MAX_ISSUES { self.out.truncated = true; self.out.confidence = 0.85; }
    }
    fn finish_record(&mut self, unclosed: bool) {
        if self.out.truncated { return; }
        self.finish_field();
        if self.row == 1 { self.out.expected_fields = Some(self.fields); }
        let end = if unclosed && self.record_ends_newline { self.physical_line.saturating_sub(1).max(self.record_start) } else { self.physical_line };
        let row = self.row;
        let record_start = self.record_start;
        let template = |kind, column, fields, expected_fields| Issue { kind, row, column,
            physical_start: record_start, physical_end: end, fields, expected_fields };
        if unclosed {
            self.add_issue(template(IssueKind::UnclosedQuote, None, Some(self.fields), self.out.expected_fields));
        } else if end > self.record_start {
            self.add_issue(template(IssueKind::Multiline, None, Some(self.fields), self.out.expected_fields));
        }
        let columns = std::mem::take(&mut self.unquoted_columns);
        for i in 0..self.unquoted_count {
            // Only the first 50 can become samples; later column values are not retained.
            let col = columns.get(i).copied();
            self.add_issue(template(IssueKind::UnquotedQuote, col, None, None));
        }
        if self.row > 1 && self.out.expected_fields.is_some_and(|expected| self.fields != expected) {
            self.add_issue(template(IssueKind::FieldCountMismatch, None, Some(self.fields), self.out.expected_fields));
        }
        self.out.total_records = self.row;
        self.row += 1;
        self.fields = 0;
        self.unquoted_count = 0;
        self.record_raw = false;
        self.record_ends_newline = false;
    }
}

/// Inspect one complete decoded document. Counts and samples use the same
/// limits as the pinned Harunobu reader, without retaining cell contents.
pub fn diagnose(text: &str, delimiter: Delimiter) -> Diagnostics {
    let mut state = Scanner::new();
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(ch) = chars.next() {
        state.record_raw = true;
        state.record_ends_newline = matches!(ch, '\r' | '\n');
        if state.out.truncated { continue; }
        if state.in_quotes {
            state.field_raw = true;
            if ch == '"' {
                if chars.peek() == Some(&'"') { chars.next(); state.record_ends_newline = false; }
                else { state.in_quotes = false; state.after_quote = true; }
            } else if ch == '\r' || ch == '\n' {
                state.physical_line += 1;
                if ch == '\r' && chars.peek() == Some(&'\n') { chars.next(); }
            }
            continue;
        }
        if ch == delimiter.character() { state.finish_field(); }
        else if ch == '\n' || ch == '\r' {
            state.finish_record(false);
            state.physical_line += 1;
            state.record_start = state.physical_line;
            if ch == '\r' && chars.peek() == Some(&'\n') { chars.next(); }
        } else if ch == '"' && state.field_start {
            state.field_raw = true;
            state.in_quotes = true;
            state.field_start = false;
        } else if ch == '"' {
            state.field_raw = true;
            state.field_unquoted_quote = true;
            state.field_start = false;
        } else {
            state.field_raw = true;
            // Python's str.isspace includes four ASCII information separators
            // which Rust's Unicode whitespace predicate omits.
            let python_space = ch.is_whitespace() || matches!(ch, '\u{1c}'..='\u{1f}');
            if state.after_quote && !python_space { state.field_unquoted_quote = true; }
            state.field_start = false;
        }
    }
    if state.in_quotes { state.finish_record(true); }
    else if state.field_raw || state.fields > 0 || state.record_raw { state.finish_record(false); }
    state.out
}

fn column_letters(mut column: usize) -> String {
    let mut letters = Vec::new();
    while column > 0 { column -= 1; letters.push((b'A' + (column % 26) as u8) as char); column /= 26; }
    letters.into_iter().rev().collect()
}

fn check_diagnostics(diag: Option<&Diagnostics>, first_row: usize, last_row: usize, quoting: bool) -> crate::machine_readability_rules::RuleEvaluation {
    use crate::machine_readability_rules::{Finding, FindingSeverity, RuleEvaluation};
    use crate::machine_readability_scoring::{RuleResult, Severity};
    let id = if quoting { "L1-15" } else { "L1-14" };
    let skipped = || RuleEvaluation { result: RuleResult { rule_id: id.to_string(), passed: true, confidence: 0.0, severity: Some(Severity::Fatal) }, findings: Vec::new() };
    let Some(diag) = diag else { return skipped(); };
    if first_row == 0 || first_row > last_row { return skipped(); }
    let relevant = |kind| if quoting { matches!(kind, IssueKind::UnquotedQuote | IssueKind::UnclosedQuote | IssueKind::FieldCountMismatch) }
        else { matches!(kind, IssueKind::Multiline | IssueKind::UnclosedQuote) };
    let kinds = [IssueKind::Multiline, IssueKind::UnclosedQuote, IssueKind::UnquotedQuote, IssueKind::FieldCountMismatch];
    let total: usize = kinds.into_iter().filter(|kind| relevant(*kind)).map(|kind| diag.count(kind)).fold(0, usize::saturating_add);
    let findings = diag.samples.iter().filter(|s| relevant(s.kind) && s.row >= first_row && s.row <= last_row).map(|sample| {
        let location = if sample.kind == IssueKind::UnquotedQuote {
            sample.column.map(|col| format!("{}{}", column_letters(col), sample.row)).unwrap_or_else(|| format!("{}:{}", sample.row, sample.row))
        } else { format!("{}:{}", sample.row, sample.row) };
        let severity = if sample.kind == IssueKind::FieldCountMismatch { FindingSeverity::Warning } else { FindingSeverity::Error };
        Finding { location, severity }
    }).collect();
    RuleEvaluation { result: RuleResult { rule_id: id.to_string(), passed: total == 0, confidence: diag.confidence, severity: Some(Severity::Fatal) }, findings }
}

/// L1-14 for a detected table's inclusive, one-based row range. Missing raw
/// diagnostics are skipped; a truncated scan retains the reference confidence.
pub fn single_line_records(diag: Option<&Diagnostics>, first_row: usize, last_row: usize) -> crate::machine_readability_rules::RuleEvaluation {
    check_diagnostics(diag, first_row, last_row, false)
}

/// L1-15 for a detected table's inclusive, one-based row range.
pub fn valid_quoting(diag: Option<&Diagnostics>, first_row: usize, last_row: usize) -> crate::machine_readability_rules::RuleEvaluation {
    check_diagnostics(diag, first_row, last_row, true)
}

/// A decoded CSV/TSV sheet as Harunobu's byte-stream reader sees it. Values
/// remain strings, including values that resemble numbers, dates or formulas.
/// Empty fields are represented as `None`; a blank line is an empty record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvSheet { pub records: Vec<Vec<Option<String>>>, pub max_columns: usize }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsvReadError { UnquotedCarriageReturn, FieldTooLong, TooManyRows, TooManyColumns }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReaderState { RecordStart, FieldStart, Unquoted, Quoted, AfterQuote }

/// Parse a decoded string with the pinned Harunobu reader's `csv.reader`
/// dialect and `io.StringIO` newline behavior. This is separate from the app's
/// CSV import, which intentionally coerces types. Encoding selection belongs
/// to the byte reader, not this function.
pub fn read_sheet(text: &str, delimiter: Delimiter) -> Result<CsvSheet, CsvReadError> {
    const MAX_FIELD: usize = 131_072; // CPython's default csv.field_size_limit
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut field_chars = 0usize;
    let mut state = ReaderState::RecordStart;
    let mut input = text.chars().peekable();
    let mut max_columns = 0usize;
    let append = |field: &mut String, count: &mut usize, c: char| -> Result<(), CsvReadError> {
        *count += 1;
        if *count > MAX_FIELD { return Err(CsvReadError::FieldTooLong); }
        field.push(c);
        Ok(())
    };
    let finish_field = |record: &mut Vec<Option<String>>, field: &mut String, count: &mut usize| -> Result<(), CsvReadError> {
        if record.len() >= u32::MAX as usize { return Err(CsvReadError::TooManyColumns); }
        record.push(if field.is_empty() { None } else { Some(std::mem::take(field)) });
        *count = 0;
        Ok(())
    };
    while let Some(ch) = input.next() {
        if state == ReaderState::Quoted {
            if ch == '"' { state = ReaderState::AfterQuote; }
            else { append(&mut field, &mut field_chars, ch)?; }
            continue;
        }
        if state == ReaderState::AfterQuote && ch == '"' {
            append(&mut field, &mut field_chars, '"')?;
            state = ReaderState::Quoted;
            continue;
        }
        if ch == delimiter.character() {
            finish_field(&mut record, &mut field, &mut field_chars)?;
            state = ReaderState::FieldStart;
        } else if ch == '\r' || ch == '\n' {
            if ch == '\r' {
                while input.peek() == Some(&'\r') { input.next(); }
                if input.peek() == Some(&'\n') { input.next(); }
                else if input.peek().is_some() { return Err(CsvReadError::UnquotedCarriageReturn); }
            }
            if state != ReaderState::RecordStart { finish_field(&mut record, &mut field, &mut field_chars)?; }
            if records.len() >= u32::MAX as usize { return Err(CsvReadError::TooManyRows); }
            max_columns = max_columns.max(record.len());
            records.push(std::mem::take(&mut record));
            state = ReaderState::RecordStart;
        } else if ch == '"' && matches!(state, ReaderState::RecordStart | ReaderState::FieldStart) {
            state = ReaderState::Quoted;
        } else {
            append(&mut field, &mut field_chars, ch)?;
            state = ReaderState::Unquoted;
        }
    }
    if state != ReaderState::RecordStart {
        finish_field(&mut record, &mut field, &mut field_chars)?;
        if records.len() >= u32::MAX as usize { return Err(CsvReadError::TooManyRows); }
        max_columns = max_columns.max(record.len());
        records.push(record);
    }
    Ok(CsvSheet { records, max_columns })
}

impl CsvSheet {
    /// Prepare unformatted, unmerged cells for preliminary layout detection.
    /// Empty fields have no layout effect; their record/column positions remain
    /// available in `records` and `max_columns`.
    pub fn layout_cells(&self) -> Result<Vec<crate::machine_readability_layout::LayoutCell>, CsvReadError> {
        use crate::machine_readability_layout::{LayoutCell, LayoutValue};
        let mut cells = Vec::new();
        for (row, record) in self.records.iter().enumerate() {
            for (column, value) in record.iter().enumerate() {
                if let Some(value) = value {
                    cells.push(LayoutCell { row: u32::try_from(row).map_err(|_| CsvReadError::TooManyRows)?, column: u32::try_from(column).map_err(|_| CsvReadError::TooManyColumns)?, value: Some(LayoutValue::Text(value.clone())), bold: false, merged: false, formula: None });
                }
            }
        }
        Ok(cells)
    }
}
