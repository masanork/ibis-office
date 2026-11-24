// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::BTreeMap;

use ibis_doc_model::{CellValue, Presentation};
use serde_json::{Value, json};

use crate::event_hooks::COHORT_LINK_MARKER;
use crate::sheet_ui::{parse_reference, resolve_workbook_reference};

pub(crate) type SourceCell = (ibis_doc_model::SlideId, u32, u32);
#[derive(Clone, Copy)]
struct Row {
    amount: f64,
    index: u32,
}

struct Rows<'a> {
    sheet: Option<&'a str>,
    column: u32,
    entries: BTreeMap<&'a str, Row>,
}

impl Rows<'_> {
    fn evidence(&self, row: Row) -> Value {
        json!({
            "sheet":self.sheet,
            "key_cell":ibis_doc_model::cell_name(self.column, row.index),
            "amount_cell":ibis_doc_model::cell_name(self.column + 1, row.index),
            "amount":row.amount,
        })
    }
}

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
        let page = workbook.slide(target).ok_or("invalid_table")?;
        let sheet = sheet_of_slide(page).ok_or("invalid_table")?;
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
            if result.insert(key.as_str(), Row { amount: *amount, index: row }).is_some() {
                return Err("duplicate_key");
            }
        }
        Ok(Rows { sheet: page.name.as_deref(), column: range.first_column, entries: result })
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

#[derive(Clone, Copy)]
struct Change<'a> {
    key: &'a str,
    before: Option<Row>,
    after: Option<Row>,
}

impl Change<'_> {
    fn amounts(&self) -> (f64, f64) {
        (self.before.map_or(0., |row| row.amount), self.after.map_or(0., |row| row.amount))
    }

    fn delta(&self) -> f64 {
        let (before, after) = self.amounts();
        after - before
    }

    fn evidence(&self, before: &Rows<'_>, after: &Rows<'_>) -> Value {
        json!({
            "key":self.key, "delta":self.delta(),
            "before":self.before.map(|row| before.evidence(row)),
            "after":self.after.map(|row| after.evidence(row)),
        })
    }
}

fn decompose(before: Rows<'_>, after: Rows<'_>, old: f64, new: f64, key: Option<&str>) -> Value {
    let close = |a: f64, b: f64| a.is_finite() && b.is_finite()
        && (a - b).abs() <= 8. * f64::EPSILON * a.abs().max(b.abs()).max(1.);
    let mut retained = Contribution::default();
    let mut added = Contribution::default();
    let mut removed = Contribution::default();
    let mut changes = Vec::with_capacity(before.entries.len().max(after.entries.len()));
    for (&key, &row) in &before.entries {
        let other = after.entries.get(key).copied();
        if let Some(other) = other {
            retained.add(row.amount, other.amount);
        } else {
            removed.add(row.amount, 0.);
        }
        changes.push(Change { key, before: Some(row), after: other });
    }
    for (&key, &row) in &after.entries {
        if !before.entries.contains_key(key) {
            added.add(0., row.amount);
            changes.push(Change { key, before: None, after: Some(row) });
        }
    }
    let delta = (retained.after - retained.before) + added.after - removed.before;
    if !close(retained.before + removed.before, old) || !close(retained.after + added.after, new)
        || !close(delta, new - old) || changes.iter().any(|change| !change.delta().is_finite())
    {
        return json!({"state":"totals_mismatch"});
    }

    let top = |increasing: bool| {
        let mut ranked: Vec<_> = changes.iter().copied().filter(|change| {
            if increasing { change.delta() > 0. } else { change.delta() < 0. }
        }).collect();
        ranked.sort_unstable_by(|a, b| b.delta().abs().total_cmp(&a.delta().abs()).then_with(|| a.key.cmp(b.key)));
        ranked.truncate(5);
        ranked
    };
    let increases = top(true);
    let decreases = top(false);
    let picked: std::collections::BTreeSet<_> = increases.iter().chain(&decreases).map(|change| change.key).collect();
    let mut remainder = Contribution::default();
    for change in changes.iter().filter(|change| !picked.contains(change.key)) {
        let (old, new) = change.amounts();
        remainder.add(old, new);
    }
    let contributor = key.and_then(|key| changes.iter().find(|change| change.key == key))
        .map(|change| change.evidence(&before, &after));
    let serialize = |changes: Vec<Change<'_>>| {
        changes.into_iter().map(|change| change.evidence(&before, &after)).collect::<Vec<_>>()
    };
    let mut result = json!({"state":"ready","retained":retained.summary(),"added":added.summary(),"removed":removed.summary(),"delta":delta,
        "increases":serialize(increases),"decreases":serialize(decreases),"remainder":remainder.summary()});
    if key.is_some() {
        result["contributor"] = contributor.unwrap_or_else(|| json!({"state":"absent"}));
    }
    result
}

