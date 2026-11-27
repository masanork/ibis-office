// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};

use ibis_doc_model::ai_sheet::parse_cell_range;
use serde_json::{Value, json};

use crate::ai_client::{ToolCall, ToolDefinition, ToolFunctionDef};

mod instructions;
mod markdown_index;
mod skills;
mod ui;
mod vault_db;
pub(crate) use ui::FolderPane;

const SCAN_LIMIT: usize = 10_000;
const TEXT_BYTES: usize = 512 * 1024;
const RESULT_BYTES: usize = 24 * 1024;
pub(crate) const OFFICE: &[&str] = &["docx", "docm", "dotx", "xlsx", "xlsm", "pptx", "pptm"];
const TEXT: &[&str] = &[
    "md", "markdown", "txt", "csv", "tsv", "json", "jsonl", "ndjson", "yaml", "yml", "sd-jwt",
    "mdoc",
];

pub(crate) fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn reference_target(reference: &str) -> &str {
    reference.strip_prefix("[[").and_then(|wiki| wiki.strip_suffix("]]"))
        .map_or(reference, |wiki| wiki.split_once('|').map_or(wiki, |(target, _)| target))
}

fn pick_obsidian_candidate<'a>(from: &str, candidates: &[&'a String]) -> Option<&'a String> {
    use std::cmp::Reverse;

    let from_dir = from.rsplit_once('/').map_or("", |(dir, _)| dir);
    let from_parts: Vec<&str> = from_dir.split('/').filter(|part| !part.is_empty()).collect();
    let score = |candidate: &str| {
        let cand_dir = candidate.rsplit_once('/').map_or("", |(dir, _)| dir);
        let same_dir = cand_dir == from_dir;
        let cand_parts = cand_dir.split('/').filter(|part| !part.is_empty());
        let common = from_parts
            .iter()
            .zip(cand_parts)
            .take_while(|(a, b)| **a == *b)
            .count();
        let depth = candidate.matches('/').count();
        (same_dir, common, Reverse(depth))
    };
    let mut best: Option<(&'a String, (bool, usize, Reverse<usize>))> = None;
    let mut tied = false;
    for &candidate in candidates {
        let current = score(candidate);
        match best {
            None => best = Some((candidate, current)),
            Some((_, best_score)) if current > best_score => {
                best = Some((candidate, current));
                tied = false;
            }
            Some((_, best_score)) if current == best_score => {
                tied = true;
            }
            _ => {}
        }
    }
    (!tied).then(|| best.map(|(candidate, _)| candidate)).flatten()
}

pub(crate) fn visible(name: &str) -> bool {
    !name.starts_with('.') && !name.starts_with("~$")
}

#[derive(Clone)]
struct ConceptFilter {
    kind: String,
    tags: Vec<String>,
    review: ibis_formats::okf::Review,
}

pub(crate) fn is_obsidian_vault(path: &Path) -> bool {
    path.join(".obsidian").is_dir()
}

#[derive(Clone)]
pub struct Folder {
    root: PathBuf,
    files: Vec<String>,
    concepts: BTreeMap<String, ConceptFilter>,
    markdown_index: markdown_index::MarkdownIndex,
    pub(crate) skills: Vec<ibis_formats::agent_skill::Metadata>,
    pub(crate) is_obsidian_vault: bool,
    pub(crate) has_git: bool,
    truncated: bool,
    skipped: usize,
}

