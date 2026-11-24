// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use serde_json::{Value, json};

use super::{Folder, FolderReference, ReferencePosition, excerpt, extension, read_text};

const INDEX_BYTES: usize = 8 * 1024 * 1024;
const INDEX_FILES: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: SystemTime,
}

impl Stamp {
    fn read(path: &Path) -> Option<Self> {
        let metadata = std::fs::File::open(path).ok()?.metadata().ok()?;
        Some(Self {
            size: metadata.len(),
            modified: metadata.modified().ok()?,
        })
    }
}

#[derive(Clone)]
struct Entry {
    stamp: Stamp,
    content: String,
    normalized: String,
    partial: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Backlink {
    pub(super) source: FolderReference,
    pub(super) line: usize,
    pub(super) excerpt: String,
}

#[derive(Clone, Default)]
pub(super) struct MarkdownIndex {
    entries: BTreeMap<String, Entry>,
    backlinks: BTreeMap<String, Vec<Backlink>>,
    pub(super) partial: bool,
}

impl MarkdownIndex {
    pub(super) fn build(folder: &Folder) -> Self {
        let mut remaining = INDEX_BYTES;
        let mut entries = BTreeMap::new();
        let mut backlinks = BTreeMap::new();
        let mut markdown = folder
            .files
            .iter()
            .filter(|path| matches!(extension(Path::new(path)).as_str(), "md" | "markdown"));
        let mut incomplete = false;
        for relative in markdown.by_ref().take(INDEX_FILES) {
            let Some(path) = folder.resolve(relative).ok() else {
                incomplete = true;
                continue;
            };
            let Some(stamp) = Stamp::read(&path) else {
                incomplete = true;
                continue;
            };
            let Ok((content, partial)) = read_text(&path) else {
                incomplete = true;
                continue;
            };
            if Stamp::read(&path) != Some(stamp) {
                incomplete = true;
                continue;
            }
            let normalized = content.to_lowercase();
            let Some(budget) = remaining.checked_sub(content.len() + normalized.len()) else {
                incomplete = true;
                break;
            };
            remaining = budget;
            incomplete |= partial;
            index_backlinks(folder, relative, &content, &mut backlinks);
            entries.insert(
                relative.clone(),
                Entry {
                    stamp,
                    content,
                    normalized,
                    partial,
                },
            );
        }
        Self {
            entries,
            backlinks,
            partial: incomplete
                || markdown.next().is_some()
                || folder.truncated
                || folder.skipped > 0,
        }
    }

    pub(super) fn backlinks(&self, relative: &str) -> &[Backlink] {
        self.backlinks
            .get(relative)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn search(
        &self,
        relative: &str,
        path: &Path,
        query: &str,
    ) -> Option<(Vec<Value>, bool)> {
        let entry = self.entries.get(relative)?;
        if Stamp::read(path) != Some(entry.stamp) {
            return None;
        }
        let matches = entry
            .content
            .lines()
            .zip(entry.normalized.lines())
            .enumerate()
            .filter(|(_, (_, normalized))| normalized.contains(query))
            .take(3)
            .map(|(index, (line, _))| json!({"line":index + 1,"text":excerpt(line, 150)}))
            .collect();
        Some((matches, entry.partial))
    }
}

fn index_backlinks(
    folder: &Folder,
    source: &str,
    content: &str,
    backlinks: &mut BTreeMap<String, Vec<Backlink>>,
) {
    let options = Options::ENABLE_WIKILINKS | Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES;
    let body = ibis_formats::markdown::body_without_frontmatter(content);
    let body_offset = content.len() - body.len();
    let newlines: Vec<_> = content
        .match_indices('\n')
        .map(|(offset, _)| offset)
        .collect();
    let mut heading = None;
    let mut pending_heading = None::<String>;
    let mut image_depth = 0;
    for (event, range) in Parser::new_ext(body, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. }) => pending_heading = Some(String::new()),
            Event::End(TagEnd::Heading(_)) => {
                heading = pending_heading
                    .take()
                    .filter(|text| !text.trim().is_empty());
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(heading) = pending_heading.as_mut() {
                    heading.push_str(&text);
                }
            }
            Event::Start(Tag::Image { .. }) => image_depth += 1,
            Event::End(TagEnd::Image) => image_depth -= 1,
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) if image_depth == 0 => {
                let reference = match link_type {
                    LinkType::WikiLink { .. } => format!("[[{dest_url}]]"),
                    LinkType::Autolink | LinkType::Email => continue,
                    _ => dest_url.into_string(),
                };
                let Ok(target) = folder.indexed_reference_path(source, &reference) else {
                    continue;
                };
                if target == source || folder.files.binary_search(&target).is_err() {
                    continue;
                }
                let references = backlinks.entry(target).or_default();
                if references
                    .last()
                    .is_some_and(|link| link.source.path == source)
                {
                    continue;
                }
                let line = newlines.partition_point(|offset| *offset < body_offset + range.start);
                let start = line.checked_sub(1).map_or(0, |index| newlines[index] + 1);
                let end = newlines.get(line).copied().unwrap_or(content.len());
                references.push(Backlink {
                    source: FolderReference {
                        path: source.to_owned(),
                        position: heading
                            .as_ref()
                            .map(|title| ReferencePosition::Heading(title.clone())),
                    },
                    line: line + 1,
                    excerpt: excerpt(content[start..end].trim(), 120),
                });
            }
            _ => {}
        }
    }
}
