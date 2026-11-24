// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::{BTreeMap, BTreeSet};

use ibis_doc_model::{CellValue, Chart, Presentation, ShapeId, ShapeKind, SlideId};
use serde_json::{Value, json};

use crate::event_hooks::{chart_drill_data, chart_drill_target, chart_source_cell};
use crate::sheet_ui::{parse_reference, resolve_workbook_reference};

type Key<'a> = (&'a str, &'a str);

#[derive(Clone, Copy)]
struct Point {
    series: usize,
    index: usize,
    value: PointValue,
    source: Option<crate::chart_cohort::SourceCell>,
}

#[derive(Clone, Copy)]
enum PointValue {
    Number(f64),
    Missing,
    NonNumeric,
    NoSource,
}

impl PointValue {
    fn number(self) -> Option<f64> {
        match self {
            Self::Number(value) if value.is_finite() => Some(value),
            _ => None,
        }
    }
}

fn points<'a>(workbook: &Presentation, sheet: SlideId, chart: &'a Chart) -> BTreeMap<Key<'a>, Option<Point>> {
    let mut points = BTreeMap::new();
    for (series, data) in chart.groups.iter().flat_map(|group| &group.series).enumerate() {
        let source = chart.cell_refs.get(series).and_then(|refs| refs.values.as_deref())
            .and_then(parse_reference)
            .and_then(|reference| resolve_workbook_reference(workbook, &reference, sheet))
            .and_then(|(sheet, range)| {
                Some((sheet, ibis_doc_model::sheet_calc::sheet_of_slide(workbook.slide(sheet)?)?, range))
            });
        for (index, category) in data.categories.iter().enumerate().filter(|(_, category)| !category.is_empty()) {
            let cell = source.and_then(|(id, sheet, range)| {
                let (column, row) = chart_source_cell(range, index)?;
                Some(((id, column, row), sheet.cell(row, column)))
            });
            let value = match cell.map(|(_, cell)| cell) {
                Some(Some(cell)) => match &cell.value {
                    CellValue::Number(value) => PointValue::Number(*value),
                    CellValue::Empty => PointValue::Missing,
                    CellValue::Text(text) if text.is_empty() => PointValue::Missing,
                    _ => PointValue::NonNumeric,
                },
                Some(None) => PointValue::Missing,
                None => PointValue::NoSource,
            };
            points.entry((data.name.as_str(), category.as_str()))
                .and_modify(|point| *point = None)
                .or_insert(Some(Point { series, index, value, source: cell.map(|(cell, _)| cell) }));
        }
    }
    points
}

fn value_info(point: Option<&Option<Point>>) -> Value {
    match point {
        None => json!({"state":"absent"}),
        Some(None) => json!({"state":"ambiguous"}),
        Some(Some(point)) => match point.value {
            PointValue::Number(value) if value.is_finite() => json!({"state":"number","value":value}),
            PointValue::Missing => json!({"state":"missing"}),
            PointValue::NoSource => json!({"state":"no_source"}),
            _ => json!({"state":"non_numeric"}),
        },
    }
}

fn number(point: Option<&Option<Point>>) -> Option<f64> {
    (*point?).and_then(|point| point.value.number())
}

fn amounts(before: f64, after: f64) -> Value {
    let delta = after - before;
    let percent = (before != 0.).then(|| delta / before.abs() * 100.);
    json!({"before":before,"after":after,"delta":delta,"percent_change":percent})
}

fn chart(workbook: &Presentation, slide: SlideId, shape: ShapeId) -> Option<&Chart> {
    match &workbook.slide(slide)?.shape(shape)?.kind {
        ShapeKind::Chart(chart) => Some(chart),
        _ => None,
    }
}

