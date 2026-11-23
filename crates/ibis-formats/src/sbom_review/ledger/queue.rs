// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::*;
use chrono::NaiveDate;

fn parse_due(text: &str) -> Option<NaiveDate> {
    ["%Y-%m-%d", "%Y/%m/%d", "%Y年%m月%d日"]
        .into_iter()
        .find_map(|format| NaiveDate::parse_from_str(text.trim(), format).ok())
}

pub(super) fn due_text(sheet: &Sheet, row: u32, column: u32, date1904: bool) -> Option<String> {
    let cell = sheet.cell(row, column)?;
    let CellValue::Number(serial) = cell.value else {
        return None;
    };
    let code = &sheet.styles.get(cell.style as usize)?.number_format;
    // Pasting an ISO date can leave its serial in a General-formatted deadline cell.
    let pasted_date = code.eq_ignore_ascii_case("General") && serial.fract() == 0.0;
    if !(ibis_calc::numfmt::is_date_format(code) || pasted_date)
        || !(0.0..2_958_466.0).contains(&serial)
    {
        return None;
    }
    let (year, month, day) =
        ibis_calc::eval::convert::date_from_serial(serial.floor() as i64, date1904);
    Some(NaiveDate::from_ymd_opt(year.try_into().ok()?, month, day)?.to_string())
}

fn reasons(item: &Previous, today: NaiveDate) -> Vec<&'static str> {
    let [owner, due, decision, _, _, flag] = &item.tracking;
    let decision = decision.trim().to_ascii_lowercase();
    let deferred = matches!(
        decision.as_str(),
        "deferred" | "risk accepted" | "accepted risk" | "延期" | "リスク受容"
    );
    let closed = matches!(
        decision.as_str(),
        "fixed"
            | "not affected"
            | "not_affected"
            | "closed"
            | "修正済み"
            | "影響なし"
            | "対応済み"
            | "完了"
    );
    let active = !item.version.is_empty() && !closed;
    let mut reasons = Vec::new();
    if flag.ends_with(" · review required") {
        reasons.push("Scan change requires review");
    }
    if active && owner.trim().is_empty() {
        reasons.push("Owner missing");
    }
    if active || deferred {
        match parse_due(due) {
            Some(date) if date < today => reasons.push(if deferred {
                "Exception expired · review required"
            } else {
                "Response overdue"
            }),
            None if !due.trim().is_empty() => reasons.push("Due date could not be read"),
            None if deferred => reasons.push("Exception deadline missing"),
            _ => {}
        }
    }
    reasons
}

/// Build a snapshot for a specific date without changing the source review.
fn action_queue_xlsx(book: &Presentation, today: NaiveDate) -> Result<Vec<u8>, String> {
    let rows = previous(book)?
        .into_iter()
        .filter_map(|(((ecosystem, package), id), item)| {
            let reasons = reasons(&item, today);
            if reasons.is_empty() {
                return None;
            }
            let mut row = vec![
                reasons.join(" / "),
                ecosystem,
                package,
                id.clone(),
                item.version,
            ];
            row.extend(item.tracking);
            row.push(item.last_reported);
            row.push(advisory_url(&id).unwrap_or_default());
            Some(row)
        })
        .collect::<Vec<_>>();
    let summary = format!("{} finding(s) require attention as of {today}.", rows.len());
    workbook([
        ("Action required", &[
            "Reasons", "Ecosystem", "Package", "OSV ID", "Latest version",
            "Owner", "Due date", "Decision", "Rationale", "Evidence", "Review flag",
            "Last reported versions", "Advisory URL",
        ], rows),
        ("Notes", &["Scope"], vec![
            vec![summary],
            vec!["A snapshot of the source review. Record decisions in the original workbook and reopen this view to refresh it.".into()],
        ]),
    ])
}

