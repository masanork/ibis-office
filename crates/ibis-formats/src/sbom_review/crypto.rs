// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::{
    Row, Table, array, comparison_input_artifacts, component_tree, inventory, parse, write_workbook,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const ASSET_COLUMNS: &[&str] = &[
    "Name",
    "Asset type",
    "BOM reference",
    "Parent",
    "OID",
    "Primitive",
    "Parameter set",
    "Curve",
    "Classical security level",
    "NIST quantum security level",
    "Protocol",
    "Protocol version",
    "Certificate subject",
    "Certificate issuer",
    "Certificate expiry",
    "Signature algorithm reference",
    "Subject public key reference",
    "Material type",
    "Material algorithm reference",
    "Material expiry",
];

fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

const RELATIONSHIP_COLUMNS: &[&str] = &[
    "Source reference",
    "Source",
    "Relationship",
    "Target reference",
    "Target",
];

mod impact;

struct CryptoInventory {
    assets: Vec<Row>,
    edges: Vec<Row>,
    graph: impact::Graph,
}

fn collect(
    doc: &Value,
    items: &[(&Value, &str)],
    trace_dependencies: bool,
) -> Result<CryptoInventory, String> {
    let mut assets = Vec::new();
    let mut crypto_refs = BTreeSet::new();
    let mut names = BTreeMap::new();
    let mut graph = impact::Graph::default();
    if trace_dependencies {
        graph.services(doc)?;
    }
    for &(component, parent) in items {
        if trace_dependencies {
            graph.describe(component, component["type"].as_str().unwrap_or_default());
        }
        let reference = component["bom-ref"].as_str().unwrap_or_default();
        if !reference.is_empty() {
            names.insert(reference, component["name"].as_str().unwrap_or_default());
        }
        if component["type"] != "cryptographic-asset" {
            continue;
        }
        if !reference.is_empty() {
            crypto_refs.insert(reference);
        }
        let crypto = &component["cryptoProperties"];
        let algorithm = &crypto["algorithmProperties"];
        let protocol = &crypto["protocolProperties"];
        let certificate = &crypto["certificateProperties"];
        let material = &crypto["relatedCryptoMaterialProperties"];
        // Only inventory metadata is exported. Material values may contain credentials.
        assets.push(vec![
            scalar(&component["name"]),
            scalar(&crypto["assetType"]),
            reference.into(),
            parent.into(),
            scalar(&crypto["oid"]),
            scalar(&algorithm["primitive"]),
            scalar(&algorithm["parameterSetIdentifier"]),
            scalar(&algorithm["curve"]),
            scalar(&algorithm["classicalSecurityLevel"]),
            scalar(&algorithm["nistQuantumSecurityLevel"]),
            scalar(&protocol["type"]),
            scalar(&protocol["version"]),
            scalar(&certificate["subjectName"]),
            scalar(&certificate["issuerName"]),
            scalar(&certificate["notValidAfter"]),
            scalar(&certificate["signatureAlgorithmRef"]),
            scalar(&certificate["subjectPublicKeyRef"]),
            scalar(&material["type"]),
            scalar(&material["algorithmRef"]),
            scalar(&material["expirationDate"]),
        ]);
    }
    if assets.is_empty() {
        return Ok(CryptoInventory {
            assets,
            edges: Vec::new(),
            graph,
        });
    }

    let mut edges = BTreeSet::new();
    for dependency in array(doc, "dependencies", true)? {
        let Some(source) = dependency["ref"].as_str() else {
            continue;
        };
        for kind in ["dependsOn", "provides"] {
            for target in array(dependency, kind, true)?
                .iter()
                .filter_map(Value::as_str)
            {
                if trace_dependencies
                    || crypto_refs.contains(source)
                    || crypto_refs.contains(target)
                {
                    edges.insert((source, kind, target));
                }
            }
        }
    }
    let rows: Vec<Row> = edges
        .into_iter()
        .filter(|(source, kind, target)| {
            if trace_dependencies {
                graph.add(source, kind, target);
            }
            crypto_refs.contains(source) || crypto_refs.contains(target)
        })
        .map(|(source, kind, target)| {
            vec![
                source.into(),
                names.get(source).copied().unwrap_or_default().into(),
                kind.into(),
                target.into(),
                names.get(target).copied().unwrap_or_default().into(),
            ]
        })
        .collect();
    Ok(CryptoInventory {
        assets,
        edges: rows,
        graph,
    })
}

