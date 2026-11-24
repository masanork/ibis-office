// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::BTreeSet;

use ibis_doc_model::{CellValue, Presentation, cell_name};
use serde_json::{Value, json};

use crate::chart_cohort::{report_comparison_definition, report_table};

#[derive(Clone, Copy)]
enum Observation {
    Number(f64),
    Missing,
    Invalid,
}

impl Observation {
    fn read(cell: Option<&ibis_doc_model::SheetCell>) -> Self {
        match cell.map(|cell| &cell.value) {
            Some(CellValue::Number(value)) if value.is_finite() => Self::Number(*value),
            None | Some(CellValue::Empty) => Self::Missing,
            Some(CellValue::Text(text)) if text.is_empty() => Self::Missing,
            _ => Self::Invalid,
        }
    }

    fn value(self) -> Value {
        match self {
            Self::Number(value) => json!(value),
            _ => Value::Null,
        }
    }
}

fn rate(numerator: f64, denominator: f64) -> Option<f64> {
    let value = numerator / denominator;
    (value.is_finite() && denominator > 0.).then_some(value)
}

fn percentage_point_delta(before: Option<f64>, after: Option<f64>) -> Option<f64> {
    let (before, after) = before.zip(after)?;
    let delta = (after - before) * 100.;
    delta.is_finite().then_some(delta)
}

#[derive(Default)]
struct RateTotals {
    numerator: f64,
    denominator: f64,
    used: usize,
    missing: usize,
    zero_denominator: usize,
    invalid: usize,
}

impl RateTotals {
    fn observe(&mut self, numerator: Observation, denominator: Observation) -> &'static str {
        use Observation::*;
        match (numerator, denominator) {
            (Invalid, _) | (_, Invalid) => {
                self.invalid += 1;
                "invalid"
            }
            (Missing, _) | (_, Missing) => {
                self.missing += 1;
                "missing"
            }
            (Number(_), Number(denominator)) if denominator == 0. => {
                self.zero_denominator += 1;
                "zero_denominator"
            }
            (Number(numerator), Number(denominator)) if denominator > 0. => {
                self.numerator += numerator;
                self.denominator += denominator;
                self.used += 1;
                "ready"
            }
            _ => {
                self.invalid += 1;
                "invalid"
            }
        }
    }

    fn rate(&self) -> Option<f64> {
        (self.numerator.is_finite() && self.denominator.is_finite())
            .then(|| rate(self.numerator, self.denominator)).flatten()
    }

    fn details(&self) -> Value {
        let state = if self.used == 0 {
            "no_eligible_rows"
        } else if self.rate().is_some() {
            "ready"
        } else {
            "non_finite"
        };
        json!({"state":state,"numerator":self.numerator,"denominator":self.denominator,"rate":self.rate(),
            "counts":{"included":self.used,"missing":self.missing,"zero_denominator":self.zero_denominator,"invalid":self.invalid}})
    }
}

struct Side {
    totals: RateTotals,
    evidence: Value,
    source: Value,
}

fn report_side(workbook: &Presentation, name: &str, metric: &str, key: Option<&str>) -> Result<Side, &'static str> {
    let definition = report_comparison_definition(workbook, name)?;
    if definition["kind"] != "rate" || definition["metric"].as_str() != Some(metric) {
        return Err("unknown_metric");
    }
    let (page, sheet, table) = report_table(workbook, name)?;
    let range = table.range;
    let column = |field: &str| {
        let name = definition[field].as_str().ok_or("invalid_definition")?;
        let mut columns = table.columns.iter().enumerate().filter(|(_, column)| column.as_str() == name);
        let (index, _) = columns.next().ok_or("invalid_table")?;
        let column = range.first_column + index as u32;
        if column > range.last_column || columns.next().is_some() { return Err("invalid_table"); }
        Ok(column)
    };
    let key_column = column("key_column")?;
    let numerator_column = column("numerator_column")?;
    let denominator_column = column("denominator_column")?;
    if table.header_rows != 1 || key_column == numerator_column || key_column == denominator_column || numerator_column == denominator_column {
        return Err("invalid_table");
    }
    if range.last_row.saturating_sub(range.first_row) > 100_000 {
        return Err("too_many_rows");
    }
    let mut keys = BTreeSet::new();
    let mut totals = RateTotals::default();
    let mut evidence = json!({"state":"absent"});
    for row in range.first_row + 1..=range.last_row.saturating_sub(table.totals_rows) {
        let Some(CellValue::Text(label)) = sheet.cell(row, key_column).map(|cell| &cell.value) else {
            return Err("invalid_key");
        };
        if label.trim().is_empty() { return Err("invalid_key"); }
        if !keys.insert(label.as_str()) { return Err("duplicate_key"); }
        let numerator = Observation::read(sheet.cell(row, numerator_column));
        let denominator = Observation::read(sheet.cell(row, denominator_column));
        let state = totals.observe(numerator, denominator);
        if key == Some(label.as_str()) {
            let value = match (numerator, denominator) {
                (Observation::Number(n), Observation::Number(d)) => rate(n, d),
                _ => None,
            };
            evidence = json!({
                "state":if state == "ready" && value.is_none() {"non_finite"} else {state},
                "sheet":page.name, "key_cell":cell_name(key_column, row), "rate":value,
                "numerator":{"cell":cell_name(numerator_column,row),"value":numerator.value()},
                "denominator":{"cell":cell_name(denominator_column,row),"value":denominator.value()},
            });
        }
    }
    let source = json!({
        "sheet":page.name,"table":table.name,
        "key_column":definition["key_column"],
        "numerator_column":definition["numerator_column"],
        "denominator_column":definition["denominator_column"],
    });
    Ok(Side { totals, evidence, source })
}

pub(crate) fn compare(base: &Presentation, candidate: &Presentation, table: &str, metric: &str, key: Option<&str>) -> Value {
    if report_comparison_definition(base, table) != report_comparison_definition(candidate, table) {
        return json!({"state":"comparison_changed"});
    }
    let before = match report_side(base, table, metric, key) {
        Ok(side) => side,
        Err(reason) => return json!({"state":reason,"side":"before"}),
    };
    let after = match report_side(candidate, table, metric, key) {
        Ok(side) => side,
        Err(reason) => return json!({"state":reason,"side":"after"}),
    };
    let delta = percentage_point_delta(before.totals.rate(), after.totals.rate());
    let mut result = json!({"state":"review","table":table,"metric":metric,"kind":"rate",
        "before":before.totals.details(),"after":after.totals.details(),"percentage_point_delta":delta,
        "source":{"before":before.source,"after":after.source}});
    if let Some(key) = key {
        let delta = percentage_point_delta(before.evidence["rate"].as_f64(), after.evidence["rate"].as_f64());
        result["contributor"] = json!({"key":key,"before":before.evidence,"after":after.evidence,"percentage_point_delta":delta});
    }
    result
}

pub(crate) fn applied(workbook: &Presentation, table: &str, metric: &str, key: &str) -> Value {
    match report_side(workbook, table, metric, Some(key)) {
        Ok(side) => json!({"state":side.evidence["state"],"kind":"rate","table":table,"metric":metric,"key":key,"evidence":side.evidence}),
        Err(reason) => json!({"state":reason}),
    }
}
