// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::SystemTime;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use serde_json::{Value, json};

use super::{Folder, FolderReference, ReferencePosition, excerpt, extension, read_text, vault_db};

const SEARCH_CACHE_BYTES: usize = 8 * 1024 * 1024;
const SEARCH_CACHE_FILES: usize = 256;
const METADATA_LIMIT: usize = super::SCAN_LIMIT;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Stamp {
    pub(super) size: u64,
    pub(super) modified: SystemTime,
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
pub(super) struct RawLink {
    pub(super) target_raw: String,
    pub(super) is_wiki: bool,
    pub(super) heading: Option<String>,
    pub(super) line: usize,
    pub(super) excerpt: String,
}

use ibis_formats::form_assist::{
    FormMapping, PresentationRequestNote, VaultCredentialSummary, VaultProfile,
    parse_credential_bytes, parse_form_mapping_markdown, parse_presentation_request_markdown,
    parse_profile_markdown,
};

#[derive(Clone, Debug)]
pub(super) struct CachedFile {
    pub(super) stamp: Stamp,
    pub(super) partial: bool,
    pub(super) tags: Vec<String>,
    pub(super) aliases: Vec<String>,
    pub(super) links: Vec<RawLink>,
    pub(super) profile: Option<VaultProfile>,
    pub(super) form_mapping: Option<FormMapping>,
    pub(super) presentation_request: Option<PresentationRequestNote>,
}

#[derive(Clone, Debug)]
pub(super) struct CachedCredential {
    pub(super) stamp: Stamp,
    pub(super) summary: Option<VaultCredentialSummary>,
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
    tags: BTreeMap<String, Vec<String>>,
    aliases: BTreeMap<String, Vec<String>>,
    profiles: Vec<VaultProfile>,
    form_mappings: Vec<FormMapping>,
    presentation_requests: Vec<PresentationRequestNote>,
    credentials: Vec<VaultCredentialSummary>,
    pub(super) partial: bool,
    #[cfg(test)]
    pub(super) cache_hits: usize,
}

impl MarkdownIndex {
    pub(super) fn build(folder: &Folder) -> Self {
        let mut cached = vault_db::load_all(&folder.root).unwrap_or_default();
        let mut remaining_search_bytes = SEARCH_CACHE_BYTES;
        let mut search_files = 0usize;
        let mut entries = BTreeMap::new();
        let mut current_files = BTreeMap::<String, CachedFile>::new();
        let mut updated_paths = Vec::<String>::new();
        let mut incomplete = false;
        #[cfg(test)]
        let mut cache_hits = 0usize;

        let mut markdown = folder
            .files
            .iter()
            .filter(|path| matches!(extension(Path::new(path)).as_str(), "md" | "markdown"));

        for relative in markdown.by_ref().take(METADATA_LIMIT) {
            let Ok(path) = folder.resolve(relative) else {
                incomplete = true;
                continue;
            };
            let Some(stamp) = Stamp::read(&path) else {
                incomplete = true;
                continue;
            };

            let maybe_cached = cached.remove(relative).filter(|entry| entry.stamp == stamp);
            if let Some(entry) = maybe_cached {
                #[cfg(test)]
                {
                    cache_hits += 1;
                }
                incomplete |= entry.partial;
                if search_files < SEARCH_CACHE_FILES && remaining_search_bytes > 0 {
                    if let Ok((content, partial)) = read_text(&path)
                        && Stamp::read(&path) == Some(stamp)
                    {
                        let normalized = content.to_lowercase();
                        if let Some(budget) =
                            remaining_search_bytes.checked_sub(content.len() + normalized.len())
                        {
                            remaining_search_bytes = budget;
                            search_files += 1;
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
                    }
                }
                current_files.insert(relative.clone(), entry);
                continue;
            }

            let Ok((content, partial)) = read_text(&path) else {
                incomplete = true;
                continue;
            };
            if Stamp::read(&path) != Some(stamp) {
                incomplete = true;
                continue;
            }
            incomplete |= partial;

            let (tags, aliases, links) = extract_markdown_metadata(relative, &content);
            let profile = parse_profile_markdown(relative, &content);
            let form_mapping = parse_form_mapping_markdown(relative, &content);
            let presentation_request = parse_presentation_request_markdown(relative, &content);
            current_files.insert(
                relative.clone(),
                CachedFile {
                    stamp,
                    partial,
                    tags,
                    aliases,
                    links,
                    profile,
                    form_mapping,
                    presentation_request,
                },
            );
            updated_paths.push(relative.clone());

            if search_files < SEARCH_CACHE_FILES && remaining_search_bytes > 0 {
                let normalized = content.to_lowercase();
                if let Some(budget) =
                    remaining_search_bytes.checked_sub(content.len() + normalized.len())
                {
                    remaining_search_bytes = budget;
                    search_files += 1;
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
            }
        }

        let removed: Vec<String> = cached.into_keys().collect();
        let updated: Vec<(String, &CachedFile)> = updated_paths
            .iter()
            .filter_map(|path| current_files.get(path).map(|entry| (path.clone(), entry)))
            .collect();
        if let Err(err) = vault_db::sync_diff(&folder.root, &removed, &updated) {
            log::warn!("Failed to persist vault index cache: {err}");
        }

        // Incrementally index `.sd-jwt` and `.mdoc` credentials inside the Vault.
        let mut cached_creds = vault_db::load_credentials(&folder.root).unwrap_or_default();
        let mut current_creds = BTreeMap::<String, CachedCredential>::new();
        let mut updated_cred_paths = Vec::<String>::new();
        for relative in folder
            .files
            .iter()
            .filter(|path| matches!(extension(Path::new(path)).as_str(), "sd-jwt" | "mdoc"))
            .take(512)
        {
            let Ok(path) = folder.resolve(relative) else {
                continue;
            };
            let Some(stamp) = Stamp::read(&path) else {
                continue;
            };
            if let Some(entry) = cached_creds
                .remove(relative)
                .filter(|entry| entry.stamp == stamp)
            {
                current_creds.insert(relative.clone(), entry);
                continue;
            }
            let summary = if stamp.size <= 1024 * 1024 {
                std::fs::read(&path)
                    .ok()
                    .and_then(|bytes| parse_credential_bytes(relative, &extension(&path), &bytes))
            } else {
                None
            };
            current_creds.insert(relative.clone(), CachedCredential { stamp, summary });
            updated_cred_paths.push(relative.clone());
        }
        let removed_creds: Vec<String> = cached_creds.into_keys().collect();
        let updated_creds: Vec<(String, &CachedCredential)> = updated_cred_paths
            .iter()
            .filter_map(|path| current_creds.get(path).map(|entry| (path.clone(), entry)))
            .collect();
        if let Err(err) =
            vault_db::sync_credentials_diff(&folder.root, &removed_creds, &updated_creds)
        {
            log::warn!("Failed to persist vault credentials cache: {err}");
        }

        let mut tags = BTreeMap::<String, Vec<String>>::new();
        let mut aliases = BTreeMap::<String, Vec<String>>::new();
        let mut profiles = Vec::new();
        let mut form_mappings = Vec::new();
        let mut presentation_requests = Vec::new();
        for (path, entry) in &current_files {
            if !entry.tags.is_empty() {
                tags.insert(path.clone(), entry.tags.clone());
            }
            for alias in &entry.aliases {
                aliases.entry(alias.clone()).or_default().push(path.clone());
            }
            if let Some(profile) = &entry.profile {
                profiles.push(profile.clone());
            }
            if let Some(mapping) = &entry.form_mapping {
                form_mappings.push(mapping.clone());
            }
            if let Some(req) = &entry.presentation_request {
                presentation_requests.push(req.clone());
            }
        }
        let credentials: Vec<VaultCredentialSummary> = current_creds
            .into_values()
            .filter_map(|entry| entry.summary)
            .collect();

        let mut backlinks = BTreeMap::<String, Vec<Backlink>>::new();
        for (source, entry) in &current_files {
            for link in &entry.links {
                let reference = if link.is_wiki {
                    format!("[[{}]]", link.target_raw)
                } else {
                    link.target_raw.clone()
                };
                let Ok(target) =
                    folder.indexed_reference_path_with_aliases(source, &reference, Some(&aliases))
                else {
                    continue;
                };
                if target == *source || folder.files.binary_search(&target).is_err() {
                    continue;
                }
                let references = backlinks.entry(target).or_default();
                if references
                    .last()
                    .is_some_and(|existing| existing.source.path == *source)
                {
                    continue;
                }
                references.push(Backlink {
                    source: FolderReference {
                        path: source.clone(),
                        position: link
                            .heading
                            .as_ref()
                            .map(|title| ReferencePosition::Heading(title.clone())),
                    },
                    line: link.line,
                    excerpt: link.excerpt.clone(),
                });
            }
        }

        Self {
            entries,
            backlinks,
            tags,
            aliases,
            profiles,
            form_mappings,
            presentation_requests,
            credentials,
            partial: incomplete
                || markdown.next().is_some()
                || folder.truncated
                || folder.skipped > 0,
            #[cfg(test)]
            cache_hits,
        }
    }

    pub(super) fn backlinks(&self, relative: &str) -> &[Backlink] {
        self.backlinks
            .get(relative)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn outgoing_links(&self, relative: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .backlinks
            .iter()
            .filter_map(|(target, sources)| {
                sources
                    .iter()
                    .any(|b| b.source.path == relative)
                    .then(|| target.clone())
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub(super) fn tags(&self, relative: &str) -> &[String] {
        self.tags
            .get(relative)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn all_tags_map(&self) -> &BTreeMap<String, Vec<String>> {
        &self.tags
    }

    pub(super) fn aliases(&self) -> &BTreeMap<String, Vec<String>> {
        &self.aliases
    }

    pub(super) fn profiles(&self) -> &[VaultProfile] {
        &self.profiles
    }

    pub(super) fn form_mappings(&self) -> &[FormMapping] {
        &self.form_mappings
    }

    pub(super) fn presentation_requests(&self) -> &[PresentationRequestNote] {
        &self.presentation_requests
    }

    pub(super) fn credentials(&self) -> &[VaultCredentialSummary] {
        &self.credentials
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

fn normalize_tag(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('#').trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_lowercase())
}

fn push_yaml_tags(value: Option<&Value>, out: &mut BTreeSet<String>) {
    match value {
        Some(Value::String(tag)) => {
            if let Some(normalized) = normalize_tag(tag) {
                out.insert(normalized);
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(tag) = item.as_str().and_then(normalize_tag) {
                    out.insert(tag);
                }
            }
        }
        _ => {}
    }
}

fn push_yaml_aliases(value: Option<&Value>, out: &mut BTreeSet<String>) {
    match value {
        Some(Value::String(alias)) => {
            let trimmed = alias.trim();
            if !trimmed.is_empty() {
                out.insert(trimmed.to_lowercase());
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(alias) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    out.insert(alias.to_lowercase());
                }
            }
        }
        _ => {}
    }
}

fn extract_inline_tags(text: &str, out: &mut BTreeSet<String>) {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (byte_idx, ch) = chars[i];
        if ch == '#' {
            let valid_before = i == 0 || {
                let prev = chars[i - 1].1;
                !prev.is_alphanumeric() && !matches!(prev, '_' | '-' | '/' | '#')
            };
            if valid_before {
                let start = byte_idx + 1;
                let mut j = i + 1;
                while j < chars.len() {
                    let c = chars[j].1;
                    if c.is_alphanumeric() || matches!(c, '_' | '-' | '/') {
                        j += 1;
                    } else {
                        break;
                    }
                }
                let end = chars.get(j).map_or(text.len(), |(idx, _)| *idx);
                let raw = text[start..end].trim_matches('/');
                if !raw.is_empty() && !raw.chars().all(|c| c.is_ascii_digit()) {
                    out.insert(raw.to_lowercase());
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
}

fn extract_markdown_metadata(relative: &str, content: &str) -> (Vec<String>, Vec<String>, Vec<RawLink>) {
    let mut tags = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut skip_reserved_okf_tags = false;

    if let Ok(Some(frontmatter)) = ibis_formats::okf::frontmatter_map(content) {
        let is_reserved_name = matches!(relative.rsplit('/').next(), Some("index.md" | "log.md"));
        let has_okf_type = frontmatter
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|v| !v.trim().is_empty());
        skip_reserved_okf_tags = is_reserved_name && has_okf_type;
        if !skip_reserved_okf_tags {
            push_yaml_tags(frontmatter.get("tags"), &mut tags);
            push_yaml_tags(frontmatter.get("tag"), &mut tags);
        }
        push_yaml_aliases(frontmatter.get("aliases"), &mut aliases);
        push_yaml_aliases(frontmatter.get("alias"), &mut aliases);
    }

    let options = Options::ENABLE_WIKILINKS | Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES;
    let body = ibis_formats::markdown::body_without_frontmatter(content);
    let body_offset = content.len() - body.len();
    let newlines: Vec<_> = content
        .match_indices('\n')
        .map(|(offset, _)| offset)
        .collect();
    let mut heading = None;
    let mut pending_heading = None::<String>;
    let mut image_depth = 0usize;
    let mut link_depth = 0usize;
    let mut code_block_depth = 0usize;
    let mut links = Vec::new();

    for (event, range) in Parser::new_ext(body, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. }) => pending_heading = Some(String::new()),
            Event::End(TagEnd::Heading(_)) => {
                heading = pending_heading
                    .take()
                    .filter(|text| !text.trim().is_empty());
            }
            Event::Start(Tag::CodeBlock(_)) => code_block_depth += 1,
            Event::End(TagEnd::CodeBlock) => code_block_depth = code_block_depth.saturating_sub(1),
            Event::Text(text) => {
                if let Some(heading) = pending_heading.as_mut() {
                    heading.push_str(&text);
                }
                if !skip_reserved_okf_tags
                    && image_depth == 0
                    && link_depth == 0
                    && code_block_depth == 0
                {
                    extract_inline_tags(&text, &mut tags);
                }
            }
            Event::Code(text) => {
                if let Some(heading) = pending_heading.as_mut() {
                    heading.push_str(&text);
                }
            }
            Event::Start(Tag::Image { .. }) => image_depth += 1,
            Event::End(TagEnd::Image) => image_depth = image_depth.saturating_sub(1),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                link_depth += 1;
                if image_depth > 0 {
                    continue;
                }
                let is_wiki = match link_type {
                    LinkType::WikiLink { .. } => true,
                    LinkType::Autolink | LinkType::Email => continue,
                    _ => false,
                };
                let line = newlines.partition_point(|offset| *offset < body_offset + range.start);
                let start = line.checked_sub(1).map_or(0, |index| newlines[index] + 1);
                let end = newlines.get(line).copied().unwrap_or(content.len());
                links.push(RawLink {
                    target_raw: dest_url.into_string(),
                    is_wiki,
                    heading: heading.clone(),
                    line: line + 1,
                    excerpt: excerpt(content[start..end].trim(), 120),
                });
            }
            Event::End(TagEnd::Link) => link_depth = link_depth.saturating_sub(1),
            _ => {}
        }
    }

    (
        tags.into_iter().collect(),
        aliases.into_iter().collect(),
        links,
    )
}
