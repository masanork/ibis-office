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

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SourceIssue {
    Missing,
    Ambiguous,
    Incompatible,
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