const ASSET_SHEET: &str = "Cryptographic assets";

/// Whether a workbook just generated by the JSON importer contains a cryptographic inventory.
pub fn is_cbom_inventory(book: &ibis_doc_model::Presentation) -> bool {
    book.workbook.as_ref().is_some_and(|book| {
        book.sheets
            .first()
            .is_some_and(|sheet| sheet.name == ASSET_SHEET)
    })
}

pub(super) fn tables(doc: &Value, items: &[(&Value, &str)]) -> Result<Vec<Table>, String> {
    let CryptoInventory { assets, edges, .. } = collect(doc, items, false)?;
    if assets.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![
        (ASSET_SHEET, ASSET_COLUMNS, assets),
        ("Crypto relationships", RELATIONSHIP_COLUMNS, edges),
        ("Crypto notes", &["Scope"], vec![
            vec!["Relationships are declarations in the source BOM. provides means implementation or provision. dependsOn records a dependency. Confirm actual runtime use separately.".into()],
            vec!["Related material values are excluded. This table contains selected inventory metadata and does not assess PQC readiness.".into()],
        ]),
    ])
}

fn comparison_input(input: (&str, &[u8])) -> Result<CryptoInventory, String> {
    let doc = parse(input)?;
    inventory(&doc)?;
    let items = component_tree(&doc)?;
    let mut refs = BTreeSet::new();
    for (component, _) in &items {
        if let Some(reference) = component["bom-ref"]
            .as_str()
            .filter(|value| !value.is_empty())
        {
            if !refs.insert(reference) {
                return Err(format!("duplicate BOM reference {reference}"));
            }
        }
    }
    collect(&doc, &items, true)
}

fn with_status(status: &str, mut row: Row) -> Row {
    row.insert(0, status.into());
    row
}

