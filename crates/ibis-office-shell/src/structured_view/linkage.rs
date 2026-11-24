// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Definition {
    format: String,
    version: u32,
    layout: Layout,
    pub header: Vec<String>,
    code_values: HashMap<String, HashSet<String>>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

#[derive(Deserialize)]
struct Layout {
    id: String,
    name: String,
    repeat_header: String,
    columns: Vec<Column>,
}

#[derive(Deserialize)]
pub(super) struct Column {
    name: String,
    field: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    required: Option<Required>,
    repeat: Option<usize>,
    code: Option<References>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Required {
    Boolean(bool),
    Conditional(String),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum References {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
pub(super) struct Condition {
    pub target: String,
    pub note: Option<String>,
    pub predicate: Option<Predicate>,
}

#[derive(Deserialize)]
pub(super) struct Predicate {
    pub field: String,
    values: Option<HashSet<String>>,
}

impl Predicate {
    pub fn matches(&self, value: &str) -> bool {
        self.values
            .as_ref()
            .map_or(!value.is_empty(), |values| values.contains(value))
    }
}

pub(super) struct Issue {
    pub record: usize,
    pub column: usize,
    pub message: &'static str,
    pub condition: Option<usize>,
}

pub(super) const DISPLAY_LIMIT: usize = 20;

pub(super) struct Review {
    pub label: String,
    pub checked: usize,
    pub issue_count: usize,
    pub issues: Vec<Issue>,
    pub unavailable: Vec<usize>,
}

impl Definition {
    pub fn sidecar(path: &Path) -> Option<Result<Self, &'static str>> {
        let file = match File::open(path.with_extension("definition.json")) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(_) => return Some(Err("structured.linkage-definition-invalid")),
        };
        let mut data = Vec::new();
        let limit = 8 * 1024 * 1024;
        Some(
            if file.take(limit + 1).read_to_end(&mut data).is_ok() && data.len() as u64 <= limit {
                Self::parse(&data).ok_or("structured.linkage-definition-invalid")
            } else {
                Err("structured.linkage-definition-invalid")
            },
        )
    }

    pub(super) fn parse(data: &[u8]) -> Option<Self> {
        let definition: Self = serde_json::from_slice(data).ok()?;
        if definition.format != "mrlgss-linkage-csv" || definition.version != 1 {
            return None;
        }
        let unique: HashSet<_> = definition.header.iter().collect();
        if unique.is_empty() || unique.len() != definition.header.len() {
            return None;
        }
        let mut headers = definition.header.iter();
        for column in &definition.layout.columns {
            let count = column.repeat.unwrap_or(1);
            if count == 0
                || count > definition.header.len()
                || matches!(&column.required, Some(Required::Conditional(value)) if value != "conditional")
            {
                return None;
            }
            for index in 1..=count {
                let next = headers.next()?;
                let matches = if count == 1 {
                    next == &column.name
                } else {
                    next == &definition
                        .layout
                        .repeat_header
                        .replacen("{name}", &column.name, 1)
                        .replacen("{n}", &index.to_string(), 1)
                };
                if !matches {
                    return None;
                }
            }
        }
        let mut conditional: HashSet<_> = definition
            .layout
            .columns
            .iter()
            .filter(|column| matches!(&column.required, Some(Required::Conditional(_))))
            .map(|column| column.name.as_str())
            .collect();
        if definition.conditions.iter().any(|condition| {
            !conditional.remove(condition.target.as_str())
                || condition.predicate.as_ref().is_some_and(|predicate| {
                    predicate
                        .values
                        .as_ref()
                        .is_some_and(|values| values.is_empty() || values.contains(""))
                })
        }) {
            return None;
        }
        headers.next().is_none().then_some(definition)
    }

    pub fn columns(&self) -> impl Iterator<Item = &Column> + Clone {
        self.layout
            .columns
            .iter()
            .flat_map(|column| std::iter::repeat_n(column, column.repeat.unwrap_or(1)))
    }

    pub fn has_field(&self, field: &str) -> bool {
        self.layout
            .columns
            .iter()
            .any(|column| column.field.as_deref() == Some(field))
    }

    pub fn condition_columns(&self, condition: &Condition) -> Option<(usize, usize)> {
        let predicate = condition.predicate.as_ref()?;
        let columns = self.columns();
        let mut sources = columns
            .clone()
            .enumerate()
            .filter(|(_, column)| column.field.as_deref() == Some(&predicate.field));
        let (source, column) = sources.next()?;
        if column.repeat.unwrap_or(1) != 1 || sources.next().is_some() {
            return None;
        }
        let (target, _) = columns.enumerate().find(|(_, column)| {
            column.name == condition.target && column.repeat.unwrap_or(1) == 1
        })?;
        Some((source, target))
    }

    pub fn issue_reason(&self, issue: &Issue) -> String {
        let message = crate::i18n::text(issue.message);
        issue
            .condition
            .and_then(|index| self.conditions.get(index))
            .and_then(|condition| condition.note.as_deref())
            .and_then(|note| note.lines().next())
            .map_or_else(|| message.to_string(), |note| format!("{message} · {note}"))
    }

    pub fn review(&self) -> Review {
        Review {
            label: format!("{} · {}", self.layout.id, self.layout.name),
            checked: 0,
            issue_count: 0,
            issues: Vec::new(),
            unavailable: Vec::new(),
        }
    }

    pub fn check(&self, column: &Column, value: &str) -> Option<&'static str> {
        if value.is_empty() {
            return (column.repeat.unwrap_or(1) == 1
                && matches!(column.required, Some(Required::Boolean(true))))
            .then_some("structured.linkage-required");
        }
        if let Some(kind) = column
            .kind
            .as_deref()
            .map(|kind| kind.trim_end_matches("[]"))
        {
            let base = kind.split_once('(').map_or(kind, |(base, _)| base);
            let half_width = |ch: char| matches!(ch, ' '..='~' | '\u{ff61}'..='\u{ff9f}');
            let digits = value
                .strip_prefix('-')
                .filter(|_| base == "S9")
                .unwrap_or(value);
            let invalid = match base {
                "X" => !value.chars().all(half_width),
                "N" => value.chars().any(half_width),
                "9" | "S9" => {
                    digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit())
                }
                _ => false,
            };
            if invalid {
                return Some("structured.linkage-type");
            }
            let max = kind
                .split_once('(')
                .and_then(|(_, n)| n.strip_suffix(')'))
                .and_then(|n| n.trim_start_matches("..").parse::<usize>().ok());
            if let Some(max) = max.filter(|_| matches!(base, "X" | "N" | "9" | "S9"))
                && digits
                    .chars()
                    .filter(|ch| !matches!(ch, '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}'))
                    .count()
                    > max
            {
                return Some("structured.linkage-length");
            }
        }
        let refs = match &column.code {
            Some(References::One(code)) => std::slice::from_ref(code),
            Some(References::Many(codes)) => codes,
            None => return None,
        };
        if refs.is_empty() {
            return None;
        }
        refs.iter()
            .all(|code| {
                self.code_values
                    .get(code)
                    .is_some_and(|values| !values.contains(value))
            })
            .then_some("structured.linkage-code")
    }
}
