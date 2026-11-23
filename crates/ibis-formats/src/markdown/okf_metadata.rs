// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::DocProperties;
use serde_json::{Map, Value};

use super::{SourceDiagnostic, SourceIssue};

fn mapping(source: &str) -> Option<Map<String, Value>> {
    serde_saphyr::from_str(source).ok()
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn tags(value: Option<&Value>) -> Option<Vec<&str>> {
    match value {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(Value::String(tag)) if tag.is_empty() => Some(Vec::new()),
        Some(Value::String(tag)) => Some(vec![tag]),
        Some(Value::Array(items)) => items.iter().map(Value::as_str).collect(),
        _ => None,
    }
}

fn properties(metadata: &Map<String, Value>) -> DocProperties {
    DocProperties {
        title: text(metadata.get("title")),
        description: text(metadata.get("description")),
        keywords: tags(metadata.get("tags"))
            .filter(|tags| !tags.is_empty())
            .map(|tags| tags.join(", ")),
        ..DocProperties::default()
    }
}

pub(super) fn import(source: &str, target: &mut DocProperties) {
    if let Some(metadata) = mapping(source) {
        let projected = properties(&metadata);
        target.title = projected.title;
        target.description = projected.description;
        target.keywords = projected.keywords;
    }
}

pub(super) fn update(
    mut source: String,
    current: &DocProperties,
    diagnostics: &mut Vec<SourceDiagnostic>,
) -> String {
    let Some(mut metadata) = mapping(&source) else {
        return source;
    };
    let baseline = properties(&metadata);
    for (key, old, new) in [
        ("title", baseline.title, current.title.as_deref()),
        (
            "description",
            baseline.description,
            current.description.as_deref(),
        ),
        ("tags", baseline.keywords, current.keywords.as_deref()),
    ] {
        let new = new.filter(|value| !value.is_empty());
        if old.as_deref() == new {
            continue;
        }
        let replacement = if key == "tags" {
            tags(metadata.get(key))
                .filter(|items| items.iter().all(|tag| !tag.contains(',')))
                .map(|_| {
                    Value::Array(
                        new.unwrap_or_default()
                            .split(',')
                            .map(str::trim)
                            .filter(|tag| !tag.is_empty())
                            .map(|tag| Value::String(tag.into()))
                            .collect(),
                    )
                })
        } else if metadata
            .get(key)
            .is_none_or(|value| value.is_null() || value.is_string())
        {
            Some(new.map_or(Value::Null, |value| Value::String(value.into())))
        } else {
            None
        };
        if let Some(value) = replacement {
            let mut expected = metadata.clone();
            expected.insert(key.into(), value.clone());
            if let Some(updated) = replace(&source, key, &value.to_string(), &metadata, &expected) {
                source = updated;
                metadata = expected;
                continue;
            }
        }
        diagnostics.push(SourceDiagnostic {
            extension: "okf",
            location: key.into(),
            issue: SourceIssue::Incompatible,
        });
    }
    source
}

fn replace(
    source: &str,
    key: &str,
    value: &str,
    original: &Map<String, Value>,
    expected: &Map<String, Value>,
) -> Option<String> {
    if !original.contains_key(key) {
        let newline = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let updated = format!("{source}{key}: {value}{newline}");
        return (mapping(&updated).as_ref() == Some(expected)).then_some(updated);
    }

    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let Some((name, tail)) = line.split_once(':') else {
            continue;
        };
        if name.trim_end() != key {
            continue;
        }
        let tail = tail.trim_end_matches(['\r', '\n']);
        let value_start = start + name.len() + 1;
        let mut value_end = value_start + tail.len();
        for (position, _) in tail.match_indices('#') {
            if position != 0 && !tail.as_bytes()[position - 1].is_ascii_whitespace() {
                continue;
            }
            let comment = value_start + position;
            let without_comment = format!("{}{}", &source[..comment], &source[value_end..]);
            if mapping(&without_comment).as_ref() == Some(original) {
                value_end = comment;
                break;
            }
        }
        let prefix = &source[value_start..value_end];
        if prefix.trim_start().starts_with(['|', '>', '&', '*', '!']) {
            return None;
        }
        let start = value_start + prefix.len() - prefix.trim_start().len();
        let end = value_start + prefix.trim_end().len();
        // Reparse the full mapping to catch multiline values, aliases and other
        // syntax whose meaning would change outside the property being edited.
        let updated = if start >= end {
            format!(
                "{} {value}{}",
                &source[..value_start],
                &source[value_start..]
            )
        } else {
            format!("{}{value}{}", &source[..start], &source[end..])
        };
        return (mapping(&updated).as_ref() == Some(expected)).then_some(updated);
    }
    None
}
