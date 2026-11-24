// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Format-level reading and path resolution for Open Knowledge Format 0.3.

use serde_json::{Map, Value};

pub mod diagnostics;
mod review;
pub use review::{Freshness, Review, Trust};

/// Structured frontmatter for a concept. The original source belongs to the caller.
pub fn concept(path: &str, text: &str) -> Result<Option<Value>, crate::Error> {
    if !path.ends_with(".md")
        || path.split('/').any(|name| name.starts_with('.'))
        || matches!(path.rsplit('/').next(), Some("index.md" | "log.md"))
    {
        return Ok(None);
    }
    let Some((metadata, _)) = markdown_frontmatter(text)? else {
        return Ok(None);
    };
    Ok(metadata
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        .then_some(Value::Object(metadata)))
}

/// Accept the OKF list and singleton forms without splitting scalar tags.
pub fn tags(value: Option<&Value>) -> Option<Vec<&str>> {
    match value {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(Value::String(tag)) if tag.is_empty() => Some(Vec::new()),
        Some(Value::String(tag)) => Some(vec![tag]),
        Some(Value::Array(items)) => items.iter().map(Value::as_str).collect(),
        _ => None,
    }
}

/// Parse the YAML mapping and return the Markdown following it.
pub(crate) fn markdown_frontmatter(text: &str) -> Result<Option<(Map<String, Value>, &str)>, crate::Error> {
    let Some(after_open) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return Ok(None);
    };
    let mut end = 0;
    for line in after_open.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let metadata = serde_saphyr::from_str(&after_open[..end])
                .map_err(|error| crate::Error::new(error.to_string()))?;
            return Ok(Some((metadata, &after_open[end + line.len()..])));
        }
        end += line.len();
    }
    Ok(None)
}

/// Resolve an OKF 0.3 URI reference against the containing file or bundle root.
/// Returns a bundle-relative path and optional fragment. Callers must still enforce
/// the bundle boundary when opening the file.
pub fn resolve<'a>(from: &str, reference: &'a str) -> Option<(String, Option<&'a str>)> {
    let (target, fragment) = reference
        .split_once('#')
        .map_or((reference, None), |(path, fragment)| (path, Some(fragment)));
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    if path.starts_with("//") || path.split('/').next()?.contains(':') {
        return None;
    }
    if path.is_empty() {
        return Some((from.to_owned(), fragment));
    }
    let mut parts = Vec::new();
    if !path.starts_with('/') {
        parts.extend(from.split('/').map(str::to_owned));
        parts.pop();
    }
    for segment in path.trim_start_matches('/').split('/') {
        let name = percent_encoding::percent_decode_str(segment)
            .decode_utf8()
            .ok()?;
        match name.as_ref() {
            "." => {}
            ".." => {
                parts.pop()?;
            }
            name if !name.is_empty()
                && !name.starts_with('.')
                && !name.contains(['/', '\\', '\0', ':']) =>
            {
                parts.push(name.to_owned());
            }
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| (parts.join("/"), fragment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_concepts_and_parses_nested_metadata() {
        let text = "---\ntype: Metric\nverified:\n  by: human:reviewer\n  at: '2026-11-23'\nx-extra: { nested: [one, two] }\n---\n";
        let metadata = concept("metrics/one.md", text).unwrap().unwrap();
        assert_eq!(metadata["verified"]["by"].as_str(), Some("human:reviewer"));
        assert_eq!(metadata["x-extra"]["nested"][1].as_str(), Some("two"));
        assert!(concept("metrics/index.md", text).unwrap().is_none());
        assert!(concept("README.md", "# Read me").unwrap().is_none());
    }

    #[test]
    fn resolves_relative_and_root_paths_without_escaping() {
        assert_eq!(
            resolve("nested/note.md", "../a%20b.md#heading"),
            Some(("a b.md".into(), Some("heading")))
        );
        assert_eq!(
            resolve("nested/note.md", "/a.md"),
            Some(("a.md".into(), None))
        );
        for invalid in [
            "https://example.com",
            "//example.com",
            "../../a.md",
            "/%2e%2e/a.md",
            "/a%2fb.md",
        ] {
            assert_eq!(resolve("nested/note.md", invalid), None);
        }
    }
}
