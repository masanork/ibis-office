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
pub(crate) use ui::FolderPane;

const SCAN_LIMIT: usize = 10_000;
const TEXT_BYTES: usize = 512 * 1024;
const RESULT_BYTES: usize = 24 * 1024;
pub(crate) const OFFICE: &[&str] = &["docx", "docm", "dotx", "xlsx", "xlsm", "pptx", "pptm"];
const TEXT: &[&str] = &["md", "markdown", "txt", "csv", "tsv", "json", "jsonl", "ndjson", "yaml", "yml"];

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

pub(crate) fn visible(name: &str) -> bool {
    !name.starts_with('.') && !name.starts_with("~$")
}

#[derive(Clone)]
struct ConceptFilter {
    kind: String,
    tags: Vec<String>,
    review: ibis_formats::okf::Review,
}

#[derive(Clone)]
pub(crate) struct Folder {
    root: PathBuf,
    files: Vec<String>,
    concepts: BTreeMap<String, ConceptFilter>,
    markdown_index: markdown_index::MarkdownIndex,
    pub(crate) skills: Vec<ibis_formats::agent_skill::Metadata>,
    truncated: bool,
    skipped: usize,
}

impl Folder {
    fn scan(path: &Path) -> Result<Self, String> {
        let root = path.canonicalize().map_err(|err| err.to_string())?;
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
        if !real.starts_with(&self.root) || !real.is_file() {
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
        if let Some(wiki) = reference.strip_prefix("[[").and_then(|text| text.strip_suffix("]]")) {
            let target = wiki.split(['|', '#']).next().unwrap_or("").trim();
            if target.is_empty() {
                return Ok(from.to_owned());
            }
            let matches_path = |path: &str| path == target || path.strip_suffix(".md") == Some(target);
            if target.contains('/') {
                return self.files.iter().find(|path| matches_path(path)).cloned()
                    .ok_or_else(|| "Wiki link target was not found in the open folder.".into());
            }
            let mut candidates = self.files.iter().filter(|path| {
                path.rsplit('/').next().is_some_and(matches_path)
            });
            let first = candidates.next().ok_or("Wiki link target was not found in the open folder.")?;
            if candidates.next().is_some() || self.truncated || self.skipped > 0 {
                return Err("Wiki link is ambiguous or the folder listing is incomplete. Use a root-relative path.".into());
            }
            return Ok(first.clone());
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
            self.concepts
                .get(path)
                .is_some_and(|concept| concept.tags.iter().any(|actual| actual == tag.trim()))
        } else {
            path.to_lowercase().contains(query)
        }
    }

    pub(crate) fn has_index(&self) -> bool {
        self.files.iter().any(|path| path == "index.md")
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

pub(crate) fn tools() -> [ToolDefinition; 7] {
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
    matches!(name, "folder_list" | "folder_search" | "folder_read" | "git_status" | "git_log" | "git_diff" | "git_read")
}

pub(crate) fn run(folder: &Folder, call: &ToolCall, skill: Option<&str>) -> Result<FolderResult, String> {
    let args: Value =
        serde_json::from_str(&call.function.arguments).map_err(|err| err.to_string())?;
    let offset = number(&args["offset"], 0)?;
    let (value, references) = match call.function.name.as_str() {
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
    let mut reader = BufReader::new(source.take(64 * 1024 + 1));
    let mut frontmatter = String::new();
    reader.read_line(&mut frontmatter).ok()?;
    if !matches!(frontmatter.as_str(), "---\n" | "---\r\n") {
        return None;
    }
    loop {
        let start = frontmatter.len();
        if reader.read_line(&mut frontmatter).ok()? == 0 || frontmatter.len() > 64 * 1024 {
            return None;
        }
        if matches!(&frontmatter[start..], "---\n" | "---\r\n" | "---") {
            return Some(frontmatter);
        }
    }
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
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("ibis-folder-{}-{nonce}", std::process::id()));
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
}