pub(crate) fn summary(base: &Presentation, candidate: &Presentation, charts: &Value) -> (Vec<Value>, bool) {
    let mut output = Vec::new();
    let mut remaining_bytes = 256 * 1024;
    for changed in charts.as_array().into_iter().flatten().filter(|chart| chart["changed"] == true) {
        let Some(shape) = changed["shape_id"].as_u64().map(ShapeId) else { continue };
        let Some(slide) = base.slides.iter().find(|slide| slide.shape(shape).is_some()) else { continue };
        let (Some(before), Some(after)) = (chart(base, slide.id, shape), chart(candidate, slide.id, shape)) else { continue };
        if output.len() == 24 {
            return (output, true);
        }
        let before = points(base, slide.id, before);
        let after = points(candidate, slide.id, after);
        let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
        let mut changes = Vec::new();
        let mut not_comparable = 0;
        for &&key in &keys {
            let (Some(old), Some(new)) = (number(before.get(&key)), number(after.get(&key))) else {
                not_comparable += 1;
                continue;
            };
            let delta = new - old;
            if !delta.is_finite() {
                not_comparable += 1;
            } else if delta != 0. {
                changes.push((key, old, new));
            }
        }
        changes.sort_by(|a, b| (b.2 - b.1).abs().total_cmp(&(a.2 - a.1).abs()).then_with(|| a.0.cmp(&b.0)));
        let mut categories = BTreeSet::new();
        let top: Vec<_> = changes.into_iter()
            .filter(|((_, category), _, _)| categories.insert(*category))
            .take(3)
            .map(|(key @ (series, category), old, new)| {
                let mut change = json!({"series":series,"category":category,"amounts":amounts(old,new)});
                if let Some(cohort) = crate::chart_cohort::compare(base, candidate,
                    before[&key].and_then(|point| point.source), after[&key].and_then(|point| point.source), old, new)
                {
                    change["cohort"] = cohort;
                }
                change
            })
            .collect();
        let summary = json!({"sheet":slide.name,"shape_id":shape.0,"title":changed["title"],"top":top,"not_comparable":not_comparable});
        let bytes = summary.to_string().len() + 1;
        if bytes > remaining_bytes {
            return (output, true);
        }
        remaining_bytes -= bytes;
        output.push(summary);
    }
    (output, false)
}

pub(crate) fn detail(base: &Presentation, candidate: &Presentation, shape: ShapeId, key: Key<'_>) -> Value {
    let Some(slide) = base.slides.iter().find(|slide| slide.shape(shape).is_some()) else {
        return json!({"state":"no_target"});
    };
    let lookup = |workbook: &Presentation| {
        let Some(chart) = chart(workbook, slide.id, shape) else {
            return (value_info(None), None, None);
        };
        let points = points(workbook, slide.id, chart);
        let point = points.get(&key);
        let drill = point.copied().flatten()
            .and_then(|point| chart_drill_target(workbook, slide.id, chart, point.series, point.index))
            .map(|target| chart_drill_data(workbook, target));
        (value_info(point), drill, point.copied().flatten().and_then(|point| point.source))
    };
    let (before, before_drill, before_source) = lookup(base);
    let (after, after_drill, after_source) = lookup(candidate);
    let values = before["value"].as_f64().zip(after["value"].as_f64())
        .filter(|(old, new)| (new - old).is_finite())
        .map(|(old, new)| amounts(old, new));
    let mut result = json!({"state":"review","shape_id":shape.0,"series":key.0,"category":key.1,"amounts":values,
        "before":{"point":before,"drill":before_drill},"after":{"point":after,"drill":after_drill}});
    if let Some((old, new)) = before["value"].as_f64().zip(after["value"].as_f64())
        && let Some(cohort) = crate::chart_cohort::compare(base, candidate, before_source, after_source, old, new)
    {
        result["cohort"] = cohort;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_diff_keeps_zero_distinct_from_missing_and_ambiguous_points() {
        let point = |value| Some(Point { series: 0, index: 0, value, source: None });
        let zero = point(PointValue::Number(0.0));
        let missing = point(PointValue::Missing);
        assert_eq!(value_info(Some(&zero)), json!({"state":"number","value":0.0}));
        assert_eq!(value_info(Some(&missing)), json!({"state":"missing"}));
        assert_eq!(value_info(Some(&None)), json!({"state":"ambiguous"}));
        assert_eq!(value_info(None), json!({"state":"absent"}));
        assert_eq!(number(Some(&zero)), Some(0.0));
        assert_eq!(number(Some(&missing)), None);
        assert_eq!(amounts(0.0, 10.0)["percent_change"], Value::Null);
        assert_eq!(amounts(10.0, 0.0)["percent_change"], -100.0);
    }
}
