// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::{Row, Table, array};
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

pub(super) fn tables(doc: &Value, items: &[(&Value, &str)]) -> Result<Vec<Table>, String> {
    let mut assets = Vec::new();
    let mut crypto_refs = BTreeSet::new();
    let mut names = BTreeMap::new();
    for &(component, parent) in items {
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
        return Ok(Vec::new());
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
                if crypto_refs.contains(source) || crypto_refs.contains(target) {
                    edges.insert((source, kind, target));
                }
            }
        }
    }
    let rows: Vec<Row> = edges
        .into_iter()
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
    Ok(vec![
        ("Cryptographic assets", ASSET_COLUMNS, assets),
        ("Crypto relationships", &["Source reference", "Source", "Relationship", "Target reference", "Target"], rows),
        ("Crypto notes", &["Scope"], vec![
            vec!["Relationships are declarations in the source BOM. provides means implementation or provision. dependsOn records a dependency. Confirm actual runtime use separately.".into()],
            vec!["Related material values are excluded. This table contains selected inventory metadata and does not assess PQC readiness.".into()],
        ]),
    ])
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use ibis_doc_model::sheet_calc::sheet_of_slide;

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
