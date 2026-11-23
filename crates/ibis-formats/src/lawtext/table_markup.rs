// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Validated XML snapshots behind editable appendix-table rows.

use super::model::{LawAppdxTable, LawDoc};
use super::xml::{child_elem, collect_text};

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

pub(super) fn parse_appdx_table(appdx: roxmltree::Node<'_, '_>) -> LawAppdxTable {
    let title = child_elem(appdx, "AppdxTableTitle")
        .map(collect_text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "別表".to_string());
    let related_article_num = child_elem(appdx, "RelatedArticleNum")
        .map(collect_text)
        .filter(|s| !s.is_empty());
    let mut rows = Vec::new();
    if let Some(ts) = child_elem(appdx, "TableStruct") {
        if let Some(tbl) = child_elem(ts, "Table") {
            for tr in tbl
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "TableRow")
            {
                let row: Vec<String> = tr
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "TableColumn")
                    .map(collect_text)
                    .collect();
                if !row.is_empty() {
                    rows.push(row);
                }
            }
        }
    }
    LawAppdxTable {
        title,
        related_article_num,
        rows,
    }
}

pub(super) fn encode_tables(doc: &LawDoc) -> Option<String> {
    let snapshots: Vec<_> = doc
        .appdx_table_xml
        .iter()
        .filter(|snapshot| doc.appdx_tables.contains(&snapshot.table))
        .map(|snapshot| snapshot.xml.as_str())
        .collect();
    if snapshots.is_empty() {
        None
    } else {
        serde_json::to_string(&snapshots).ok()
    }
}

pub(super) fn restore_tables(doc: &mut LawDoc, encoded: &str, padded_markdown_rows: bool) {
    let Ok(fragments) = serde_json::from_str::<Vec<String>>(encoded) else {
        return;
    };
    let snapshots: Vec<_> = fragments
        .iter()
        .filter_map(|xml| SavedAppdxTable::read(xml))
        .collect();
    for table in &mut doc.appdx_tables {
        let mut matching = snapshots.iter().filter(|snapshot| {
            let stored = &snapshot.table;
            stored.title == table.title
                && stored.related_article_num == table.related_article_num
                && (stored.rows == table.rows
                    || padded_markdown_rows
                        && table.rows.len() == stored.rows.len()
                        && table
                            .rows
                            .iter()
                            .zip(&stored.rows)
                            .all(|(visible, original)| {
                                visible.starts_with(original)
                                    && visible[original.len()..].iter().all(String::is_empty)
                            }))
        });
        if let Some(snapshot) = matching.next()
            && matching.next().is_none()
        {
            table.rows.clone_from(&snapshot.table.rows);
            doc.appdx_table_xml.push(snapshot.clone());
        }
    }
}
