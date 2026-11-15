// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Rule names and descriptions for Harunobu-compatible reports.
//!
//! The generated strings are from Harunobu 0.2.2, copyright (c) 2026
//! Digital Agency of Japan, under MIT; see ../HARUNOBU-MIT.txt.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleMetadata {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

include!("machine_readability_rule_metadata_generated.rs");

/// Unknown rule IDs have no metadata, rather than an invented description.
pub fn rule_metadata(id: &str) -> Option<&'static RuleMetadata> {
    RULE_METADATA.iter().find(|rule| rule.id == id)
}
