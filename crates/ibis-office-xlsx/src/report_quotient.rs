// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::BTreeMap;

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

#[derive(Clone, Copy)]
enum Metric {
    Rate,
    Average,
}

impl Metric {
    fn from_definition(definition: &Value) -> Option<Self> {
        match definition["kind"].as_str()? {
            "rate" => Some(Self::Rate),
            "average" => Some(Self::Average),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self { Self::Rate => "rate", Self::Average => "average" }
    }

    fn fields(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::Rate => ("numerator_column", "denominator_column", "numerator", "denominator"),
            Self::Average => ("sum_column", "count_column", "sum", "count"),
        }
    }

    fn zero_state(self) -> &'static str {
        match self { Self::Rate => "zero_denominator", Self::Average => "zero_count" }
    }

    fn delta(self, before: Option<f64>, after: Option<f64>) -> (&'static str, Option<f64>) {
        let (key, scale) = match self {
            Self::Rate => ("percentage_point_delta", 100.),
            Self::Average => ("delta", 1.),
        };
        let delta = before.zip(after).map(|(before, after)| (after - before) * scale)
            .filter(|value| value.is_finite());
        (key, delta)
    }
}

pub(crate) fn supports(definition: &Value) -> bool {
    Metric::from_definition(definition).is_some()
}

const MAX_EXACT_COUNT: u128 = 1 << 53;

fn quotient(numerator: f64, denominator: f64) -> Option<f64> {
    let value = numerator / denominator;
    (value.is_finite() && denominator > 0.).then_some(value)
}

#[derive(Default)]
struct QuotientTotals {
    numerator: f64,
    denominator: f64,
    exact_count: u128,
    used: usize,
    missing: usize,
    zero_denominator: usize,
    invalid: usize,
}

impl QuotientTotals {
    fn observe(&mut self, metric: Metric, numerator: Observation, denominator: Observation) -> &'static str {
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
                metric.zero_state()
            }
            (Number(numerator), Number(denominator)) if denominator > 0. && (matches!(metric, Metric::Rate)
                || (denominator.fract() == 0. && denominator <= MAX_EXACT_COUNT as f64)) => {
                if matches!(metric, Metric::Average) {
                    self.exact_count += denominator as u128;
                }
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

    fn value(&self, metric: Metric) -> Option<f64> {
        if matches!(metric, Metric::Average) && self.exact_count > MAX_EXACT_COUNT {
            return None;
        }
        (self.numerator.is_finite() && self.denominator.is_finite())
            .then(|| quotient(self.numerator, self.denominator)).flatten()
    }

    fn details(&self, metric: Metric) -> Value {
        let state = if self.used == 0 {
            "no_eligible_rows"
        } else if matches!(metric, Metric::Average) && self.exact_count > MAX_EXACT_COUNT {
            "count_out_of_range"
        } else if self.value(metric).is_some() {
            "ready"
        } else {
            "non_finite"
        };
        let (_, _, left, right) = metric.fields();
        let mut result = json!({"state":state,
            "counts":{"included":self.used,"missing":self.missing,(metric.zero_state()):self.zero_denominator,"invalid":self.invalid}});
        result[left] = json!(self.numerator);
        result[right] = match metric {
            Metric::Average => json!(u64::try_from(self.exact_count).ok().filter(|count| *count as u128 <= MAX_EXACT_COUNT)),
            Metric::Rate => json!(self.denominator),
        };
        result[metric.name()] = json!(self.value(metric));
        result
    }
}

#[derive(Clone, Copy)]
struct Group {
    value: f64,
    denominator: f64,
}

struct Effect {
    within: f64,
    composition: f64,
    before_share: f64,
    after_share: f64,
}

fn group_effect(before: Option<Group>, after: Option<Group>, before_total: f64, after_total: f64) -> Effect {
    let before_share = before.map_or(0., |group| group.denominator / before_total);
    let after_share = after.map_or(0., |group| group.denominator / after_total);
    // A group with an observation on only one side has no measured change
    // in value. Its entire contribution is assigned to composition.
    let before_value = before.or(after).map_or(0., |group| group.value);
    let after_value = after.or(before).map_or(0., |group| group.value);
    let mean_share = (before_share + after_share) * 0.5;
    Effect {
        within: after_value * mean_share - before_value * mean_share,
        composition: (before_value * 0.5 + after_value * 0.5) * (after_share - before_share),
        before_share,
        after_share,
    }
}