/// Compare the stated cryptographic assets and relationships in two CycloneDX inventories.
pub fn compare_xlsx(inputs: [(&str, &[u8]); 2]) -> Result<Vec<u8>, String> {
    let [old, new] =
        inputs.map(|input| comparison_input(input).map_err(|err| format!("{}: {err}", input.0)));
    let (old, new) = (old?, new?);
    let mut asset_changes = Vec::new();
    let mut field_changes = Vec::new();
    let index_assets =
        |rows: &[Row], unmatched: &str, changes: &mut Vec<Row>| -> BTreeMap<String, Row> {
            let mut indexed = BTreeMap::new();
            for row in rows {
                if row[2].is_empty() {
                    changes.push(with_status(unmatched, row.clone()));
                } else {
                    indexed.insert(row[2].clone(), row.clone());
                }
            }
            indexed
        };
    let mut after = index_assets(&new.assets, "Unmatched in new BOM", &mut asset_changes);
    for (reference, before) in index_assets(&old.assets, "Unmatched in old BOM", &mut asset_changes)
    {
        if let Some(current) = after.remove(&reference) {
            if before != current {
                for ((field, from), to) in ASSET_COLUMNS.iter().zip(&before).zip(&current) {
                    if from != to {
                        field_changes.push(vec![
                            reference.clone(),
                            current[0].clone(),
                            (*field).into(),
                            from.clone(),
                            to.clone(),
                        ]);
                    }
                }
                asset_changes.push(with_status("Changed", current));
            }
        } else {
            asset_changes.push(with_status("Removed", before));
        }
    }
    asset_changes.extend(after.into_values().map(|row| with_status("Added", row)));
    let index_edges = |rows: &[Row]| -> BTreeMap<[String; 3], Row> {
        rows.iter()
            .map(|row| {
                (
                    [row[0].clone(), row[2].clone(), row[3].clone()],
                    row.clone(),
                )
            })
            .collect()
    };
    let old_edges = index_edges(&old.edges);
    let new_edges = index_edges(&new.edges);
    let mut relationship_changes = Vec::new();
    for (status, from, to) in [
        ("Removed", &old_edges, &new_edges),
        ("Added", &new_edges, &old_edges),
    ] {
        relationship_changes.extend(
            from.iter()
                .filter(|(key, _)| !to.contains_key(*key))
                .map(|(_, row)| with_status(status, row.clone())),
        );
    }
    let changed_refs: BTreeSet<_> = asset_changes
        .iter()
        .map(|row| &row[3])
        .chain(
            relationship_changes
                .iter()
                .flat_map(|row| [&row[1], &row[4]]),
        )
        .filter(|reference| !reference.is_empty())
        .cloned()
        .collect();
    let (dependents, providers) = impact::rows(
        [
            (impact::SNAPSHOT_LABELS[0], &old),
            (impact::SNAPSHOT_LABELS[1], &new),
        ],
        &changed_refs,
    );
    let systems = impact::systems(&dependents, &old.graph, &new.graph);
    // These headers match the asset view with a leading change column.
    const ASSET_DIFF_COLUMNS: &[&str] = &[
        "Change",
        "Name",
        "Asset type",
        "BOM reference",
        "Parent",
        "OID",
        "Primitive",
        "Parameter set",
        "Curve",
        "Classical security level",
        "NIST quantum security level",
        "Protocol",
        "Protocol version",
        "Certificate subject",
        "Certificate issuer",
        "Certificate expiry",
        "Signature algorithm reference",
        "Subject public key reference",
        "Material type",
        "Material algorithm reference",
        "Material expiry",
    ];
    let first_sheet = if !systems.is_empty() {
        "System impact summary"
    } else if !asset_changes.is_empty() {
        "Asset changes"
    } else if !relationship_changes.is_empty() {
        "Relationship changes"
    } else {
        "Comparison notes"
    };
    let overview = if first_sheet == "Comparison notes" {
        "No differences in the compared asset fields or declared relationships."
    } else {
        "System impact summary groups changes by application or service. Asset changes lists the underlying asset differences."
    };
    let mut tables: Vec<Table> = vec![
        ("System impact summary", impact::SYSTEM_COLUMNS, systems),
        ("Asset changes", ASSET_DIFF_COLUMNS, asset_changes),
        ("Dependency impact", impact::DEPENDENCY_COLUMNS, dependents),
        ("Asset providers", impact::PROVIDER_COLUMNS, providers),
        ("Changed asset fields", &["BOM reference", "Name", "Field", "Old", "New"], field_changes),
        ("Relationship changes", &["Change", "Source reference", "Source", "Relationship", "Target reference", "Target"], relationship_changes),
        comparison_input_artifacts(["Old CBOM", "New CBOM"].into_iter().zip(inputs)),
        ("Comparison notes", &["Topic", "Details"], vec![
            vec!["Start here".into(), overview.into()],
            vec!["Dependency and provision".into(), "Dependency impact shows one shortest declared path per dependent and snapshot. Asset providers shows direct provides declarations.".into()],
            vec!["Matching".into(), "The comparison matches exact BOM references. Assets without a reference appear as unmatched. Changing an edge target appears as a removal and an addition.".into()],
            vec!["Blank asset cell".into(), "No declared path connects the system to a changed asset in that snapshot.".into()],
            vec!["Scope".into(), "These sheets compare metadata and relationships declared in the CBOMs. Confirm actual runtime use separately.".into()],
        ]),
    ];
    let first = tables
        .iter()
        .position(|table| table.0 == first_sheet)
        .unwrap();
    // Preserve the reading order of the other sheets.
    tables[..=first].rotate_right(1);
    write_workbook(tables)
}

#[cfg(test)]
mod tests {
    use super::super::{Digest, Sha256, open_table};
    use super::{Value, compare_xlsx as compare_cbom_xlsx};
    use ibis_doc_model::sheet_calc::sheet_of_slide;

