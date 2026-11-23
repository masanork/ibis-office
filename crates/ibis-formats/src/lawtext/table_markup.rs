// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Validated XML snapshots behind editable appendix-table rows.

use std::borrow::Cow;

use crate::markdown::roundtrip::{SourceDiagnostic, SourceIssue, reconcile_source};

use super::model::{LawAppdxTable, LawDoc};
use super::xml::{child_elem, collect_text, push_xml_escaped};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SavedAppdxTable {
    pub(super) table: LawAppdxTable,
    pub(super) xml: String,
}

impl SavedAppdxTable {
    pub(super) fn read(xml: &str) -> Option<Self> {
        let document = roxmltree::Document::parse(xml).ok()?;
        let root = document.root_element();
        if !root.has_tag_name("AppdxTable") || root.tag_name().namespace().is_some() {
            return None;
        }
        child_elem(child_elem(root, "TableStruct")?, "Table")?;
        Some(Self {
            table: parse_appdx_table(root),
            xml: xml[root.range()].to_string(),
        })
    }
}

fn table_rows<'a, 'input>(appdx: roxmltree::Node<'a, 'input>) -> impl Iterator<Item = Vec<roxmltree::Node<'a, 'input>>> {
    child_elem(appdx, "TableStruct")
        .and_then(|structure| child_elem(structure, "Table"))
        .into_iter()
        .flat_map(|table| table.children().filter(|node| node.has_tag_name("TableRow")))
        .map(|row| row.children().filter(|node| node.has_tag_name("TableColumn")).collect::<Vec<_>>())
        .filter(|row| !row.is_empty())
}

pub(super) fn parse_appdx_table(appdx: roxmltree::Node<'_, '_>) -> LawAppdxTable {
    let title = child_elem(appdx, "AppdxTableTitle")
        .map(collect_text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "別表".to_string());
    let related_article_num = child_elem(appdx, "RelatedArticleNum")
        .map(collect_text)
        .filter(|s| !s.is_empty());
    let rows = table_rows(appdx)
        .map(|row| row.into_iter().map(collect_text).collect())
        .collect();
    LawAppdxTable {
        title,
        related_article_num,
        rows,
    }
}

fn same_shape(saved: &LawAppdxTable, edited: &LawAppdxTable, padded: bool) -> bool {
    saved.title == edited.title
        && saved.related_article_num == edited.related_article_num
        && saved.rows.len() == edited.rows.len()
        && saved.rows.iter().zip(&edited.rows).all(|(stored, visible)| {
            stored.len() == visible.len()
                || padded && visible.get(stored.len()..)
                    .is_some_and(|padding| padding.iter().all(String::is_empty))
        })
}

fn update_cells(
    snapshot: &SavedAppdxTable,
    edited: &LawAppdxTable,
) -> Option<SavedAppdxTable> {
    let document = roxmltree::Document::parse(&snapshot.xml).ok()?;
    let mut xml = String::new();
    let mut cursor = 0;
    for ((cells, stored), visible) in table_rows(document.root_element())
        .zip(&snapshot.table.rows)
        .zip(&edited.rows)
    {
        for ((cell, before), after) in cells.into_iter().zip(stored).zip(visible) {
            if before == after {
                continue;
            }
            // The flat editor can replace a single plain sentence unambiguously.
            let mut elements = cell.children().filter(roxmltree::Node::is_element);
            let sentence = elements.next()?;
            if !sentence.has_tag_name("Sentence")
                || elements.next().is_some()
                || sentence.children().any(|child| !child.is_text())
            {
                return None;
            }
            let range = sentence.range();
            let source = &snapshot.xml[range.clone()];
            xml.push_str(&snapshot.xml[cursor..range.start]);
            if let Some(open) = source.strip_suffix("/>") {
                xml.push_str(open);
                xml.push('>');
            } else {
                let opening_end = sentence.first_child().map_or(source.rfind("</")?, |text| {
                    text.range().start - range.start
                });
                xml.push_str(&source[..opening_end]);
            }
            push_xml_escaped(&mut xml, after);
            xml.push_str("</Sentence>");
            cursor = range.end;
        }
    }
    xml.push_str(&snapshot.xml[cursor..]);
    let updated = SavedAppdxTable::read(&xml)?;
    updated.table.rows.iter().zip(&edited.rows)
        .all(|(stored, visible)| visible.starts_with(stored))
        .then_some(updated)
}

