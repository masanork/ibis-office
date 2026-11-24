// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::HashMap;

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
                Some(EditableLabel {
                    shape_id: shape.attribute("ID")?.parse().ok()?,
                    text: label_text(plain_text_node(shape)?.text()?).to_owned(),
                })
            })
            .collect())
    }

    /// Replace a local, single-run label and return a new VSDX package.
    /// The original archive and the other package parts are preserved.
    pub fn replace_label(&self, page: usize, shape_id: u32, replacement: &str) -> Result<Vec<u8>> {
        self.replace_labels(page, &[(shape_id, replacement)])
    }

    /// Apply several independent label edits on one page in a single save.
    pub fn replace_labels(&self, page: usize, edits: &[(u32, &str)]) -> Result<Vec<u8>> {
        let mut replacements = HashMap::new();
        let mut total = 0usize;
        for &(shape, replacement) in edits {
            total = total.saturating_add(replacement.len());
            if replacement.is_empty() || total > MAX_LABEL_CHARS || replacements.insert(shape, replacement).is_some()
                || replacement.chars().any(|ch| !matches!(ch, '\t' | '\n' | ' '..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
            {
                return Err(Error::new("Invalid or oversized diagram label"));
            }
        }
        let page = self.pages.get(page).ok_or_else(invalid)?;
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
        let mut source = part(&zip, &page.name)?;
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
                let label = plain_text_node(target).ok_or_else(|| {
                    Error::new("This Visio label uses unsupported formatting or fields")
                })?;
                if label.text().map(label_text) != Some(replacement) {
                    let mut range = label.range();
                    let raw = &source[range.clone()];
                    // Keep the final paragraph terminator emitted by Visio.
                    if raw.ends_with("\r\n") {
                        range.end -= 2;
                    } else if raw.ends_with(['\n', '\r']) {
                        range.end -= 1;
                    }
                    patches.push((range, ibis_formats::docx_convert::xml_escape(replacement)));
                }
            }
        }
        if patches.is_empty() {
            return Ok(self.bytes.to_vec());
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
        package.set_bytes(&page.name, source.into_bytes());
        package.save(SaveOptions::default()).map_err(package_error)
    }
}

fn label_text(text: &str) -> &str {
    text.strip_suffix('\n').unwrap_or(text)
}

fn shapes<'a, 'i>(root: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    root.descendants().filter(|&n| named(n, "Shape"))
}

fn plain_text_node<'a, 'i>(shape: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    let label = child(shape, "Text")?;
    let mut text_nodes = label.children().filter(Node::is_text);
    let text = text_nodes.next().filter(|_| text_nodes.next().is_none())?;
    let mut markers = [false; 3];
    for node in label.children().filter(Node::is_element) {
        let index = ["cp", "pp", "tp"].iter().position(|tag| named(node, tag))?;
        if markers[index] || node.range().start > text.range().start || node.first_child().is_some()
        {
            return None;
        }
        markers[index] = true;
    }
    let raw = label.document().input_text().get(text.range())?;
    (!raw.contains('<')).then_some(text)
}

fn package_error(error: ibis_ooxml::Error) -> Error {
    Error::new(error.to_string())
}