impl Folder {
    fn scan(path: &Path) -> Result<Self, String> {
        let root = path.canonicalize().map_err(|err| err.to_string())?;
        let is_obsidian_vault = is_obsidian_vault(&root);
        let has_git = crate::git_workspace::GitWorkspace::for_selected_folder(&root).is_some();
        let mut files = Vec::new();
        let mut remaining = SCAN_LIMIT;
        let mut skipped = 0;
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(error) if dir == root => return Err(error.to_string()),
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            for entry in entries {
                if remaining == 0 {
                    break;
                }
                remaining -= 1;
                let Ok(entry) = entry else {
                    skipped += 1;
                    continue;
                };
                let name = entry.file_name();
                if !name.to_str().is_some_and(visible) {
                    continue;
                }
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    skipped += 1;
                    continue;
                };
                if kind.is_dir() {
                    dirs.push(path);
                } else if kind.is_file() {
                    let ext = extension(&path);
                    if ibis_formats::Kind::from_extension(&ext).is_some()
                        || TEXT.contains(&ext.as_str())
                    {
                        let relative = path.strip_prefix(&root).expect("entry below root");
                        files.push(
                            relative
                                .iter()
                                .map(|part| part.to_string_lossy())
                                .collect::<Vec<_>>()
                                .join("/"),
                        );
                    }
                }
            }
            if remaining == 0 {
                break;
            }
        }
        files.sort_unstable();
        let mut folder = Self {
            root,
            files,
            concepts: BTreeMap::new(),
            markdown_index: markdown_index::MarkdownIndex::default(),
            skills: Vec::new(),
            is_obsidian_vault,
            has_git,
            truncated: remaining == 0,
            skipped,
        };
        folder.concepts = folder
            .files
            .iter()
            .filter(|path| path.ends_with(".md"))
            .filter_map(|path| {
                let frontmatter = read_frontmatter(&folder.resolve(path).ok()?)?;
                let metadata = ibis_formats::okf::concept(path, &frontmatter).ok()??;
                let kind = metadata["type"].as_str()?.to_lowercase();
                let tags = ibis_formats::okf::tags(metadata.get("tags"))
                    .unwrap_or_default()
                    .into_iter()
                    .map(str::to_lowercase)
                    .collect();
                let review = ibis_formats::okf::Review::from_metadata(&metadata);
                Some((path.clone(), ConceptFilter { kind, tags, review }))
            })
            .collect();
        folder.markdown_index = markdown_index::MarkdownIndex::build(&folder);
        folder.skills = folder.scan_skills();
        Ok(folder)
    }

    pub(crate) fn name(&self) -> String {
        self.root
            .file_name()
            .unwrap_or(self.root.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    fn resolve(&self, relative: &str) -> Result<PathBuf, String> {
        if relative.is_empty()
            || relative.contains('\\')
            || relative
                .split('/')
                .any(|name| name.is_empty() || !visible(name))
        {
            return Err("Use a visible, folder-relative path with forward slashes.".into());
        }
        self.resolve_file(relative)
    }

    fn resolve_file(&self, relative: &str) -> Result<PathBuf, String> {
        let real = self.resolve_entry(relative)?;
        if !real.is_file() {
            return Err("Path must identify a file inside the open folder.".into());
        }
        Ok(real)
    }

    fn resolve_entry(&self, relative: &str) -> Result<PathBuf, String> {
        if self.root.canonicalize().ok().as_ref() != Some(&self.root) || !self.root.is_dir() {
            return Err("The reference folder moved. Open it again.".into());
        }
        let mut path = self.root.clone();
        for component in Path::new(relative).components() {
            let Component::Normal(name) = component else {
                return Err("Path is outside the open folder.".into());
            };
            path.push(name);
            let meta = path.symlink_metadata().map_err(|err| err.to_string())?;
            if meta.file_type().is_symlink() {
                return Err("Symbolic links are excluded from the open folder.".into());
            }
        }
        let real = path.canonicalize().map_err(|err| err.to_string())?;
        if !real.starts_with(&self.root) {
            return Err("Path must identify a file inside the open folder.".into());
        }
        Ok(real)
    }

    fn link_from_file(&self, file: &Path, url: &str) -> Option<Result<FolderReference, String>> {
        if url.starts_with("//")
            || url.split(['/', '?', '#']).next().is_some_and(|head| head.contains(':'))
            || !matches!(extension(file).as_str(), "md" | "markdown")
        {
            return None;
        }
        let relative = file.strip_prefix(&self.root).ok()?;
        let components = relative.iter().map(|part| part.to_str()).collect::<Option<Vec<_>>>()?;
        let from = components.join("/");
        Some(self.reference_path(&from, url).map(|path| FolderReference {
            path,
            position: reference_target(url).split_once('#').and_then(|(_, fragment)| {
                (!fragment.is_empty()).then(|| ReferencePosition::Heading(fragment.to_owned()))
            }),
        }))
    }

    fn reference_path(&self, from: &str, reference: &str) -> Result<String, String> {
        self.resolve(from)?;
        self.indexed_reference_path(from, reference)
    }

    fn indexed_reference_path(&self, from: &str, reference: &str) -> Result<String, String> {
        self.indexed_reference_path_with_aliases(
            from,
            reference,
            Some(self.markdown_index.aliases()),
        )
    }

    pub(super) fn indexed_reference_path_with_aliases(
        &self,
        from: &str,
        reference: &str,
        aliases: Option<&BTreeMap<String, Vec<String>>>,
    ) -> Result<String, String> {
        if let Some(wiki) = reference
            .strip_prefix("[[")
            .and_then(|text| text.strip_suffix("]]"))
        {
            let target = wiki.split(['|', '#']).next().unwrap_or("").trim();
            if target.is_empty() {
                return Ok(from.to_owned());
            }
            let matches_path =
                |path: &str| path == target || path.strip_suffix(".md") == Some(target);
            let target_lower = target.to_lowercase();
            let matches_path_ci = |path: &str| {
                let lower = path.to_lowercase();
                lower == target_lower || lower.strip_suffix(".md") == Some(target_lower.as_str())
            };
            if target.contains('/') {
                if let Some(exact) = self.files.iter().find(|path| matches_path(path)) {
                    return Ok(exact.clone());
                }
                let ci: Vec<&String> = self
                    .files
                    .iter()
                    .filter(|path| matches_path_ci(path))
                    .collect();
                if ci.len() == 1 {
                    return Ok(ci[0].clone());
                }
                if ci.len() > 1 {
                    return Err("Wiki link is ambiguous or the folder listing is incomplete. Use a root-relative path.".into());
                }
                return Err("Wiki link target was not found in the open folder.".into());
            }

            let mut candidates: Vec<&String> = self
                .files
                .iter()
                .filter(|path| path.rsplit('/').next().is_some_and(matches_path))
                .collect();
            if candidates.is_empty() {
                candidates = self
                    .files
                    .iter()
                    .filter(|path| path.rsplit('/').next().is_some_and(matches_path_ci))
                    .collect();
            }
            if candidates.is_empty()
                && let Some(alias_paths) = aliases.and_then(|map| map.get(&target_lower))
            {
                candidates = alias_paths
                    .iter()
                    .filter(|path| self.files.binary_search(path).is_ok())
                    .collect();
            }
            if candidates.is_empty() {
                return Err("Wiki link target was not found in the open folder.".into());
            }
            if self.truncated || self.skipped > 0 {
                return Err("Wiki link is ambiguous or the folder listing is incomplete. Use a root-relative path.".into());
            }
            if candidates.len() == 1 {
                return Ok(candidates[0].clone());
            }
            if self.is_obsidian_vault
                && let Some(best) = pick_obsidian_candidate(from, &candidates)
            {
                return Ok(best.clone());
            }
            return Err("Wiki link is ambiguous or the folder listing is incomplete. Use a root-relative path.".into());
        }
        ibis_formats::okf::resolve(from, reference)
            .map(|(path, _)| path)
            .ok_or_else(|| "The link does not identify a file inside the open folder.".into())
    }

    fn matches(&self, path: &str, query: &str) -> bool {
        if let Some(kind) = query.strip_prefix("type:") {
            self.concepts
                .get(path)
                .is_some_and(|concept| concept.kind.contains(kind.trim()))
        } else if let Some(tag) = query.strip_prefix("tag:") {
            let tag_query = tag.trim().trim_start_matches('#').trim_matches('/');
            if tag_query.is_empty() {
                return false;
            }
            let tag_matches = |actual: &str| {
                actual == tag_query
                    || actual
                        .strip_prefix(tag_query)
                        .is_some_and(|rest| rest.starts_with('/'))
            };
            self.concepts
                .get(path)
                .is_some_and(|concept| concept.tags.iter().any(|actual| tag_matches(actual)))
                || self
                    .markdown_index
                    .tags(path)
                    .iter()
                    .any(|actual| tag_matches(actual))
        } else {
            path.to_lowercase().contains(query)
        }
    }

    pub(crate) fn has_index(&self) -> bool {
        self.files.iter().any(|path| path == "index.md")
    }

    pub(super) fn search_markdown_content(
        &self,
        query: &str,
        max_hits: usize,
    ) -> Vec<ContentSearchHit> {
        let query = query.trim().to_lowercase();
        if query.is_empty() || max_hits == 0 {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for relative in self
            .files
            .iter()
            .filter(|path| matches!(extension(Path::new(path)).as_str(), "md" | "markdown"))
        {
            let Ok(path) = self.resolve(relative) else {
                continue;
            };
            if let Some((matches, _)) = self.markdown_index.search(relative, &path, &query) {
                for item in matches {
                    if let (Some(line), Some(text)) =
                        (item["line"].as_u64(), item["text"].as_str())
                    {
                        hits.push(ContentSearchHit {
                            path: relative.clone(),
                            line: usize::try_from(line).unwrap_or(1),
                            excerpt: text.to_owned(),
                        });
                        if hits.len() >= max_hits {
                            return hits;
                        }
                    }
                }
                continue;
            }
            let Ok((content, _)) = read_text(&path) else {
                continue;
            };
            let mut file_hits = 0usize;
            for (index, line) in content.lines().enumerate() {
                if line.to_lowercase().contains(&query) {
                    hits.push(ContentSearchHit {
                        path: relative.clone(),
                        line: index + 1,
                        excerpt: excerpt(line.trim(), 120),
                    });
                    file_hits += 1;
                    if hits.len() >= max_hits {
                        return hits;
                    }
                    if file_hits >= 3 {
                        break;
                    }
                }
            }
        }
        hits
    }

    /// Create a new Markdown note for an unresolved Obsidian Wiki link inside the vault.
    pub(super) fn create_wikilink_note(&self, file: &Path, url: &str) -> Result<Option<String>, String> {
        if !self.is_obsidian_vault {
            return Ok(None);
        }
        let Some(wiki) = url.strip_prefix("[[").and_then(|text| text.strip_suffix("]]")) else {
            return Ok(None);
        };
        let target = wiki.split(['|', '#']).next().unwrap_or("").trim();
        if target.is_empty()
            || target.contains(['\\', ':', '\0', '\r', '\n', '[', ']', '?', '*'])
            || target.starts_with('/')
            || target.ends_with('/')
        {
            return Ok(None);
        }
        if target
            .split('/')
            .any(|part| part.trim().is_empty() || !visible(part))
        {
            return Ok(None);
        }
        let relative_from = file
            .strip_prefix(&self.root)
            .ok()
            .and_then(|rel| rel.iter().map(|p| p.to_str()).collect::<Option<Vec<_>>>())
            .map(|parts| parts.join("/"))
            .unwrap_or_default();

        let mut rel = if target.contains('/') {
            target.to_owned()
        } else if let Some((dir, _)) = relative_from.rsplit_once('/') {
            format!("{dir}/{target}")
        } else {
            target.to_owned()
        };
        let ext = extension(Path::new(&rel));
        if !matches!(ext.as_str(), "md" | "markdown") {
            rel.push_str(".md");
        }
        if self.files.binary_search(&rel).is_ok() {
            return Ok(Some(rel));
        }

        // Verify the parent chain stays inside the vault and crosses no symlinks.
        let mut cursor = self.root.clone();
        let parts: Vec<&str> = rel.split('/').collect();
        let (file_name, parent_parts) = parts.split_last().ok_or("Invalid note path.")?;
        for part in parent_parts {
            cursor.push(part);
            match cursor.symlink_metadata() {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err("Symbolic links are excluded from the open folder.".into());
                }
                Ok(meta) if !meta.is_dir() => {
                    return Err("A file with the same name blocks creating the note folder.".into());
                }
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&cursor).map_err(|e| e.to_string())?;
                }
                Err(err) => return Err(err.to_string()),
            }
        }
        let canon_parent = cursor.canonicalize().map_err(|err| err.to_string())?;
        if !canon_parent.starts_with(&self.root) {
            return Err("Path is outside the open folder.".into());
        }
        let note_path = canon_parent.join(file_name);
        if note_path.exists() {
            return Ok(Some(rel));
        }
        let title = file_name
            .strip_suffix(".md")
            .or_else(|| file_name.strip_suffix(".markdown"))
            .unwrap_or(file_name);
        std::fs::write(&note_path, format!("# {title}\n")).map_err(|err| err.to_string())?;
        Ok(Some(rel))
    }
}

const OFFICE_READS: &[&str] = &[
    "docx_get_outline",
    "docx_get_text",
    "docx_list_paragraphs",
    "docx_get_paragraph",
    "docx_find_text",
    "xlsx_list_sheets",
    "xlsx_get_used_range",
    "xlsx_get_cells",
    "xlsx_find_text",
    "pptx_list_slides",
    "pptx_get_slide",
    "pptx_get_shape_text",
    "pptx_get_notes",
];