/// Build an action queue using the local date.
pub fn open_action_queue(book: &Presentation) -> Result<crate::Opened, String> {
    let bytes = action_queue_xlsx(book, chrono::Local::now().date_naive())?;
    crate::open(bytes).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(owner: &str, due: &str, decision: &str) -> Previous {
        Previous {
            version: "1.0".into(),
            tracking: [
                owner.into(),
                due.into(),
                decision.into(),
                "Keep this rationale".into(),
                String::new(),
                String::new(),
            ],
            ..Previous::default()
        }
    }

    #[test]
    fn obligations_change_with_the_calendar_without_a_new_scan() {
        let today = parse_due("2026-11-23").unwrap();
        assert_eq!(
            reasons(&item("", "2026-11-22", ""), today),
            ["Owner missing", "Response overdue"]
        );
        assert!(reasons(&item("Alice", "2026-11-23", "延期"), today).is_empty());
        assert_eq!(
            reasons(&item("Alice", "2026年11月22日", "延期"), today),
            ["Exception expired · review required"]
        );
        assert_eq!(
            reasons(&item("Alice", "", "Risk accepted"), today),
            ["Exception deadline missing"]
        );
        assert_eq!(
            reasons(&item("Alice", "2026-02-30", ""), today),
            ["Due date could not be read"]
        );

        let mut fixed = item("", "2026/11/22", "Fixed");
        assert!(reasons(&fixed, today).is_empty());
        fixed.tracking[5] = "Re-detected · review required".into();
        assert_eq!(reasons(&fixed, today), ["Scan change requires review"]);
        fixed.tracking[5] = "Not reported in latest scan".into();
        fixed.version.clear();
        assert!(reasons(&fixed, today).is_empty());
    }

    #[test]
    fn dates_and_pasted_serials_use_the_workbook_epoch() {
        use ibis_doc_model::{
            CellStyle,
            sheet::{CellKind, SheetCell},
        };
        for date1904 in [false, true] {
            let value = CellValue::Number(ibis_calc::eval::convert::serial_from_date(
                2026, 11, 22, date1904,
            ) as f64);
            let mut sheet = Sheet::default();
            sheet.styles = vec![CellStyle {
                number_format: "m/d/yy".into(),
                ..CellStyle::default()
            }];
            sheet.cells = vec![SheetCell {
                row: 1,
                column: 0,
                text: "11/22/26".into(),
                kind: CellKind::of(&value),
                value,
                formula: None,
                style: 0,
                phonetic: None,
                runs: None,
            }]
            .into();
            assert_eq!(
                due_text(&sheet, 1, 0, date1904).as_deref(),
                Some("2026-11-22")
            );
            sheet.styles[0].number_format = "General".into();
            assert_eq!(
                due_text(&sheet, 1, 0, date1904).as_deref(),
                Some("2026-11-22")
            );
            sheet.cells[0].value = CellValue::Number(7.5);
            assert!(due_text(&sheet, 1, 0, date1904).is_none());
        }
    }

    #[test]
    fn dated_snapshot_keeps_original_decision_and_rationale() {
        let mut row = vec![
            "npm",
            "demo",
            "1.0",
            "TEST-1",
            "",
            "https://osv.dev/vulnerability/TEST-1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        row.extend(item("Alice", "2026-11-22", "Deferred").tracking);
        row.push(String::new());
        let source = crate::open(workbook([("Findings", HEADERS, vec![row])]).unwrap()).unwrap();
        let report = crate::open(
            action_queue_xlsx(&source.presentation, parse_due("2026-11-23").unwrap()).unwrap(),
        )
        .unwrap();
        let sheet = sheet_of_slide(&report.presentation.slides[0]).unwrap();
        assert_eq!(
            sheet.cell(1, 0).unwrap().text,
            "Exception expired · review required"
        );
        assert_eq!(sheet.cell(1, 7).unwrap().text, "Deferred");
        assert_eq!(sheet.cell(1, 8).unwrap().text, "Keep this rationale");
        assert_eq!(
            findings_sheet(&source.presentation)
                .unwrap()
                .cell(1, 8)
                .unwrap()
                .text,
            "Deferred"
        );
    }
}
