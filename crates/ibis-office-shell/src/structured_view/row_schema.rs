// SPDX-License-Identifier: AGPL-3.0-only

//! Local JSON Schema subset emitted by mrlgss for flat row objects.

use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

use serde::Deserialize;
use serde_json::value::RawValue;

use super::{Source, linkage::DISPLAY_LIMIT};

const INVALID: &str = "structured.row-schema-invalid";

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Schema {
    #[serde(rename = "$schema")]
    dialect: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "type")]
    kind: String,
    properties: BTreeMap<String, Property>,
    #[serde(default)]
    all_of: Vec<Rule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    #[serde(rename = "$comment")]
    comment: Option<String>,
    #[serde(rename = "if")]
    when: Option<Fields>,
    then: Option<Fields>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    required: Vec<String>,
    properties: BTreeMap<String, Property>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Property {
    #[serde(rename = "type")]
    kind: Option<Kinds>,
    #[serde(rename = "enum")]
    values: Option<Vec<String>>,
    min_length: Option<usize>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Kinds {
    One(Kind),
    Many(Vec<Kind>),
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    String,
    Null,
}

impl Kind {
    fn accepts(&self, value: &RawValue) -> bool {
        match self {
            Self::String => value.get().starts_with('"'),
            Self::Null => value.get() == "null",
        }
    }
}

impl Property {
    fn failure(&self, value: &RawValue) -> Option<&'static str> {
        if self.kind.as_ref().is_some_and(|kind| match kind {
            Kinds::One(kind) => !kind.accepts(value),
            Kinds::Many(kinds) => !kinds.iter().any(|kind| kind.accepts(value)),
        }) {
            return Some("structured.row-schema-type");
        }
        if self.values.is_some() || self.min_length.is_some() {
            let text = serde_json::from_str::<String>(value.get()).ok();
            if self
                .values
                .as_ref()
                .is_some_and(|values| !text.as_ref().is_some_and(|text| values.contains(text)))
            {
                return Some("structured.row-schema-enum");
            }
            if let Some((minimum, text)) = self.min_length.zip(text)
                && text.chars().take(minimum).count() < minimum
            {
                return Some("structured.row-schema-length");
            }
        }
        None
    }
}

impl Fields {
    fn failures<'a>(
        &'a self,
        row: &'a BTreeMap<String, &RawValue>,
    ) -> impl Iterator<Item = (&'a str, &'static str)> {
        self.required
            .iter()
            .filter(|name| !row.contains_key(*name))
            .map(|name| (name.as_str(), "structured.row-schema-required"))
            .chain(self.properties.iter().filter_map(|(name, property)| {
                Some((name.as_str(), property.failure(row.get(name)?)?))
            }))
    }
}

pub(super) struct Issue {
    pub record: usize,
    pub field: Option<String>,
    pub message: &'static str,
}

pub(super) struct Review {
    pub title: String,
    pub issue_count: usize,
    pub issues: Vec<Issue>,
    pub comments: Vec<String>,
}

pub(super) fn sidecar(path: &Path) -> Option<std::path::PathBuf> {
    path.extension()
        .filter(|ext| ext.eq_ignore_ascii_case("jsonl") || ext.eq_ignore_ascii_case("ndjson"))?;
    let schema = path.with_extension("schema.json");
    schema.exists().then_some(schema)
}

impl Schema {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let schema: Self = serde_json::from_slice(bytes).ok()?;
        if schema.dialect != "https://json-schema.org/draft/2020-12/schema"
            || schema.kind != "object"
            || schema
                .all_of
                .iter()
                .any(|rule| rule.when.is_some() != rule.then.is_some())
        {
            return None;
        }
        Some(schema)
    }
}

pub(super) fn review(path: &Path, source: &Source) -> Result<Review, &'static str> {
    let mut bytes = Vec::new();
    let limit = 8 * 1024 * 1024;
    File::open(path)
        .and_then(|file| file.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|_| INVALID)?;
    let schema = (bytes.len() as u64 <= limit)
        .then(|| Schema::parse(&bytes))
        .flatten()
        .ok_or(INVALID)?;
    Ok(review_rows(&schema, source))
}

fn review_rows(schema: &Schema, source: &Source) -> Review {
    let mut review = Review {
        title: schema.title.clone(),
        issue_count: 0,
        issues: Vec::new(),
        comments: schema
            .all_of
            .iter()
            .filter(|rule| rule.when.is_none())
            .filter_map(|rule| rule.comment.clone())
            .collect(),
    };
    for (record, span) in source.records.iter().enumerate() {
        let mut add = |field: Option<&str>, message| {
            review.issue_count += 1;
            if review.issues.len() < DISPLAY_LIMIT {
                review.issues.push(Issue {
                    record,
                    field: field.map(str::to_owned),
                    message,
                });
            }
        };
        let row = match serde_json::from_slice::<&RawValue>(&source.bytes[span.clone()]) {
            Ok(raw) if raw.get().starts_with('{') => {
                let Ok(row) = serde_json::from_str::<BTreeMap<String, &RawValue>>(raw.get()) else {
                    add(None, "structured.row-schema-malformed");
                    continue;
                };
                row
            }
            Ok(_) => {
                add(None, "structured.row-schema-object");
                continue;
            }
            Err(_) => {
                add(None, "structured.row-schema-malformed");
                continue;
            }
        };
        for (name, property) in &schema.properties {
            if let Some(message) = row.get(name).and_then(|value| property.failure(value)) {
                add(Some(name), message);
            }
        }
        for rule in &schema.all_of {
            if let (Some(when), Some(then)) = (&rule.when, &rule.then)
                && when.failures(&row).next().is_none()
            {
                for (field, message) in then.failures(&row) {
                    add(Some(field), message);
                }
            }
        }
    }
    review
}

#[cfg(test)]
mod tests {
    use super::{Schema, Source, review_rows};

    #[test]
    fn row_schema_reports_physical_lines_and_missing_fields() {
        let schema = Schema::parse(
            br#"{
            "$schema":"https://json-schema.org/draft/2020-12/schema","type":"object",
            "properties":{"code":{"type":["string","null"]}},
            "allOf":[{"$comment":"unavailable"},
                {"if":{"required":["code"],"properties":{"code":{"enum":["01"]}}},
                "then":{"required":["x"],"properties":{"x":{"type":"string","minLength":1}}}}]
        }"#,
        )
        .unwrap();
        let source = Source::new(b"{\"code\":\"01\",\"x\":\"\"}\r\n{bad}\n{\"code\":\"01\"}\n{\"code\":\"1\"}\n{\"code\":null}\n{}\n".to_vec(), true);
        let review = review_rows(&schema, &source);
        assert_eq!(review.comments, ["unavailable"]);
        assert_eq!(review.issue_count, 3);
        assert_eq!(
            review
                .issues
                .iter()
                .map(|issue| (issue.record, issue.field.as_deref(), issue.message))
                .collect::<Vec<_>>(),
            [
                (0, Some("x"), "structured.row-schema-length"),
                (1, None, "structured.row-schema-malformed"),
                (2, Some("x"), "structured.row-schema-required"),
            ]
        );
        assert!(Schema::parse(br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{},"$ref":"elsewhere"}"#).is_none());
    }
}
