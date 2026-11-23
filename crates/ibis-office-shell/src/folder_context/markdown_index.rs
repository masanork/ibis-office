// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use serde_json::{Value, json};

use super::{Folder, excerpt, extension, read_text};

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

#[derive(Clone, Default)]
pub(super) struct MarkdownIndex(BTreeMap<String, Entry>);

impl MarkdownIndex {
    pub(super) fn build(folder: &Folder) -> Self {
        let mut remaining = INDEX_BYTES;
        let mut entries = BTreeMap::new();
        for relative in folder
            .files
            .iter()
            .filter(|path| matches!(extension(Path::new(path)).as_str(), "md" | "markdown"))
            .take(INDEX_FILES)
        {
            let Some(path) = folder.resolve(relative).ok() else {
                continue;
            };
            let Some(stamp) = Stamp::read(&path) else {
                continue;
            };
            let Ok((content, partial)) = read_text(&path) else {
                continue;
            };
            if Stamp::read(&path) != Some(stamp) {
                continue;
            }
            let normalized = content.to_lowercase();
            let Some(budget) = remaining.checked_sub(content.len() + normalized.len()) else {
                break;
            };
            remaining = budget;
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
        Self(entries)
    }

    pub(super) fn search(
        &self,
        relative: &str,
        path: &Path,
        query: &str,
    ) -> Option<(Vec<Value>, bool)> {
        let entry = self.0.get(relative)?;
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
