// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::BTreeMap;
use std::ops::Range;

use roxmltree::{Document, Node};

use super::{
    LawDoc, parse_law_xml, parse_lawtext, parse_markdown_law_with_diagnostics,
    to_law_xml, to_lawtext, to_markdown,
};
use crate::Error;

/// The editable representation generated from the XML original.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LawEditFormat {
    Lawtext,
    Markdown,
}

impl LawEditFormat {
    fn render(self, law: &LawDoc) -> String {
        match self {
            Self::Lawtext => to_lawtext(law),
            Self::Markdown => to_markdown(law),
        }
    }

    fn parse(self, text: &str) -> Result<(LawDoc, Vec<LawPatchDiagnostic>), Error> {
        match self {
            Self::Lawtext => Ok((parse_lawtext(text)?, Vec::new())),
            Self::Markdown => {
                let (law, diagnostics) = parse_markdown_law_with_diagnostics(text)?;
                Ok((law, diagnostics.into_iter().map(|diagnostic| LawPatchDiagnostic {
                    path: diagnostic.location,
                    reason: diagnostic.issue.as_str(),
                }).collect()))
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct LawPatchDiagnostic {
    pub path: String,
    pub reason: &'static str,
}

/// A withheld patch has diagnostics and no XML output.
#[derive(Debug, PartialEq, Eq)]
pub struct LawSourcePatch {
    pub xml: Option<String>,
    pub applied_edits: usize,
    pub diagnostics: Vec<LawPatchDiagnostic>,
}

impl LawSourcePatch {
    fn withheld(diagnostics: Vec<LawPatchDiagnostic>) -> Self {
        Self { xml: None, applied_edits: 0, diagnostics }
    }
}

/// Apply unambiguous plain sentence edits to the original XML without
/// reserializing its other nodes. The batch is withheld if any edit is unsafe.
pub fn apply_law_source_edits(
    original_xml: &str,
    edited: &str,
    format: LawEditFormat,
) -> Result<LawSourcePatch, Error> {
    let source = parse_law_xml(original_xml)?;
    let baseline = format.render(&source);
    if lines(&baseline).eq(lines(edited)) {
        return Ok(LawSourcePatch {
            xml: Some(original_xml.to_owned()), applied_edits: 0, diagnostics: Vec::new(),
        });
    }

    let (after, diagnostics) = format.parse(edited)?;
    if !diagnostics.is_empty() {
        return Ok(LawSourcePatch::withheld(diagnostics));
    }
    let (before, _) = format.parse(&baseline)?;
    let before_xml = to_law_xml(&before);
    let after_xml = to_law_xml(&after);
    let primary = parse_tree(original_xml)?;
    let before = parse_tree(&before_xml)?;
    let after = parse_tree(&after_xml)?;

    let mut plan = PatchPlan { markdown: format == LawEditFormat::Markdown, ..PatchPlan::default() };
    plan.compare(before.root_element(), after.root_element(), Some(primary.root_element()), "/Law");
    if !plan.diagnostics.is_empty() {
        return Ok(LawSourcePatch::withheld(plan.diagnostics));
    }

    let mut output = original_xml.to_owned();
    plan.replacements.sort_by_key(|(range, _)| range.start);
    for (range, text) in plan.replacements.iter().rev() {
        output.replace_range(range.clone(), text);
    }

    // Check the edit against the XML we actually patched. This catches edits
    // silently dropped by either parser as well as incorrectly matched nodes.
    let replay = format.render(&parse_law_xml(&output)?);
    if !editable_lines(&replay).eq(editable_lines(edited))
        || !generated_lines(&baseline).eq(generated_lines(edited))
    {
        return Ok(LawSourcePatch::withheld(vec![LawPatchDiagnostic {
            path: "/Law".to_owned(),
            reason: "edit_not_reproducible",
        }]));
    }
    Ok(LawSourcePatch {
        xml: Some(output),
        applied_edits: plan.replacements.len(),
        diagnostics: Vec::new(),
    })
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.trim_start_matches('\u{feff}').trim_end().lines().map(str::trim_end)
}

// The projection may regenerate the table snapshot after a cell edit. The
// edited document must still carry the snapshot generated from the original.
fn generated_lines(text: &str) -> impl Iterator<Item = &str> {
    lines(text).filter_map(|line| {
        if is_table_snapshot(line) {
            return Some(line);
        }
        sentence_comment(line).map(|(_, metadata)| metadata)
    })
}

fn editable_lines(text: &str) -> impl Iterator<Item = &str> {
    lines(text).filter(|line| !is_table_snapshot(line))
        .map(|line| sentence_comment(line).map_or(line, |(body, _)| body.trim_end()))
}

fn sentence_comment(line: &str) -> Option<(&str, &str)> {
    let (body, metadata) = line.rsplit_once(super::MARKDOWN_SENTENCE_COMMENT)?;
    metadata.ends_with(" -->").then_some((body, metadata))
}

fn is_table_snapshot(line: &str) -> bool {
    line.starts_with("law_appdx_tables: ") || line.starts_with(":law-appdx-tables: ")
}

fn parse_tree(xml: &str) -> Result<Document<'_>, Error> {
    Document::parse(xml).map_err(|err| Error::new(err.to_string()))
}

#[derive(Default)]
struct PatchPlan {
    markdown: bool,
    replacements: Vec<(Range<usize>, String)>,
    diagnostics: Vec<LawPatchDiagnostic>,
}

impl PatchPlan {
    fn reject(&mut self, path: &str, reason: &'static str) {
        self.diagnostics.push(LawPatchDiagnostic { path: path.to_owned(), reason });
    }

    fn compare(
        &mut self,
        before: Node<'_, '_>,
        after: Node<'_, '_>,
        original: Option<Node<'_, '_>>,
        path: &str,
    ) {
        let plain_markdown_sentence = self.markdown
            && before.has_tag_name("Sentence")
            && after.attributes().len() == 0
            && direct_text(before) != direct_text(after);
        if before.tag_name() != after.tag_name()
            || (!same_attributes(before, after) && !plain_markdown_sentence)
        {
            self.reject(path, "structure_or_attributes_changed");
            return;
        }
        let old_children: Vec<_> = before.children().filter(Node::is_element).collect();
        let new_children: Vec<_> = after.children().filter(Node::is_element).collect();
        if old_children.len() != new_children.len() {
            self.reject(path, "structure_or_attributes_changed");
            return;
        }

        let old_text = direct_text(before);
        let new_text = direct_text(after);
        if old_text != new_text {
            if !old_children.is_empty() || !before.has_tag_name("Sentence") {
                self.reject(path, "unsupported_text_edit");
                return;
            }
            let Some(text_node) = original.and_then(|node| {
                let mut children = node.children();
                let text = children.next()?;
                (text.is_text() && text.text() == Some(&old_text) && children.next().is_none())
                    .then_some(text)
            }) else {
                self.reject(path, "original_text_not_unambiguous");
                return;
            };
            let mut escaped = String::new();
            super::xml::push_xml_escaped(&mut escaped, &new_text);
            self.replacements.push((text_node.range(), escaped));
        }

        let mut original_by_identity = BTreeMap::<_, Vec<_>>::new();
        if let Some(parent) = original {
            for child in parent.children().filter(Node::is_element) {
                original_by_identity.entry(node_identity(child)).or_default().push(child);
            }
        }
        let mut counts = BTreeMap::<_, usize>::new();
        for &old in &old_children {
            *counts.entry(node_identity(old)).or_default() += 1;
        }
        let mut occurrences = BTreeMap::<_, usize>::new();
        for (index, (&old, new)) in old_children.iter().zip(new_children).enumerate() {
            let identity = node_identity(old);
            let occurrence = occurrences.entry(identity).or_default();
            let count = counts[&identity];
            let child_path = match identity.1 {
                Some(key) => format!("{path}/{}[{key}]", identity.0),
                None => format!("{path}/{}[{}]", identity.0, *occurrence + 1),
            };
            let original_child = original_by_identity.get(&identity)
                .filter(|candidates| candidates.len() == count && (identity.1.is_none() || count == 1))
                .map(|candidates| candidates[*occurrence]);
            *occurrence += 1;
            // A move among siblings without distinct identifiers can otherwise
            // appear to be a series of ordinary sentence replacements.
            if count > 1 && node_xml(old) != node_xml(new)
                && old_children.iter().enumerate().any(|(other_index, other)| {
                    index != other_index && node_identity(*other) == identity
                        && node_xml(*other) == node_xml(new)
                })
            {
                self.reject(&child_path, "ambiguous_sibling_reorder");
            } else {
                self.compare(old, new, original_child, &child_path);
            }
        }
    }
}

fn node_identity<'a, 'input>(node: Node<'a, 'input>) -> (&'a str, Option<&'a str>) {
    (
        node.tag_name().name(),
        node.attribute("AmendLawNum").or_else(|| node.attribute("Num")),
    )
}

fn node_xml<'a, 'input>(node: Node<'a, 'input>) -> &'input str {
    &node.document().input_text()[node.range()]
}

fn direct_text(node: Node<'_, '_>) -> String {
    node.children().filter(Node::is_text)
        .filter_map(|text| text.text())
        .filter(|text| !text.trim().is_empty())
        .collect()
}

fn same_attributes(before: Node<'_, '_>, after: Node<'_, '_>) -> bool {
    before.attributes().len() == after.attributes().len()
        && before.attributes().all(|attr| {
            let value = match attr.namespace() {
                Some(uri) => after.attribute((uri, attr.name())),
                None => after.attribute(attr.name()),
            };
            value == Some(attr.value())
        })
}
