// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::{BTreeMap, HashMap};

use ibis_ooxml::package::{Package, SaveOptions};
use ibis_pdf::{Error, Result};
use roxmltree::Node;

use super::{
    MAX_LABEL_CHARS, MAX_PART, VsdxContainer, child, invalid, named, open_archive, part,
    relationships, xml,
};

#[derive(Debug, PartialEq, Eq)]
pub struct EditableLabel {
    pub shape_id: u32,
    pub text: String,
    pub has_multiple_runs: bool,
}

impl VsdxContainer {
    /// In page order, the local labels whose formatting can be preserved by a text-only edit.
    pub fn editable_labels(&self, page: usize) -> Result<Vec<EditableLabel>> {
        let page = self.pages.get(page).ok_or_else(invalid)?;
        let zip = open_archive(&self.bytes)?;
        let source = part(&zip, &page.name)?;
        let doc = xml(&source)?;
        Ok(shapes(doc.root_element())
            .filter_map(|shape| {
                let runs = text_runs(shape)?;
                Some(EditableLabel {
                    shape_id: shape.attribute("ID")?.parse().ok()?,
                    text: joined_label(&runs),
                    has_multiple_runs: runs.len() > 1,
                })
            })
            .collect())
    }

    /// Replace a local label and return a new VSDX package.
    /// The original archive and the other package parts are preserved.
    pub fn replace_label(&self, page: usize, shape_id: u32, replacement: &str) -> Result<Vec<u8>> {
        self.replace_labels(page, &[(shape_id, replacement)])
    }

    /// Apply several independent label edits on one page in a single save.
    pub fn replace_labels(&self, page: usize, edits: &[(u32, &str)]) -> Result<Vec<u8>> {
        self.replace_document_labels(edits.iter().map(|&(shape, text)| (page, shape, text)))
    }

