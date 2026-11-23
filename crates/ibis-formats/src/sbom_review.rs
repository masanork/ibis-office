// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

mod crypto;
pub use crypto::compare_xlsx as compare_cbom_xlsx;
pub use crypto::is_cbom_inventory;
mod ledger;
pub use ledger::{can_refresh, open_action_queue, refresh_xlsx};

type Package = (String, String);
type Inventory = BTreeMap<Package, BTreeSet<String>>;
type Findings = BTreeMap<(Package, String), Finding>;

#[derive(Default)]
struct Finding {
    versions: BTreeSet<String>,
    fixed: BTreeSet<String>,
}

fn required(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("expected a nonempty {key} string"))
}

fn array<'a>(value: &'a Value, key: &str, optional: bool) -> Result<&'a [Value], String> {
    match value.get(key) {
        None if optional => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        _ => Err(format!("expected a {key} array")),
    }
}

fn package(ecosystem: String, name: String) -> Package {
    let name = if ecosystem == "PyPI" {
        name.to_ascii_lowercase()
            .replace(['_', '.'], "-")
            .split('-')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    } else {
        name
    };
    (ecosystem, name)
}

fn decode(segment: &str) -> Option<String> {
    let mut bytes = segment.bytes();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).ok()
}

fn component_identity(purl: &str) -> Option<(Package, Option<String>)> {
    let (kind, path) = purl.strip_prefix("pkg:")?.split_once('/')?;
    let ecosystem = match kind {
        "cargo" => "crates.io",
        "npm" => "npm",
        "pypi" => "PyPI",
        "golang" => "Go",
        _ => return None,
    };
    let path = path.split(['?', '#']).next()?;
    let (name, version) = match path.rsplit_once('@') {
        Some((name, version)) if !name.is_empty() => (name, Some(decode(version)?)),
        _ => (path, None),
    };
    Some((package(ecosystem.into(), decode(name)?), version))
}

fn add_component(component: &Value, bom: &mut Inventory) -> Result<(), String> {
    if !component.is_object() {
        return Err("expected a CycloneDX component object".into());
    }
    if let Some((key, purl_version)) = component["purl"].as_str().and_then(component_identity) {
        let version = component["version"].as_str();
        let versions = bom.entry(key).or_default();
        for observed in version.into_iter().chain(purl_version.as_deref()) {
            versions.insert(observed.to_owned());
        }
        if version.is_none() && purl_version.is_none() {
            versions.insert(String::new());
        }
    }
    for child in array(component, "components", true)? {
        add_component(child, bom)?;
    }
    Ok(())
}

fn inventory(doc: &Value) -> Result<Inventory, String> {
    if doc["bomFormat"] != "CycloneDX" {
        return Err("expected CycloneDX JSON".into());
    }
    required(doc, "specVersion")?;
    let mut bom = Inventory::new();
    if let Some(subject) = doc["metadata"].get("component") {
        add_component(subject, &mut bom)?;
    }
    for entry in array(doc, "components", true)? {
        add_component(entry, &mut bom)?;
    }
    Ok(bom)
}

