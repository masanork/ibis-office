// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The M code of a query, as the 詳細エディター shows it: each step a line of
//! `let`, named as Power Query names the steps.
use super::{Agg, Clause, JoinKind, Op, Query, SplitBy, Source, Step};

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn list(items: &[String]) -> String {
    format!("{{{}}}", items.iter().map(|s| q(s)).collect::<Vec<_>>().join(", "))
}

/// A name used as a step: `#"Changed Type"` unless it is a plain identifier.
fn id(name: &str) -> String {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        name.to_string()
    } else {
        format!("#\"{}\"", name.replace('"', "\"\""))
    }
}

/// A value typed in a step, as an M literal: a number stays one.
fn lit(s: &str) -> String {
    if s.trim().parse::<f64>().is_ok() { s.trim().to_string() } else { q(s) }
}

fn agg_m(a: Agg) -> &'static str {
    match a {
        Agg::Sum => "List.Sum",
        Agg::Average => "List.Average",
        Agg::Min => "List.Min",
        Agg::Max => "List.Max",
        Agg::Count => "List.Count",
        Agg::CountDistinct => "each List.Count(List.Distinct(_))",
        Agg::Median => "List.Median",
        Agg::First => "List.First",
    }
}

fn cond_m(column: &str, op: Op, value: &str) -> String {
    let c = format!("[{column}]");
    match op {
        Op::Equals => format!("{c} = {}", lit(value)),
        Op::NotEquals => format!("{c} <> {}", lit(value)),
        Op::Greater => format!("{c} > {}", lit(value)),
        Op::GreaterOrEqual => format!("{c} >= {}", lit(value)),
        Op::Less => format!("{c} < {}", lit(value)),
        Op::LessOrEqual => format!("{c} <= {}", lit(value)),
        Op::Contains => format!("Text.Contains({c}, {})", q(value)),
        Op::DoesNotContain => format!("not Text.Contains({c}, {})", q(value)),
        Op::BeginsWith => format!("Text.StartsWith({c}, {})", q(value)),
        Op::EndsWith => format!("Text.EndsWith({c}, {})", q(value)),
        Op::IsNull => format!("{c} = null"),
        Op::IsNotNull => format!("{c} <> null"),
    }
}