pub(crate) fn tools() -> [ToolDefinition; 8] {
    let tool = |name, description, properties, required| ToolDefinition {
        kind: "function",
        function: ToolFunctionDef {
            name,
            description,
            parameters: json!({
                "type": "object", "properties": properties, "required": required, "additionalProperties": false
            }),
        },
    };
    [
        tool(
            "folder_diagnose_okf",
            "Inspect OKF YAML metadata and local references in YAML and Markdown bodies, including Markdown heading fragments in the open folder. Findings are advisory and no files are changed. Pass next_offset to inspect more Markdown files.",
            json!({"offset":{"type":"integer","minimum":0}}),
            json!([]),
        ),
        tool(
            "folder_list",
            "List reference files in the open folder, optionally filtering by path text, knowledge type (type:Metric), or tag (tag:finance). Paths are relative to the folder.",
            json!({"query":{"type":"string"},"offset":{"type":"integer","minimum":0}}),
            json!([]),
        ),
        tool(
            "folder_search",
            "Search contents of Markdown/text/CSV/JSON/YAML and DOCX/XLSX/PPTX files in the open folder. Pass next_offset to continue.",
            json!({"query":{"type":"string"},"offset":{"type":"integer","minimum":0},"path_filter":{"type":"string"}}),
            json!(["query"]),
        ),
        tool(
            "folder_read",
            "Read a folder reference. Markdown (including OKF index.md), text, CSV, JSON and YAML use offset (0-based line) and limit. OKF concepts also return okf_review with trust, freshness and evidence. With from, resolve path as an OKF URI reference or a Wiki link in the folder. For Office files use an operation and its arguments; examples: docx_list_paragraphs with offset and limit, xlsx_get_cells with sheet and range, pptx_get_slide with slide. Omit operation for the outline, sheets or slides.",
            json!({"path":{"type":"string"},"from":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1},
                "operation":{"type":"string","enum":OFFICE_READS},"arguments":{"type":"object"}}),
            json!(["path"]),
        ),
        tool("git_status", "Read Git index and working-tree changes under the open folder. Optional path is relative to the folder. Pass next_offset to continue.",
            json!({"path":{"type":"string"},"offset":{"type":"integer","minimum":0}}), json!([])),
        tool("git_log", "Read first-parent commits that changed a file or directory under the open folder. Omit path for the folder itself. Pass next_cursor to continue, even if a page is empty.",
            json!({"path":{"type":"string"},"cursor":{"type":"string"}}), json!([])),
        tool("git_read", "Read a file at a full commit SHA from git_log. With from, resolve path as an OKF URI reference within the same revision and folder. Pass next_offset to continue.",
            json!({"path":{"type":"string"},"from":{"type":"string"},"commit":{"type":"string"},"offset":{"type":"integer","minimum":0}}), json!(["path","commit"])),
        tool("git_diff", "Compare one folder-relative file between two full commit SHAs from git_log. Office files are compared as extracted Markdown. Pass next_offset to continue.",
            json!({"path":{"type":"string"},"old_commit":{"type":"string"},"new_commit":{"type":"string"},"offset":{"type":"integer","minimum":0}}),
            json!(["path","old_commit","new_commit"])),

    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentSearchHit {
    pub path: String,
    pub line: usize,
    pub excerpt: String,
}

/// Public snapshot of a Markdown backlink inside an open Vault folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultBacklink {
    pub path: String,
    pub line: usize,
    pub excerpt: String,
}

