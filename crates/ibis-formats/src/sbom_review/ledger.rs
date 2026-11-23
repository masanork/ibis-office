// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::*;
use ibis_doc_model::{CellValue, Presentation, sheet::Sheet, sheet_calc::sheet_of_slide};

pub(super) const TRACKING: [&str; 6] = [
    "Owner",
    "Due date",
    "Decision",
    "Rationale",
    "Evidence",
    "Review flag",
];
pub(super) const HEADERS: &[&str] = &[
    "Ecosystem",
    "Package",
    "Version",
    "OSV ID",
    "Fixed versions",
    "Advisory URL",
    "Owner",
    "Due date",
    "Decision",
    "Rationale",
    "Evidence",
    "Review flag",
    "Last reported versions",
];

type Key = (Package, String);

#[derive(Default)]
struct Previous {
    version: String,
    last_reported: String,
    tracking: [String; TRACKING.len()],
}

fn findings_sheet(book: &Presentation) -> Option<&Sheet> {
    let sheet = book
        .workbook
        .as_ref()?
        .sheets
        .iter()
        .find(|sheet| sheet.name == "Findings")?;
    let id = sheet.slide?;
    book.slides
        .iter()
        .find(|slide| slide.id == id)
        .and_then(sheet_of_slide)
        .map(AsRef::as_ref)
}

fn header(sheet: &Sheet, name: &str) -> Option<u32> {
    sheet
        .cells_in_rows(0, 0)
        .iter()
        .find(|cell| cell.text == name)
        .map(|cell| cell.column)
}

pub fn can_refresh(book: &Presentation) -> bool {
    findings_sheet(book).is_some_and(|sheet| {
        let has = |name| header(sheet, name).is_some();
        ["Ecosystem", "Package", "OSV ID", "Advisory URL"]
            .into_iter()
            .all(has)
            && ((has("Version") && TRACKING.into_iter().all(has))
                || ["New scan", "Old scan", "Fixed versions (old scan)"]
                    .into_iter()
                    .all(has))
    })
}

fn previous(book: &Presentation) -> Result<BTreeMap<Key, Previous>, String> {
    let sheet = findings_sheet(book)
        .filter(|_| can_refresh(book))
        .ok_or("Expected a vulnerability review workbook")?;
    let col = |name| header(sheet, name).unwrap();
    let version = header(sheet, "Version").unwrap_or_else(|| col("New scan"));
    let last_reported =
        header(sheet, "Last reported versions").or_else(|| header(sheet, "Old scan"));
    let identity = ["Ecosystem", "Package", "OSV ID"].map(col);
    let annotations = TRACKING.map(|name| header(sheet, name));
    let mut entries = BTreeMap::new();
    let rows: BTreeSet<_> = sheet
        .cells
        .iter()
        .filter_map(|cell| (cell.row > 0).then_some(cell.row))
        .collect();
    for row in rows {
        let text = |column| {
            sheet
                .cell(row, column)
                .map_or("", |cell| match &cell.value {
                    CellValue::Text(value) if cell.formula.is_none() => value.as_str(),
                    _ => cell.text.as_str(),
                })
        };
        let [ecosystem, package, id] = identity.map(text);
        let tracking = annotations.map(|column| column.map(text).unwrap_or_default().to_owned());
        if [ecosystem, package, id]
            .iter()
            .any(|value| value.is_empty())
        {
            if [ecosystem, package, id]
                .iter()
                .all(|value| value.is_empty())
                && tracking.iter().all(String::is_empty)
            {
                continue;
            }
            return Err(format!(
                "Findings row {} has an incomplete identity",
                row + 1
            ));
        }
        let observed = |column| match text(column) {
            "—" => "",
            value => value,
        };
        let item = Previous {
            version: observed(version).into(),
            last_reported: last_reported.map(observed).unwrap_or_default().into(),
            tracking,
        };
        if entries
            .insert(((ecosystem.into(), package.into()), id.into()), item)
            .is_some()
        {
            return Err(format!(
                "Findings row {} repeats an advisory and package",
                row + 1
            ));
        }
    }
    Ok(entries)
}

pub(super) fn rows(findings: Findings) -> Vec<Row> {
    merge(findings, BTreeMap::new(), false)
}

fn merge(findings: Findings, mut previous: BTreeMap<Key, Previous>, updating: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    for ((package, id), finding) in findings {
        let version = versions(Some(&finding.versions), str::to_owned);
        let fixed = versions(
            Some(&finding.fixed).filter(|values| !values.is_empty()),
            str::to_owned,
        );
        let previous = previous.remove(&(package.clone(), id.clone()));
        let (tracking, last_reported) = match previous {
            Some(old) => {
                let mut tracking = old.tracking;
                if old.version.is_empty() {
                    tracking[5] = "Re-detected · review required".into();
                } else if old.version != version {
                    tracking[5] = "Version changed · review required".into();
                }
                (
                    tracking,
                    if old.version.is_empty() {
                        old.last_reported
                    } else {
                        old.version
                    },
                )
            }
            None => {
                let mut tracking: [String; TRACKING.len()] = Default::default();
                if updating {
                    tracking[5] = "Newly detected · review required".into();
                }
                (tracking, String::new())
            }
        };
        let url = advisory_url(&id).unwrap_or_default();
        let mut row = vec![package.0, package.1, version, id, fixed, url];
        row.extend(tracking);
        row.push(last_reported);
        rows.push(row);
    }
    for (((ecosystem, package), id), old) in previous {
        let url = advisory_url(&id).unwrap_or_default();
        let mut row = vec![ecosystem, package, String::new(), id, String::new(), url];
        let mut tracking = old.tracking;
        tracking[5] = "Not reported in latest scan".into();
        row.extend(tracking);
        row.push(if old.version.is_empty() {
            old.last_reported
        } else {
            old.version
        });
        rows.push(row);
    }
    rows
}