    #[test]
    fn compares_rsa_to_ml_dsa_inventory_and_relationships() {
        let before = include_bytes!("../../../../test/fixtures/cbom/payment-before.cdx.json");
        let after = include_bytes!("../../../../test/fixtures/cbom/payment-after.cdx.json");
        let report = compare_cbom_xlsx([("before.json", before), ("after.json", after)]).unwrap();
        let reopened = crate::open(report).unwrap();
        let book = &reopened.presentation;
        let workbook = book.workbook.as_ref().unwrap();
        assert_eq!(
            workbook.sheets[workbook.active_tab].name,
            "System impact summary"
        );
        assert_eq!(
            workbook
                .sheets
                .iter()
                .map(|tab| tab.name.as_str())
                .collect::<Vec<_>>(),
            [
                "System impact summary",
                "Asset changes",
                "Dependency impact",
                "Asset providers",
                "Changed asset fields",
                "Relationship changes",
                "Input artifacts",
                "Comparison notes",
            ]
        );
        let sheet = |name: &str| {
            let tab = workbook.sheets.iter().find(|tab| tab.name == name).unwrap();
            sheet_of_slide(book.slide(tab.slide.unwrap()).unwrap()).unwrap()
        };
        let values = |name, row, columns: &[u32]| -> Vec<&str> {
            columns
                .iter()
                .map(|column| sheet(name).cell(row, *column).unwrap().text.as_str())
                .collect()
        };
        assert_eq!(
            values("Asset changes", 1, &[0, 1]),
            ["Changed", "Demo payment certificate"]
        );
        assert_eq!(
            values("Asset changes", 2, &[0, 1]),
            ["Removed", "Demo key entry"]
        );
        assert_eq!(values("Asset changes", 3, &[0, 1]), ["Removed", "RSA-2048"]);
        assert_eq!(values("Asset changes", 4, &[0, 1]), ["Added", "ML-DSA-65"]);
        assert_eq!(
            values("Changed asset fields", 2, &[2, 3, 4]),
            ["Signature algorithm reference", "rsa", "ml-dsa"]
        );
        assert_eq!(
            values("Relationship changes", 1, &[0, 1, 3, 4]),
            ["Removed", "cert", "dependsOn", "rsa"]
        );
        assert_eq!(
            values("Relationship changes", 2, &[0, 1, 3, 4]),
            ["Removed", "provider", "provides", "rsa"]
        );
        assert_eq!(
            values("Relationship changes", 3, &[0, 1, 3, 4]),
            ["Added", "cert", "dependsOn", "ml-dsa"]
        );
        assert_eq!(
            values("Relationship changes", 4, &[0, 1, 3, 4]),
            ["Added", "provider", "provides", "ml-dsa"]
        );
        assert!(sheet("Relationship changes").cell(5, 0).is_none());
        assert_eq!(
            values("Dependency impact", 1, &[0, 1, 3, 7]),
            ["Old CBOM", "rsa", "payments", "payments → cert → rsa"]
        );
        assert_eq!(
            values("Dependency impact", 2, &[0, 1, 3, 7]),
            ["Old CBOM", "cert", "payments", "payments → cert"]
        );
        assert_eq!(
            values("Dependency impact", 3, &[0, 1, 3, 7]),
            ["New CBOM", "ml-dsa", "payments", "payments → cert → ml-dsa"]
        );
        assert_eq!(
            values("Dependency impact", 4, &[0, 1, 3, 7]),
            ["New CBOM", "cert", "payments", "payments → cert"]
        );
        assert_eq!(
            values("System impact summary", 1, &[0, 1, 2, 3, 4]),
            [
                "payments",
                "Demo payment service",
                "Demo payment service",
                "application",
                "application"
            ]
        );
        assert_eq!(
            values("System impact summary", 1, &[5, 6]),
            [
                "Demo payment certificate [cert] · RSA-2048 [rsa]",
                "Demo payment certificate [cert] · ML-DSA-65 [ml-dsa]"
            ]
        );
        assert!(sheet("System impact summary").cell(2, 0).is_none());
        assert!(sheet("Dependency impact").cell(5, 0).is_none());
        assert_eq!(
            values("Asset providers", 1, &[0, 1, 3, 6]),
            ["Old CBOM", "rsa", "provider", "provides"]
        );
        assert_eq!(
            values("Asset providers", 2, &[0, 1, 3, 6]),
            ["New CBOM", "ml-dsa", "provider", "provides"]
        );
        assert_eq!(
            sheet("Input artifacts").cell(1, 2).unwrap().text,
            format!("{:x}", Sha256::digest(before))
        );
        assert_eq!(
            sheet("Input artifacts").cell(2, 2).unwrap().text,
            format!("{:x}", Sha256::digest(after))
        );
        assert!(
            book.slides
                .iter()
                .filter_map(sheet_of_slide)
                .flat_map(|sheet| sheet.cells.iter())
                .all(|cell| !cell.text.contains("SYNTHETIC-MATERIAL-VALUE"))
        );
    }

