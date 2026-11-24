// SPDX-License-Identifier: AGPL-3.0-only

use std::{borrow::Cow, collections::HashMap, ops::Range};

use ibis_formats::csv::{OriginalEncoding, raw_csv_record_ranges, raw_utf8_csv_record_fields};

use super::{MAX_RECORDS, Node, Tree, excerpt};

pub(super) struct Csv {
    columns: Vec<Column>,
    pub records: Vec<Range<usize>>,
    pub truncated: bool,
}

struct Column {
    name: String,
    group: Option<String>,
}

fn cell_text(bytes: &[u8]) -> Cow<'_, str> {
    let raw = std::str::from_utf8(bytes).expect("CSV was validated as UTF-8");
    match raw
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
    {
        Some(quoted) if quoted.contains("\"\"") => Cow::Owned(quoted.replace("\"\"", "\"")),
        Some(quoted) => Cow::Borrowed(quoted),
        None => Cow::Borrowed(raw),
    }
}

fn repeated_name(column: &str) -> Option<(&str, usize)> {
    let (name, index) = column.rsplit_once('_')?;
    let index = index.parse::<usize>().ok()?;
    (!name.is_empty() && index > 0).then_some((name, index))
}

impl Csv {
    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        let encoding = if bytes.starts_with(b"\xef\xbb\xbf") {
            OriginalEncoding::Utf8Bom
        } else {
            OriginalEncoding::Utf8
        };
        let mut records = raw_csv_record_ranges(bytes, encoding, ',', MAX_RECORDS + 2)
            .map_err(|error| error.to_string())?;
        let Some(header) = records.first().cloned() else {
            return Err(crate::i18n::text("structured.csv-header-required").to_string());
        };
        let names: Vec<_> = raw_utf8_csv_record_fields(&bytes[header.clone()])
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|range| {
                cell_text(&bytes[header.start + range.start..header.start + range.end]).into_owned()
            })
            .collect();
        let mut groups = HashMap::new();
        for (name, index) in names.iter().filter_map(|name| repeated_name(name)) {
            let first = groups.entry(name).or_insert((index, false));
            first.1 |= index != first.0;
        }
        let columns = names
            .iter()
            .map(|name| Column {
                name: name.clone(),
                group: repeated_name(name)
                    .filter(|(name, _)| groups[name].1)
                    .map(|(name, _)| name.to_owned()),
            })
            .collect();
        let truncated = records.len() > MAX_RECORDS + 1;
        records.remove(0);
        records.truncate(MAX_RECORDS);
        Ok(Self {
            columns,
            records,
            truncated,
        })
    }

    pub fn tree(&self, bytes: &[u8], index: usize, span: Range<usize>) -> Tree {
        let fields =
            raw_utf8_csv_record_fields(&bytes[span.clone()]).expect("record was validated");
        let mut tree = Tree::default();
        let root = tree.push(branch(format!("#{}", index + 1), span.clone(), true, None));
        tree.roots.push(root);
        let mut groups = HashMap::new();
        for column in 0..self.columns.len().max(fields.len()) {
            let header = self.columns.get(column);
            let parent = if let Some(group) = header.and_then(|header| header.group.as_deref()) {
                *groups.entry(group).or_insert_with(|| {
                    let id = tree.push(branch(group.to_owned(), 0..0, false, Some(root)));
                    tree.nodes[root].children.as_mut().unwrap().push(id);
                    id
                })
            } else {
                root
            };
            let field = fields
                .get(column)
                .map(|range| span.start + range.start..span.start + range.end);
            let detail = field.as_ref().map_or_else(
                || crate::i18n::text("structured.csv-missing").to_string(),
                |range| {
                    let value = cell_text(&bytes[range.clone()]);
                    if value.is_empty() {
                        return if range.is_empty() {
                            crate::i18n::text("structured.csv-unquoted-empty").to_string()
                        } else {
                            "\"\"".to_owned()
                        };
                    }
                    excerpt(&value, 256)
                },
            );
            let name = header.map_or("", |header| header.name.as_str());
            let id = tree.push(Node {
                name: format!("[{}] {}", column + 1, name),
                detail,
                span: field.unwrap_or(0..0),
                children: Some(Vec::new()),
                expanded: false,
                paragraph: None,
                parent: Some(parent),
                more: false,
            });
            tree.nodes[parent].children.as_mut().unwrap().push(id);
        }
        tree
    }
}

fn branch(name: String, span: Range<usize>, expanded: bool, parent: Option<usize>) -> Node {
    Node {
        name,
        detail: String::new(),
        span,
        children: Some(Vec::new()),
        expanded,
        paragraph: None,
        parent,
        more: false,
    }
}

#[cfg(test)]
mod tests {
    use super::Csv;

    #[test]
    fn retains_csv_spelling_and_groups_wide_records() {
        let header = (1..=7442)
            .map(|i| format!("\"項目_{i}\""))
            .collect::<Vec<_>>()
            .join(",");
        let data = format!(
            "\u{feff}{header},\"備考\",\"備考\"\r\n\"011002\",\"12345678901234567890\",\"=1+2\",\"行1\r\n行2\",\"\",,\r\n"
        );
        let csv = Csv::read(data.as_bytes()).unwrap();
        assert_eq!(csv.records.len(), 1);
        let tree = csv.tree(data.as_bytes(), 0, csv.records[0].clone());
        let group = &tree.nodes[1];
        assert_eq!(group.name, "項目");
        let children = group.children.as_ref().unwrap();
        assert_eq!(children.len(), 7442);
        assert_eq!(tree.nodes[children[0]].detail, "011002");
        assert_eq!(tree.nodes[children[1]].detail, "12345678901234567890");
        assert_eq!(tree.nodes[children[2]].detail, "=1+2");
        assert_eq!(tree.nodes[children[3]].detail, "行1\r\n行2");
        assert_eq!(tree.nodes[children[4]].detail, "\"\"");
        assert_ne!(tree.nodes[children[4]].span, tree.nodes[children[5]].span);
        assert_eq!(tree.nodes[children[7]].span, 0..0);
        assert_eq!(tree.nodes[7444].name, "[7443] 備考");
        assert_eq!(tree.nodes[7445].name, "[7444] 備考");
    }
}