/// A step's name and the M that makes it from `prev`.
pub fn step_m(step: &Step, prev: &str) -> (String, String) {
    let p = id(prev);
    match step {
        Step::PromoteHeaders => ("Promoted Headers".into(), format!("Table.PromoteHeaders({p}, [PromoteAllScalars=true])")),
        Step::DemoteHeaders => ("Demoted Headers".into(), format!("Table.DemoteHeaders({p})")),
        Step::ChangeType { column, ty, culture } => {
            let locale = if culture.name().is_empty() { String::new() } else { format!(", {}", q(culture.name())) };
            ("Changed Type".into(), format!("Table.TransformColumnTypes({p}, {{{{{}, {}}}}}{locale})", q(column), ty.m_name()))
        }
        Step::SelectColumns(c) => ("Removed Other Columns".into(), format!("Table.SelectColumns({p}, {})", list(c))),
        Step::RemoveColumns(c) => ("Removed Columns".into(), format!("Table.RemoveColumns({p}, {})", list(c))),
        Step::RenameColumn { from, to } => ("Renamed Columns".into(), format!("Table.RenameColumns({p}, {{{{{}, {}}}}})", q(from), q(to))),
        Step::ReorderColumns(c) => ("Reordered Columns".into(), format!("Table.ReorderColumns({p}, {})", list(c))),
        Step::FilterRows { column, op, value } => ("Filtered Rows".into(), format!("Table.SelectRows({p}, each {})", cond_m(column, *op, value))),
        Step::KeepTopRows(n) => ("Kept First Rows".into(), format!("Table.FirstN({p}, {n})")),
        Step::KeepBottomRows(n) => ("Kept Last Rows".into(), format!("Table.LastN({p}, {n})")),
        Step::RemoveTopRows(n) => ("Removed Top Rows".into(), format!("Table.Skip({p}, {n})")),
        Step::RemoveBottomRows(n) => ("Removed Bottom Rows".into(), format!("Table.RemoveLastN({p}, {n})")),
        Step::RemoveDuplicates(c) => ("Removed Duplicates".into(), if c.is_empty() { format!("Table.Distinct({p})") } else { format!("Table.Distinct({p}, {})", list(c)) }),
        Step::RemoveBlankRows => ("Removed Blank Rows".into(), format!("Table.SelectRows({p}, each not List.IsEmpty(List.RemoveMatchingItems(Record.FieldValues(_), {{\"\", null}})))")),
        Step::RemoveErrors(c) => ("Removed Errors".into(), if c.is_empty() { format!("Table.RemoveRowsWithErrors({p})") } else { format!("Table.RemoveRowsWithErrors({p}, {})", list(c)) }),
        Step::ReplaceErrors { columns, with } => ("Replaced Errors".into(), format!("Table.ReplaceErrorValues({p}, {{{}}})", columns.iter().map(|c| format!("{{{}, {}}}", q(c), lit(with))).collect::<Vec<_>>().join(", "))),
        Step::Sort(keys) => ("Sorted Rows".into(), format!("Table.Sort({p}, {{{}}})", keys.iter().map(|(c, asc)| format!("{{{}, Order.{}}}", q(c), if *asc { "Ascending" } else { "Descending" })).collect::<Vec<_>>().join(", "))),
        Step::SplitColumn { column, by } => {
            let splitter = match by {
                SplitBy::Delimiter(d) => format!("Splitter.SplitTextByDelimiter({}, QuoteStyle.Csv)", q(d)),
                SplitBy::Length(n) => format!("Splitter.SplitTextByRepeatedLengths({n})"),
            };
            ("Split Column".into(), format!("Table.SplitColumn({p}, {}, {splitter})", q(column)))
        }
        Step::MergeColumns { columns, separator, name } => ("Merged Columns".into(), format!("Table.CombineColumns({p}, {}, Combiner.CombineTextByDelimiter({}, QuoteStyle.None), {})", list(columns), q(separator), q(name))),
        Step::ReplaceValue { column, find, with, whole_cell } => (
            "Replaced Value".into(),
            format!("Table.ReplaceValue({p}, {}, {}, Replacer.{}, {{{}}})", lit(find), lit(with), if *whole_cell { "ReplaceValue" } else { "ReplaceText" }, q(column)),
        ),
        Step::ReplaceNull { columns, with } => ("Replaced Value".into(), format!("Table.ReplaceValue({p}, null, {}, Replacer.ReplaceValue, {})", lit(with), list(columns))),
        Step::FillDown(c) => ("Filled Down".into(), format!("Table.FillDown({p}, {})", list(c))),
        Step::FillUp(c) => ("Filled Up".into(), format!("Table.FillUp({p}, {})", list(c))),
        Step::Transpose => ("Transposed Table".into(), format!("Table.Transpose({p})")),
        Step::ReverseRows => ("Reversed Rows".into(), format!("Table.ReverseRows({p})")),
        Step::Trim(c) => ("Trimmed Text".into(), format!("Table.TransformColumns({p}, {{{}}})", c.iter().map(|c| format!("{{{}, Text.Trim, type text}}", q(c))).collect::<Vec<_>>().join(", "))),
        Step::Upper(c) => ("Uppercased Text".into(), format!("Table.TransformColumns({p}, {{{}}})", c.iter().map(|c| format!("{{{}, Text.Upper, type text}}", q(c))).collect::<Vec<_>>().join(", "))),
        Step::Lower(c) => ("Lowercased Text".into(), format!("Table.TransformColumns({p}, {{{}}})", c.iter().map(|c| format!("{{{}, Text.Lower, type text}}", q(c))).collect::<Vec<_>>().join(", "))),
        Step::Pivot { attribute, values, agg } => (
            "Pivoted Column".into(),
            format!("Table.Pivot({p}, List.Distinct({p}[{attribute}]), {}, {}, {})", q(attribute), q(values), agg_m(*agg)),
        ),
        Step::Unpivot { columns, attribute, value } => ("Unpivoted Columns".into(), format!("Table.UnpivotOtherColumns({p}, List.Difference(Table.ColumnNames({p}), {}), {}, {})", list(columns), q(attribute), q(value))),
        Step::UnpivotOther { keep, attribute, value } => ("Unpivoted Other Columns".into(), format!("Table.UnpivotOtherColumns({p}, {}, {}, {})", list(keep), q(attribute), q(value))),
        Step::GroupBy { keys, aggregations } => (
            "Grouped Rows".into(),
            format!(
                "Table.Group({p}, {}, {{{}}})",
                list(keys),
                aggregations
                    .iter()
                    .map(|(name, agg, col)| match (agg, col) {
                        (Agg::Count, _) => format!("{{{}, each Table.RowCount(_), Int64.Type}}", q(name)),
                        (_, Some(c)) => format!("{{{}, each {}([{c}]), type nullable number}}", q(name), agg_m(*agg).trim_start_matches("each ")),
                        _ => format!("{{{}, each Table.RowCount(_), Int64.Type}}", q(name)),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        Step::Merge { right, left_keys, right_keys, kind, expand } => {
            let k = match kind {
                JoinKind::LeftOuter => "LeftOuter",
                JoinKind::RightOuter => "RightOuter",
                JoinKind::FullOuter => "FullOuter",
                JoinKind::Inner => "Inner",
                JoinKind::LeftAnti => "LeftAnti",
                JoinKind::RightAnti => "RightAnti",
            };
            (
                "Merged Queries".into(),
                format!("Table.ExpandTableColumn(Table.NestedJoin({p}, {}, {}, {}, {}, JoinKind.{k}), {}, {})", list(left_keys), id(right), list(right_keys), q(right), q(right), list(expand)),
            )
        }
        Step::Append { other } => ("Appended Query".into(), format!("Table.Combine({{{p}, {}}})", id(other))),
        Step::AddColumn { name, formula } => ("Added Custom".into(), format!("Table.AddColumn({p}, {}, each {formula})", q(name))),
        Step::AddConditionalColumn { name, clauses, otherwise } => {
            let mut text = String::new();
            for Clause { column, op, value, output } in clauses {
                text.push_str(&format!("if {} then {} else ", cond_m(column, *op, value), lit(output)));
            }
            text.push_str(&lit(otherwise));
            ("Added Conditional Column".into(), format!("Table.AddColumn({p}, {}, each {text})", q(name)))
        }
        Step::AddIndexColumn { name, start, step } => ("Added Index".into(), format!("Table.AddIndexColumn({p}, {}, {start}, {step}, Int64.Type)", q(name))),
    }
}

/// The M code of `query`: `let Source = …, step = …  in step`.
pub fn query_to_m(query: &Query) -> String {
    let source = match &query.source {
        Source::Sqlite { .. } => "Ibis.SqliteSource()".to_string(),
        Source::Avro { .. } => "Ibis.AvroSource()".to_string(),
        Source::ArrowIpc { .. } => "Ibis.ArrowIpcSource()".to_string(),
        Source::JsonLines { .. } => "Ibis.JsonLinesSource()".to_string(),
        Source::Parquet { .. } => "Ibis.ParquetSource()".to_string(),
        Source::Blank => "#table({}, {})".to_string(),
        Source::Table(name) => format!("Excel.CurrentWorkbook(){{[Name={}]}}[Content]", q(name)),
        Source::Range { sheet, range } => format!("Excel.CurrentWorkbook(){{[Name={}]}}[Content] /* {range} */", q(sheet)),
    };
    let mut lines = vec![format!("    Source = {source}")];
    let mut prev = "Source".to_string();
    let mut used = vec!["Source".to_string()];
    for step in &query.steps {
        let (base, body) = step_m(step, &prev);
        let name = if used.contains(&base) { (1..).map(|i| format!("{base}{i}")).find(|n| !used.contains(n)).unwrap_or(base.clone()) } else { base };
        lines.push(format!("    {} = {body}", id(&name)));
        used.push(name.clone());
        prev = name;
    }
    format!("let\n{}\nin\n    {}", lines.join(",\n"), id(&prev))
}
