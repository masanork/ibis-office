// SPDX-License-Identifier: AGPL-3.0-only

use std::{borrow::Cow, collections::HashMap, ops::Range};

use ibis_formats::csv::{OriginalEncoding, raw_csv_record_ranges, raw_utf8_csv_record_fields};

use super::{
    ADDRESSEE_ID, MAX_RECORDS, Node, RECORD_NAME_FIELDS, Tree, excerpt, linkage, record_heading,
};

pub(super) struct Csv {
    columns: Vec<Column>,
    addressee: Option<usize>,
    names: Vec<usize>,
    pub records: Vec<Range<usize>>,
    pub truncated: bool,
    pub data_count: usize,
    pub definition: Option<linkage::Definition>,
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

#[cfg(not(target_family = "wasm"))]
fn write_report_row(out: &mut String, fields: impl IntoIterator<Item = impl AsRef<str>>) {
    for (index, field) in fields.into_iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        for ch in field.as_ref().chars() {
            if ch == '"' {
                out.push('"');
            }
            out.push(ch);
        }
        out.push('"');
    }
    out.push_str("\r\n");
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
        let (mut records, count) = raw_csv_record_ranges(bytes, encoding, ',', MAX_RECORDS + 1)
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
        let data_count = count - 1;
        let truncated = data_count > MAX_RECORDS;
        records.remove(0);
        Ok(Self {
            columns,
            addressee: names.iter().position(|name| name == ADDRESSEE_ID),
            names: RECORD_NAME_FIELDS
                .iter()
                .filter_map(|field| names.iter().position(|name| name == *field))
                .collect(),
            records,
            truncated,
            data_count,
            definition: None,
        })
    }

    pub fn record_heading(&self, bytes: &[u8]) -> Option<String> {
        if self.addressee.is_none() && self.names.is_empty() {
            return None;
        }
        let fields = raw_utf8_csv_record_fields(bytes).ok()?;
        let read = |column| {
            let range: &Range<usize> = fields.get(column)?;
            let value = cell_text(&bytes[range.clone()]);
            (!value.trim().is_empty()).then_some(value)
        };
        let id = self.addressee.and_then(read);
        let name = self.names.iter().find_map(|&column| read(column));
        record_heading(id.as_deref(), name.as_deref())
    }

    pub fn review(
        &self,
        bytes: &[u8],
        definition: &linkage::Definition,
    ) -> Result<linkage::Review, &'static str> {
        let mut review = definition.review();
        review.unavailable = self.scan(bytes, &self.records, definition, |issue, _, _| {
            review.issue_count += 1;
            if review.issues.len() < linkage::DISPLAY_LIMIT {
                review.issues.push(issue);
            }
            Ok(())
        })?;
        review.checked = self.records.len();
        Ok(review)
    }

    fn scan(
        &self,
        bytes: &[u8],
        records: &[Range<usize>],
        definition: &linkage::Definition,
        mut emit: impl FnMut(linkage::Issue, &[u8], &[Range<usize>]) -> Result<(), &'static str>,
    ) -> Result<Vec<usize>, &'static str> {
        if !self
            .columns
            .iter()
            .map(|column| &column.name)
            .eq(&definition.header)
        {
            return Err("structured.linkage-header-mismatch");
        }
        let columns: Vec<_> = definition.columns().collect();
        let mut unavailable = Vec::new();
        let conditions: Vec<_> = definition
            .conditions
            .iter()
            .enumerate()
            .filter_map(|(index, condition)| {
                let Some((source, target)) = definition.condition_columns(condition) else {
                    unavailable.push(index);
                    return None;
                };
                Some((index, source, target, condition.predicate.as_ref()?))
            })
            .collect();
        for (record, span) in records.iter().enumerate() {
            let raw = &bytes[span.clone()];
            let fields =
                raw_utf8_csv_record_fields(raw).map_err(|_| "structured.linkage-csv-invalid")?;
            let mut add = |column, message, condition| {
                emit(
                    linkage::Issue {
                        record,
                        column,
                        message,
                        condition,
                    },
                    raw,
                    &fields,
                )
            };
            if fields.len() != columns.len() {
                add(
                    fields.len().min(columns.len()),
                    "structured.linkage-column-count",
                    None,
                )?;
                continue;
            }
            for &(index, source, target, predicate) in &conditions {
                if predicate.matches(&cell_text(&raw[fields[source].clone()]))
                    && cell_text(&raw[fields[target].clone()]).is_empty()
                {
                    add(
                        target,
                        "structured.linkage-conditional-required",
                        Some(index),
                    )?;
                }
            }
            for (column, (field, definition_column)) in fields.iter().zip(&columns).enumerate() {
                if let Some(message) =
                    definition.check(definition_column, &cell_text(&raw[field.clone()]))
                {
                    add(column, message, None)?;
                }
            }
        }
        Ok(unavailable)
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn issue_workbook(
        &self,
        bytes: &[u8],
        records: &[Range<usize>],
    ) -> Result<Vec<u8>, String> {
        let definition = self.definition.as_ref().ok_or_else(|| {
            crate::i18n::text("structured.linkage-definition-invalid").to_string()
        })?;
        let header = [
            "structured.linkage-export-record",
            "structured.linkage-export-id",
            "structured.linkage-export-field",
            "structured.linkage-export-value",
            "structured.linkage-export-reason",
        ]
        .map(crate::i18n::text);
        let mut table = String::new();
        write_report_row(&mut table, header.iter().map(|heading| heading.as_ref()));
        let mut rows = 1;
        self.scan(bytes, records, definition, |issue, raw, fields| {
            if rows >= 1_048_576 {
                return Err("structured.linkage-export-limit");
            }
            let cell = |column| {
                fields
                    .get(column)
                    .map_or(Cow::Borrowed(""), |range: &Range<usize>| {
                        cell_text(&raw[range.clone()])
                    })
            };
            let id = self.addressee.map_or(Cow::Borrowed(""), cell);
            let value = cell(issue.column);
            let name = self
                .columns
                .get(issue.column)
                .map_or("", |column| column.name.as_str());
            let reason = definition.issue_reason(&issue);
            if [&id[..], &value[..], name, &reason]
                .iter()
                .any(|text| text.encode_utf16().count() > 32_767)
            {
                return Err("structured.linkage-export-cell-limit");
            }
            write_report_row(
                &mut table,
                [
                    (issue.record + 1).to_string().as_str(),
                    &id,
                    name,
                    &value,
                    &reason,
                ],
            );
            rows += 1;
            if table.len() as u64 > super::MAX_BYTES {
                return Err("structured.linkage-export-limit");
            }
            Ok(())
        })
        .map_err(|key| crate::i18n::text(key).to_string())?;
        ibis_formats::csv::workbook_from_literal_fields(&table, "Issues", None)
            .map_err(|error| error.to_string())
    }

    pub fn tree(&self, bytes: &[u8], index: usize, span: Range<usize>) -> Tree {
        let fields =
            raw_utf8_csv_record_fields(&bytes[span.clone()]).expect("record was validated");
        let mut tree = Tree::default();
        let root = tree.push(branch(format!("#{}", index + 1), span.clone(), true, None));
        tree.roots.push(root);
        let mut groups = HashMap::new();
        let mut definitions = self
            .definition
            .iter()
            .flat_map(linkage::Definition::columns);
        for column in 0..self.columns.len().max(fields.len()) {
            let kind = definitions.next().and_then(|column| column.kind.as_deref());
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
                detail: match kind {
                    Some(kind) => format!("{detail} · {kind}"),
                    None => detail,
                },
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
    use super::{Csv, linkage};

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn issue_workbook_preserves_literal_cells_and_includes_all_findings() {
        use ibis_doc_model::{CellValue, sheet_calc::sheet_of_slide};
        let mut data = String::from("宛名番号,項目\r\n");
        for _ in 0..25 {
            data.push_str("000042,=1+2\r\n");
        }
        let mut csv = Csv::read(data.as_bytes()).unwrap();
        csv.definition = Some(
            linkage::Definition::parse(
                r#"{
            "format":"mrlgss-linkage-csv","version":1,"header":["宛名番号","項目"],"codeValues":{},
            "layout":{"id":"example","name":"Example","repeat_header":"{name}_{n}",
              "columns":[{"name":"宛名番号","type":"X(6)"},{"name":"項目","type":"X(1)"}]}
        }"#
                .as_bytes(),
            )
            .unwrap(),
        );
        let review = csv
            .review(data.as_bytes(), csv.definition.as_ref().unwrap())
            .unwrap();
        assert_eq!(review.issues.len(), linkage::DISPLAY_LIMIT);
        assert_eq!(review.issue_count, 25);
        let bytes = csv.issue_workbook(data.as_bytes(), &csv.records).unwrap();
        let presentation = ibis_formats::read_xlsx(&bytes).unwrap();
        let sheet = sheet_of_slide(&presentation.slides[0]).unwrap();
        for (row, number) in [(1, "1"), (25, "25")] {
            for (column, expected) in [(0, number), (1, "000042"), (2, "項目"), (3, "=1+2")] {
                let cell = sheet.cell(row, column).unwrap();
                assert_eq!(cell.value, CellValue::Text(expected.into()));
                assert!(cell.formula.is_none());
            }
            let reason = sheet.cell(row, 4).unwrap();
            assert_eq!(
                reason.value,
                CellValue::Text(
                    crate::i18n::text("structured.linkage-length")
                        .to_string()
                        .into()
                )
            );
        }
    }

    #[test]
    fn csv_record_heading_preserves_literal_id_and_decodes_the_name() {
        let data = "市区町村コード,氏名,宛名番号\r\n131016,\"山田 \"\"花子\"\"\",000042\r\n131016,李 明,\r\n131016, ,\r\n";
        let csv = Csv::read(data.as_bytes()).unwrap();
        let headings = csv
            .records
            .iter()
            .map(|span| csv.record_heading(&data.as_bytes()[span.clone()]))
            .collect::<Vec<_>>();
        assert_eq!(
            headings,
            [
                Some("000042 · 山田 \"花子\"".into()),
                Some("李 明".into()),
                None
            ]
        );
    }

    #[test]
    fn linkage_conditions_use_literal_values_and_report_unavailable_references() {
        let definition = linkage::Definition::parse(
            br#"{
            "format": "mrlgss-linkage-csv", "version": 1, "codeValues": {},
            "layout": {"id":"example","name":"Example","repeat_header":"{name}_{n}","columns":[
                {"name":"trigger","field":"012-00001"},
                {"name":"equals","required":"conditional"},
                {"name":"nonempty","required":"conditional"},
                {"name":"missing","required":"conditional"}
            ]},
            "header":["trigger","equals","nonempty","missing"],
            "conditions":[
                {"target":"equals","predicate":{"field":"012-00001","values":["01"]}},
                {"target":"nonempty","predicate":{"field":"012-00001"}},
                {"target":"missing","note":"missing reference","predicate":{"field":"012-00002"}}
            ]
        }"#,
        )
        .unwrap();
        let bytes = b"trigger,equals,nonempty,missing\n01,,,\n1,,,\n\"\",,,\n,,,\n01,ok,ok,";
        let review = Csv::read(bytes)
            .unwrap()
            .review(bytes, &definition)
            .unwrap();
        assert_eq!(review.checked, 5);
        assert_eq!(review.unavailable, [2]);
        let positions: Vec<_> = review
            .issues
            .iter()
            .map(|issue| (issue.record, issue.column, issue.condition))
            .collect();
        assert_eq!(
            positions,
            [(0, 1, Some(0)), (0, 2, Some(1)), (1, 2, Some(1))]
        );
    }

    #[test]
    fn linkage_definition_checks_literal_cells_and_order() {
        let definition = linkage::Definition::parse(
            br#"{
            "format": "mrlgss-linkage-csv", "version": 1, "codeValues": {"A": ["01"]},
            "layout": {"id": "example", "name": "Example", "repeat_header": "{name}_{n}",
                "columns": [{"name": "id", "type": "X(6)", "required": true},
                            {"name": "code", "type": "X(2)", "code": "A"}]},
            "header": ["id", "code"]
        }"#,
        )
        .unwrap();
        let literal = b"id,code\r\n000001,01\r\n0000001,1\r\n,01\r\n";
        let csv = Csv::read(literal).unwrap();
        let review = csv.review(literal, &definition).unwrap();
        let issues: Vec<_> = review
            .issues
            .iter()
            .map(|issue| (issue.record, issue.column, issue.message))
            .collect();
        assert_eq!(review.checked, 3);
        assert_eq!(
            issues,
            [
                (1, 0, "structured.linkage-length"),
                (1, 1, "structured.linkage-code"),
                (2, 0, "structured.linkage-required"),
            ]
        );
        let reordered = b"code,id\n01,000001";
        assert_eq!(
            Csv::read(reordered)
                .unwrap()
                .review(reordered, &definition)
                .err(),
            Some("structured.linkage-header-mismatch")
        );
    }

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
        assert_eq!(csv.data_count, 1);
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
        assert_eq!(tree.visible("[7444]"), [(7445, 1)]);
        assert_eq!(tree.visible("12345678901234567890"), [(children[1], 2)]);
    }
}
