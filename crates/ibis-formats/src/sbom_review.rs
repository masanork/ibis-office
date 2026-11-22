// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

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

fn versions(values: Option<&BTreeSet<String>>) -> String {
    values.map_or_else(
        || "—".into(),
        |values| {
            values
                .iter()
                .map(|v| {
                    if v.is_empty() {
                        "unknown".into()
                    } else {
                        cell(v)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        },
    )
}

/// Compare two CycloneDX inventories and OSV-Scanner reports, with source hashes.
/// Each input contains a display name and the original JSON bytes.
pub fn markdown(inputs: [(&str, &[u8]); 4]) -> Result<String, String> {
    let [old_bom, new_bom, old_scan, new_scan] = inputs.map(|(name, bytes)| {
        serde_json::from_slice::<Value>(bytes).map_err(|e| format!("{name}: {e}"))
    });
    let old_bom = inventory(&old_bom?).map_err(|e| format!("Old SBOM: {e}"))?;
    let new_bom = inventory(&new_bom?).map_err(|e| format!("New SBOM: {e}"))?;
    let old_scan = scan(&old_scan?).map_err(|e| format!("Old scan: {e}"))?;
    let new_scan = scan(&new_scan?).map_err(|e| format!("New scan: {e}"))?;

    let mut out = String::from("# Vulnerability remediation review\n\n");
    out.push_str("Status compares exact OSV IDs and package identities in the supplied scans. SBOM evidence uses Package URLs for crates.io, npm, PyPI and Go. A missing detection can reflect scan coverage or configuration. Verify scan coverage, asset identity and deployment separately.\n\n");
    out.push_str("| Status | Ecosystem | Package | OSV advisory | Old scan | New scan | Old SBOM | New SBOM | Advisory fixed versions (old scan) |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    let keys: BTreeSet<_> = old_scan.keys().chain(new_scan.keys()).collect();
    for key in &keys {
        let (package, id) = key;
        let old = old_scan.get(*key);
        let new = new_scan.get(*key);
        let before = old_bom.get(package);
        let after = new_bom.get(package);
        let status = match (old, new) {
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
        };
        let advisory = if id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            format!("[{id}](https://osv.dev/vulnerability/{id})")
        } else {
            cell(id)
        };
        writeln!(
            out,
            "| {status} | {} | {} | {advisory} | {} | {} | {} | {} | {} |",
            cell(&package.0),
            cell(&package.1),
            versions(old.map(|f| &f.versions)),
            versions(new.map(|f| &f.versions)),
            versions(before),
            versions(after),
            versions(old.map(|f| &f.fixed).filter(|f| !f.is_empty())),
        )
        .unwrap();
    }
    if keys.is_empty() {
        out.push_str("\nThe supplied scans contain no reported advisories.\n");
    }
    out.push_str("\n## Input artifacts\n\n| Role | File | SHA-256 |\n| --- | --- | --- |\n");
    for (role, (name, bytes)) in ["Old SBOM", "New SBOM", "Old scan", "New scan"]
        .into_iter()
        .zip(inputs)
    {
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
