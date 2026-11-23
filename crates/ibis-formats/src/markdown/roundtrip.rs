// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Shared Markdown extension boundaries. Each extension defines how to validate or update its source.

use super::split_frontmatter_body;

pub(crate) fn frontmatter(text: &str) -> Option<(&str, &str)> {
    let content = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n"))?;
    split_frontmatter_body(content)
}

pub(crate) fn has_document_type(text: &str, expected: &str) -> bool {
    let Some((metadata, _)) = frontmatter(text) else {
        return false;
    };
    document_type_is(metadata, expected)
}

pub(crate) fn document_type_is(metadata: &str, expected: &str) -> bool {
    metadata.lines().any(|line| {
        if line.starts_with(char::is_whitespace) {
            return false;
        }
        line.split_once(':').is_some_and(|(key, value)| {
            key.trim_end() == "type"
                && value.trim().trim_matches(['\'', '"']) == expected
        })
    })
}

const PROFILE_PROPERTY: &str = "ibisOffice.MarkdownProfile";

pub(super) fn store_profile(profile: String, properties: &mut ibis_doc_model::DocProperties) {
    properties.custom.retain(|property| property.name != PROFILE_PROPERTY);
    properties.custom.push(ibis_doc_model::CustomProperty {
        name: PROFILE_PROPERTY.to_owned(),
        value: ibis_doc_model::CustomPropertyValue::Text(profile),
    });
}

pub(super) fn saved_profile(properties: &ibis_doc_model::DocProperties) -> Option<&str> {
    let property = properties.custom.iter().find(|property| property.name == PROFILE_PROPERTY)?;
    match &property.value {
        ibis_doc_model::CustomPropertyValue::Text(text) => Some(text),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SourceIssue {
    Missing,
    Ambiguous,
    Incompatible,
    Invalid,
}

impl SourceIssue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Ambiguous => "ambiguous",
            Self::Incompatible => "incompatible",
            Self::Invalid => "invalid",
        }
    }
}

/// An authored Markdown extension that could not be written back to its source.
#[derive(Debug, PartialEq, Eq)]
pub struct SourceDiagnostic {
    pub extension: &'static str,
    pub location: String,
    pub issue: SourceIssue,
}

/// Reuse or update source only when a single eligible snapshot can be reconciled.
pub(crate) fn reconcile_source<T, U>(
    candidates: impl IntoIterator<Item = T>,
    reconcile: impl FnOnce(T) -> Option<U>,
) -> Result<U, SourceIssue> {
    let mut candidates = candidates.into_iter();
    let source = candidates.next().ok_or(SourceIssue::Missing)?;
    if candidates.next().is_some() {
        return Err(SourceIssue::Ambiguous);
    }
    reconcile(source).ok_or(SourceIssue::Incompatible)
}
