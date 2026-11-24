// SPDX-License-Identifier: AGPL-3.0-only

//! Local validation for a subset of JSON Schema Draft 2020-12.

use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

use serde::Deserialize;
use serde_json::{Value, value::RawValue};

use super::{Source, linkage::DISPLAY_LIMIT};

const INVALID: &str = "structured.row-schema-invalid";

struct Schema {
    title: String,
    rule: Rule,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Rule {
    #[serde(rename = "$comment")]
    comment: Option<String>,
    #[serde(rename = "type")]
    kind: Option<Kinds>,
    #[serde(rename = "enum")]
    values: Option<Vec<Literal>>,
    #[serde(rename = "const", default, deserialize_with = "literal")]
    constant: Option<Literal>,
    min_length: Option<usize>,
    #[serde(default)]
    required: Vec<String>,
    #[serde(default)]
    properties: BTreeMap<String, Rule>,
    #[serde(default)]
    all_of: Vec<Rule>,
    #[serde(rename = "if")]
    when: Option<Box<Rule>>,
    then: Option<Box<Rule>>,
    items: Option<Box<Rule>>,
    contains: Option<Box<Rule>>,
    min_contains: Option<usize>,
    max_contains: Option<usize>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Literal {
    Text(String),
    Boolean(bool),
    Null(()),
}

fn literal<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Literal>, D::Error> {
    Literal::deserialize(d).map(Some)
}

impl Literal {
    fn matches(&self, value: &Value) -> bool {
        match self {
            Self::Text(text) => value.as_str() == Some(text),
            Self::Boolean(boolean) => value.as_bool() == Some(*boolean),
            Self::Null(()) => value.is_null(),
        }
    }
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
    Object,
    Array,
    String,
    Boolean,
    Null,
}

impl Kind {
    fn accepts(&self, value: &RawValue) -> bool {
        match self {
            Self::Object => value.get().starts_with('{'),
            Self::Array => value.get().starts_with('['),
            Self::String => value.get().starts_with('"'),
            Self::Boolean => matches!(value.get(), "true" | "false"),
            Self::Null => value.get() == "null",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Segment {
    Key(String),
    Index(usize),
}

pub(super) fn pointer(path: &[Segment]) -> String {
    let mut result = String::new();
    for segment in path {
        result.push('/');
        match segment {
            Segment::Key(key) => result.push_str(&key.replace('~', "~0").replace('/', "~1")),
            Segment::Index(index) => result.push_str(&index.to_string()),
        }
    }
    result
}

impl Rule {
    fn valid(&self, value: &RawValue, path: &mut Vec<Segment>) -> bool {
        let mut valid = true;
        self.check(value, path, &mut |_, _| valid = false);
        valid
    }

    fn check(
        &self,
        value: &RawValue,
        path: &mut Vec<Segment>,
        emit: &mut dyn FnMut(&[Segment], &'static str),
    ) {
        if self.kind.as_ref().is_some_and(|kind| match kind {
            Kinds::One(kind) => !kind.accepts(value),
            Kinds::Many(kinds) => !kinds.iter().any(|kind| kind.accepts(value)),
        }) {
            emit(path, "structured.row-schema-type");
            return;
        }
        if self.values.is_some() || self.constant.is_some() || self.min_length.is_some() {
            let decoded = serde_json::from_str::<Value>(value.get()).ok();
            let matches = |literal: &Literal| decoded.as_ref().is_some_and(|v| literal.matches(v));
            if self
                .values
                .as_ref()
                .is_some_and(|values| !values.iter().any(matches))
                || self
                    .constant
                    .as_ref()
                    .is_some_and(|literal| !matches(literal))
            {
                emit(path, "structured.row-schema-enum");
            }
            if let Some((minimum, text)) = self
                .min_length
                .zip(decoded.as_ref().and_then(Value::as_str))
                && text.chars().take(minimum).count() < minimum
            {
                emit(path, "structured.row-schema-length");
            }
        }
        if (!self.properties.is_empty() || !self.required.is_empty())
            && let Ok(row) = serde_json::from_str::<BTreeMap<String, &RawValue>>(value.get())
        {
            for key in self.required.iter().filter(|key| !row.contains_key(*key)) {
                path.push(Segment::Key(key.clone()));
                emit(path, "structured.row-schema-required");
                path.pop();
            }
            for (key, rule) in &self.properties {
                if let Some(child) = row.get(key) {
                    path.push(Segment::Key(key.clone()));
                    rule.check(child, path, emit);
                    path.pop();
                }
            }
        }
        if (self.items.is_some() || self.contains.is_some())
            && let Ok(children) = serde_json::from_str::<Vec<&RawValue>>(value.get())
        {
            let mut matched = 0;
            for (index, child) in children.iter().enumerate() {
                if let Some(rule) = &self.items {
                    path.push(Segment::Index(index));
                    rule.check(child, path, emit);
                    path.pop();
                }
                if self
                    .contains
                    .as_ref()
                    .is_some_and(|rule| rule.valid(child, path))
                {
                    matched += 1;
                }
            }
            if self.contains.is_some() {
                if matched < self.min_contains.unwrap_or(1) {
                    emit(path, "structured.row-schema-contains-few");
                }
                if self.max_contains.is_some_and(|max| matched > max) {
                    emit(path, "structured.row-schema-contains-many");
                }
            }
        }
        for rule in &self.all_of {
            rule.check(value, path, emit);
        }
        if let (Some(when), Some(then)) = (&self.when, &self.then)
            && when.valid(value, path)
        {
            then.check(value, path, emit);
        }
    }
}

pub(super) struct Issue {
    pub record: usize,
    pub path: Vec<Segment>,
    pub message: &'static str,
}

pub(super) struct Review {
    pub title: String,
    pub issue_count: usize,
    pub issues: Vec<Issue>,
    pub comments: Vec<String>,
}

pub(super) fn sidecar(path: &Path) -> Option<std::path::PathBuf> {
    path.extension().filter(|ext| {
        ["json", "jsonl", "ndjson"]
            .iter()
            .any(|e| ext.eq_ignore_ascii_case(e))
    })?;
    let schema = path.with_extension("schema.json");
    schema.exists().then_some(schema)
}

impl Schema {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let Value::Object(mut object) = serde_json::from_slice(bytes).ok()? else {
            return None;
        };
        if object.remove("$schema")?.as_str()? != "https://json-schema.org/draft/2020-12/schema" {
            return None;
        }
        let title = match object.remove("title") {
            Some(Value::String(title)) => title,
            None => String::new(),
            _ => return None,
        };
        let rule = serde_json::from_value(Value::Object(object)).ok()?;
        Some(Self { title, rule })
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
            .rule
            .all_of
            .iter()
            .filter(|rule| rule.when.is_none())
            .filter_map(|rule| rule.comment.clone())
            .collect(),
    };
    for (record, span) in source.records.iter().enumerate() {
        let mut add = |path: &[Segment], message| {
            review.issue_count += 1;
            if review.issues.len() < DISPLAY_LIMIT {
                review.issues.push(Issue {
                    record,
                    path: path.to_vec(),
                    message,
                });
            }
        };
        match serde_json::from_slice::<&RawValue>(&source.bytes[span.clone()]) {
            Ok(raw) => schema.rule.check(raw, &mut Vec::new(), &mut add),
            Err(_) => add(&[], "structured.row-schema-malformed"),
        }
    }
    review
}

#[cfg(test)]
mod tests {
    use super::{Schema, Source, pointer, review_rows};

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
                .map(|issue| (issue.record, pointer(&issue.path), issue.message))
                .collect::<Vec<_>>(),
            [
                (0, "/x".into(), "structured.row-schema-length"),
                (1, String::new(), "structured.row-schema-malformed"),
                (2, "/x".into(), "structured.row-schema-required"),
            ]
        );
        assert!(Schema::parse(br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{},"$ref":"elsewhere"}"#).is_none());
    }

    #[test]
    fn nested_item_and_array_violations_have_distinct_paths() {
        let schema = Schema::parse(br#"{
          "$schema":"https://json-schema.org/draft/2020-12/schema",
          "properties":{"team/~":{"properties":{"contacts":{"type":"array",
            "items":{"type":"object","required":["name"],"properties":{"name":{"type":"string","minLength":1}}},
            "contains":{"type":"object","required":["primary"],"properties":{"primary":{"const":true}}},
            "minContains":1,"maxContains":1
          }}}}
        }"#).unwrap();
        for (rows, expected) in [
            (
                r#"[{"name":"A","primary":true},{"name":"B","primary":false}]"#,
                vec![],
            ),
            (
                r#"[]"#,
                vec![("/team~1~0/contacts", "structured.row-schema-contains-few")],
            ),
            (
                r#"[{"name":"","primary":true},{"name":"B","primary":true}]"#,
                vec![
                    ("/team~1~0/contacts/0/name", "structured.row-schema-length"),
                    ("/team~1~0/contacts", "structured.row-schema-contains-many"),
                ],
            ),
        ] {
            let source = Source::new(
                format!(r#"{{"team/~":{{"contacts":{rows}}}}}"#).into_bytes(),
                false,
            );
            let review = review_rows(&schema, &source);
            assert_eq!(
                review
                    .issues
                    .iter()
                    .map(|issue| (pointer(&issue.path), issue.message))
                    .collect::<Vec<_>>(),
                expected
                    .into_iter()
                    .map(|(path, message)| (path.to_owned(), message))
                    .collect::<Vec<_>>()
            );
        }
        assert!(Schema::parse(br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","items":{"$ref":"remote"}}"#).is_none());
    }
}
