// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
use serde_json::Value;

use std::ops::Range;

pub struct Finding {
    pub field: String,
    pub code: &'static str,
    pub target: Option<String>,
    pub line: Option<usize>,
    pub severity: Option<&'static str>,
}

/// Inspect a bounded Markdown frontmatter prefix. Unresolved local targets are advisory.
pub fn inspect(path: &str, prefix: &str, exists: impl Fn(&str) -> bool) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut add = |field: &str, code, target| {
        findings.push(Finding {
            field: field.into(),
            code,
            target,
            line: None,
            severity: None,
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
        if allows_descriptor && target.contains(char::is_whitespace) {
            return;
        }
        if let Some(code) = path_problem(path, target, |relative, _| {
            if exists(relative) {
                LinkTarget::Present
            } else {
                LinkTarget::MissingFile
            }
        }) {
            add(field, code, Some(target.into()));
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

pub enum LinkTarget {
    Present,
    MissingFile,
    MissingFragment,
}

fn path_problem(
    path: &str,
    target: &str,
    check: impl FnOnce(&str, Option<&str>) -> LinkTarget,
) -> Option<&'static str> {
    let first = target.split(['/', '?', '#']).next().unwrap_or_default();
    if target.starts_with("//") || first.contains(':') {
        return None;
    }
    match super::resolve_reference(path, target, true) {
        Some((relative, fragment)) if !relative.is_empty() => match check(&relative, fragment) {
            LinkTarget::Present => None,
            LinkTarget::MissingFile => Some("unresolved_local_target"),
            LinkTarget::MissingFragment => Some("unresolved_heading_fragment"),
        },
        Some(_) => None,
        None => Some("outside_or_invalid_path"),
    }
}

/// Find broken CommonMark links in a body whose first line is at `first_line` in the file.
pub fn inspect_body<'a>(
    path: &'a str,
    body: &'a str,
    first_line: usize,
    mut check: impl FnMut(&str, Option<&str>) -> LinkTarget + 'a,
) -> impl Iterator<Item = Finding> + 'a {
    let newlines: Vec<_> = body.match_indices('\n').map(|(offset, _)| offset).collect();
    body_events(body).filter_map(move |(event, range)| {
        let Event::Start(Tag::Link {
            link_type,
            dest_url,
            ..
        }) = event
        else {
            return None;
        };
        if matches!(link_type, LinkType::Email | LinkType::Autolink) || dest_url.is_empty() {
            return None;
        }
        Some(Finding {
            field: "body.link".into(),
            code: path_problem(path, &dest_url, &mut check)?,
            target: Some(dest_url.into_string()),
            line: Some(line_at(&newlines, first_line, range.start)),
            severity: None,
        })
    })
}

pub fn inspect_reserved_body(
    path: &str,
    body: &str,
    first_line: usize,
    limit: usize,
) -> Vec<Finding> {
    super::reserved::inspect(path, body, first_line, limit)
}

pub(super) fn body_events(body: &str) -> impl Iterator<Item = (Event<'_>, Range<usize>)> {
    Parser::new_ext(body, Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES).into_offset_iter()
}

pub(super) fn line_at(newlines: &[usize], first_line: usize, offset: usize) -> usize {
    first_line + newlines.partition_point(|newline| *newline < offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_links_use_occurrence_lines_and_ignore_code_and_non_links() {
        let body = r#"[Inline](missing.md)
[Reference][Ref]

[Ref]: /nested/missing.md

`[inline code](ignored.md)`

~~~md
[Fenced](ignored.md)
~~~

    [Indented](ignored.md)

![Image](ignored.png)
[[wiki]]
<https://example.org/>
[Parent directory](../)
[Existing asset](/assets/a%20b.png?raw=1)
[Fragment](#heading)
[Escape](../../elsewhere.md)
"#;
        let findings: Vec<_> = inspect_body("nested/note.md", body, 5, |path, _| {
            if matches!(path, "nested/note.md" | "assets/a b.png") {
                LinkTarget::Present
            } else {
                LinkTarget::MissingFile
            }
        })
        .map(|finding| (finding.line, finding.code, finding.target.unwrap()))
        .collect();
        assert_eq!(
            findings,
            [
                (Some(5), "unresolved_local_target", "missing.md".into()),
                (
                    Some(6),
                    "unresolved_local_target",
                    "/nested/missing.md".into()
                ),
                (
                    Some(24),
                    "outside_or_invalid_path",
                    "../../elsewhere.md".into()
                ),
            ]
        );
    }
}