impl Folder {
    /// Scan a directory (including `.obsidian` vault detection, Git detection, and Markdown index).
    pub fn scan_dir(path: &Path) -> Result<Self, String> {
        Self::scan(path)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn display_name(&self) -> String {
        self.name()
    }

    pub fn is_obsidian_vault(&self) -> bool {
        self.is_obsidian_vault
    }

    pub fn has_git(&self) -> bool {
        self.has_git
    }

    pub fn matches_query(&self, path: &str, query: &str) -> bool {
        self.matches(path, query)
    }

    pub fn resolve_file_path(&self, relative: &str) -> Result<PathBuf, String> {
        self.resolve(relative)
    }

    pub fn search_content(&self, query: &str, limit: usize) -> Vec<ContentSearchHit> {
        self.search_markdown_content(query, limit)
    }

    pub fn backlinks_for(&self, relative: &str) -> Vec<VaultBacklink> {
        self.markdown_index
            .backlinks(relative)
            .iter()
            .map(|b| VaultBacklink {
                path: b.source.path.clone(),
                line: b.line,
                excerpt: b.excerpt.clone(),
            })
            .collect()
    }

    /// Unique Vault-relative paths linked from `relative` via Markdown or Wiki links.
    pub fn outgoing_links_for(&self, relative: &str) -> Vec<String> {
        self.markdown_index.outgoing_links(relative)
    }

    /// Tags attached to `relative` (frontmatter `tags`, inline `#tag`, and OKF concept tags).
    pub fn tags_for(&self, relative: &str) -> Vec<String> {
        let mut tags: Vec<String> = self.markdown_index.tags(relative).to_vec();
        if let Some(concept) = self.concepts.get(relative) {
            for t in &concept.tags {
                if !tags.iter().any(|existing| existing.eq_ignore_ascii_case(t)) {
                    tags.push(t.clone());
                }
            }
        }
        tags.sort();
        tags
    }

    /// Every tag found across the Vault with its file count, sorted by frequency then name.
    pub fn all_tags(&self) -> Vec<(String, usize)> {
        let mut counts: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
            std::collections::BTreeMap::new();
        for (path, tags) in self.markdown_index.all_tags_map() {
            for tag in tags {
                let normalized = tag.trim_start_matches('#').trim().to_owned();
                if !normalized.is_empty() {
                    counts.entry(normalized).or_default().insert(path.clone());
                }
            }
        }
        for (path, concept) in &self.concepts {
            for tag in &concept.tags {
                let normalized = tag.trim_start_matches('#').trim().to_owned();
                if !normalized.is_empty() {
                    counts.entry(normalized).or_default().insert(path.clone());
                }
            }
        }
        let mut items: Vec<(String, usize)> = counts
            .into_iter()
            .map(|(tag, files)| (tag, files.len()))
            .collect();
        items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        items
    }

    /// Vault-relative paths carrying `tag` (supports Obsidian hierarchical tag prefixes like `a/b`).
    pub fn files_with_tag(&self, tag: &str) -> Vec<String> {
        let needle = tag.trim_start_matches('#').trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let matches_tag = |actual: &str| {
            let a = actual.trim_start_matches('#').trim().to_lowercase();
            a == needle || a.starts_with(&format!("{needle}/"))
        };
        let mut files: Vec<String> = self
            .files
            .iter()
            .filter(|path| {
                self.markdown_index
                    .tags(path)
                    .iter()
                    .any(|t| matches_tag(t))
                    || self
                        .concepts
                        .get(path.as_str())
                        .is_some_and(|c| c.tags.iter().any(|t| matches_tag(t)))
            })
            .cloned()
            .collect();
        files.sort();
        files
    }

    pub fn relative_path_for(&self, path: &Path) -> Option<String> {
        let canonical = path.canonicalize().ok()?;
        let rel = canonical.strip_prefix(&self.root).ok()?;
        let mut parts = Vec::new();
        for comp in rel.components() {
            if let Component::Normal(s) = comp {
                parts.push(s.to_str()?);
            } else {
                return None;
            }
        }
        (!parts.is_empty()).then(|| parts.join("/"))
    }

    pub fn vault_profiles(&self) -> &[ibis_formats::form_assist::VaultProfile] {
        self.markdown_index.profiles()
    }

    pub fn vault_credentials(&self) -> &[ibis_formats::form_assist::VaultCredentialSummary] {
        self.markdown_index.credentials()
    }

    pub fn vault_form_mappings(&self) -> &[ibis_formats::form_assist::FormMapping] {
        self.markdown_index.form_mappings()
    }

    pub fn vault_presentation_requests(
        &self,
    ) -> &[ibis_formats::form_assist::PresentationRequestNote] {
        self.markdown_index.presentation_requests()
    }

    pub fn matching_form_mappings_for_sheet(
        &self,
        presentation: &ibis_doc_model::Presentation,
        slide: ibis_doc_model::SlideId,
        file_sha256: Option<&str>,
    ) -> Vec<ibis_formats::form_assist::FormMapping> {
        let mut scored: Vec<(u32, ibis_formats::form_assist::FormMapping)> = self
            .markdown_index
            .form_mappings()
            .iter()
            .filter_map(|m| {
                ibis_formats::form_assist::score_form_mapping_match(
                    m,
                    presentation,
                    slide,
                    file_sha256,
                )
                .map(|score| (score, m.clone()))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
        scored.into_iter().map(|(_, m)| m).collect()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FolderReference {
    path: String,
    position: Option<ReferencePosition>,
}

#[derive(Clone, Debug)]
enum ReferencePosition {
    Cells { sheet: String, range: String },
    Paragraph(u32),
    Heading(String),
    Slide(usize),
}

impl FolderReference {
    fn new(path: &str, result: &Value) -> Self {
        let position = (|| {
            if let Some(sheet) = result["sheet"].as_str()
                && let Some(range) = result["range"].as_str().or_else(|| result["cell"].as_str())
            {
                return Some(ReferencePosition::Cells {
                    sheet: sheet.into(),
                    range: range.into(),
                });
            }
            if let Some(id) = result["para_id"]
                .as_u64()
                .and_then(|id| u32::try_from(id).ok())
            {
                return Some(ReferencePosition::Paragraph(id));
            }
            result["slide"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .map(ReferencePosition::Slide)
        })();
        Self {
            path: path.into(),
            position,
        }
    }

    pub(crate) fn label(&self) -> String {
        match &self.position {
            Some(ReferencePosition::Cells { sheet, range }) => {
                format!("{} · {sheet}!{range}", self.path)
            }
            Some(ReferencePosition::Slide(n)) => format!("{} · {n}", self.path),
            _ => self.path.clone(),
        }
    }
}

pub(crate) struct FolderResult {
    pub(crate) value: Value,
    pub(crate) references: Vec<FolderReference>,
}

pub(crate) fn recognizes(name: &str) -> bool {
    matches!(name, "folder_diagnose_okf" | "folder_list" | "folder_search" | "folder_read" | "git_status" | "git_log" | "git_diff" | "git_read")
}

pub(crate) fn run(folder: &Folder, call: &ToolCall, skill: Option<&str>) -> Result<FolderResult, String> {
    let args: Value =
        serde_json::from_str(&call.function.arguments).map_err(|err| err.to_string())?;
    let offset = number(&args["offset"], 0)?;
    let (value, references) = match call.function.name.as_str() {
        "folder_diagnose_okf" => {
            let mut files = folder.files.iter().filter(|path| path.ends_with(".md")).skip(offset);
            let mut examined = 0;
            let mut issues = Vec::new();
            let mut references = Vec::new();
            let mut headings = BTreeMap::new();
            let mut partial = folder.truncated || folder.skipped > 0;
            for path in files.by_ref().take(32) {
                examined += 1;
                let Some((content, truncated)) = folder.resolve(path).ok()
                    .and_then(|path| read_text(&path).ok())
                else {
                    partial = true;
                    continue;
                };
                partial |= truncated;
                let Some(prefix) = frontmatter_prefix(content.as_bytes()) else {
                    partial = true;
                    continue;
                };
                let exists = |target: &str| folder.resolve_entry(target).is_ok();
                let mut findings = ibis_formats::okf::diagnostics::inspect(path, &prefix, exists);
                if prefix.is_empty() || prefix.lines().skip(1).last() == Some("---") {
                    let body = &content[prefix.len()..];
                    let body = if truncated {
                        body.rsplit_once('\n').map_or("", |(complete, _)| complete)
                    } else {
                        body
                    };
                    let first_line = 1 + prefix.bytes().filter(|byte| *byte == b'\n').count();
                    if !truncated {
                        findings.extend(ibis_formats::okf::diagnostics::inspect_reserved_body(
                            path, body, first_line, 13usize.saturating_sub(findings.len()),
                        ));
                    }
                    findings.extend(
                        ibis_formats::okf::diagnostics::inspect_body(path, body, first_line, |target, fragment| {
                            let (status, unchecked) = check_okf_body_target(
                                folder, &mut headings, path, &content, truncated, target, fragment,
                            );
                            partial |= unchecked;
                            status
                        })
                            .take(13usize.saturating_sub(findings.len()))
                    );
                }
                partial |= findings.len() > 12;
                if !findings.is_empty() && references.len() < 8 {
                    references.push(FolderReference { path: path.clone(), position: None });
                }
                for finding in findings.into_iter().take(12) {
                    if issues.len() >= 16 {
                        partial = true;
                        break;
                    }
                    issues.push(json!({
                        "path": excerpt(path, 120),
                        "field": finding.field,
                        "line": finding.line,
                        "code": finding.code,
                        "severity": finding.severity,
                        "target": finding.target.as_deref().map(|target| excerpt(target, 100)),
                    }));
                }
                if issues.len() >= 16 {
                    break;
                }
            }
            let next = files.next().map(|_| offset.saturating_add(examined));
            (json!({"issues":issues,"examined":examined,"next_offset":next,"partial":partial}), references)
        }
        "folder_list" => {
            let query = args["query"].as_str().unwrap_or_default().to_lowercase();
            let mut matches = folder
                .files
                .iter()
                .filter(|path| folder.matches(path, &query))
                .skip(offset);
            let files: Vec<_> = matches.by_ref().take(50).collect();
            let next = matches.next().map(|_| offset.saturating_add(files.len()));
            (
                json!({"files":files,"next_offset":next,"partial":folder.truncated || folder.skipped > 0}),
                Vec::new(),
            )
        }
        "folder_search" => search(folder, &args, offset)?,
        "folder_read" => {
            let target = required_string(&args, "path")?;
            let resolved;
            let relative = if let Some(from) = args["from"].as_str() {
                resolved = if skill.is_some_and(|name| skills::is_skill_file(name, from)) {
                    folder.resolve_file(from)?;
                    ibis_formats::okf::resolve(from, target).map(|(path, _)| path)
                        .ok_or("Skill reference must resolve inside the open folder.")?
                } else {
                    folder.reference_path(from, target)?
                };
                &resolved
            } else {
                target
            };
            let skill_reference = skill.is_some_and(|name| {
                skills::is_reference(name, relative)
            });
            let path = if skill_reference {
                folder.resolve_file(relative)?
            } else {
                folder.resolve(relative)?
            };
            let ext = extension(&path);
            let mut review = None;
            let result = if TEXT.contains(&ext.as_str()) {
                let limit = number(&args["limit"], 35)?.clamp(1, 50);
                let (content, partial) = read_text(&path)?;
                review = (ext == "md")
                    .then(|| bounded_frontmatter(content.as_bytes()))
                    .flatten()
                    .and_then(|frontmatter| ibis_formats::okf::concept(relative, &frontmatter).ok().flatten())
                    .map(|metadata| ibis_formats::okf::Review::from_metadata(&metadata));
                let mut lines = content.lines().enumerate().skip(offset);
                let page: Vec<_> = lines
                    .by_ref()
                    .take(limit)
                    .map(|(index, line)| {
                        json!({
                            "line": index + 1, "text": excerpt(line, 300)
                        })
                    })
                    .collect();
                let next = lines.next().map(|_| offset.saturating_add(page.len()));
                json!({"lines":page,"next_offset":next,"partial_file":partial})
            } else if OFFICE.contains(&ext.as_str()) {
                let operation = match args["operation"].as_str() {
                    Some(name) if OFFICE_READS.contains(&name) => name,
                    Some(_) => return Err("Unsupported read operation.".into()),
                    None => match ext.as_str() {
                        "docx" | "docm" | "dotx" => "docx_get_outline",
                        "xlsx" | "xlsm" => "xlsx_list_sheets",
                        _ => "pptx_list_slides",
                    },
                };
                let arguments = args["arguments"].as_object().cloned().unwrap_or_default();
                office_read(folder, &path, operation, arguments)?
            } else {
                return Err("This file format is available for opening in the editor only.".into());
            };
            let location = result["paragraphs"]
                .get(0)
                .filter(|item| item["para_id"].is_u64())
                .or_else(|| result["matches"].get(0))
                .unwrap_or(&result);
            let references = if skill_reference {
                Vec::new()
            } else {
                vec![FolderReference::new(relative, location)]
            };
            let mut value = json!({"path":relative,"content":result});
            if let Some(review) = review {
                value["okf_review"] = review.report();
            }
            (value, references)
        }
        name @ ("git_status" | "git_log" | "git_diff" | "git_read") => (
            crate::git_workspace::assistant::run(&folder.root, name, &args, offset)
                .map_err(|error| error.to_string())?,
            Vec::new(),
        ),
        _ => return Err("Unknown folder tool.".into()),
    };
    if !call.function.name.starts_with("git_") && value.to_string().len() > RESULT_BYTES {
        return Err("Result too large. Request a smaller range or fewer lines.".into());
    }
    Ok(FolderResult { value, references })
}

fn number(value: &Value, default: usize) -> Result<usize, String> {
    if value.is_null() {
        return Ok(default);
    }
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| "Expected a non-negative integer.".into())
}

fn required_string<'a>(args: &'a Value, field: &str) -> Result<&'a str, String> {
    args[field]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Provide a non-empty {field}."))
}

fn check_okf_body_target(
    folder: &Folder,
    headings: &mut BTreeMap<String, Option<ibis_formats::markdown::HeadingFragments>>,
    source_path: &str,
    source_text: &str,
    source_truncated: bool,
    target: &str,
    fragment: Option<&str>,
) -> (ibis_formats::okf::diagnostics::LinkTarget, bool) {
    use ibis_formats::okf::diagnostics::LinkTarget;

    let Ok(resolved) = folder.resolve_entry(target) else {
        return (LinkTarget::MissingFile, false);
    };
    let Some(fragment) = fragment.filter(|fragment| !fragment.is_empty()) else {
        return (LinkTarget::Present, false);
    };
    if !resolved.is_file() || !matches!(extension(&resolved).as_str(), "md" | "markdown") {
        return (LinkTarget::Present, false);
    }
    if !headings.contains_key(target) && headings.len() >= 32 {
        return (LinkTarget::Present, true);
    }
    let entry = headings.entry(target.into()).or_insert_with(|| {
        let owned;
        let text = if target == source_path {
            if source_truncated {
                return None;
            }
            source_text
        } else {
            let (content, truncated) = read_text(&resolved).ok()?;
            if truncated {
                return None;
            }
            owned = content;
            &owned
        };
        if ibis_formats::markdown::is_presentation_markdown(text.as_bytes()) {
            return None;
        }
        let document = ibis_formats::markdown::read_markdown(text.as_bytes()).ok()?;
        Some(ibis_formats::markdown::HeadingFragments::new(document.flow.as_deref()?))
    });
    match entry {
        Some(index) if index.resolve(fragment).is_none() => (LinkTarget::MissingFragment, false),
        Some(_) => (LinkTarget::Present, false),
        None => (LinkTarget::Present, true),
    }
}

fn excerpt(line: &str, limit: usize) -> String {
    match line.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.into(),
    }
}

fn read_frontmatter(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    bounded_frontmatter(file)
}

fn bounded_frontmatter(source: impl Read) -> Option<String> {
    frontmatter_prefix(source).filter(|prefix| {
        (prefix.starts_with("---\n") || prefix.starts_with("---\r\n"))
            && prefix.lines().skip(1).last() == Some("---")
    })
}

fn frontmatter_prefix(source: impl Read) -> Option<String> {
    let mut reader = BufReader::new(source.take(64 * 1024 + 1));
    let mut frontmatter = String::new();
    reader.read_line(&mut frontmatter).ok()?;
    if !matches!(frontmatter.as_str(), "---\n" | "---\r\n") {
        return Some(String::new());
    }
    loop {
        let start = frontmatter.len();
        if reader.read_line(&mut frontmatter).ok()? == 0 || frontmatter.len() > 64 * 1024 {
            frontmatter.truncate(start);
            break;
        }
        if matches!(&frontmatter[start..], "---\n" | "---\r\n" | "---") {
            break;
        }
    }
    Some(frontmatter)
}

fn read_text(path: &Path) -> Result<(String, bool), String> {
    let file = std::fs::File::open(path).map_err(|err| err.to_string())?;
    let mut bytes = Vec::new();
    file.take(TEXT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| err.to_string())?;
    let partial = bytes.len() > TEXT_BYTES;
    bytes.truncate(TEXT_BYTES);
    if partial
        && let Err(error) = std::str::from_utf8(&bytes)
        && error.error_len().is_none()
    {
        bytes.truncate(error.valid_up_to());
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| "Only UTF-8 text is supported for folder references.")?;
    Ok((content, partial))
}

fn open_office(folder: &Folder, path: &Path) -> Result<(ibis_doc_service::Service, Value), String> {
    let limits = ibis_doc_service::Limits {
        max_file_size: 10 * 1024 * 1024,
        page_bytes: 16 * 1024,
        ..Default::default()
    };
    let mut service =
        ibis_doc_service::Service::new(std::slice::from_ref(&folder.root), true, limits);
    let path = path.to_str().ok_or("The file path is not valid Unicode.")?;
    let info = service
        .call("app_open_document", &json!({"path":path}))
        .map_err(|err| err.message)?;
    Ok((service, info["document_id"].clone()))
}

fn search_slides(folder: &Folder, path: &Path, query: &str) -> Result<(Vec<Value>, bool), String> {
    const SLIDES_PER_FILE: usize = 40;
    let (mut service, id) = open_office(folder, path)?;
    let outline = service
        .call("pptx_list_slides", &json!({"document_id":id}))
        .map_err(|err| err.message)?;
    let slide_count = outline["slides"].as_array().map_or(0, Vec::len);
    let mut hits = Vec::new();
    for slide in 1..=slide_count.min(SLIDES_PER_FILE) {
        let page = service
            .call("pptx_get_slide", &json!({"document_id":id,"slide":slide}))
            .map_err(|err| err.message)?;
        if let Some(text) = page["shapes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|shape| shape["text"].as_str())
            .find(|text| text.to_lowercase().contains(query))
        {
            hits.push(json!({"slide":slide,"text":excerpt(text, 150)}));
            if hits.len() == 3 {
                break;
            }
        }
    }
    Ok((hits, slide_count > SLIDES_PER_FILE))
}

fn office_read(
    folder: &Folder,
    path: &Path,
    operation: &str,
    mut args: serde_json::Map<String, Value>,
) -> Result<Value, String> {
    if operation == "xlsx_get_cells" {
        let range = args
            .get("range")
            .and_then(Value::as_str)
            .and_then(parse_cell_range)
            .ok_or("Provide an A1 range.")?;
        let width = range.last_column - range.first_column + 1;
        if width > 200 {
            return Err("Choose a range narrower than 201 columns.".into());
        }
        let limit =
            number(args.get("limit").unwrap_or(&Value::Null), 20)?.clamp(1, 200 / width as usize);
        args.insert("limit".into(), json!(limit));
    }
    let (mut service, id) = open_office(folder, path)?;
    args.insert("document_id".into(), id.clone());
    let mut result = service
        .call(operation, &Value::Object(args))
        .map_err(|err| err.message)?;
    if operation == "docx_find_text"
        && let Some(matches) = result["matches"].as_array_mut()
    {
        for hit in matches.iter_mut().take(3) {
            if let Ok(paragraph) = service.call(
                "docx_get_paragraph",
                &json!({
                    "document_id":id, "paragraph":hit["paragraph"]
                }),
            ) {
                hit["para_id"] = paragraph["para_id"].clone();
                if let Some(text) = paragraph["text"].as_str() {
                    hit["text"] = json!(excerpt(text, 200));
                }
            }
        }
    }
    Ok(result)
}

fn search(
    folder: &Folder,
    args: &Value,
    offset: usize,
) -> Result<(Value, Vec<FolderReference>), String> {
    let query = required_string(args, "query")?.to_lowercase();
    let filter = args["path_filter"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    let mut candidates = folder
        .files
        .iter()
        .filter(|path| {
            let ext = extension(Path::new(path));
            path.to_lowercase().contains(&filter)
                && (TEXT.contains(&ext.as_str()) || OFFICE.contains(&ext.as_str()))
        })
        .enumerate()
        .skip(offset)
        .peekable();
    let mut hits = Vec::new();
    let mut references = Vec::new();
    let mut skipped = 0;
    for _ in 0..12 {
        let Some((_, relative)) = candidates.next() else {
            break;
        };
        let result = folder.resolve(relative).and_then(|path| {
            let ext = extension(&path);
            if let Some((matches, partial)) = folder.markdown_index.search(relative, &path, &query) {
                skipped += usize::from(partial);
                Ok(matches)
            } else if TEXT.contains(&ext.as_str()) {
                let (content, partial) = read_text(&path)?;
                skipped += usize::from(partial);
                let matches = content
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| line.to_lowercase().contains(&query))
                    .take(3)
                    .map(|(index, line)| json!({"line":index + 1,"text":excerpt(line, 150)}))
                    .collect();
                Ok(matches)
            } else if matches!(ext.as_str(), "pptx" | "pptm") {
                let (matches, partial) = search_slides(folder, &path, &query)?;
                skipped += usize::from(partial);
                Ok(matches)
            } else {
                let operation = if matches!(ext.as_str(), "xlsx" | "xlsm") {
                    "xlsx_find_text"
                } else {
                    "docx_find_text"
                };
                let args = json!({"query":query,"limit":3});
                let found =
                    office_read(folder, &path, operation, args.as_object().unwrap().clone())?;
                Ok(found["matches"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .cloned()
                    .map(|mut hit| {
                        if let Some(text) = hit["text"].as_str() {
                            hit["text"] = json!(excerpt(text, 150));
                        }
                        hit
                    })
                    .collect())
            }
        });
        match result {
            Ok(matches) if !matches.is_empty() => {
                hits.push(json!({"path":relative,"matches":matches}));
                references.push(FolderReference::new(relative, &matches[0]));
            }
            Ok(_) => {}
            Err(_) => skipped += 1,
        }
        if hits.len() == 6 {
            break;
        }
    }
    Ok((
        json!({"hits":hits,"next_offset":candidates.peek().map(|(n, _)| *n),"skipped_or_partial_files":skipped,
        "partial_folder":folder.truncated || folder.skipped > 0}),
        references,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_client::ToolCallFunction;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
            let fixture_id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("ibis-folder-{}-{nonce}-{fixture_id}", std::process::id()));
            std::fs::create_dir_all(path.join("資料")).unwrap();
            Self(path)
        }

        fn write(&self, path: &str, contents: impl AsRef<[u8]>) {
            std::fs::write(self.0.join(path), contents).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn call(folder: &Folder, name: &str, args: Value) -> Result<FolderResult, String> {
        run(
            folder,
            &ToolCall {
                id: "call".into(),
                kind: "function".into(),
                function: ToolCallFunction {
                    name: name.into(),
                    arguments: args.to_string(),
                },
            },
            None,
        )
    }

    #[test]
    fn agent_context_follows_only_the_open_folder_and_document_ancestors() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.0.join("資料/monthly")).unwrap();
        std::fs::create_dir_all(fixture.0.join("unrelated")).unwrap();
        fixture.write("AGENTS.md", "General report rules.");
        fixture.write("資料/AGENTS.md", "Use local definitions.");
        fixture.write("資料/monthly/AGENTS.md", "Use this month's cutoff.");
        fixture.write("unrelated/AGENTS.md", "Do not include this.");
        fixture.write("MEMORY.md", "Reconciliation is due on Friday.");
        let folder = Folder::scan(&fixture.0).unwrap();
        fixture.write("資料/monthly/report.xlsx", "fixture");
        let doc = fixture.0.join("資料/monthly/report.xlsx");
        let paths: Vec<_> = folder.agent_instructions(Some(&doc))
            .into_iter().map(|entry| entry["path"].as_str().unwrap().to_owned()).collect();
        assert_eq!(paths, ["AGENTS.md", "資料/AGENTS.md", "資料/monthly/AGENTS.md"]);
        assert_eq!(folder.agent_instructions(Some(Path::new("/elsewhere/report.xlsx"))).len(), 1);
        assert_eq!(folder.agent_memory().as_deref(), Some("Reconciliation is due on Friday."));
        fixture.write("MEMORY.md", "x".repeat(16 * 1024 + 1));
        assert!(folder.agent_memory().is_none());
    }

    #[test]
    fn skills_are_discovered_separately_and_references_require_selection() {
        let fixture = Fixture::new();
        let root = ".agents/skills/report-check";
        std::fs::create_dir_all(fixture.0.join(format!("{root}/references"))).unwrap();
        fixture.write(&format!("{root}/SKILL.md"), "---\nname: report-check\ndescription: Check a report\n---\nRead the checklist.");
        fixture.write(&format!("{root}/references/checklist.md"), "Check totals.");
        fixture.write("metric.md", "---\ntype: Metric\n---\n# Totals");
        let folder = Folder::scan(&fixture.0).unwrap();
        assert_eq!(folder.skills.len(), 1);
        assert_eq!(folder.load_skill("report-check").unwrap().instructions, "Read the checklist.");
        assert!(folder.load_skill("../report-check").is_err());
        assert!(folder.files.iter().all(|path| !path.starts_with('.')));

        let read = |skill, path: &str| {
            run(&folder, &ToolCall {
                id: "call".into(),
                kind: "function".into(),
                function: ToolCallFunction {
                    name: "folder_read".into(),
                    arguments: json!({"from":format!("{root}/SKILL.md"), "path":path}).to_string(),
                },
            }, skill)
        };
        assert!(read(None, "references/checklist.md").is_err());
        assert!(read(Some("other-skill"), "references/checklist.md").is_err());
        assert!(read(Some("report-check"), "../../../../outside.md").is_err());
        assert_eq!(read(Some("report-check"), "references/checklist.md")
            .unwrap().value["content"]["lines"][0]["text"], "Check totals.");
        assert_eq!(read(Some("report-check"), "../../../metric.md")
            .unwrap().value["content"]["lines"][3]["text"], "# Totals");
    }

    #[test]
    fn okf_type_filter_and_relative_read_share_the_open_folder() {
        let fixture = Fixture::new();
        fixture.write("資料/metric.md", "---\ntype: Metric\ntags: 'finance, ops'\n---\n# Metric\n");
        fixture.write("index.md", "---\ntype: Metric\ntags: [finance, ops]\n---\n# Index\n");
        fixture.write("README.md", "# Read me\n");
        fixture.write("ledger.md", "---\ntype: Source\ntags: [finance, ops]\n---\n# Ledger\n");
        let folder = Folder::scan(&fixture.0).unwrap();
        let list = call(&folder, "folder_list", json!({"query":"type:metric"})).unwrap();
        assert_eq!(list.value["files"], json!(["資料/metric.md"]));
        for (query, paths) in [
            ("tag:finance, ops", json!(["資料/metric.md"])),
            ("tag:FINANCE", json!(["ledger.md"])),
            ("tag:fin", json!([])),
        ] {
            let list = call(&folder, "folder_list", json!({"query": query})).unwrap();
            assert_eq!(list.value["files"], paths, "{query}");
        }
        let read = call(
            &folder,
            "folder_read",
            json!({"from":"資料/metric.md","path":"../README.md"}),
        )
        .unwrap();
        assert_eq!(read.references[0].path, "README.md");
        let source = folder.root.join("資料/metric.md");
        let reference = folder.link_from_file(&source, "../README.md#read-me").unwrap().unwrap();
        assert_eq!(reference.path, "README.md");
        assert!(matches!(reference.position, Some(ReferencePosition::Heading(fragment)) if fragment == "read-me"));
        assert!(folder.link_from_file(&source, "../../README.md").unwrap().is_err());
        assert!(folder.link_from_file(&source, "https://example.com/README.md").is_none());
        assert_eq!(folder.reference_path("index.md", "[[metric]]").unwrap(), "資料/metric.md");
        fixture.write("metric.md", "# Duplicate\n");
        let folder = Folder::scan(&fixture.0).unwrap();
        assert!(folder.reference_path("index.md", "[[metric]]").is_err());
        assert_eq!(folder.reference_path("index.md", "[[資料/metric]]").unwrap(), "資料/metric.md");
        let reference = folder.link_from_file(&source, "[[資料/metric#Details]]").unwrap().unwrap();
        assert_eq!(reference.path, "資料/metric.md");
        assert!(matches!(reference.position, Some(ReferencePosition::Heading(heading)) if heading == "Details"));
        assert!(folder.link_from_file(&source, "[[metric]]").unwrap().is_err());
        assert!(
            call(
                &folder,
                "folder_read",
                json!({"from":"資料/metric.md","path":"../../README.md"})
            )
            .is_err()
        );
    }

    #[test]
    fn folder_backlinks_resolve_markdown_and_wiki_links_from_the_index() {
        let fixture = Fixture::new();
        fixture.write("資料/共通要件.md", "# 認証\n");
        fixture.write("資料/予算.xlsx", "attachment");
        fixture.write("relative.md", r#"---
example: "[ignored](/資料/共通要件.md)"
---

# 手続

[共通要件](/資料/共通要件.md#認証) と [再掲](/資料/共通要件.md)。
"#);
        fixture.write("資料/wiki.markdown", "# 手順\n\n[[共通要件#認証|認証を見る]]\n\n[予算][sheet]\n\n[sheet]: 予算.xlsx\n");
        fixture.write("ignored.md", r#"`[[共通要件]]`

~~~markdown
[example](/資料/共通要件.md)
~~~

![[資料/共通要件]]
[external](https://example.org/資料/共通要件.md)
"#);
        let folder = Folder::scan(&fixture.0).unwrap();
        let backlinks = folder.markdown_index.backlinks("資料/共通要件.md");
        assert_eq!(backlinks.len(), 2);
        assert_eq!((backlinks[0].source.path.as_str(), backlinks[0].line), ("relative.md", 7));
        assert_eq!((backlinks[1].source.path.as_str(), backlinks[1].line), ("資料/wiki.markdown", 3));
        assert!(matches!(&backlinks[0].source.position, Some(ReferencePosition::Heading(heading)) if heading == "手続"));
        assert!(backlinks[0].excerpt.contains("共通要件"));
        assert_eq!(folder.markdown_index.backlinks("資料/予算.xlsx")[0].line, 5);
        assert!(!folder.markdown_index.partial);

        fixture.write("共通要件.md", "# Same name outside the subfolder\n");
        let refreshed = Folder::scan(&fixture.0).unwrap();
        assert_eq!(refreshed.markdown_index.backlinks("資料/共通要件.md").len(), 1);
        assert!(refreshed.markdown_index.backlinks("共通要件.md").is_empty());

        fixture.write("thematic.md", "---\n[Still visible](/資料/共通要件.md)\n---\n");
        let refreshed = Folder::scan(&fixture.0).unwrap();
        let backlinks = refreshed.markdown_index.backlinks("資料/共通要件.md");
        assert_eq!((backlinks[1].source.path.as_str(), backlinks[1].line), ("thematic.md", 2));
    }

    #[test]
    fn folder_okf_diagnostics_are_read_only_and_distinguish_missing_local_targets() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("nested")).unwrap();
        std::fs::create_dir(fixture.0.join("assets")).unwrap();
        let concept = "---\ntype: Metric\ntags: [finance, false]\nresource: /assets/\ncomputation: ../missing.md\nsuperseded_by: ../../outside.md\nsources: {resource: https://example.org/data}\n---\n\nRead [detail][source].\n\n[source]: ../body-absent.md\n";
        fixture.write("nested/metric.md", concept);
        fixture.write("assets/raw.bin", "asset");
        fixture.write("broken.md", "---\n[one, two]\n---\n");
        fixture.write("unterminated.md", "---\ntype: Metric\n");
        fixture.write("nested/log.md", "---\ntitle: Log\n---\n## 2025-01-01\n\n- Change\n");
        fixture.write("index.md", "---\nokf_version: 0.3\n---\n# Index\n\n- [Ordinary](ordinary.md) - Other notes\n");
        fixture.write("ordinary.md", "# Ordinary Markdown");
        let folder = Folder::scan(&fixture.0).unwrap();
        let result = call(&folder, "folder_diagnose_okf", json!({})).unwrap();
        assert_eq!(result.value["partial"], false);
        assert_eq!(result.value["next_offset"], Value::Null);
        let codes: Vec<_> = result.value["issues"].as_array().unwrap().iter()
            .map(|issue| issue["code"].as_str().unwrap()).collect();
        assert_eq!(codes, ["invalid_yaml_mapping", "invalid_version", "unexpected_reserved_frontmatter",
            "invalid_tags", "unresolved_local_target", "outside_or_invalid_path", "unresolved_local_target", "incomplete_frontmatter"]);
        let body_issue = &result.value["issues"][6];
        assert_eq!(body_issue["field"], "body.link");
        assert_eq!(body_issue["line"], 10);
        assert_eq!(body_issue["target"], "../body-absent.md");
        assert_eq!(std::fs::read_to_string(fixture.0.join("nested/metric.md")).unwrap(), concept);
    }

    #[test]
    fn okf_reserved_body_issues_include_file_line_and_severity() {
        let fixture = Fixture::new();
        fixture.write("entry.md", "Body");
        fixture.write("index.md", "---\nokf_version: \"0.3\"\n---\n# Entries\n\n- [Entry](entry.md)\n");
        fixture.write("log.md", "# History\n\n## 2025-02-29\n\n- Added entry\n");
        let folder = Folder::scan(&fixture.0).unwrap();
        let result = call(&folder, "folder_diagnose_okf", json!({})).unwrap();
        assert_eq!(
            result.references.iter().map(FolderReference::label).collect::<Vec<_>>(),
            ["index.md", "log.md"],
        );
        let line = call(&folder, "folder_read", json!({"path":result.references[0].path, "offset": 5, "limit": 1})).unwrap();
        assert_eq!(line.value["content"]["lines"][0]["text"], "- [Entry](entry.md)");
        assert_eq!(result.value["issues"], json!([
            {"path":"index.md","field":"body.index","line":6,"code":"index_entry_without_description","severity":"warning","target":null},
            {"path":"log.md","field":"body.log","line":3,"code":"log_invalid_date","severity":"error","target":null},
        ]));
    }

    #[test]
    fn okf_heading_diagnostics_match_desktop_bookmarks_and_slugs() {
        let fixture = Fixture::new();
        fixture.write("note.md", "# Local heading\n\n[Local](#local-heading)\n[Missing local](#gone)\n[Duplicate](target.md#topic-1)\n[Explicit](target.md#explicit)\n[Encoded](target.md#%E6%97%A5%E6%9C%AC)\n[Missing remote](target.md#gone)\n[Asset](raw.bin#anything)\n");
        fixture.write("target.md", "# Topic\n\n# Topic\n\n## 日本 {#explicit}\n\n```md\n# Gone\n```\n");
        fixture.write("raw.bin", "data");
        let folder = Folder::scan(&fixture.0).unwrap();
        let result = call(&folder, "folder_diagnose_okf", json!({})).unwrap();
        assert_eq!(result.value["partial"], false);
        let findings = result.value["issues"].as_array().unwrap();
        assert_eq!(findings.len(), 2, "{findings:?}");
        for (finding, line) in findings.iter().zip([4, 8]) {
            assert_eq!(finding["code"], "unresolved_heading_fragment");
            assert_eq!(finding["line"], line);
        }
    }

    #[test]
    fn folder_read_reports_current_okf_verification_from_the_file() {
        let fixture = Fixture::new();
        fixture.write("metric.md", "---\ntype: Metric\n---\n");
        let folder = Folder::scan(&fixture.0).unwrap();
        fixture.write("metric.md", "---\ntype: Metric\nverified: {by: 'human:reviewer', at: 2026-11-24}\nstale_after: invalid\n---\n");
        let result = call(&folder, "folder_read", json!({"path":"metric.md"})).unwrap();
        let review = &result.value["okf_review"];
        assert_eq!(review["trust"], "human-reviewed");
        assert_eq!(review["verified"]["by"], "human:reviewer");
        assert_eq!(review["freshness"], "malformed");
    }

    #[test]
    fn markdown_search_index_rebuilds_and_does_not_return_stale_lines() {
        let fixture = Fixture::new();
        fixture.write("資料/notes.md", "# Heading\nFirst version.\n");
        let folder = Folder::scan(&fixture.0).unwrap();
        let path = folder.resolve("資料/notes.md").unwrap();
        let (hits, partial) = folder.markdown_index.search("資料/notes.md", &path, "first").unwrap();
        assert!(!partial);
        assert_eq!(hits, [json!({"line":2,"text":"First version."})]);
        fixture.write("資料/notes.md", "# Heading\nA completely new version.\n");
        assert!(folder.markdown_index.search("資料/notes.md", &path, "first").is_none());
        let result = search(&folder, &json!({"query":"completely new"}), 0).unwrap();
        assert_eq!(result.0["hits"][0]["matches"][0]["text"], "A completely new version.");
        let refreshed = Folder::scan(&fixture.0).unwrap();
        let (hits, _) = refreshed.markdown_index.search("資料/notes.md", &path, "completely new").unwrap();
        assert_eq!(hits[0]["line"], 2);
    }

    #[test]
    fn folder_list_search_and_read_use_the_same_relative_paths() {
        let fixture = Fixture::new();
        fixture.write("index.md", "# 目次\n[概要](資料/概要.md)");
        fixture.write("資料/概要.md", "# 手順\n保守費は月額です。\n");
        fixture.write(".secret.md", "保守費");
        fixture.write("~$lock.docx", b"");
        let folder = Folder::scan(&fixture.0).unwrap();
        assert!(folder.has_index());
        let list = call(&folder, "folder_list", json!({})).unwrap();
        assert_eq!(list.value["files"], json!(["index.md", "資料/概要.md"]));
        let found = call(&folder, "folder_search", json!({"query":"保守費"})).unwrap();
        assert_eq!(found.value["hits"][0]["path"], "資料/概要.md");
        assert_eq!(found.value["hits"][0]["matches"][0]["line"], 2);
        assert_eq!(found.references[0].path, "資料/概要.md");
        let read = call(
            &folder,
            "folder_read",
            json!({"path":"資料/概要.md","offset":1,"limit":1}),
        )
        .unwrap();
        assert_eq!(
            read.value["content"]["lines"],
            json!([{"line":2,"text":"保守費は月額です。"}])
        );
        for path in ["../index.md", ".secret.md", "資料/../index.md", "/index.md"] {
            assert!(call(&folder, "folder_read", json!({"path":path})).is_err());
        }
    }

    #[test]
    fn folder_reads_office_documents_through_the_read_only_service() {
        let fixture = Fixture::new();
        fixture.write(
            "見積.xlsx",
            ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap(),
        );
        let folder = Folder::scan(&fixture.0).unwrap();
        let sheets = call(&folder, "folder_read", json!({"path":"見積.xlsx"})).unwrap();
        let sheet = sheets.value["content"]["sheets"][0]["name"]
            .as_str()
            .unwrap();
        let read = call(&folder, "folder_read", json!({
            "path":"見積.xlsx","operation":"xlsx_get_cells","arguments":{"sheet":sheet,"range":"A1:B2"},
        })).unwrap();
        assert_eq!(
            read.value["content"]["rows"],
            json!([[null, null], [null, null]])
        );
        assert_eq!(
            read.references[0].label(),
            format!("見積.xlsx · {sheet}!A1:B2")
        );
        assert!(
            call(
                &folder,
                "folder_read",
                json!({
                    "path":"見積.xlsx","operation":"xlsx_set_cells","arguments":{"sheet":sheet}
                })
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn folder_symlinks_cannot_add_or_replace_references() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        outside.write("notes.md", "secret");
        fixture.write("index.md", "# index");
        let folder = Folder::scan(&fixture.0).unwrap();
        std::fs::remove_file(fixture.0.join("index.md")).unwrap();
        std::os::unix::fs::symlink(outside.0.join("notes.md"), fixture.0.join("index.md")).unwrap();
        std::os::unix::fs::symlink(&outside.0, fixture.0.join("external")).unwrap();
        assert!(Folder::scan(&fixture.0).unwrap().files.is_empty());
        assert!(folder.resolve("index.md").is_err());
        assert!(folder.resolve("external/notes.md").is_err());
    }

    #[test]
    fn obsidian_vault_tags_aliases_and_nearest_wikilinks_work_without_okf_type() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.0.join(".obsidian")).unwrap();
        std::fs::create_dir_all(fixture.0.join("notes/sub")).unwrap();
        std::fs::create_dir_all(fixture.0.join("other")).unwrap();

        fixture.write(
            "notes/sub/guide.md",
            "---\ntags: [project/2026, '#Review']\naliases: [導入ガイド]\n---\n\n# ガイド\n\n本文に #議事録 と `#ignored` と #123 がある。\n",
        );
        fixture.write("notes/sub/readme.md", "# Local readme\n");
        fixture.write("other/readme.md", "# Other readme\n");
        fixture.write(
            "notes/sub/hub.md",
            "[[導入ガイド]] と [[GUIDE]] と [[readme]] を参照。\n",
        );

        let folder = Folder::scan(&fixture.0).unwrap();
        assert!(folder.is_obsidian_vault);
        assert!(is_obsidian_vault(&fixture.0));

        // Hierarchical, frontmatter, and inline tags work on plain Markdown notes.
        for (query, expected) in [
            ("tag:project", json!(["notes/sub/guide.md"])),
            ("tag:project/2026", json!(["notes/sub/guide.md"])),
            ("tag:#review", json!(["notes/sub/guide.md"])),
            ("tag:議事録", json!(["notes/sub/guide.md"])),
            ("tag:ignored", json!([])),
            ("tag:123", json!([])),
            ("tag:proj", json!([])),
        ] {
            let list = call(&folder, "folder_list", json!({"query": query})).unwrap();
            assert_eq!(list.value["files"], expected, "{query}");
        }

        // Alias, case-insensitive, and nearest-directory Wiki link resolution.
        assert_eq!(
            folder.reference_path("notes/sub/hub.md", "[[導入ガイド]]").unwrap(),
            "notes/sub/guide.md"
        );
        assert_eq!(
            folder.reference_path("notes/sub/hub.md", "[[GUIDE]]").unwrap(),
            "notes/sub/guide.md"
        );
        assert_eq!(
            folder.reference_path("notes/sub/hub.md", "[[readme]]").unwrap(),
            "notes/sub/readme.md"
        );
        assert_eq!(
            folder.markdown_index.backlinks("notes/sub/guide.md").len(),
            1
        );
        assert_eq!(
            folder.markdown_index.backlinks("notes/sub/readme.md").len(),
            1
        );
    }

    #[test]
    fn turso_vault_index_scales_past_256_files_and_reuses_cache() {
        let fixture = Fixture::new();
        fixture.write("target.md", "# Target\n");
        for i in 0..270 {
            let body = if i == 0 || i == 269 {
                format!("# Note {i}\n\nSee [[target]] #batch/item.\n")
            } else {
                format!("# Note {i}\n\nPlain body.\n")
            };
            fixture.write(&format!("n{i:03}.md"), body);
        }

        let first = Folder::scan(&fixture.0).unwrap();
        assert!(!first.markdown_index.partial);
        assert_eq!(first.markdown_index.backlinks("target.md").len(), 2);
        let tagged = call(&first, "folder_list", json!({"query": "tag:batch"})).unwrap();
        assert_eq!(tagged.value["files"], json!(["n000.md", "n269.md"]));

        // Second scan with no changes should hit the Turso cache for all 271 Markdown files.
        let second = Folder::scan(&fixture.0).unwrap();
        assert_eq!(second.markdown_index.cache_hits, 271);
        assert_eq!(second.markdown_index.backlinks("target.md").len(), 2);

        // Updating one file invalidates only that file and refreshes backlinks.
        fixture.write("n100.md", "# Note 100\n\nNow links to [[target]].\n");
        let third = Folder::scan(&fixture.0).unwrap();
        assert_eq!(third.markdown_index.cache_hits, 270);
        assert_eq!(third.markdown_index.backlinks("target.md").len(), 3);
    }

    #[test]
    fn folder_tree_rows_group_directories_first_and_respect_collapsed_state() {
        let files: Vec<String> = [
            "README.md",
            "docs/api.md",
            "docs/sub/detail.md",
            "index.md",
            "notes/todo.md",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();

        let expanded = ui::build_folder_tree_rows(&files, &std::collections::BTreeSet::new());
        let names: Vec<(&str, usize)> = expanded
            .iter()
            .map(|row| match row {
                ui::TreeRow::Dir { path, depth, .. } => (path.as_str(), *depth),
                ui::TreeRow::File { path, depth, .. } => (path.as_str(), *depth),
            })
            .collect();
        assert_eq!(
            names,
            [
                ("index.md", 0),
                ("docs", 0),
                ("docs/sub", 1),
                ("docs/sub/detail.md", 2),
                ("docs/api.md", 1),
                ("notes", 0),
                ("notes/todo.md", 1),
                ("README.md", 0),
            ]
        );

        let mut collapsed = std::collections::BTreeSet::new();
        collapsed.insert("docs".to_owned());
        let folded = ui::build_folder_tree_rows(&files, &collapsed);
        assert!(folded.iter().all(|row| match row {
            ui::TreeRow::Dir { path, .. } => path != "docs/sub",
            ui::TreeRow::File { path, .. } => !path.starts_with("docs/"),
        }));
    }

    #[test]
    fn folder_content_search_and_obsidian_wikilink_creation_enforce_vault_boundary() {
        let fixture = Fixture::new();
        fixture.write("資料/guide.md", "# ガイド\n\n特別なキーワードがここにあります。\n");
        let plain = Folder::scan(&fixture.0).unwrap();
        let hits = plain.search_markdown_content("特別なキーワード", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].path.as_str(), hits[0].line), ("資料/guide.md", 3));
        // Non-Obsidian folders do not auto-create missing Wiki link targets.
        let source = plain.root.join("資料/guide.md");
        assert_eq!(plain.create_wikilink_note(&source, "[[新しいノート]]").unwrap(), None);

        std::fs::create_dir_all(fixture.0.join(".obsidian")).unwrap();
        let vault = Folder::scan(&fixture.0).unwrap();
        let created = vault
            .create_wikilink_note(&source, "[[新しいノート|表示名]]")
            .unwrap();
        assert_eq!(created.as_deref(), Some("資料/新しいノート.md"));
        assert_eq!(
            std::fs::read_to_string(fixture.0.join("資料/新しいノート.md")).unwrap(),
            "# 新しいノート\n"
        );
        let nested = vault
            .create_wikilink_note(&source, "[[drafts/idea#見出し]]")
            .unwrap();
        assert_eq!(nested.as_deref(), Some("drafts/idea.md"));
        assert!(fixture.0.join("drafts/idea.md").is_file());
        // Path traversal and hidden paths are refused.
        assert_eq!(vault.create_wikilink_note(&source, "[[../outside]]").unwrap(), None);
        assert_eq!(vault.create_wikilink_note(&source, "[[.hidden]]").unwrap(), None);
    }

    #[test]
    fn vault_db_v2_indexes_and_caches_profiles_form_mappings_and_credentials() {
        let fixture = Fixture::new();
        for dir in ["profiles", "forms", "credentials", "requests"] {
            std::fs::create_dir_all(fixture.0.join(dir)).unwrap();
        }
        let (_, company_md) = ibis_formats::form_assist::starter_profile_markdown("organization");
        fixture.write("profiles/company.md", company_md);
        fixture.write(
            "forms/invoice.form-map.md",
            "---\ntype: form-mapping\ntitle: テスト請求書\nx-sorane-form-mapping:\n  version: 1\n  form_id: invoice-test\n  format: xlsx\n  match:\n    sheet: 請求書\n  fields:\n    - id: org\n      label: 会社名\n      target: { cell: B3 }\n      sources:\n        - any: organization.name\n---\n",
        );
        let (req_path, req_md) =
            ibis_formats::form_assist::starter_presentation_request_markdown();
        fixture.write(req_path, req_md);
        fixture.write(
            "credentials/training.sd-jwt",
            include_str!("../../../test/fixtures/structured-preview/training.sd-jwt"),
        );

        let first = Folder::scan(&fixture.0).unwrap();
        assert_eq!(first.vault_profiles().len(), 1);
        assert_eq!(first.vault_profiles()[0].scope, "organization");
        assert_eq!(first.vault_form_mappings().len(), 1);
        assert_eq!(first.vault_form_mappings()[0].form_id, "invoice-test");
        assert_eq!(first.vault_presentation_requests().len(), 1);
        assert_eq!(
            first.vault_presentation_requests()[0].request_id,
            "sample-verifier"
        );
        assert_eq!(first.vault_credentials().len(), 1);
        assert_eq!(first.vault_credentials()[0].format, "sd-jwt");
        assert!(
            first.vault_credentials()[0]
                .claims
                .contains_key("研修名")
        );

        // Second scan hits SQLite v3 cache and preserves all indexed profiles, mappings, requests, and credentials.
        let second = Folder::scan(&fixture.0).unwrap();
        assert!(second.markdown_index.cache_hits >= 3);
        assert_eq!(second.vault_profiles(), first.vault_profiles());
        assert_eq!(second.vault_form_mappings(), first.vault_form_mappings());
        assert_eq!(
            second.vault_presentation_requests(),
            first.vault_presentation_requests()
        );
        assert_eq!(second.vault_credentials(), first.vault_credentials());
    }
}