/// Create a new review workbook from a scan and the current Findings sheet.
pub fn refresh_xlsx(book: &Presentation, input: (&str, &[u8])) -> Result<Vec<u8>, String> {
    let previous = previous(book)?;
    let findings = scan(&parse(input)?)?;
    workbook([
        ("Findings", HEADERS, merge(findings, previous, true)),
        input_artifact(input),
        ("Notes", &["Scope"], vec![vec![
            "Review flags describe the supplied scan. A missing detection can reflect scan coverage or configuration. Verify scan coverage and deployment separately.".into(),
        ]]),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scan_json(entries: &[(&str, &str)]) -> String {
        let packages: Vec<_> = entries
            .iter()
            .map(|(name, version)| {
                json!({
                    "package": {"ecosystem": "npm", "name": name, "version": version},
                    "vulnerabilities": [{"id": "TEST-1"}]
                })
            })
            .collect();
        json!({"results": [{"source": {"path": "lock.json", "type": "lockfile"}, "packages": packages}]}).to_string()
    }

    fn findings(bytes: Vec<u8>) -> crate::Opened {
        crate::open(bytes).unwrap()
    }

    #[test]
    fn refresh_preserves_human_fields_and_flags_changes_and_reappearances() {
        let source = scan_json(&[("updated", "1.0"), ("absent", "1.0"), ("same", "1.0")]);
        let mut records = rows(scan(&serde_json::from_str(&source).unwrap()).unwrap());
        for record in &mut records {
            record[6..11].clone_from_slice(&[
                "Alice".into(),
                "2026-12-01".into(),
                "Deferred".into(),
                "=literal notes".into(),
                "https://example.test/ticket".into(),
            ]);
        }
        let opened = findings(workbook([("Findings", HEADERS, records)]).unwrap());
        let latest = scan_json(&[("updated", "2.0"), ("new", "1.0"), ("same", "1.0")]);
        let updated = findings(
            refresh_xlsx(&opened.presentation, ("latest.osv.json", latest.as_bytes())).unwrap(),
        );
        let sheet = findings_sheet(&updated.presentation).unwrap();
        let row_for = |name| {
            sheet
                .cells
                .iter()
                .find(|cell| cell.column == 1 && cell.text == name)
                .unwrap()
                .row
        };
        let updated_row = row_for("updated");
        for (column, text) in [
            (6, "Alice"),
            (7, "2026-12-01"),
            (8, "Deferred"),
            (9, "=literal notes"),
            (10, "https://example.test/ticket"),
        ] {
            let cell = sheet.cell(updated_row, column).unwrap();
            assert_eq!(cell.text, text);
            assert!(cell.formula.is_none());
        }
        assert_eq!(
            sheet.cell(updated_row, 11).unwrap().text,
            "Version changed · review required"
        );
        assert_eq!(
            sheet.cell(row_for("new"), 11).unwrap().text,
            "Newly detected · review required"
        );
        assert!(sheet.cell(row_for("same"), 11).is_none());
        let absent = row_for("absent");
        assert_eq!(
            sheet.cell(absent, 11).unwrap().text,
            "Not reported in latest scan"
        );
        assert_eq!(sheet.cell(absent, 12).unwrap().text, "1.0");

        let latest = scan_json(&[("absent", "1.0"), ("updated", "2.0")]);
        let updated = findings(
            refresh_xlsx(&updated.presentation, ("next.osv.json", latest.as_bytes())).unwrap(),
        );
        let sheet = findings_sheet(&updated.presentation).unwrap();
        assert_eq!(
            sheet.cell(1, 11).unwrap().text,
            "Re-detected · review required"
        );
        assert_eq!(sheet.cell(1, 8).unwrap().text, "Deferred");
        assert_eq!(
            sheet.cell(2, 11).unwrap().text,
            "Version changed · review required"
        );
    }

    #[test]
    fn earlier_comparison_workbooks_can_be_refreshed() {
        let previous = findings(
            workbook([(
                "Findings",
                &[
                    "Ecosystem",
                    "Package",
                    "OSV ID",
                    "Old scan",
                    "New scan",
                    "Fixed versions (old scan)",
                    "Advisory URL",
                ],
                vec![vec![
                    "npm".into(),
                    "demo".into(),
                    "TEST-1".into(),
                    "1.0".into(),
                    "1.0".into(),
                ]],
            )])
            .unwrap(),
        );
        let next = scan_json(&[("demo", "2.0")]);
        let result =
            findings(refresh_xlsx(&previous.presentation, ("scan.json", next.as_bytes())).unwrap());
        let sheet = findings_sheet(&result.presentation).unwrap();
        assert_eq!(
            sheet.cell(1, 11).unwrap().text,
            "Version changed · review required"
        );
    }

    #[test]
    fn opening_detects_supported_json_and_rejects_unrelated_json() {
        assert!(
            open_table(("generic.json", br#"{"results":[]}"#))
                .unwrap()
                .is_none()
        );
        let empty = open_table(("empty.osv.json", br#"{"results":[]}"#))
            .unwrap()
            .unwrap();
        assert!(can_refresh(&empty.presentation));
        let input = scan_json(&[("demo", "1.0")]);
        assert!(
            open_table(("scan.json", input.as_bytes()))
                .unwrap()
                .is_some()
        );
        let bom = br#"{"bomFormat":"CycloneDX","specVersion":"1.6","components":[]}"#;
        let opened = open_table(("bom.json", bom)).unwrap().unwrap();
        assert!(!can_refresh(&opened.presentation));
        assert!(refresh_xlsx(&empty.presentation, ("bom.json", bom)).is_err());
    }
}