pub(super) fn snapshot_for<'a>(
    snapshots: &'a [SavedAppdxTable],
    table: &LawAppdxTable,
    padded: bool,
) -> Option<Cow<'a, SavedAppdxTable>> {
    let same_text = |snapshot: &&SavedAppdxTable| {
        same_shape(&snapshot.table, table, padded)
            && snapshot.table.rows.iter().zip(&table.rows)
                .all(|(stored, visible)| visible.starts_with(stored))
    };
    match reconcile_source(snapshots.iter().filter(same_text), |snapshot| Some(Cow::Borrowed(snapshot))) {
        Err(SourceIssue::Missing) => reconcile_source(
            snapshots.iter().filter(|snapshot| same_shape(&snapshot.table, table, padded)),
            |snapshot| update_cells(snapshot, table).map(Cow::Owned),
        ).ok(),
        result => result.ok(),
    }
}

pub(super) fn document_snapshot(doc: &LawDoc, index: usize) -> Option<Cow<'_, SavedAppdxTable>> {
    if doc.unresolved_appdx_tables.contains(&index) {
        return None;
    }
    let table = &doc.appdx_tables[index];
    if let Some(saved) = doc.appdx_table_anchors.get(&index)
        .and_then(|&slot| doc.appdx_table_xml.get(slot))
        .filter(|saved| saved.table == *table)
    {
        return Some(Cow::Borrowed(saved));
    }
    snapshot_for(&doc.appdx_table_xml, table, false)
}

pub(super) fn encode_tables(doc: &LawDoc) -> Option<String> {
    let snapshots: Vec<_> = (0..doc.appdx_tables.len())
        .filter_map(|index| document_snapshot(doc, index))
        .collect();
    let fragments: Vec<_> = snapshots.iter().map(|snapshot| snapshot.xml.as_str()).collect();
    (!fragments.is_empty()).then(|| serde_json::to_string(&fragments).ok()).flatten()
}

pub(super) fn markdown_tables(doc: &LawDoc) -> (Vec<Option<usize>>, Vec<String>) {
    let mut slots = vec![String::new(); doc.appdx_table_markdown_ids.values().max().map_or(0, |id| id + 1)];
    let ids = (0..doc.appdx_tables.len()).map(|index| {
        let snapshot = document_snapshot(doc, index)?;
        let id = if let Some(&id) = doc.appdx_table_markdown_ids.get(&index) {
            slots[id] = snapshot.xml.clone();
            id
        } else {
            let id = slots.len();
            slots.push(snapshot.xml.clone());
            id
        };
        Some(id)
    }).collect();
    (ids, slots)
}

pub(super) fn restore_tables(doc: &mut LawDoc, encoded: &str, padded_markdown_rows: bool) {
    restore_tables_with_anchors(doc, Some(encoded), padded_markdown_rows, &[], false);
}

fn table_diagnostic(location: impl Into<String>, issue: SourceIssue) -> SourceDiagnostic {
    SourceDiagnostic { extension: "lawtext", location: location.into(), issue }
}