    #[test]
    fn opens_unchanged_comparisons_with_an_explanation() {
        let input = include_bytes!("../../../../test/fixtures/cbom/payment-before.cdx.json");
        let saved = compare_cbom_xlsx([("old.json", input), ("new.json", input)]).unwrap();
        let opened = crate::open(saved).unwrap();
        let book = opened.presentation.workbook.as_ref().unwrap();
        let tab = &book.sheets[book.active_tab];
        assert_eq!(tab.name, "Comparison notes");
        let slide = opened.presentation.slide(tab.slide.unwrap()).unwrap();
        assert!(
            sheet_of_slide(slide)
                .unwrap()
                .cell(1, 1)
                .unwrap()
                .text
                .starts_with("No differences")
        );
    }

    #[test]
    fn reports_missing_identity_without_guessing_and_rejects_duplicate_references() {
        let doc = |assets: Value| {
            serde_json::json!({
                "bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1, "components": assets
            })
            .to_string()
        };
        let input = doc(serde_json::json!([{"type": "cryptographic-asset", "name": "Example"}]));
        let report =
            compare_cbom_xlsx([("old", input.as_bytes()), ("new", input.as_bytes())]).unwrap();
        let opened = crate::open(report).unwrap();
        let book = opened.presentation.workbook.as_ref().unwrap();
        assert_eq!(book.sheets[book.active_tab].name, "Asset changes");
        let sheet = sheet_of_slide(&opened.presentation.slides[0]).unwrap();
        assert_eq!(sheet.cell(1, 0).unwrap().text, "Unmatched in new BOM");
        assert_eq!(sheet.cell(2, 0).unwrap().text, "Unmatched in old BOM");

        let duplicate = doc(serde_json::json!([
            {"type": "cryptographic-asset", "name": "One", "bom-ref": "same"},
            {"type": "cryptographic-asset", "name": "Two", "bom-ref": "same"}
        ]));
        let error = compare_cbom_xlsx([("old", duplicate.as_bytes()), ("new", input.as_bytes())])
            .unwrap_err();
        assert_eq!(error, "old: duplicate BOM reference same");
    }

    #[test]
    fn opens_and_reopens_synthetic_cbom_with_only_inventory_metadata() {
        let input = (
            "payment-demo.cdx.json",
            include_bytes!("../../../../test/fixtures/cbom/payment-demo.cdx.json").as_slice(),
        );
        let opened = open_table(input).unwrap().unwrap();
        let saved = opened
            .source
            .save(&opened.presentation, crate::SaveOptions::default())
            .unwrap();
        let reopened = crate::open(saved).unwrap();
        let book = &reopened.presentation;
        let tabs: Vec<_> = book
            .workbook
            .as_ref()
            .unwrap()
            .sheets
            .iter()
            .map(|tab| tab.name.as_str())
            .collect();
        assert_eq!(
            tabs,
            [
                "Cryptographic assets",
                "Crypto relationships",
                "Crypto notes",
                "Components",
                "Input artifact"
            ]
        );
        let sheet = |index: usize| sheet_of_slide(&book.slides[index]).unwrap();
        assert_eq!(sheet(0).cell(1, 0).unwrap().text, "RSA-2048");
        assert_eq!(sheet(0).cell(1, 3).unwrap().text, "provider");
        assert_eq!(sheet(0).cell(2, 9).unwrap().text, "3");
        assert_eq!(sheet(0).cell(3, 14).unwrap().text, "2028-12-31T00:00:00Z");
        assert_eq!(sheet(0).cell(4, 10).unwrap().text, "tls");
        assert_eq!(sheet(0).cell(5, 17).unwrap().text, "private-key");
        assert!(sheet(1).cells.iter().any(|cell| cell.text == "provides"));
        assert!(sheet(1).cells.iter().any(|cell| cell.text == "dependsOn"));
        assert_eq!(
            sheet(4).cell(1, 1).unwrap().text,
            format!("{:x}", Sha256::digest(input.1))
        );
        assert!(
            book.slides
                .iter()
                .filter_map(sheet_of_slide)
                .flat_map(|sheet| sheet.cells.iter())
                .all(|cell| !cell.text.contains("SYNTHETIC-MATERIAL-VALUE"))
        );
    }
}
