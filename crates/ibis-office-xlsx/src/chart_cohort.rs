// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::BTreeMap;

use ibis_doc_model::{CellValue, Presentation};
use serde_json::{Value, json};

use crate::event_hooks::COHORT_LINK_MARKER;
use crate::sheet_ui::{parse_reference, resolve_workbook_reference};

pub(crate) type SourceCell = (ibis_doc_model::SlideId, u32, u32);
type Rows<'a> = BTreeMap<&'a str, f64>;

fn rows(workbook: &Presentation, (slide, column, row): SourceCell) -> Option<Result<Rows<'_>, &'static str>> {
    use ibis_doc_model::sheet_calc::sheet_of_slide;
    let sheet = sheet_of_slide(workbook.slide(slide)?)?;
    let mut links = sheet.hyperlinks.iter().filter(|link| {
        link.tooltip.as_deref() == Some(COHORT_LINK_MARKER)
            && (link.range.first_row..=link.range.last_row).contains(&row)
            && (link.range.first_column..=link.range.last_column).contains(&column)
    });
    let link = links.next()?;
    Some((|| {
        if links.next().is_some() {
            return Err("ambiguous_link");
        }
        let (target, anchor) = link.location.as_deref().filter(|_| link.target.is_none())
            .and_then(parse_reference)
            .and_then(|reference| resolve_workbook_reference(workbook, &reference, slide))
            .ok_or("invalid_link")?;
        let sheet = workbook.slide(target).and_then(sheet_of_slide).ok_or("invalid_table")?;
        let table = sheet.tables.iter().find(|table| {
            table.range.first_column == anchor.first_column && table.range.first_row == anchor.first_row
                && table.columns == ["key", "amount"] && table.header_rows == 1
        }).ok_or("invalid_table")?;
        let range = table.range;
        if range.last_row.saturating_sub(range.first_row) > 100_000 {
            return Err("too_many_rows");
        }
        let mut result = BTreeMap::new();
        for row in range.first_row + 1..=range.last_row.saturating_sub(table.totals_rows) {
            let key = sheet.cell(row, range.first_column).map(|cell| &cell.value);
            let amount = sheet.cell(row, range.first_column + 1).map(|cell| &cell.value);
            let (Some(CellValue::Text(key)), Some(CellValue::Number(amount))) = (key, amount) else {
                return Err("invalid_row");
            };
            if key.trim().is_empty() || !amount.is_finite() {
                return Err("invalid_row");
            }
            if result.insert(key.as_str(), *amount).is_some() {
                return Err("duplicate_key");
            }
        }
        Ok(result)
    })())
}

#[derive(Default)]
struct Contribution {
    count: usize,
    before: f64,
    after: f64,
}

impl Contribution {
    fn add(&mut self, before: f64, after: f64) {
        self.count += 1;
        self.before += before;
        self.after += after;
    }

    fn summary(&self) -> Value {
        json!({"count":self.count,"before":self.before,"after":self.after,"delta":self.after-self.before})
    }
}

fn decompose(before: Rows<'_>, after: Rows<'_>, old: f64, new: f64) -> Value {
    let close = |a: f64, b: f64| a.is_finite() && b.is_finite()
        && (a - b).abs() <= 8. * f64::EPSILON * a.abs().max(b.abs()).max(1.);
    let mut retained = Contribution::default();
    let mut added = Contribution::default();
    let mut removed = Contribution::default();
    for (key, amount) in &before {
        match after.get(key) {
            Some(other) => retained.add(*amount, *other),
            None => removed.add(*amount, 0.),
        }
    }
    for (key, amount) in &after {
        if !before.contains_key(key) {
            added.add(0., *amount);
        }
    }
    let delta = (retained.after - retained.before) + added.after - removed.before;
    if !close(retained.before + removed.before, old) || !close(retained.after + added.after, new)
        || !close(delta, new - old)
    {
        return json!({"state":"totals_mismatch"});
    }
    json!({"state":"ready","retained":retained.summary(),"added":added.summary(),"removed":removed.summary(),"delta":delta})
}

pub(crate) fn compare(
    base: &Presentation, candidate: &Presentation,
    before: Option<SourceCell>, after: Option<SourceCell>, old: f64, new: f64,
) -> Option<Value> {
    let before = before.and_then(|cell| rows(base, cell));
    let after = after.and_then(|cell| rows(candidate, cell));
    Some(match (before, after) {
        (None, None) => return None,
        (Some(Err(reason)), _) => json!({"state":reason,"side":"before"}),
        (_, Some(Err(reason))) => json!({"state":reason,"side":"after"}),
        (Some(Ok(before)), Some(Ok(after))) => decompose(before, after, old, new),
        _ => json!({"state":"link_missing"}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cohort_counts_and_signed_contributions_reconcile() {
        let before = BTreeMap::from([("001", 110.), ("002", 20.), ("004", 0.)]);
        let after = BTreeMap::from([("001", 130.), ("003", 40.), ("004", 0.)]);
        let got = decompose(before.clone(), after.clone(), 130., 170.);
        assert_eq!(got["retained"], json!({"count":2,"before":110.,"after":130.,"delta":20.}));
        assert_eq!(got["added"]["delta"], 40.);
        assert_eq!(got["removed"]["delta"], -20.);
        assert_eq!(got["delta"], 40.);
        assert_eq!(decompose(before, after, 120., 170.)["state"], "totals_mismatch");
    }
}
