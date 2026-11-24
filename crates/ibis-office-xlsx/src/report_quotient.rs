// SPDX-License-Identifier: AGPL-3.0-only
use std::{collections::BTreeMap, sync::Arc};

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

    fn number(self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Metric {
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

    pub(crate) fn fields(self) -> (&'static str, &'static str, &'static str, &'static str) {
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

#[derive(Clone, Copy)]
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

struct ReportRow {
    index: u32,
    state: &'static str,
    group: Option<Group>,
    observations: [Observation; 2],
}

struct Side {
    metric: Metric,
    totals: QuotientTotals,
    rows: BTreeMap<Arc<str>, ReportRow>,
    columns: [u32; 3],
    sheet: Option<String>,
    source_table: String,
    source_columns: [String; 3],
}

pub(crate) struct EvidenceCell {
    pub(crate) address: String,
    pub(crate) value: Option<f64>,
}

pub(crate) struct Evidence {
    state: &'static str,
    pub(crate) sheet: Option<String>,
    key_cell: String,
    value: Option<f64>,
    pub(crate) cells: [EvidenceCell; 2],
}

fn evidence_json(evidence: Option<&Evidence>, metric: Metric) -> Value {
    let Some(evidence) = evidence else { return json!({"state":"absent"}); };
    let (_, _, left, right) = metric.fields();
    let cell = |cell: &EvidenceCell| json!({"cell":cell.address,"value":cell.value});
    json!({"state":evidence.state,"sheet":evidence.sheet,"key_cell":evidence.key_cell,
        (metric.name()):evidence.value,(left):cell(&evidence.cells[0]),(right):cell(&evidence.cells[1])})
}

impl Side {
    fn evidence(&self, key: &str) -> Option<Evidence> {
        let row = self.rows.get(key)?;
        let [key_column, numerator, denominator] = self.columns;
        let value = row.group.and_then(|group| group.value.is_finite().then_some(group.value));
        Some(Evidence {
            state: if row.state == "ready" && value.is_none() { "non_finite" } else { row.state },
            sheet: self.sheet.clone(),
            key_cell: cell_name(key_column, row.index),
            value,
            cells: [(numerator, row.observations[0]), (denominator, row.observations[1])].map(|(column, observation)|
                EvidenceCell { address: cell_name(column, row.index), value: observation.number() }),
        })
    }
}

fn report_side(workbook: &Presentation, name: &str, metric: &str) -> Result<Side, &'static str> {
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
    let (left_column, right_column, _, _) = kind.fields();
    let numerator_column = column(left_column)?;
    let denominator_column = column(right_column)?;
    if table.header_rows != 1 || key_column == numerator_column || key_column == denominator_column || numerator_column == denominator_column {
        return Err("invalid_table");
    }
    if range.last_row.saturating_sub(range.first_row) > 100_000 {
        return Err("too_many_rows");
    }
    let mut rows = BTreeMap::new();
    let mut totals = QuotientTotals::default();
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
        if rows.insert(Arc::from(label.as_str()), ReportRow { index: row, state, group, observations: [numerator, denominator] }).is_some() {
            return Err("duplicate_key");
        }
    }
    let source_columns = ["key_column", left_column, right_column].map(|key| definition[key].as_str().unwrap_or_default().to_owned());
    Ok(Side { metric: kind, totals, rows, columns: [key_column, numerator_column, denominator_column],
        sheet: page.name.clone(), source_table: table.name.clone(), source_columns })
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RankingDirection {
    #[default]
    All,
    Increase,
    Decrease,
}

impl RankingDirection {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "increase" => Some(Self::Increase),
            "decrease" => Some(Self::Decrease),
            _ => None,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct RankingFilter {
    pub(crate) direction: RankingDirection,
    pub(crate) query: String,
}

impl RankingFilter {
    pub(crate) fn active(&self) -> bool {
        self.direction != RankingDirection::All || !self.query.trim().is_empty()
    }
}

const RANKED_LIMIT: usize = 10;
const REMAINDER_PAGE_SIZE: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExclusionReason {
    Direction,
    Name,
    Both,
}

impl ExclusionReason {
    pub(crate) const ALL: [Self; 3] = [Self::Direction, Self::Name, Self::Both];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Direction => "direction",
            Self::Name => "name",
            Self::Both => "both",
        }
    }

    fn from_conditions(direction: bool, name: bool) -> Option<Self> {
        match (direction, name) {
            (true, false) => Some(Self::Direction),
            (false, true) => Some(Self::Name),
            (true, true) => Some(Self::Both),
            (false, false) => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum RankingPage {
    Remainder(usize),
    Excluded(usize, Option<ExclusionReason>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckState { Ready, Mismatch, NonFinite }

impl CheckState {
    fn check(tolerance: f64, residuals: &[f64]) -> Self {
        if residuals.iter().any(|value| !value.is_finite()) { Self::NonFinite }
        else if residuals.iter().any(|value| value.abs() > tolerance) { Self::Mismatch }
        else { Self::Ready }
    }

    fn wire(self) -> &'static str {
        match self { Self::Ready => "ready", Self::Mismatch => "totals_mismatch", Self::NonFinite => "non_finite" }
    }
}

pub(crate) struct RankedRow {
    pub(crate) key: String,
    pub(crate) contribution: f64,
    pub(crate) within_group: f64,
    pub(crate) composition: f64,
    before_share: f64,
    after_share: f64,
    pub(crate) before: Option<Evidence>,
    pub(crate) after: Option<Evidence>,
    pub(crate) reason: Option<ExclusionReason>,
}

pub(crate) struct RankPage {
    pub(crate) offset: usize,
    pub(crate) groups: Vec<RankedRow>,
    pub(crate) page_size: usize,
    pub(crate) next_offset: Option<usize>,
    pub(crate) reason: Option<ExclusionReason>,
}

pub(crate) struct Subtotal {
    pub(crate) count: usize,
    pub(crate) contribution: f64,
}

pub(crate) struct Ranking {
    pub(crate) groups: Vec<RankedRow>,
    pub(crate) group_count: usize,
    pub(crate) top_contribution: f64,
    pub(crate) remainder: Subtotal,
    pub(crate) page: Option<RankPage>,
}

pub(crate) struct FilteredRanking {
    pub(crate) ranking: Ranking,
    pub(crate) total_contribution: f64,
    pub(crate) excluded: Subtotal,
    pub(crate) by_reason: [Subtotal; 3],
    pub(crate) excluded_page: Option<RankPage>,
    pub(crate) state: CheckState,
    residual: f64,
    reason_residual: f64,
}

pub(crate) struct RankingResult {
    pub(crate) ranking: Ranking,
    pub(crate) filtered: Option<FilteredRanking>,
    pub(crate) state: CheckState,
    partition_residual: f64,
}

type RankedGroup<'a> = (&'a str, Effect, f64);

fn compare_rank(left: &RankedGroup<'_>, right: &RankedGroup<'_>) -> std::cmp::Ordering {
    right.2.abs().total_cmp(&left.2.abs()).then_with(|| left.0.cmp(right.0))
}

fn rank_page(
    rows: &mut [RankedGroup<'_>], offset: usize, row: impl Fn(&RankedGroup<'_>) -> RankedRow,
) -> RankPage {
    let offset = offset.min(rows.len());
    let end = offset.saturating_add(REMAINDER_PAGE_SIZE).min(rows.len());
    let groups = if offset == end {
        &mut rows[..0]
    } else {
        if end < rows.len() {
            rows.select_nth_unstable_by(end, compare_rank);
        }
        let prefix = &mut rows[..end];
        if offset > 0 {
            prefix.select_nth_unstable_by(offset, compare_rank);
        }
        let groups = &mut prefix[offset..];
        groups.sort_unstable_by(compare_rank);
        groups
    };
    RankPage { offset, groups: groups.iter().map(row).collect(), page_size: REMAINDER_PAGE_SIZE,
        next_offset: (end < rows.len()).then_some(end), reason: None }
}

fn rank_groups(
    ranked: &mut [RankedGroup<'_>], remainder_offset: Option<usize>,
    row: impl Fn(&RankedGroup<'_>) -> RankedRow,
) -> Ranking {
    let group_count = ranked.len();
    if group_count > RANKED_LIMIT {
        ranked.select_nth_unstable_by(RANKED_LIMIT, compare_rank);
    }
    let (top, remainder) = ranked.split_at_mut(group_count.min(RANKED_LIMIT));
    top.sort_unstable_by(compare_rank);
    let groups: Vec<_> = top.iter().map(&row).collect();
    // Sum both partitions before sorting the optional page.
    let subtotal = |rows: &[(&str, Effect, f64)]| {
        let mut sum = CompensatedSum::default();
        for &(_, _, contribution) in rows { sum.add(contribution); }
        sum.sum
    };
    let top_contribution = subtotal(top);
    let remainder_contribution = subtotal(remainder);
    let other = Subtotal { count: remainder.len(), contribution: remainder_contribution };
    let page = remainder_offset.map(|offset| rank_page(remainder, offset, row));
    Ranking { groups, group_count, top_contribution, remainder: other, page }
}

struct Decomposition {
    scale: f64,
    delta: f64,
    within: f64,
    composition: f64,
    total: f64,
    residual: f64,
    group_residual: f64,
    tolerance: f64,
    counts: [usize; 3],
    ranked: Vec<(Arc<str>, Effect, f64)>,
}

impl Decomposition {
    fn prepare(before: &Side, after: &Side, delta: Option<f64>) -> Result<Self, &'static str> {
        let delta = delta.ok_or("unavailable")?;
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
        let mut accumulate = |label: &Arc<str>, before: Option<Group>, after: Option<Group>| {
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
            ranked.push((label.clone(), effect, contribution));
        };
        for (label, row) in &before.rows {
            accumulate(label, row.group, after.rows.get(label).and_then(|row| row.group));
        }
        for (label, row) in &after.rows {
            if !before.rows.contains_key(label) { accumulate(label, None, row.group); }
        }
        let within = within.sum * scale;
        let composition = composition.sum * scale;
        let residual = (within + composition) - delta;
        let group_residual = total.sum - delta;
        let tolerance = (8. * f64::EPSILON * magnitude).max(8. * f64::EPSILON * (delta.abs() / scale).max(1.)) * scale;
        if !residual.is_finite() || !group_residual.is_finite() || !tolerance.is_finite() {
            return Err("non_finite");
        }
        Ok(Self { scale, delta, within, composition, total: total.sum, residual, group_residual, tolerance,
            counts: [continued, entered, exited], ranked })
    }

    fn rankings(&self, before: &Side, after: &Side, page: Option<RankingPage>, filter: &RankingFilter) -> RankingResult {
        let Self { scale, delta, residual, group_residual, tolerance, .. } = *self;
        let mut ranked: Vec<_> = self.ranked.iter().map(|(key, effect, contribution)| (key.as_ref(), *effect, *contribution)).collect();
        let row = |&(key, ref effect, contribution): &RankedGroup<'_>| RankedRow {
            key: key.to_owned(), contribution, within_group: effect.within * scale,
            composition: effect.composition * scale, before_share: effect.before_share, after_share: effect.after_share,
            before: before.evidence(key), after: after.evidence(key), reason: None,
        };
        let remainder_offset = match page {
            Some(RankingPage::Remainder(offset)) => Some(offset),
            _ => None,
        };
        let filtered = filter.active().then(|| {
            let query = filter.query.trim().to_lowercase();
            let mut matches = Vec::new();
            let mut matched = CompensatedSum::default();
            let mut excluded = CompensatedSum::default();
            let mut excluded_groups = Vec::new();
            let mut reasons = [(0usize, CompensatedSum::default()), (0, CompensatedSum::default()), (0, CompensatedSum::default())];
            let exclusion = |name: &str, contribution: f64| {
                let direction = match filter.direction {
                    RankingDirection::All => false,
                    RankingDirection::Increase => contribution <= 0.,
                    RankingDirection::Decrease => contribution >= 0.,
                };
                let name = !query.is_empty() && !name.to_lowercase().contains(&query);
                (direction, name)
            };
            for &group @ (name, _, contribution) in &ranked {
                let (direction, name) = exclusion(name, contribution);
                if let Some(reason) = ExclusionReason::from_conditions(direction, name) {
                    excluded.add(contribution);
                    let (count, subtotal) = &mut reasons[reason as usize];
                    *count += 1;
                    subtotal.add(contribution);
                    if let Some(RankingPage::Excluded(_, requested)) = page
                        && requested.is_none_or(|requested| requested == reason)
                    {
                        excluded_groups.push(group);
                    }
                } else {
                    matched.add(contribution);
                    matches.push(group);
                }
            }
            let residual = (matched.sum + excluded.sum) - delta;
            let excluded_count = ranked.len() - matches.len();
            let ranking = rank_groups(&mut matches, remainder_offset, row);
            let mut reason_total = CompensatedSum::default();
            let by_reason = reasons.map(|(count, subtotal)| {
                reason_total.add(subtotal.sum);
                Subtotal { count, contribution: subtotal.sum }
            });
            let reason_residual = reason_total.sum - excluded.sum;
            let excluded_page = if let Some(RankingPage::Excluded(offset, reason)) = page {
                let mut page = rank_page(&mut excluded_groups, offset, |entry| {
                    let (direction, name) = exclusion(entry.0, entry.2);
                    let mut value = row(entry);
                    value.reason = ExclusionReason::from_conditions(direction, name);
                    value
                });
                page.reason = reason;
                Some(page)
            } else { None };
            FilteredRanking { ranking, total_contribution: matched.sum,
                excluded: Subtotal { count: excluded_count, contribution: excluded.sum },
                by_reason, excluded_page, state: CheckState::check(tolerance, &[residual, reason_residual]),
                residual, reason_residual }
        });
        let ranking = rank_groups(&mut ranked, remainder_offset.filter(|_| filtered.is_none()), row);
        let partition_residual = (ranking.top_contribution + ranking.remainder.contribution) - delta;
        RankingResult { ranking, filtered, partition_residual,
            state: CheckState::check(tolerance, &[residual, group_residual, partition_residual]) }
    }

}

pub(crate) struct AnalysisError {
    pub(crate) state: &'static str,
    side: Option<&'static str>,
}

impl AnalysisError {
    pub(crate) fn wire(&self) -> Value {
        match self.side {
            Some(side) => json!({"state":self.state,"side":side}),
            None => json!({"state":self.state}),
        }
    }
}

pub(crate) struct ReportAnalysis {
    before: Side,
    after: Side,
    decomposition: Result<Decomposition, &'static str>,
    pub(crate) kind: Metric,
    pub(crate) delta: Option<f64>,
    pub(crate) unit: Option<String>,
}

impl ReportAnalysis {
    pub(crate) fn prepare(base: &Presentation, candidate: &Presentation, table: &str, metric: &str) -> Result<Self, AnalysisError> {
        if report_comparison_definition(base, table) != report_comparison_definition(candidate, table) {
            return Err(AnalysisError { state: "comparison_changed", side: None });
        }
        let before = report_side(base, table, metric).map_err(|state| AnalysisError { state, side: Some("before") })?;
        let after = report_side(candidate, table, metric).map_err(|state| AnalysisError { state, side: Some("after") })?;
        let kind = before.metric;
        let (_, delta) = kind.delta(before.totals.value(kind), after.totals.value(kind));
        let decomposition = Decomposition::prepare(&before, &after, delta);
        let unit = report_comparison_definition(base, table).ok().and_then(|definition| definition["unit"].as_str()).map(str::to_owned);
        Ok(Self { before, after, decomposition, kind, delta, unit })
    }

    pub(crate) fn rankings(&self, page: Option<RankingPage>, filter: &RankingFilter) -> Option<RankingResult> {
        self.decomposition.as_ref().ok().map(|data| data.rankings(&self.before, &self.after, page, filter))
    }

    pub(crate) fn response(&self, table: &str, metric: &str, key: Option<&str>, rankings: Option<&RankingResult>) -> Value {
        let kind = self.kind;
        let (delta_key, _) = kind.delta(None, None);
        let source = |side: &Side| {
            let (left, right, _, _) = kind.fields();
            json!({"table":side.source_table,"sheet":side.sheet,"key_column":side.source_columns[0],
                (left):side.source_columns[1],(right):side.source_columns[2]})
        };
        let mut result = json!({"state":"review","table":table,"metric":metric,"kind":kind.name(),(delta_key):self.delta,
            "before":self.before.totals.details(kind),"after":self.after.totals.details(kind),
            "source":{"before":source(&self.before),"after":source(&self.after)}});
        if let Some(unit) = &self.unit { result["unit"] = json!(unit); }
        result["decomposition"] = match &self.decomposition {
            Ok(data) => data.wire(rankings.expect("prepared quotient ranking"), kind, key),
            Err(state) => json!({"state":state}),
        };
        if let Some(key) = key {
            let before = self.before.evidence(key);
            let after = self.after.evidence(key);
            let (_, delta) = kind.delta(before.as_ref().and_then(|row| row.value), after.as_ref().and_then(|row| row.value));
            result["contributor"] = json!({"key":key,"before":evidence_json(before.as_ref(), kind),
                "after":evidence_json(after.as_ref(), kind),(delta_key):delta});
        }
        result
    }
}

fn ranking_json(ranking: &Ranking, kind: Metric) -> Value {
    let groups: Vec<_> = ranking.groups.iter().map(|group| group.wire(kind)).collect();
    let mut remainder = ranking.remainder.wire();
    if let Some(page) = &ranking.page { remainder["page"] = page.wire(kind); }
    json!({"groups":groups,"group_count":ranking.group_count,"top_contribution":ranking.top_contribution,"remainder":remainder})
}

impl Subtotal {
    fn wire(&self) -> Value { json!({"count":self.count,"contribution":self.contribution}) }
}

impl RankPage {
    fn wire(&self, kind: Metric) -> Value {
        let groups: Vec<_> = self.groups.iter().map(|row| row.wire(kind)).collect();
        let mut result = json!({"offset":self.offset,"groups":groups,"page_size":self.page_size,"next_offset":self.next_offset});
        if let Some(reason) = self.reason { result["reason"] = json!(reason.key()); }
        result
    }
}

impl RankedRow {
    fn wire(&self, kind: Metric) -> Value {
        let mut row = json!({"key":self.key,"contribution":self.contribution,"within_group":self.within_group,
            "composition":self.composition,"before_share":self.before_share,"after_share":self.after_share,
            "before":evidence_json(self.before.as_ref(), kind),"after":evidence_json(self.after.as_ref(), kind)});
        if let Some(reason) = self.reason {
            row["excluded_by"] = json!({"direction":reason != ExclusionReason::Name,"name":reason != ExclusionReason::Direction});
        }
        row
    }
}

impl Decomposition {
    fn wire(&self, rankings: &RankingResult, kind: Metric, key: Option<&str>) -> Value {
        if rankings.state == CheckState::NonFinite { return json!({"state":"non_finite"}); }
        let mut ranking = ranking_json(&rankings.ranking, kind);
        ranking["total_contribution"] = json!(self.total);
        ranking["residual"] = json!(self.group_residual);
        ranking["partition_residual"] = json!(rankings.partition_residual);
        if let Some(filtered) = &rankings.filtered {
            let mut result = ranking_json(&filtered.ranking, kind);
            let mut excluded = filtered.excluded.wire();
            let by_reason: serde_json::Map<_, _> = ExclusionReason::ALL.into_iter()
                .map(|reason| (reason.key().to_owned(), filtered.by_reason[reason as usize].wire())).collect();
            excluded["by_reason"] = Value::Object(by_reason);
            excluded["reason_residual"] = json!(filtered.reason_residual);
            if let Some(page) = &filtered.excluded_page { excluded["page"] = page.wire(kind); }
            result["excluded"] = excluded;
            result["total_contribution"] = json!(filtered.total_contribution);
            result["partition_residual"] = json!(filtered.residual);
            result["state"] = json!(filtered.state.wire());
            ranking["filtered"] = result;
        }
        let contributor = key.and_then(|key| self.ranked.iter().find(|row| row.0.as_ref() == key)).map(|(_, effect, _)| json!({
            "within_group":effect.within * self.scale,"composition":effect.composition * self.scale,
            "before_share":effect.before_share,"after_share":effect.after_share,
        }));
        let [continued, entered, exited] = self.counts;
        json!({"state":rankings.state.wire(),"within_group":self.within,"composition":self.composition,
            "residual":self.residual,"tolerance":self.tolerance,
            "counts":{"continued":continued,"entered":entered,"exited":exited},"contributor":contributor,"ranking":ranking})
    }
}

pub(crate) fn applied(workbook: &Presentation, table: &str, metric: &str, key: &str) -> Value {
    match report_side(workbook, table, metric) {
        Ok(side) => {
            let evidence = evidence_json(side.evidence(key).as_ref(), side.metric);
            json!({"state":evidence["state"],"kind":side.metric.name(),"table":table,"metric":metric,"key":key,"evidence":evidence})
        }
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