#[derive(Default)]
struct CompensatedSum {
    sum: f64,
    correction: f64,
}

impl CompensatedSum {
    fn add(&mut self, value: f64) {
        let adjusted = value - self.correction;
        let next = self.sum + adjusted;
        self.correction = (next - self.sum) - adjusted;
        self.sum = next;
    }
}

struct Side<'a> {
    metric: Metric,
    totals: QuotientTotals,
    groups: BTreeMap<&'a str, Option<Group>>,
    evidence: Value,
    source: Value,
}

fn report_side<'a>(workbook: &'a Presentation, name: &str, metric: &str, key: Option<&str>) -> Result<Side<'a>, &'static str> {
    let definition = report_comparison_definition(workbook, name)?;
    let kind = Metric::from_definition(definition).ok_or("unknown_metric")?;
    if definition["metric"].as_str() != Some(metric) {
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
    let (left_column, right_column, left, right) = kind.fields();
    let numerator_column = column(left_column)?;
    let denominator_column = column(right_column)?;
    if table.header_rows != 1 || key_column == numerator_column || key_column == denominator_column || numerator_column == denominator_column {
        return Err("invalid_table");
    }
    if range.last_row.saturating_sub(range.first_row) > 100_000 {
        return Err("too_many_rows");
    }
    let mut groups = BTreeMap::new();
    let mut totals = QuotientTotals::default();
    let mut evidence = json!({"state":"absent"});
    for row in range.first_row + 1..=range.last_row.saturating_sub(table.totals_rows) {
        let Some(CellValue::Text(label)) = sheet.cell(row, key_column).map(|cell| &cell.value) else {
            return Err("invalid_key");
        };
        if label.trim().is_empty() { return Err("invalid_key"); }
        let numerator = Observation::read(sheet.cell(row, numerator_column));
        let denominator = Observation::read(sheet.cell(row, denominator_column));
        let state = totals.observe(kind, numerator, denominator);
        let group = match (numerator, denominator) {
            (Observation::Number(n), Observation::Number(d)) if state == "ready" =>
                Some(Group { value: n / d, denominator: d }),
            _ => None,
        };
        if groups.insert(label.as_str(), group).is_some() { return Err("duplicate_key"); }
        if key == Some(label.as_str()) {
            let value = group.and_then(|group| group.value.is_finite().then_some(group.value));
            evidence = json!({
                "state":if state == "ready" && value.is_none() {"non_finite"} else {state},
                "sheet":page.name, "key_cell":cell_name(key_column, row), (kind.name()):value,
                (left):{"cell":cell_name(numerator_column,row),"value":numerator.value()},
                (right):{"cell":cell_name(denominator_column,row),"value":denominator.value()},
            });
        }
    }
    let source = json!({
        "sheet":page.name,"table":table.name,
        "key_column":definition["key_column"],
        (left_column):definition[left_column],
        (right_column):definition[right_column],
    });
    Ok(Side { metric: kind, totals, groups, evidence, source })
}

fn decomposition<'a>(before: &Side<'a>, after: &Side<'a>, delta: Option<f64>, key: Option<&str>) -> Value {
    let Some(delta) = delta else { return json!({"state":"unavailable"}); };
    let scale = match before.metric { Metric::Rate => 100., Metric::Average => 1. };
    let before_total = before.totals.denominator;
    let after_total = after.totals.denominator;
    let mut within = CompensatedSum::default();
    let mut composition = CompensatedSum::default();
    let mut total = CompensatedSum::default();
    let mut ranked = Vec::new();
    let mut magnitude = 0.;
    let mut continued = 0;
    let mut entered = 0;
    let mut exited = 0;
    let mut contributor = None;
    let mut accumulate = |label: &'a str, before: Option<Group>, after: Option<Group>| {
        match (before, after) {
            (Some(_), Some(_)) => continued += 1,
            (None, Some(_)) => entered += 1,
            (Some(_), None) => exited += 1,
            (None, None) => return,
        }
        let effect = group_effect(before, after, before_total, after_total);
        within.add(effect.within);
        composition.add(effect.composition);
        let contribution = (effect.within + effect.composition) * scale;
        total.add(contribution);
        magnitude += effect.within.abs() + effect.composition.abs();
        if key == Some(label) {
            contributor = Some(json!({
                "within_group":effect.within * scale, "composition":effect.composition * scale,
                "before_share":effect.before_share, "after_share":effect.after_share,
            }));
        }
        ranked.push((label, effect, contribution));
    };
    for (&label, &group) in &before.groups {
        accumulate(label, group, after.groups.get(label).copied().flatten());
    }
    for (&label, &group) in &after.groups {
        if !before.groups.contains_key(label) { accumulate(label, None, group); }
    }
    let within = within.sum * scale;
    let composition = composition.sum * scale;
    let residual = (within + composition) - delta;
    let group_residual = total.sum - delta;
    let tolerance = (8. * f64::EPSILON * magnitude).max(8. * f64::EPSILON * (delta.abs() / scale).max(1.)) * scale;
    if !residual.is_finite() || !group_residual.is_finite() || !tolerance.is_finite() {
        return json!({"state":"non_finite"});
    }
    let group_count = ranked.len();
    const RANKED_LIMIT: usize = 10;
    let rank = |left: &(&str, Effect, f64), right: &(&str, Effect, f64)| {
        right.2.abs().total_cmp(&left.2.abs()).then_with(|| left.0.cmp(right.0))
    };
    if group_count > RANKED_LIMIT {
        ranked.select_nth_unstable_by(RANKED_LIMIT, rank);
        ranked.truncate(RANKED_LIMIT);
    }
    ranked.sort_unstable_by(rank);
    let groups: Vec<_> = ranked.into_iter().map(|(key, effect, contribution)| json!({
        "key":key, "contribution":contribution, "within_group":effect.within * scale,
        "composition":effect.composition * scale,
        "before_share":effect.before_share, "after_share":effect.after_share,
    })).collect();
    json!({
        "state":if residual.abs() <= tolerance && group_residual.abs() <= tolerance {"ready"} else {"totals_mismatch"},
        "within_group":within,"composition":composition,"residual":residual,"tolerance":tolerance,
        "counts":{"continued":continued,"entered":entered,"exited":exited},
        "contributor":contributor,
        "ranking":{"groups":groups,"group_count":group_count,"total_contribution":total.sum,
            "residual":group_residual},
    })
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
    let kind = before.metric;
    let (delta_key, delta) = kind.delta(before.totals.value(kind), after.totals.value(kind));
    let mut result = json!({"state":"review","table":table,"metric":metric,"kind":kind.name(),
        "before":before.totals.details(kind),"after":after.totals.details(kind),(delta_key):delta,
        "source":{"before":before.source,"after":after.source},
        "decomposition":decomposition(&before, &after, delta, key)});
    if let Ok(definition) = report_comparison_definition(base, table)
        && let Some(unit) = definition.get("unit")
    {
        result["unit"] = unit.clone();
    }
    if let Some(key) = key {
        let (_, delta) = kind.delta(before.evidence[kind.name()].as_f64(), after.evidence[kind.name()].as_f64());
        result["contributor"] = json!({"key":key,"before":before.evidence,"after":after.evidence,(delta_key):delta});
    }
    result
}

pub(crate) fn applied(workbook: &Presentation, table: &str, metric: &str, key: &str) -> Value {
    match report_side(workbook, table, metric, Some(key)) {
        Ok(side) => json!({"state":side.evidence["state"],"kind":side.metric.name(),"table":table,"metric":metric,"key":key,"evidence":side.evidence}),
        Err(reason) => json!({"state":reason}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_and_exits_are_counted_only_in_composition() {
        let group = Group { value: 3., denominator: 2. };
        let entered = group_effect(None, Some(group), 4., 4.);
        assert_eq!(entered.within, 0.);
        assert_eq!(entered.composition, 1.5);
        let exited = group_effect(Some(group), None, 4., 4.);
        assert_eq!(exited.within, 0.);
        assert_eq!(exited.composition, -1.5);
    }
}