pub(super) fn restore_tables_with_anchors(
    doc: &mut LawDoc,
    encoded: Option<&str>,
    padded: bool,
    anchors: &[Option<Result<usize, SourceIssue>>],
    require_ids: bool,
) -> Vec<SourceDiagnostic> {
    let anchored = require_ids || anchors.iter().any(Option::is_some);
    if anchored && anchors.len() != doc.appdx_tables.len() {
        return vec![table_diagnostic("law_appdx_source_ids", SourceIssue::Invalid)];
    }
    let fragments = match encoded {
        Some(encoded) => match serde_json::from_str::<Vec<String>>(encoded) {
            Ok(fragments) => fragments,
            Err(_) => return vec![table_diagnostic("law_appdx_tables", SourceIssue::Invalid)],
        },
        None if require_ids => return vec![table_diagnostic("law_appdx_tables", SourceIssue::Missing)],
        None => Vec::new(),
    };
    // Empty entries intentionally reserve IDs after table deletion.
    let slots: Vec<_> = fragments.iter().map(|xml| SavedAppdxTable::read(xml)).collect();
    let legacy: Vec<_> = slots.iter().flatten().cloned().collect();
    let mut claims = std::collections::BTreeMap::<usize, usize>::new();
    for id in anchors.iter().filter_map(|anchor| anchor.as_ref()?.as_ref().ok()) {
        *claims.entry(*id).or_default() += 1;
    }
    let mut diagnostics = Vec::new();
    for (id, (xml, snapshot)) in fragments.iter().zip(&slots).enumerate() {
        if xml.is_empty() || claims.contains_key(&id) {
            continue;
        }
        match snapshot {
            None => diagnostics.push(table_diagnostic(format!("law_appdx_tables[{id}]"), SourceIssue::Invalid)),
            Some(snapshot) if anchored => diagnostics.push(table_diagnostic(
                &snapshot.table.title, SourceIssue::Missing,
            )),
            Some(_) => {}
        }
    }
    for (index, table) in doc.appdx_tables.iter_mut().enumerate() {
        let anchor = anchors.get(index).and_then(Option::as_ref);
        let mut id = None;
        let result = match anchor {
            Some(Err(_)) => Err(SourceIssue::Invalid),
            Some(Ok(source_id)) if claims[source_id] > 1 => Err(SourceIssue::Ambiguous),
            Some(Ok(source_id)) => {
                id = Some(*source_id);
                match slots.get(*source_id) {
                    Some(Some(source)) => {
                        normalize_rich_header(table);
                        snapshot_for(std::slice::from_ref(source), table, padded)
                            .map(Cow::into_owned)
                            .ok_or(SourceIssue::Incompatible)
                    }
                    Some(None) if !fragments[*source_id].is_empty() => Err(SourceIssue::Invalid),
                    _ => Err(SourceIssue::Missing),
                }
            }
            None if anchored => continue,
            None => {
                if !legacy.iter().any(|saved| saved.table.title == table.title) {
                    continue;
                }
                snapshot_for(&legacy, table, padded).map(Cow::into_owned)
                    .ok_or(SourceIssue::Incompatible)
            }
        };
        match result {
            Ok(snapshot) => {
                table.rows.clone_from(&snapshot.table.rows);
                if let Some(id) = id {
                    doc.appdx_table_markdown_ids.insert(index, id);
                }
                doc.appdx_table_anchors.insert(index, doc.appdx_table_xml.len());
                doc.appdx_table_xml.push(snapshot);
            }
            Err(issue) => {
                doc.unresolved_appdx_tables.insert(index);
                diagnostics.push(table_diagnostic(&table.title, issue));
            }
        }
    }
    diagnostics
}

fn normalize_rich_header(table: &mut LawAppdxTable) {
    // Rich Markdown renders each GFM header cell as bold.
    if let Some(header) = table.rows.first_mut()
        && header.iter().filter(|cell| !cell.is_empty()).all(|cell| {
            cell.strip_prefix("**").and_then(|text| text.strip_suffix("**")).is_some()
        })
    {
        for cell in header.iter_mut().filter(|cell| !cell.is_empty()) {
            *cell = cell[2..cell.len() - 2].to_owned();
        }
    }
}
