// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Shared Markdown extension boundaries. Each extension defines how to validate or update its source.

use super::split_frontmatter_body;

/// Physical framing does not imply that the content is usable YAML.
fn frontmatter_frame(text: &str) -> Option<(&str, &str)> {
    let content = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    split_frontmatter_body(content)
}

fn yaml_mapping(metadata: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    match serde_saphyr::from_str(metadata).ok()? {
        serde_json::Value::Object(fields) => Some(fields),
        _ => None,
    }
}

fn law_profile(fields: &serde_json::Map<String, serde_json::Value>) -> bool {
    fields.get("x-ibis-format").or_else(|| fields.get("type"))
        .and_then(serde_json::Value::as_str) == Some("lawtext")
}

// The writer puts this canonical marker on the first metadata line.
// A nested field, legacy type alias, or later prose line is not an
// independent declaration when the YAML cannot be validated.
fn has_recoverable_law_marker(metadata: &str) -> bool {
    metadata.lines().find(|line| !line.trim().is_empty())
        .is_some_and(|line| line.trim_end() == "x-ibis-format: lawtext")
}

pub fn law_profile_is_declared(metadata: &str) -> bool {
    yaml_mapping(metadata).as_ref().map_or_else(
        || has_recoverable_law_marker(metadata), law_profile,
    )
}

pub struct DocumentFrontmatter<'a> {
    pub metadata: &'a str,
    pub body: &'a str,
    pub lawtext: bool,
    pub damaged: bool,
}

pub fn document_frontmatter(text: &str) -> Option<DocumentFrontmatter<'_>> {
    let (metadata, body) = frontmatter_frame(text)?;
    if let Some(fields) = yaml_mapping(metadata) {
        return Some(DocumentFrontmatter { metadata, body, lawtext: law_profile(&fields), damaged: false });
    }
    has_recoverable_law_marker(metadata).then_some(DocumentFrontmatter {
        metadata, body, lawtext: true, damaged: true,
    })
}

pub(crate) fn frontmatter(text: &str) -> Option<(&str, &str)> {
    let (metadata, body) = frontmatter_frame(text)?;
    // Unmarked ambiguous thematic breaks are Markdown, not metadata.
    yaml_mapping(metadata).map(|_| (metadata, body))
}

pub(crate) fn metadata_value<'a>(metadata: &'a str, key: &str) -> Option<&'a str> {
    metadata
        .lines()
        .filter(|line| !line.starts_with(char::is_whitespace))
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim_end() == key).then(|| value.trim().trim_matches(['\'', '"']))
        })
}

const PROFILE_PROPERTY: &str = "ibisOffice.MarkdownProfile";

pub(super) fn store_profile(profile: String, properties: &mut ibis_doc_model::DocProperties) {
    store_text_property(PROFILE_PROPERTY, profile, properties);
}

fn store_text_property(name: &str, value: String, properties: &mut ibis_doc_model::DocProperties) {
    properties.custom.retain(|property| property.name != name);
    properties.custom.push(ibis_doc_model::CustomProperty {
        name: name.to_owned(),
        value: ibis_doc_model::CustomPropertyValue::Text(value),
    });
}

pub(super) fn saved_profile(properties: &ibis_doc_model::DocProperties) -> Option<&str> {
    saved_text_property(PROFILE_PROPERTY, properties)
}

fn saved_text_property<'a>(name: &str, properties: &'a ibis_doc_model::DocProperties) -> Option<&'a str> {
    let property = properties
        .custom
        .iter()
        .find(|property| property.name == name)?;
    match &property.value {
        ibis_doc_model::CustomPropertyValue::Text(text) => Some(text),
        _ => None,
    }
}

const FRONTMATTER_PROPERTY: &str = "ibisOffice.MarkdownFrontmatter";

pub(super) fn store_unknown_frontmatter(source: &str, properties: &mut ibis_doc_model::DocProperties) {
    use base64::Engine as _;
    if !source.is_empty() {
        store_text_property(FRONTMATTER_PROPERTY, base64::engine::general_purpose::STANDARD.encode(source), properties);
    }
}

pub(super) fn saved_unknown_frontmatter(properties: &ibis_doc_model::DocProperties) -> Option<String> {
    use base64::Engine as _;
    let source = saved_text_property(FRONTMATTER_PROPERTY, properties)?;
    String::from_utf8(base64::engine::general_purpose::STANDARD.decode(source).ok()?).ok()
}

const OKF_SOURCE_PREFIX: &str = "okf-editable-base64:";
const LEGACY_OKF_SOURCE_PREFIX: &str = "okf-base64:";

pub(super) fn store_okf_source(source: &str, properties: &mut ibis_doc_model::DocProperties) {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(source);
    store_profile(format!("{OKF_SOURCE_PREFIX}{encoded}"), properties);
}

pub(super) fn decode_okf_source(profile: &str) -> Option<(String, bool)> {
    use base64::Engine as _;
    let (encoded, editable) = profile
        .strip_prefix(OKF_SOURCE_PREFIX)
        .map(|encoded| (encoded, true))
        .or_else(|| {
            profile
                .strip_prefix(LEGACY_OKF_SOURCE_PREFIX)
                .map(|encoded| (encoded, false))
        })?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    Some((String::from_utf8(data).ok()?, editable))
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
pub fn reconcile_source<T, U>(
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