    /// Apply label edits across pages atomically, keeping all other package parts.
    pub fn replace_document_labels<'a>(
        &self,
        edits: impl IntoIterator<Item = (usize, u32, &'a str)>,
    ) -> Result<Vec<u8>> {
        let mut by_page = BTreeMap::<_, HashMap<_, _>>::new();
        let mut total = 0usize;
        for (page, shape, replacement) in edits {
            total = total.saturating_add(replacement.len());
            if self.pages.get(page).is_none()
                || total > MAX_LABEL_CHARS
                || by_page.entry(page).or_default().insert(shape, replacement).is_some()
                || replacement.chars().any(|ch| !matches!(ch, '\t' | '\n' | ' '..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
            {
                return Err(Error::new("Invalid or oversized diagram label"));
            }
        }
        if by_page.is_empty() {
            return Ok(self.bytes.to_vec());
        }
        self.update_pages(
            by_page
                .into_iter()
                .map(|(page, replacements)| (page, move |source| patch_page(source, replacements))),
        )
    }

    pub(crate) fn update_pages(
        &self,
        updates: impl IntoIterator<Item = (usize, impl FnOnce(String) -> Result<Option<Vec<u8>>>)>,
    ) -> Result<Vec<u8>> {
        let zip = open_archive(&self.bytes)?;
        let root_rels = relationships(&zip, "")?;
        let mut package = Package::open(self.bytes.clone()).map_err(package_error)?;
        if root_rels
            .iter()
            .any(|r| r.kind.contains("/digital-signature/"))
            || package
                .names()
                .any(|name| name.to_ascii_lowercase().starts_with("_xmlsignatures/"))
        {
            return Err(Error::new("Signed Visio drawings cannot be edited"));
        }
        let mut changed = false;
        for (page, update) in updates {
            let name = &self.pages.get(page).ok_or_else(invalid)?.name;
            if let Some(updated) = update(part(&zip, name)?)? {
                if updated.len() > MAX_PART {
                    return Err(Error::new("Drawing page is too large"));
                }
                package.set_bytes(name, updated);
                changed = true;
            }
        }
        if changed {
            package.save(SaveOptions::default()).map_err(package_error)
        } else {
            Ok(self.bytes.to_vec())
        }
    }
}

fn patch_page(mut source: String, replacements: HashMap<u32, &str>) -> Result<Option<Vec<u8>>> {
    let mut patches = Vec::new();
    {
        let doc = xml(&source)?;
        let mut targets = HashMap::new();
        for shape in shapes(doc.root_element()) {
            if let Some(id) = shape.attribute("ID").and_then(|id| id.parse::<u32>().ok())
                && replacements.contains_key(&id)
                && targets.insert(id, shape).is_some()
            {
                return Err(invalid());
            }
        }
        for (id, replacement) in replacements {
            let target = *targets.get(&id).ok_or_else(invalid)?;
            let runs = text_runs(target)
                .ok_or_else(|| Error::new("This Visio label uses unsupported markup or fields"))?;
            let original = joined_label(&runs);
            if original != replacement {
                let (run, updated) = replace_run(&runs, &original, replacement)?;
                let mut range = run.range();
                let raw = &source[range.clone()];
                if run == runs[runs.len() - 1] {
                    // Keep the final paragraph terminator emitted by Visio.
                    if raw.ends_with("\r\n") {
                        range.end -= 2;
                    } else if raw.ends_with(['\n', '\r']) {
                        range.end -= 1;
                    }
                }
                let suffix = if updated.is_empty() && range.end == run.range().end {
                    "\n"
                } else {
                    ""
                };
                patches.push((
                    range,
                    ibis_formats::docx_convert::xml_escape(&updated) + suffix,
                ));
            }
        }
    }
    if patches.is_empty() {
        return Ok(None);
    }
    let removed = patches.iter().map(|(range, _)| range.len()).sum::<usize>();
    let added = patches.iter().map(|(_, text)| text.len()).sum::<usize>();
    if source.len() - removed + added > MAX_PART {
        return Err(Error::new("Drawing page is too large"));
    }
    patches.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, text) in patches {
        source.replace_range(range, &text);
    }
    Ok(Some(source.into_bytes()))
}

fn joined_label(runs: &[Node<'_, '_>]) -> String {
    let mut label = String::new();
    for run in runs {
        label.push_str(run.text().unwrap_or_default());
    }
    if label.ends_with('\n') {
        label.pop();
    }
    label
}

fn replace_run<'a, 'i>(
    runs: &[Node<'a, 'i>],
    original: &str,
    replacement: &str,
) -> Result<(Node<'a, 'i>, String)> {
    if let [run] = runs {
        return Ok((*run, replacement.to_owned()));
    }
    // Match whole Unicode characters so each offset is also a UTF-8 boundary.
    let start: usize = original
        .chars()
        .zip(replacement.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum();
    let suffix: usize = original[start..]
        .chars()
        .rev()
        .zip(replacement[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum();
    let end = original.len() - suffix;
    let mut offset = 0;
    for (index, run) in runs.iter().enumerate() {
        let last = index + 1 == runs.len();
        let text = run.text().unwrap_or_default();
        let text = if last {
            text.strip_suffix('\n').unwrap_or(text)
        } else {
            text
        };
        let run_end = offset + text.len();
        // Inserting exactly between two runs has no unambiguous format.
        if start >= offset
            && end <= run_end
            && (start != end || (start != offset || index == 0) && (start != run_end || last))
        {
            let mut updated = text.to_owned();
            updated.replace_range(
                start - offset..end - offset,
                &replacement[start..replacement.len() - suffix],
            );
            if !updated.is_empty() {
                return Ok((*run, updated));
            }
        }
        offset = run_end;
    }
    Err(Error::new(
        "Edit one formatted run at a time and keep at least one character in that run",
    ))
}

fn shapes<'a, 'i>(root: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    root.descendants().filter(|&n| named(n, "Shape"))
}

fn text_runs<'a, 'i>(shape: Node<'a, 'i>) -> Option<Vec<Node<'a, 'i>>> {
    let label = child(shape, "Text")?;
    let mut runs = Vec::new();
    for node in label.children() {
        if node.is_text() {
            let raw = label.document().input_text().get(node.range())?;
            if raw.contains('<') {
                return None;
            }
            runs.push(node);
        } else if node.is_element()
            && (!(named(node, "cp") || named(node, "pp") || named(node, "tp"))
                || node.first_child().is_some())
        {
            return None;
        }
    }
    (!runs.is_empty()).then_some(runs)
}

fn package_error(error: ibis_ooxml::Error) -> Error {
    Error::new(error.to_string())
}
