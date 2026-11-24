// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use chrono::NaiveDate;
use pulldown_cmark::{Event, HeadingLevel, Tag, TagEnd};

use super::diagnostics::{Finding, body_events, line_at};

#[derive(Clone, Copy)]
enum FileKind {
    Index,
    Log,
}

struct Section {
    line: usize,
    has_entries: bool,
}

#[derive(Default)]
struct Item {
    line: usize,
    has_link: bool,
    has_description: bool,
    has_prose: bool,
}

pub(super) fn inspect(path: &str, body: &str, first_line: usize, limit: usize) -> Vec<Finding> {
    let kind = match path.rsplit('/').next() {
        Some("index.md") => FileKind::Index,
        Some("log.md") => FileKind::Log,
        _ => return Vec::new(),
    };
    if limit == 0 {
        return Vec::new();
    }
    let newlines: Vec<_> = body.match_indices('\n').map(|(offset, _)| offset).collect();
    let mut findings = Vec::new();
    let mut depth = 0usize;
    let mut section: Option<Section> = None;
    let mut previous_date = None;
    let mut heading = None;
    let mut item: Option<Item> = None;
    let mut in_link = false;
    let mut saw_block = false;

    for (event, range) in body_events(body) {
        let line = line_at(&newlines, first_line, range.start);
        match event {
            Event::Start(tag) => {
                match (&tag, depth) {
                    (Tag::Heading { level, .. }, 0) => {
                        heading = Some((*level, line, String::new(), !saw_block));
                    }
                    (Tag::Item, 1) => {
                        item = Some(Item {
                            line,
                            ..Item::default()
                        });
                        if let Some(section) = &mut section {
                            section.has_entries = true;
                        }
                    }
                    (Tag::List(_), _) if item.is_some() && matches!(kind, FileKind::Log) => {
                        findings.push(finding(kind, "log_nested_list", line, false));
                    }
                    (Tag::Link { .. }, _) if item.is_some() => in_link = true,
                    (Tag::List(_), 0) if section.is_none() => {
                        findings.push(finding(kind, "reserved_list_without_section", line, false));
                    }
                    (Tag::List(_) | Tag::HtmlBlock, 0) => {}
                    (_, 0) => {
                        findings.push(finding(kind, "reserved_unexpected_block", line, false))
                    }
                    _ => {}
                }
                saw_block |= !matches!(tag, Tag::HtmlBlock);
                depth += 1;
            }
            Event::End(tag) => {
                depth -= 1;
                match (tag, depth) {
                    (TagEnd::Heading(_), 0) => {
                        let (level, line, title, first) =
                            heading.take().expect("top-level heading");
                        if matches!(kind, FileKind::Log) {
                            let title = title.trim();
                            if let Some(date) = parse_date(title) {
                                if previous_date.is_some_and(|previous| previous < date) {
                                    findings.push(finding(kind, "log_date_order", line, false));
                                }
                                previous_date = Some(date);
                            } else if first
                                && level == HeadingLevel::H1
                                && !title.as_bytes().get(..5).is_some_and(|prefix| {
                                    prefix[4] == b'-' && prefix[..4].iter().all(u8::is_ascii_digit)
                                })
                            {
                                continue;
                            } else {
                                findings.push(finding(kind, "log_invalid_date", line, false));
                            }
                        }
                        report_empty_section(
                            kind,
                            section.replace(Section {
                                line,
                                has_entries: false,
                            }),
                            &mut findings,
                        );
                    }
                    (TagEnd::Link, _) if item.is_some() => {
                        in_link = false;
                        if let Some(item) = &mut item {
                            item.has_link = true;
                        }
                    }
                    (TagEnd::Item, 1) => {
                        let item = item.take().expect("top-level list item");
                        match kind {
                            FileKind::Index if !item.has_link => findings.push(finding(
                                kind,
                                "index_entry_without_link",
                                item.line,
                                false,
                            )),
                            FileKind::Index if !item.has_description => findings.push(finding(
                                kind,
                                "index_entry_without_description",
                                item.line,
                                true,
                            )),
                            FileKind::Log if !item.has_prose => findings.push(finding(
                                kind,
                                "log_entry_without_prose",
                                item.line,
                                false,
                            )),
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, _, title, _)) = &mut heading {
                    title.push_str(&text);
                }
                if let Some(item) = &mut item
                    && text.contains(|ch: char| ch.is_alphanumeric())
                {
                    item.has_prose = true;
                    item.has_description |= item.has_link && !in_link;
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, _, title, _)) = &mut heading {
                    title.push(' ');
                }
            }
            _ => {}
        }
        if findings.len() >= limit {
            break;
        }
    }
    if findings.len() < limit {
        if section.is_none() {
            findings.push(finding(kind, "reserved_missing_section", first_line, false));
        } else {
            report_empty_section(kind, section, &mut findings);
        }
    }
    findings.truncate(limit);
    findings.sort_by_key(|issue| issue.line);
    findings
}

fn parse_date(text: &str) -> Option<NaiveDate> {
    let bytes = text.as_bytes();
    if bytes.len() != 10
        || !bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7) {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return None;
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

fn report_empty_section(kind: FileKind, section: Option<Section>, findings: &mut Vec<Finding>) {
    if let Some(section) = section
        && !section.has_entries
    {
        findings.push(finding(kind, "reserved_empty_section", section.line, false));
    }
}

fn finding(kind: FileKind, code: &'static str, line: usize, advisory: bool) -> Finding {
    Finding {
        field: match kind {
            FileKind::Index => "body.index",
            FileKind::Log => "body.log",
        }
        .into(),
        code,
        target: None,
        line: Some(line),
        severity: Some(if advisory { "warning" } else { "error" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_checks_grouping_link_and_description_separately() {
        let body = "- Unsorted\n\n# Sales\n\n- [Revenue](revenue.md) — monthly revenue\n- [Costs][costs]\n- Plain text\n\n# Empty\n\n[costs]: costs.md\n";
        let issues: Vec<_> = inspect("nested/index.md", body, 4, 10)
            .into_iter()
            .map(|issue| (issue.line, issue.code, issue.severity))
            .collect();
        assert_eq!(
            issues,
            [
                (Some(4), "reserved_list_without_section", Some("error")),
                (Some(4), "index_entry_without_link", Some("error")),
                (Some(9), "index_entry_without_description", Some("warning")),
                (Some(10), "index_entry_without_link", Some("error")),
                (Some(12), "reserved_empty_section", Some("error")),
            ]
        );
        assert!(inspect("notes.md", body, 4, 10).is_empty());
    }

    #[test]
    fn log_requires_calendar_dates_newest_first_and_flat_entries() {
        let body = "# 2024 change log\n\n## 2024-02-29\n\n- Added a chart\n  - Nested\n\n## 2024-03-01\n\n- **Update**: Corrected a figure\n\n## 2025-02-29\n\n- Retired an item\n";
        let issues: Vec<_> = inspect("log.md", body, 1, 10)
            .into_iter()
            .map(|issue| (issue.line, issue.code))
            .collect();
        assert_eq!(
            issues,
            [
                (Some(6), "log_nested_list"),
                (Some(8), "log_date_order"),
                (Some(12), "log_invalid_date"),
            ]
        );
        assert!(
            inspect(
                "log.md",
                "# Update log\n\n## 2024-02-29\n\n- Added data\n\n## 2024-02-25\n\n- Initial entry",
                1,
                10
            )
            .is_empty()
        );
    }
}
