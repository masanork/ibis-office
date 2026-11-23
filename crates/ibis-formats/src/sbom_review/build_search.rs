// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::{component_tree, parse, validate_cyclonedx, write_workbook};
use ibis_doc_model::{
    CellValue,
    query::{DiskDataset, QColumn, QTable, QType, SqliteQuerySession, SqliteScalar},
};
use sha2::{Digest, Sha256};

const HEADERS: &[&str] = &[
    "Input file",
    "SHA-256",
    "Subject",
    "Subject version",
    "Component",
    "Component version",
    "Package URL",
    "BOM reference",
];

fn package_key(purl: &str) -> &str {
    let path = purl.split(['?', '#']).next().unwrap_or(purl);
    let last_segment = path.rsplit('/').next().unwrap_or(path);
    match last_segment.split_once('@') {
        Some((_, version)) => &path[..path.len() - version.len() - 1],
        None => path,
    }
}

/// Search local CycloneDX snapshots by exact PURL or its versionless package coordinate.
/// Retains each matching occurrence with its source filename and digest.
pub fn search_xlsx(inputs: &[(&str, &[u8])], purl: &str) -> Result<Vec<u8>, String> {
    const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
    const MAX_COMPONENTS: usize = 100_000;
    if !purl.starts_with("pkg:") || purl.len() <= 4 || purl.chars().any(char::is_whitespace) {
        return Err("Supply a Package URL starting with pkg:".into());
    }
    if inputs.is_empty() {
        return Err("Supply at least one CycloneDX SBOM".into());
    }
    if inputs
        .iter()
        .map(|(_, bytes)| bytes.len())
        .fold(0usize, usize::saturating_add)
        > MAX_INPUT_BYTES
    {
        return Err("The combined SBOM inputs exceed 32 MiB".into());
    }

    let columns = [
        "file",
        "sha256",
        "subject",
        "subject_version",
        "component",
        "component_version",
        "purl",
        "bom_reference",
        "package_key",
    ];
    let mut data = QTable {
        columns: columns
            .into_iter()
            .map(|name| QColumn {
                name: name.into(),
                ty: QType::Text,
            })
            .collect(),
        rows: Vec::new(),
    };
    let mut remaining_text = 64 * 1024 * 1024usize;
    for &(name, bytes) in inputs {
        let doc = parse((name, bytes))?;
        validate_cyclonedx(&doc).map_err(|error| format!("{name}: {error}"))?;
        let components = component_tree(&doc).map_err(|error| format!("{name}: {error}"))?;
        if components.len() > MAX_COMPONENTS - data.rows.len() {
            return Err("The SBOM inputs exceed 100,000 component occurrences".into());
        }
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        let subject = &doc["metadata"]["component"];
        for (component, _) in components {
            let purl = component["purl"].as_str().unwrap_or_default();
            let fields = [
                name,
                &sha256,
                subject["name"].as_str().unwrap_or_default(),
                subject["version"].as_str().unwrap_or_default(),
                component["name"].as_str().unwrap_or_default(),
                component["version"].as_str().unwrap_or_default(),
                purl,
                component["bom-ref"].as_str().unwrap_or_default(),
                package_key(purl),
            ];
            let bytes = fields.iter().map(|field| field.len()).sum();
            remaining_text = remaining_text
                .checked_sub(bytes)
                .ok_or("The projected SBOM data exceeds 64 MiB of text")?;
            data.rows.push(
                fields
                    .into_iter()
                    .map(|field| CellValue::Text(field.into()))
                    .collect(),
            );
        }
    }

    let session =
        SqliteQuerySession::from_disk_datasets(&[("components".into(), DiskDataset::Table(data))])?;
    let result = session.query(
        "SELECT file, sha256, subject, subject_version, component, component_version, purl, bom_reference
         FROM components WHERE purl = ?1 OR package_key = ?1
         ORDER BY file, sha256, purl, bom_reference, component, component_version",
        &[Some(SqliteScalar::Text(purl))],
        10_000,
        16 * 1024 * 1024,
    )?;
    if result.truncated {
        return Err(
            "Search results exceed the export limit. Try a version-specific Package URL.".into(),
        );
    }
    let rows = result
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or("Expected text in SBOM search result")
                })
                .collect::<Result<_, _>>()
        })
        .collect::<Result<_, _>>()?;
    write_workbook([("Matches", HEADERS, rows)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibis_doc_model::sheet_calc::sheet_of_slide;
    use serde_json::json;

    fn snapshot(release: &str, dependency: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.6",
            "metadata": {"component": {"name": "Checkout", "version": release}},
            "components": [
                {"name": "widget", "version": dependency, "bom-ref": "dep",
                 "purl": format!("pkg:npm/widget@{dependency}")},
                {"name": "widget", "version": dependency, "bom-ref": "unknown"},
                {"name": "unrelated", "purl": format!("pkg:npm/widget-extra@{dependency}")}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn finds_each_snapshot_and_excludes_components_without_purl() {
        let a = snapshot("build-1", "1.0.0");
        let b = snapshot("build-2", "1.0.0");
        let c = snapshot("build-3", "2.0.0");
        let inputs = [
            ("a.json", a.as_slice()),
            ("b.json", b.as_slice()),
            ("c.json", c.as_slice()),
        ];
        for (purl, expected_count) in [
            ("pkg:npm/widget", 3),
            ("pkg:npm/widget@1.0.0", 2),
            ("pkg:npm/widget@0.0.0", 0),
        ] {
            let bytes = search_xlsx(&inputs, purl).unwrap();
            let book = crate::open(bytes).unwrap().presentation;
            assert_eq!(book.workbook.as_ref().unwrap().sheets[0].name, "Matches");
            let sheet = sheet_of_slide(&book.slides[0]).unwrap();
            for (index, (file, source)) in inputs[..expected_count].iter().enumerate() {
                let row = index as u32 + 1;
                let text = |column| sheet.cell(row, column).unwrap().text.as_str();
                assert_eq!(text(0), *file);
                assert_eq!(text(1), format!("{:x}", Sha256::digest(source)));
                assert_eq!(text(3), format!("build-{}", index + 1));
                assert_eq!(text(5), if index == 2 { "2.0.0" } else { "1.0.0" });
                assert_eq!(text(7), "dep");
            }
            assert!(sheet.cell(expected_count as u32 + 1, 0).is_none());
        }
        assert!(search_xlsx(&[("other.json", b"{}")], "pkg:npm/widget").is_err());
    }
}