pub(crate) fn applied_row(workbook: &Presentation, source: SourceCell, key: &str) -> Value {
    match rows(workbook, source) {
        Some(Ok(rows)) => rows.entries.get(key).map_or_else(
            || json!({"state":"absent","key":key}),
            |row| json!({"state":"ready","key":key,"evidence":rows.evidence(*row)}),
        ),
        Some(Err(state)) => json!({"state":state}),
        None => json!({"state":"no_link"}),
    }
}

pub(crate) fn compare(
    base: &Presentation, candidate: &Presentation,
    before: Option<SourceCell>, after: Option<SourceCell>, old: f64, new: f64, key: Option<&str>,
) -> Option<Value> {
    let before = before.and_then(|cell| rows(base, cell));
    let after = after.and_then(|cell| rows(candidate, cell));
    Some(match (before, after) {
        (None, None) => return None,
        (Some(Err(reason)), _) => json!({"state":reason,"side":"before"}),
        (_, Some(Err(reason))) => json!({"state":reason,"side":"after"}),
        (Some(Ok(before)), Some(Ok(after))) => decompose(before, after, old, new, key),
        _ => json!({"state":"link_missing"}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cohort_counts_and_signed_contributions_reconcile() {
        let rows = |items: [(&'static str, f64); 3]| Rows {
            sheet: Some("Evidence"),
            column: 0,
            entries: items.into_iter().enumerate().map(|(index, (key, amount))| {
                (key, Row { amount, index: index as u32 + 1 })
            }).collect(),
        };
        let before = || rows([("001", 110.), ("002", 20.), ("004", 0.)]);
        let after = || rows([("001", 130.), ("003", 40.), ("004", 0.)]);
        let got = decompose(before(), after(), 130., 170., Some("001"));
        assert_eq!(got["retained"], json!({"count":2,"before":110.,"after":130.,"delta":20.}));
        assert_eq!(got["added"]["delta"], 40.);
        assert_eq!(got["removed"]["delta"], -20.);
        assert_eq!(got["delta"], 40.);
        assert_eq!(got["increases"][0]["key"], "003");
        assert!(got["increases"][0]["before"].is_null());
        assert_eq!(got["decreases"][0]["key"], "002");
        assert!(got["decreases"][0]["after"].is_null());
        assert_eq!(got["remainder"]["count"], 1);
        assert_eq!(got["contributor"]["after"]["amount_cell"], "B2");
        assert_eq!(decompose(before(), after(), 120., 170., None)["state"], "totals_mismatch");
    }
    #[test]
    fn tied_contributors_keep_key_order_and_remainder_reconciles() {
        let keys = ["01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12", "13", "14"];
        let rows = |amounts: [f64; 14]| Rows {
            sheet: None,
            column: 0,
            entries: keys.into_iter().zip(amounts).enumerate().map(|(index, (key, amount))| {
                (key, Row { amount, index: index as u32 + 1 })
            }).collect(),
        };
        let before = rows([20.; 14]);
        let after = rows([25., 25., 25., 25., 25., 25., 25., 19., 19., 19., 19., 19., 19., 19.]);
        let got = decompose(before, after, 280., 308., Some("07"));
        assert_eq!(got["increases"][0]["key"], "01");
        assert_eq!(got["increases"][4]["key"], "05");
        assert_eq!(got["decreases"][0]["key"], "08");
        assert_eq!(got["decreases"][4]["key"], "12");
        assert_eq!(got["remainder"], json!({"count":4,"before":80.,"after":88.,"delta":8.}));
        assert_eq!(got["contributor"]["key"], "07");
    }

}
