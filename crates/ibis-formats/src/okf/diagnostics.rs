// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use serde_json::Value;

pub struct Finding {
    pub field: String,
    pub code: &'static str,
    pub target: Option<String>,
}

/// Inspect a bounded Markdown frontmatter prefix. Unresolved local targets are advisory.
pub fn inspect(path: &str, prefix: &str, exists: impl Fn(&str) -> bool) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut add = |field: &str, code, target| {
        findings.push(Finding {
            field: field.into(),
            code,
            target,
        })
    };
    let (metadata, _) = match super::markdown_frontmatter(prefix) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            if prefix.starts_with("---\n") || prefix.starts_with("---\r\n") {
                add("frontmatter", "incomplete_frontmatter", None);
            }
            return findings;
        }
        Err(_) => {
            add("frontmatter", "invalid_yaml_mapping", None);
            return findings;
        }
    };
    match path.rsplit('/').next() {
        Some("log.md" | "index.md") if path != "index.md" => {
            add("frontmatter", "unexpected_reserved_frontmatter", None);
            return findings;
        }
        _ => {}
    }
    if path == "index.md" {
        if metadata
            .get("okf_version")
            .is_some_and(|value| super::review::present(value) && !value.is_string())
        {
            add("okf_version", "invalid_version", None);
        }
        return findings;
    }
    let metadata = Value::Object(metadata);
    match &metadata["type"] {
        Value::Null => return findings,
        Value::String(value) if value.trim().is_empty() => return findings,
        Value::String(_) => {}
        _ => {
            add("type", "invalid_type", None);
            return findings;
        }
    }
    if super::tags(metadata.get("tags")).is_none() {
        add("tags", "invalid_tags", None);
    }
    let review = super::Review::from_metadata(&metadata);
    if review.malformed_generation {
        add("generated.at", "invalid_timestamp", None);
    }
    if review.freshness() == super::Freshness::Malformed {
        add("stale_after", "invalid_timestamp", None);
    }
    let verified = &metadata["verified"];
    if !verified.is_null() && verified.as_str() != Some("") {
        for (index, event) in verified
            .as_array()
            .map_or_else(|| std::slice::from_ref(verified), Vec::as_slice)
            .iter()
            .enumerate()
        {
            if event["by"].as_str().is_none_or(|by| by.trim().is_empty()) {
                add(&format!("verified[{index}].by"), "invalid_verifier", None);
            }
            if super::review::timestamp(&event["at"]).is_none() {
                add(&format!("verified[{index}].at"), "invalid_timestamp", None);
            }
        }
    }
    let mut check = |field: &str, value: &Value, allows_descriptor: bool| {
        if value.is_null() || value.as_str() == Some("") {
            return;
        }
        let Some(target) = value.as_str() else {
            add(field, "invalid_path_value", None);
            return;
        };
        let first = target.split(['/', '?', '#']).next().unwrap_or_default();
        if target.starts_with("//") || first.contains(':') {
            return;
        }
        if allows_descriptor && target.contains(char::is_whitespace) {
            return;
        }
        let path_target = target.split(['?', '#']).next().unwrap_or_default();
        let resolved = super::resolve(path, path_target.trim_end_matches('/'));
        if let Some((relative, _)) = resolved {
            if !exists(&relative) {
                add(field, "unresolved_local_target", Some(target.into()));
            }
        } else if path_target != "/" {
            add(field, "outside_or_invalid_path", Some(target.into()));
        }
    };
    for (field, value) in [
        ("resource", &metadata["resource"]),
        ("computation", &metadata["computation"]),
        ("executor.resource", &metadata["executor"]["resource"]),
        ("attester.resource", &metadata["attester"]["resource"]),
        ("superseded_by", &metadata["superseded_by"]),
    ] {
        check(field, value, false);
    }
    let sources = &metadata["sources"];
    for (index, source) in sources
        .as_array()
        .map_or_else(|| std::slice::from_ref(sources), Vec::as_slice)
        .iter()
        .enumerate()
    {
        check(
            &format!("sources[{index}].resource"),
            &source["resource"],
            true,
        );
    }
    findings
}