fn scan(doc: &Value) -> Result<Findings, String> {
    let mut findings = Findings::new();
    for result in array(doc, "results", false)? {
        for entry in array(result, "packages", false)? {
            if !entry.is_object() {
                return Err("expected an OSV-Scanner package result object".into());
            }
            for osv in array(entry, "vulnerabilities", true)? {
                let item = &entry["package"];
                let key = package(required(item, "ecosystem")?, required(item, "name")?);
                let id = required(osv, "id")?;
                let finding = findings.entry((key.clone(), id)).or_default();
                finding
                    .versions
                    .insert(item["version"].as_str().unwrap_or_default().into());
                for affected in array(osv, "affected", true)? {
                    let Some(item) = affected.get("package") else {
                        continue;
                    };
                    if package(required(item, "ecosystem")?, required(item, "name")?) != key {
                        continue;
                    }
                    for range in array(affected, "ranges", true)? {
                        if matches!(range["type"].as_str(), Some("SEMVER" | "ECOSYSTEM")) {
                            for event in array(range, "events", false)? {
                                if let Some(version) =
                                    event["fixed"].as_str().filter(|s| !s.is_empty())
                                {
                                    finding.fixed.insert(version.into());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(findings)
}

// Encode punctuation as character references so imported fields cannot create
// Markdown delimiters, HTML or links, even when rendered from a table cell.
fn cell(value: &str) -> String {
    let mut escaped = String::new();
    for c in value.chars() {
        if c.is_control() {
            escaped.push(' ');
        } else if matches!(
            c,
            '&' | '<' | '>' | '|' | '\\' | '`' | '*' | '_' | '[' | ']' | '~' | ':'
        ) {
            write!(escaped, "&#{};", u32::from(c)).unwrap();
        } else {
            escaped.push(c);
        }
    }
    escaped
}

fn versions(values: Option<&BTreeSet<String>>, render: impl Fn(&str) -> String) -> String {
    values.map_or_else(
        || "—".into(),
        |values| {
            values
                .iter()
                .map(|v| {
                    if v.is_empty() {
                        "unknown".into()
                    } else {
                        render(v)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        },
    )
}

const ROLES: [&str; 4] = ["Old SBOM", "New SBOM", "Old scan", "New scan"];
const COVERAGE: &str = "Status compares exact OSV IDs and package identities in the supplied scans. SBOM evidence uses Package URLs for crates.io, npm, PyPI and Go. A missing detection can reflect scan coverage or configuration. Verify scan coverage, asset identity and deployment separately.";

struct Review {
    documents: [Value; 2],
    old_bom: Inventory,
    new_bom: Inventory,
    old_scan: Findings,
    new_scan: Findings,
}

fn parse((name, bytes): (&str, &[u8])) -> Result<Value, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("{name}: {e}"))
}

fn read_review(inputs: [(&str, &[u8]); 4]) -> Result<Review, String> {
    let [old_bom, new_bom, old_scan, new_scan] = inputs.map(parse);
    let documents = [old_bom?, new_bom?];
    Ok(Review {
        old_bom: inventory(&documents[0]).map_err(|e| format!("Old SBOM: {e}"))?,
        new_bom: inventory(&documents[1]).map_err(|e| format!("New SBOM: {e}"))?,
        old_scan: scan(&old_scan?).map_err(|e| format!("Old scan: {e}"))?,
        new_scan: scan(&new_scan?).map_err(|e| format!("New scan: {e}"))?,
        documents,
    })
}

fn status(
    old: Option<&Finding>,
    new: Option<&Finding>,
    after: Option<&BTreeSet<String>>,
) -> &'static str {
    match (old, new) {
        (Some(_), Some(_)) => "Still detected",
        (None, Some(_)) => "Newly detected",
        (Some(old), None)
            if after.is_some_and(|after| {
                after.len() == 1
                    && after.iter().all(|version| {
                        old.fixed.contains(version) && !old.versions.contains(version)
                    })
            }) =>
        {
            "No longer reported · advisory fixed version listed in new SBOM"
        }
        (Some(_), None) => "No longer reported · inventory evidence inconclusive",
        (None, None) => unreachable!(),
    }
}

fn advisory_url(id: &str) -> Option<String> {
    id.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        .then(|| format!("https://osv.dev/vulnerability/{id}"))
}

/// Compare two CycloneDX inventories and OSV-Scanner reports, with source hashes.
/// Each input contains a display name and the original JSON bytes.
pub fn markdown(inputs: [(&str, &[u8]); 4]) -> Result<String, String> {
    let Review {
        old_bom,
        new_bom,
        old_scan,
        new_scan,
        ..
    } = read_review(inputs)?;

    let mut out = String::from("# Vulnerability remediation review\n\n");
    writeln!(out, "{COVERAGE}\n").unwrap();
    out.push_str("| Status | Ecosystem | Package | OSV advisory | Old scan | New scan | Old SBOM | New SBOM | Advisory fixed versions (old scan) |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    let keys: BTreeSet<_> = old_scan.keys().chain(new_scan.keys()).collect();
    for key in &keys {
        let (package, id) = key;
        let old = old_scan.get(*key);
        let new = new_scan.get(*key);
        let before = old_bom.get(package);
        let after = new_bom.get(package);
        let status = status(old, new, after);
        let advisory = advisory_url(id).map_or_else(|| cell(id), |url| format!("[{id}]({url})"));
        writeln!(
            out,
            "| {status} | {} | {} | {advisory} | {} | {} | {} | {} | {} |",
            cell(&package.0),
            cell(&package.1),
            versions(old.map(|f| &f.versions), cell),
            versions(new.map(|f| &f.versions), cell),
            versions(before, cell),
            versions(after, cell),
            versions(old.map(|f| &f.fixed).filter(|f| !f.is_empty()), cell),
        )
        .unwrap();
    }
    if keys.is_empty() {
        out.push_str("\nThe supplied scans contain no reported advisories.\n");
    }
    out.push_str("\n## Input artifacts\n\n| Role | File | SHA-256 |\n| --- | --- | --- |\n");
    for (role, (name, bytes)) in ROLES.into_iter().zip(inputs) {
        writeln!(
            out,
            "| {role} | {} | `{:x}` |",
            cell(name),
            Sha256::digest(bytes)
        )
        .unwrap();
    }
    Ok(out)
}

type Row = Vec<String>;
type Table = (&'static str, &'static [&'static str], Vec<Row>);

const COMPONENT_COLUMNS: &[&str] = &[
    "Parent",
    "Type",
    "Group",
    "Name",
    "Version",
    "Package URL",
    "BOM reference",
];

fn component_tree(doc: &Value) -> Result<Vec<(&Value, &str)>, String> {
    fn add<'a>(
        value: &'a Value,
        parent: &'a str,
        items: &mut Vec<(&'a Value, &'a str)>,
    ) -> Result<(), String> {
        if !value.is_object() {
            return Err("expected a CycloneDX component object".into());
        }
        items.push((value, parent));
        let next_parent = value["bom-ref"]
            .as_str()
            .or_else(|| value["name"].as_str())
            .unwrap_or_default();
        for child in array(value, "components", true)? {
            add(child, next_parent, items)?;
        }
        Ok(())
    }

    let mut items = Vec::new();
    if let Some(subject) = doc["metadata"].get("component") {
        add(subject, "", &mut items)?;
    }
    for value in array(doc, "components", true)? {
        add(value, "", &mut items)?;
    }
    Ok(items)
}

fn component_rows(items: &[(&Value, &str)]) -> Vec<Row> {
    items
        .iter()
        .map(|(value, parent)| {
            let mut row = vec![(*parent).to_owned()];
            row.extend(
                ["type", "group", "name", "version", "purl", "bom-ref"]
                    .map(|key| value[key].as_str().unwrap_or_default().to_owned()),
            );
            row
        })
        .collect()
}

fn components(doc: &Value) -> Result<Vec<Row>, String> {
    Ok(component_rows(&component_tree(doc)?))
}

/// Export the review, component inventories, and input evidence as an XLSX workbook.
pub fn review_xlsx(inputs: [(&str, &[u8]); 4]) -> Result<Vec<u8>, String> {
    let Review {
        documents,
        old_bom,
        new_bom,
        old_scan,
        new_scan,
    } = read_review(inputs)?;
    let keys: BTreeSet<_> = old_scan.keys().chain(new_scan.keys()).collect();
    let rows = keys
        .into_iter()
        .map(|key| {
            let (package, id) = key;
            let old = old_scan.get(key);
            let new = new_scan.get(key);
            let after = new_bom.get(package);
            let plain = str::to_owned;
            let mut row = vec![
                status(old, new, after).into(),
                package.0.clone(),
                package.1.clone(),
                id.clone(),
                versions(old.map(|f| &f.versions), plain),
                versions(new.map(|f| &f.versions), plain),
                versions(old_bom.get(package), plain),
                versions(after, plain),
                versions(old.map(|f| &f.fixed).filter(|f| !f.is_empty()), plain),
                advisory_url(id).unwrap_or_default(),
            ];
            row.resize(row.len() + ledger::TRACKING.len(), String::new());
            row
        })
        .collect();
    let tables: [Table; 5] = [
        (
            "Findings",
            &[
                "Status",
                "Ecosystem",
                "Package",
                "OSV ID",
                "Old scan",
                "New scan",
                "Old SBOM",
                "New SBOM",
                "Fixed versions (old scan)",
                "Advisory URL",
                "Owner",
                "Due date",
                "Decision",
                "Rationale",
                "Evidence",
                "Review flag",
            ],
            rows,
        ),
        (
            "Old components",
            COMPONENT_COLUMNS,
            components(&documents[0])?,
        ),
        (
            "New components",
            COMPONENT_COLUMNS,
            components(&documents[1])?,
        ),
        comparison_input_artifacts(ROLES.into_iter().zip(inputs)),
        ("Notes", &["Scope"], vec![vec![COVERAGE.into()]]),
    ];
    write_workbook(tables)
}

#[derive(Clone, Copy)]
enum ReportFormat {
    CycloneDx,
    OsvScanner,
}

impl ReportFormat {
    fn detect(doc: &Value, name: &str) -> Option<Self> {
        if doc["bomFormat"] == "CycloneDX" {
            return Some(Self::CycloneDx);
        }
        let osv = doc["results"].as_array().is_some_and(|results| {
            doc.get("experimental_config").is_some()
                || (!results.is_empty()
                    && results.iter().all(|result| {
                        result.get("source").is_some() && result.get("packages").is_some()
                    }))
                || name.to_ascii_lowercase().ends_with(".osv.json")
        });
        osv.then(|| Self::for_explicit_export(doc))
    }

    fn for_explicit_export(doc: &Value) -> Self {
        if doc.get("bomFormat").is_some() {
            Self::CycloneDx
        } else {
            Self::OsvScanner
        }
    }
}

/// Export a single CycloneDX SBOM or OSV-Scanner report as an XLSX workbook.
pub fn table_xlsx(input: (&str, &[u8])) -> Result<Vec<u8>, String> {
    let doc = parse(input)?;
    write_workbook(report_tables(
        ReportFormat::for_explicit_export(&doc),
        &doc,
        input,
    )?)
}

/// Recognize supported JSON exports without claiming unrelated JSON files.
pub fn open_table(input: (&str, &[u8])) -> Result<Option<crate::Opened>, String> {
    let Ok(doc) = parse(input) else {
        return Ok(None);
    };
    let Some(format) = ReportFormat::detect(&doc, input.0) else {
        return Ok(None);
    };
    let tables = report_tables(format, &doc, input)?;
    drop(doc);
    crate::open(write_workbook(tables)?)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn report_tables(
    format: ReportFormat,
    doc: &Value,
    input: (&str, &[u8]),
) -> Result<Vec<Table>, String> {
    match format {
        ReportFormat::CycloneDx => {
            if doc["bomFormat"] != "CycloneDX" {
                return Err("expected CycloneDX JSON".into());
            }
            required(doc, "specVersion")?;
            let items = component_tree(doc)?;
            let mut tables = crypto::tables(doc, &items)?;
            tables.push(("Components", COMPONENT_COLUMNS, component_rows(&items)));
            tables.push(input_artifact(input));
            Ok(tables)
        }
        ReportFormat::OsvScanner => Ok(vec![
            ("Findings", ledger::HEADERS, ledger::rows(scan(doc)?)),
            input_artifact(input),
        ]),
    }
}

fn input_artifact((name, bytes): (&str, &[u8])) -> Table {
    (
        "Input artifact",
        &["File", "SHA-256"],
        vec![vec![name.into(), format!("{:x}", Sha256::digest(bytes))]],
    )
}

fn comparison_input_artifacts<'a>(
    inputs: impl IntoIterator<Item = (&'static str, (&'a str, &'a [u8]))>,
) -> Table {
    let rows = inputs
        .into_iter()
        .map(|(role, (name, bytes))| {
            vec![
                role.into(),
                name.into(),
                format!("{:x}", Sha256::digest(bytes)),
            ]
        })
        .collect();
    ("Input artifacts", &["Role", "File", "SHA-256"], rows)
}

fn write_workbook(tables: impl IntoIterator<Item = Table>) -> Result<Vec<u8>, String> {
    let opened = assemble_workbook(tables)?;
    opened
        .source
        .save(&opened.presentation, crate::SaveOptions::default())
        .map_err(|e| e.to_string())
}

fn assemble_workbook(tables: impl IntoIterator<Item = Table>) -> Result<crate::Opened, String> {
    use ibis_doc_model::{
        AutoFilter, CellRange, CellStyle, CellValue,
        sheet::{CellKind, SheetCell},
    };

    crate::workbook_assembly::assemble(|template| {
        let mut sheets = Vec::new();
        for (name, headers, rows) in tables {
            let last_row = rows.len();
            if last_row >= 1_048_576 {
                return Err(crate::Error::new(format!(
                    "{name} exceeds the XLSX worksheet row limit"
                )));
            }
            let mut sheet = template.clone();
            let header_style = sheet.styles.len() as u32;
            sheet.styles.push(CellStyle {
                bold: true,
                ..sheet.styles.first().cloned().unwrap_or_default()
            });
            let header = headers.iter().map(|header| (*header).into()).collect();
            let mut widths = vec![12; headers.len()];
            let mut cells = Vec::new();
            for (row, fields) in std::iter::once(header).chain(rows).enumerate() {
                for (column, text) in fields.into_iter().enumerate() {
                    if text.is_empty() {
                        continue;
                    }
                    if text.encode_utf16().count() > 32_767 {
                        return Err(crate::Error::new(format!(
                            "{name}: cell in row {} exceeds the XLSX text limit",
                            row + 1
                        )));
                    }
                    let text: String = text
                        .chars()
                        .map(|ch| match ch {
                            '\t' | '\n' | '\r' => ch,
                            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => '\u{fffd}',
                            _ => ch,
                        })
                        .collect();
                    widths[column] = widths[column].max((text.chars().count() + 3).min(70));
                    let value = CellValue::Text(text.clone());
                    cells.push(SheetCell {
                        row: row as u32,
                        column: column as u32,
                        kind: CellKind::of(&value),
                        value,
                        text,
                        style: if row == 0 { header_style } else { 0 },
                        formula: None,
                        phonetic: None,
                        runs: None,
                    });
                }
            }
            sheet.cells = cells.into();
            sheet.columns = widths
                .into_iter()
                .map(|width| width as i64 * 76_200)
                .collect();
            sheet.custom_columns = (0..headers.len() as u32).collect();
            sheet.frozen = Some((0, 1));
            if last_row > 0 && headers.len() > 1 {
                sheet.auto_filter = Some(AutoFilter {
                    range: CellRange {
                        first_row: 0,
                        first_column: 0,
                        last_row: last_row as u32,
                        last_column: headers.len() as u32 - 1,
                    },
                    columns: Vec::new(),
                });
            }
            sheets.push((name.into(), sheet));
        }
        Ok(sheets)
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(before: Value, after: Value, old: Value, new: Value) -> Result<String, String> {
        let bytes = [before, after, old, new].map(|value| value.to_string().into_bytes());
        markdown([
            ("old.cdx.json", &bytes[0]),
            ("new.cdx.json", &bytes[1]),
            ("old.osv.json", &bytes[2]),
            ("new.osv.json", &bytes[3]),
        ])
    }

    fn bom(versions: &[&str]) -> Value {
        let components: Vec<_> = versions
            .iter()
            .map(|version| {
                json!({
                    "type": "library",
                    "name": "example-crate",
                    "purl": format!("pkg:cargo/example-crate@{version}")
                })
            })
            .collect();
        json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.6",
            "metadata": { "component": {
                "type": "application",
                "name": "synthetic",
                "components": components
            }}
        })
    }

    fn scan_with(id: &str, version: &str, affected_package: &str, range: &str) -> Value {
        json!({"results": [{
            "source": {"path": "Cargo.lock", "type": "lockfile"},
            "packages": [{
                "package": {"name": "example-crate", "ecosystem": "crates.io", "version": version},
                "vulnerabilities": [{"id": id, "affected": [{
                    "package": {"name": affected_package, "ecosystem": "crates.io"},
                    "ranges": [{"type": range, "events": [{"introduced": "0"}, {"fixed": "1.1.0"}]}]
                }]}]
            }]
        }]})
    }

    #[test]
    fn auto_open_and_explicit_export_keep_their_detection_rules() {
        let empty_scan = br#"{"results":[]}"#;
        assert!(open_table(("generic.json", empty_scan)).unwrap().is_none());
        assert!(open_table(("scan.OSV.JSON", empty_scan)).unwrap().is_some());
        assert!(table_xlsx(("generic.json", empty_scan)).is_ok());

        let malformed_bom = br#"{"bomFormat":"CycloneDX","specVersion":"1.6","components":[{"components":[null]}]}"#;
        let expected = "expected a CycloneDX component object";
        assert_eq!(
            open_table(("bom.json", malformed_bom)).err().as_deref(),
            Some(expected)
        );
        assert_eq!(
            table_xlsx(("bom.json", malformed_bom)).err().as_deref(),
            Some(expected)
        );

        assert!(open_table(("generic.json", b"{")).unwrap().is_none());
        assert!(table_xlsx(("generic.json", b"{")).is_err());
    }

    #[test]
    #[ignore = "manual SBOM open timing"]
    fn large_sbom_open_timing() {
        use std::time::Instant;

        let counts =
            std::env::var("IBIS_SBOM_BENCH_ROWS").unwrap_or_else(|_| "1000,10000,50000".into());
        for count in counts
            .split(',')
            .map(|count| count.parse::<usize>().unwrap())
        {
            let components = (0..count)
                .map(|index| {
                    json!({
                        "type": "library",
                        "name": format!("demo-{index}"),
                        "version": "1.0.0",
                        "purl": format!("pkg:cargo/demo-{index}@1.0.0"),
                        "bom-ref": format!("ref-{index}"),
                    })
                })
                .collect::<Vec<_>>();
            let bytes = serde_json::to_vec(&json!({
                "bomFormat": "CycloneDX",
                "specVersion": "1.6",
                "components": components,
            }))
            .unwrap();
            let input = ("synthetic.cdx.json", bytes.as_slice());

            let start = Instant::now();
            let doc = parse(input).unwrap();
            let format = ReportFormat::detect(&doc, input.0).unwrap();
            let parsed = Instant::now();
            let tables = report_tables(format, &doc, input).unwrap();
            drop(doc);
            let projected = Instant::now();
            let opened = assemble_workbook(tables).unwrap();
            let assembled = Instant::now();
            let xlsx = opened
                .source
                .save(&opened.presentation, crate::SaveOptions::default())
                .unwrap();
            drop(opened);
            let saved = Instant::now();
            let reopened = crate::open(xlsx).unwrap();
            let finished = Instant::now();

            let slide = &reopened.presentation.slides[0];
            let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(slide).unwrap();
            assert_eq!(
                sheet.cell(count as u32, 3).unwrap().text,
                format!("demo-{}", count - 1)
            );
            eprintln!(
                "{count} components / {} bytes: parse {:?}, project {:?}, assemble {:?}, save {:?}, reopen {:?}, total {:?}",
                bytes.len(),
                parsed - start,
                projected - parsed,
                assembled - projected,
                saved - assembled,
                finished - saved,
                finished - start,
            );
        }
    }

    #[test]
    fn distinguishes_fixed_boundary_evidence_from_absent_detection() {
        let old = scan_with("TEST-100", "1.0.0", "example-crate", "SEMVER");
        let empty = json!({"results": []});
        let out = report(bom(&["1.0.0"]), bom(&["1.1.0"]), old.clone(), empty.clone()).unwrap();
        assert!(out.contains("advisory fixed version listed in new SBOM"));
        assert!(out.contains("[TEST-100](https://osv.dev/vulnerability/TEST-100)"));
        assert!(out.contains("| Old SBOM | old.cdx.json | `"));
        for versions in [&[][..], &["1.0.0"], &["1.2.0"], &["1.0.0", "1.1.0"]] {
            let out = report(bom(&["1.0.0"]), bom(versions), old.clone(), empty.clone()).unwrap();
            assert!(out.contains("inventory evidence inconclusive"));
        }
        for (name, range) in [("other-crate", "SEMVER"), ("example-crate", "GIT")] {
            let out = report(
                bom(&["1.0.0"]),
                bom(&["1.1.0"]),
                scan_with("TEST-100", "1.0.0", name, range),
                empty.clone(),
            )
            .unwrap();
            assert!(out.contains("inventory evidence inconclusive"));
        }
    }

    #[test]
    fn compares_exact_ids_and_escapes_untrusted_cells() {
        let old = scan_with("TEST-100", "1.0.0", "example-crate", "SEMVER");
        let empty = json!({"results": []});
        let persistent = report(bom(&["1.0.0"]), bom(&["1.0.0"]), old.clone(), old).unwrap();
        assert!(persistent.contains("Still detected"));
        let new = scan_with(
            "TEST|[x](https://x)\n<h1>",
            "1.1.0",
            "example-crate",
            "SEMVER",
        );
        let out = report(bom(&[]), bom(&["1.1.0"]), empty, new).unwrap();
        assert!(out.contains("Newly detected"));
        assert!(out.contains("TEST&#124;&#91;x&#93;"));
        assert!(!out.contains("<h1>"));
        assert!(!out.contains("https://x"));
    }

    #[test]
    fn rejects_missing_or_malformed_results() {
        for bad in [
            json!({}),
            json!({"results": null}),
            json!({"results": [{}]}),
            json!({"results": [{"packages": [{"vulnerabilities": null}]}]}),
        ] {
            let error = report(bom(&[]), bom(&[]), json!({"results": []}), bad).unwrap_err();
            assert!(error.starts_with("New scan:"));
        }
    }

    #[test]
    fn review_workbook_reopens_with_findings_and_inventory() {
        let bytes = [
            bom(&["1.0.0"]),
            bom(&["1.1.0"]),
            scan_with("TEST-100", "1.0.0", "example-crate", "SEMVER"),
            json!({"results": []}),
        ]
        .map(|value| value.to_string().into_bytes());
        let output = review_xlsx([
            ("old.cdx.json", &bytes[0]),
            ("new.cdx.json", &bytes[1]),
            ("old.osv.json", &bytes[2]),
            ("new.osv.json", &bytes[3]),
        ])
        .unwrap();
        let opened = crate::open(output).unwrap();
        let slides = &opened.presentation.slides;
        assert_eq!(slides.len(), 5);
        let sheet =
            |index: usize| ibis_doc_model::sheet_calc::sheet_of_slide(&slides[index]).unwrap();
        let findings = sheet(0);
        assert!(
            findings
                .cell(1, 0)
                .unwrap()
                .text
                .contains("fixed version listed")
        );
        assert_eq!(findings.frozen, Some((0, 1)));
        assert_eq!(findings.auto_filter.as_ref().unwrap().range.last_column, 15);
        assert_eq!(
            sheet(2).cell(2, 5).unwrap().text,
            "pkg:cargo/example-crate@1.1.0"
        );
        let artifacts = sheet(3);
        for (column, header) in (0..).zip(["Role", "File", "SHA-256"]) {
            assert_eq!(artifacts.cell(0, column).unwrap().text, header);
        }
        for (row, ((role, name), input)) in (1..).zip(
            [
                ("Old SBOM", "old.cdx.json"),
                ("New SBOM", "new.cdx.json"),
                ("Old scan", "old.osv.json"),
                ("New scan", "new.osv.json"),
            ]
            .into_iter()
            .zip(&bytes),
        ) {
            assert_eq!(artifacts.cell(row, 0).unwrap().text, role);
            assert_eq!(artifacts.cell(row, 1).unwrap().text, name);
            assert_eq!(
                artifacts.cell(row, 2).unwrap().text,
                format!("{:x}", Sha256::digest(input))
            );
        }
        assert_eq!(sheet(4).cell(1, 0).unwrap().text, COVERAGE);
    }

    #[test]
    fn standalone_workbooks_preserve_text_and_reject_other_json() {
        let input = json!({"bomFormat": "CycloneDX", "specVersion": "1.6", "components": [
            {"type": "library", "name": "=1+2", "version": "001", "purl": "pkg:deb/debian/example@001"}
        ]}).to_string();
        let opened = crate::open(table_xlsx(("bom.json", input.as_bytes())).unwrap()).unwrap();
        let sheet =
            ibis_doc_model::sheet_calc::sheet_of_slide(&opened.presentation.slides[0]).unwrap();
        assert_eq!(sheet.cell(1, 3).unwrap().text, "=1+2");
        assert_eq!(sheet.cell(1, 3).unwrap().formula, None);
        assert_eq!(
            sheet.cell(1, 4).unwrap().value,
            ibis_doc_model::CellValue::Text("001".into())
        );

        let input = scan_with("TEST-100", "1.0.0", "example-crate", "SEMVER").to_string();
        let opened = crate::open(table_xlsx(("scan.json", input.as_bytes())).unwrap()).unwrap();
        let sheet =
            ibis_doc_model::sheet_calc::sheet_of_slide(&opened.presentation.slides[0]).unwrap();
        assert_eq!(sheet.cell(1, 3).unwrap().text, "TEST-100");
        assert_eq!(sheet.cell(1, 4).unwrap().text, "1.1.0");
        assert!(table_xlsx(("other.json", b"{}")).is_err());
    }

    #[test]
    fn interprets_supported_package_urls() {
        assert_eq!(
            component_identity("pkg:npm/%40scope/tool@1.2.3?repository_url=example"),
            Some((("npm".into(), "@scope/tool".into()), Some("1.2.3".into())))
        );
        assert_eq!(
            component_identity("pkg:pypi/Some._pkg"),
            Some((("PyPI".into(), "some-pkg".into()), None))
        );
        assert!(component_identity("pkg:rpm/fedora/pkg@1").is_none());
    }
}
