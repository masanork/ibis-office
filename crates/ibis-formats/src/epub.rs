// SPDX-License-Identifier: AGPL-3.0-only
//! Read reflowable EPUB 3 into the editable document model through Markdown.

use std::collections::HashMap;
use std::fmt::Write;

use base64::{Engine, engine::general_purpose::STANDARD};
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use roxmltree::{Document, Node, NodeId, ParsingOptions};

use crate::{Error, Opened, zip::Archive};

const EPUB: &str = "http://www.idpf.org/2007/ops";
const MAX_XML: usize = 8 * 1024 * 1024;
const MAX_TOTAL: usize = 64 * 1024 * 1024;

pub fn is_epub(data: &[u8]) -> bool {
    // OCF requires the stored mimetype to be first, with no local-header extra field.
    data.starts_with(b"PK\x03\x04") && data.get(30..58) == Some(b"mimetypeapplication/epub+zip")
}

pub fn open(data: &[u8]) -> Result<Opened, Error> {
    // The editable source is DOCX, so Save As cannot overwrite the EPUB.
    crate::open(to_docx(data)?)
}

pub fn to_docx(data: &[u8]) -> Result<Vec<u8>, Error> {
    crate::markdown::import_markdown_to_docx_bytes(to_markdown(data)?.as_bytes())
}

fn xml(text: &str) -> Result<Document<'_>, Error> {
    Document::parse_with_options(
        text.trim_start_matches('\u{feff}'),
        ParsingOptions {
            nodes_limit: 250_000,
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|error| Error::new(format!("Invalid EPUB XML: {error}")))
}

fn part(zip: &Archive<'_>, name: &str, remaining: &mut usize) -> Result<String, Error> {
    let bytes = zip
        .read_bounded(name, MAX_XML.min(*remaining))?
        .ok_or_else(|| Error::new(format!("Missing EPUB part {name}")))?;
    *remaining -= bytes.len();
    String::from_utf8(bytes).map_err(|_| Error::new("EPUB XML must be UTF-8"))
}

fn local_target(base: &str, href: &str) -> Option<(String, String)> {
    let (path, fragment) = href.split_once('#').unwrap_or((href, ""));
    let path = percent_decode_str(path).decode_utf8().ok()?;
    let fragment = percent_decode_str(fragment)
        .decode_utf8()
        .ok()?
        .into_owned();
    if path.is_empty() {
        return Some((base.to_owned(), fragment));
    }
    if path.starts_with('/') || path.contains([':', '\\', '?', '\0']) {
        return None;
    }
    let mut parts: Vec<_> = base
        .rsplit_once('/')
        .map_or(Vec::new(), |(dir, _)| dir.split('/').collect());
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(component),
        }
    }
    Some((parts.join("/"), fragment))
}

fn named(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name
}

fn epub_type(node: Node<'_, '_>, kind: &str) -> bool {
    node.attribute((EPUB, "type"))
        .is_some_and(|types| types.split_ascii_whitespace().any(|t| t == kind))
}

fn note(node: Node<'_, '_>) -> bool {
    epub_type(node, "footnote")
        || epub_type(node, "endnote")
        || matches!(node.attribute("role"), Some("doc-footnote" | "doc-endnote"))
}

fn plain(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect::<String>()
}

fn writing_mode(declarations: &str) -> Option<bool> {
    declarations
        .split(';')
        .filter_map(|decl| {
            let (name, value) = decl.split_once(':')?;
            if !matches!(
                name.trim(),
                "writing-mode" | "-epub-writing-mode" | "-webkit-writing-mode"
            ) {
                return None;
            }
            match value.split('!').next()?.trim() {
                "vertical-rl" | "tb-rl" => Some(true),
                "horizontal-tb" | "vertical-lr" => Some(false),
                _ => None,
            }
        })
        .last()
}

// Recognize explicit root writing mode without applying a general CSS cascade.
fn root_css_mode(mut css: &str) -> Option<bool> {
    let mut clean = String::new();
    while let Some((before, comment)) = css.split_once("/*") {
        clean.push_str(before);
        css = comment.split_once("*/")?.1;
    }
    clean.push_str(css);
    if clean.contains("@media") || clean.contains("@supports") || clean.contains("@import") {
        return None;
    }
    clean
        .split('}')
        .filter_map(|rule| {
            let (selector, declarations) = rule.split_once('{')?;
            let selector = selector.rsplit(';').next()?.trim();
            if selector
                .split(',')
                .any(|s| matches!(s.trim(), "html" | "body" | ":root"))
            {
                writing_mode(declarations)
            } else {
                None
            }
        })
        .last()
}

fn vertical_book(
    zip: &Archive<'_>,
    chapters: &[(String, String)],
    docs: &[Document<'_>],
    remaining: &mut usize,
) -> bool {
    let mut styles = HashMap::new();
    let modes = chapters.iter().zip(docs).filter_map(|((path, _), doc)| {
        let mut mode = None;
        for node in doc.descendants().filter(Node::is_element) {
            let candidate = match node.tag_name().name() {
                "link"
                    if node
                        .attribute("rel")
                        .is_some_and(|rel| rel.eq_ignore_ascii_case("stylesheet"))
                        && node
                            .attribute("media")
                            .is_none_or(|media| matches!(media.trim(), "" | "all")) =>
                {
                    node.attribute("href")
                        .and_then(|href| local_target(path, href))
                        .and_then(|(part_name, _)| {
                            *styles.entry(part_name.clone()).or_insert_with(|| {
                                part(zip, &part_name, remaining)
                                    .ok()
                                    .and_then(|css| root_css_mode(&css))
                            })
                        })
                }
                "style" if node.attribute("media").is_none() => node.text().and_then(root_css_mode),
                _ => None,
            };
            mode = candidate.or(mode);
        }
        for node in doc
            .descendants()
            .filter(|n| named(*n, "html") || named(*n, "body"))
        {
            mode = node.attribute("style").and_then(writing_mode).or(mode);
        }
        mode
    });
    let mut vertical = false;
    for mode in modes {
        if !mode {
            return false;
        }
        vertical = true;
    }
    vertical
}

fn escape_table_pipes(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut backslashes = 0;
    for ch in text.chars() {
        if ch == '|' && backslashes % 2 == 0 {
            result.push('\\');
        }
        result.push(ch);
        if ch == '\\' {
            backslashes += 1;
        } else {
            backslashes = 0;
        }
    }
    result
}

fn escaped(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !space {
                result.push(' ');
            }
            space = true;
        } else {
            if matches!(
                ch,
                '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '{' | '}' | '#' | '~'
            ) {
                result.push('\\');
            }
            result.push(ch);
            space = false;
        }
    }
    result
}

type Target = (String, String);

struct Renderer<'a> {
    zip: &'a Archive<'a>,
    anchors: HashMap<Target, String>,
    notes: HashMap<Target, usize>,
    block_anchors: HashMap<(String, NodeId), String>,
    images: HashMap<String, Option<String>>,
    remaining: usize,
}

impl Renderer<'_> {
    fn anchor(&self, node: Node<'_, '_>, path: &str) -> Option<&str> {
        self.block_anchors
            .get(&(path.into(), node.id()))
            .map(String::as_str)
    }

    fn table(&mut self, table: Node<'_, '_>, path: &str, depth: usize) -> Option<String> {
        let rows: Vec<_> = table
            .children()
            .flat_map(|node| {
                if matches!(node.tag_name().name(), "thead" | "tbody" | "tfoot") {
                    node.children().collect()
                } else {
                    vec![node]
                }
            })
            .filter(|node| named(*node, "tr"))
            .take(1025)
            .collect();
        if rows.is_empty() || rows.len() > 1024 {
            return None;
        }
        let mut grid = vec![Vec::<Option<String>>::new(); rows.len()];
        for (r, row) in rows.iter().enumerate() {
            let mut c = 0;
            for cell in row
                .children()
                .filter(|n| named(*n, "th") || named(*n, "td"))
            {
                while grid[r].get(c).is_some_and(Option::is_some) {
                    c += 1;
                }
                let span = |attr| {
                    cell.attribute(attr)
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(1)
                        .max(1)
                };
                let width = span("colspan");
                let height = span("rowspan").min(rows.len() - r);
                let end = c.checked_add(width).filter(|end| *end <= 128)?;
                let body = if cell.descendants().any(|n| named(n, "table")) {
                    escaped(&plain(cell))
                } else {
                    self.children(cell, path, depth)
                };
                let body = body
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>()
                    .join("<br>");
                let body = escape_table_pipes(&body);
                for (offset, target) in grid[r..r + height].iter_mut().enumerate() {
                    target.resize_with(target.len().max(end), || None);
                    if target[c..end].iter().any(Option::is_some) {
                        return None;
                    }
                    let mut text = if offset == 0 {
                        body.clone()
                    } else {
                        String::new()
                    };
                    if width > 1 {
                        let _ = write!(text, " <!-- colspan:{width} -->");
                    }
                    if height > 1 {
                        text.push_str(if offset == 0 {
                            " <!-- vmerge:start -->"
                        } else {
                            " <!-- vmerge:continue -->"
                        });
                    }
                    target[c] = Some(text);
                    for covered in &mut target[c + 1..end] {
                        *covered = Some(String::new());
                    }
                }
                c = end;
            }
        }
        let cols = grid.iter().map(Vec::len).max().filter(|n| *n > 0)?;
        let mut output = String::from("\n\n");
        if let Some(id) = self.anchor(table, path) {
            let _ = write!(output, "\u{200b} {{#{id}}}\n\n");
        }
        if let Some(caption) = table.children().find(|n| named(*n, "caption")) {
            let text = self.render(caption, path, depth + 1);
            let _ = write!(output, "{}\n\n", text.trim());
        }
        let header = rows[0].children().any(|n| named(n, "th"));
        for (r, row) in grid.into_iter().enumerate() {
            output.push('|');
            for c in 0..cols {
                if (r, c) == (0, 0) && !header {
                    let _ = write!(output, " {}", crate::markdown::PPTX_NO_TABLE_HEADER);
                }
                let _ = write!(
                    output,
                    " {} |",
                    row.get(c).and_then(Option::as_deref).unwrap_or("")
                );
            }
            output.push('\n');
            if r == 0 {
                output.push('|');
                output.push_str(&" --- |".repeat(cols));
                output.push('\n');
            }
        }
        output.push('\n');
        Some(output)
    }

    fn children(&mut self, node: Node<'_, '_>, path: &str, depth: usize) -> String {
        node.children()
            .map(|child| self.render(child, path, depth + 1))
            .collect()
    }

    fn render(&mut self, node: Node<'_, '_>, path: &str, depth: usize) -> String {
        if depth > 96 {
            return String::new();
        }
        if node.is_text() {
            return escaped(node.text().unwrap_or(""));
        }
        if !node.is_element()
            || note(node)
            || node
                .attribute("id")
                .is_some_and(|id| self.notes.contains_key(&(path.into(), id.into())))
        {
            return String::new();
        }
        let tag = node.tag_name().name();
        match tag {
            "script" | "style" | "rt" | "rp" | "nav" => String::new(),
            "br" => "  \n".into(),
            "strong" | "b" | "em" | "i" => {
                let content = self.children(node, path, depth);
                let marker = if matches!(tag, "strong" | "b") {
                    "**"
                } else {
                    "*"
                };
                if content.trim().is_empty() {
                    content
                } else {
                    format!("{marker}{}{marker}", content.trim())
                }
            }
            "ruby" => {
                let readings: Vec<_> = node.children().filter(|n| named(*n, "rt")).collect();
                let base: String = node
                    .children()
                    .filter(|n| !named(*n, "rt") && !named(*n, "rp"))
                    .map(plain)
                    .collect();
                if let [reading] = readings.as_slice()
                    && !base.is_empty()
                    && !base.contains(['{', '}', '|', '\\', '\n'])
                {
                    let text = plain(*reading);
                    if !text.is_empty() && !text.contains(['{', '}', '|', '\\', '\n']) {
                        return format!("{{{base}|{text}}}");
                    }
                }
                escaped(&base)
            }
            "img" => {
                let alt = escaped(node.attribute("alt").unwrap_or(""));
                let Some((image, _)) = node
                    .attribute("src")
                    .and_then(|src| local_target(path, src))
                else {
                    return alt;
                };
                let uri = self.images.entry(image.clone()).or_insert_with(|| {
                    let ext = image.rsplit('.').next()?.to_ascii_lowercase();
                    let mime = match ext.as_str() {
                        "jpg" | "jpeg" => "jpeg",
                        "png" => "png",
                        "gif" => "gif",
                        "svg" => "svg+xml",
                        "webp" => "webp",
                        _ => return None,
                    };
                    let data = self
                        .zip
                        .read_bounded(&image, MAX_XML.min(self.remaining))
                        .ok()??;
                    self.remaining -= data.len();
                    Some(format!(
                        "data:image/{mime};base64,{}",
                        STANDARD.encode(data)
                    ))
                });
                let Some(uri) = uri else {
                    return alt;
                };
                // Charge repeated references as well as unique image bytes.
                let Some(remaining) = self.remaining.checked_sub(uri.len()) else {
                    return alt;
                };
                self.remaining = remaining;
                format!("![{alt}]({uri})")
            }
            "a" => {
                let label = self.children(node, path, depth);
                let Some(href) = node.attribute("href") else {
                    return label;
                };
                if let Some(target) = local_target(path, href) {
                    if let Some(index) = self.notes.get(&target) {
                        return format!("[^{index}]");
                    }
                    if let Some(id) = self.anchors.get(&target) {
                        return format!("[{}](#{id})", label.trim());
                    }
                }
                if href.starts_with("https://")
                    || href.starts_with("http://")
                    || href.starts_with("mailto:")
                {
                    const SAFE: percent_encoding::AsciiSet = NON_ALPHANUMERIC
                        .remove(b'/')
                        .remove(b':')
                        .remove(b'?')
                        .remove(b'&')
                        .remove(b'=')
                        .remove(b'%')
                        .remove(b'#')
                        .remove(b'.')
                        .remove(b'-')
                        .remove(b'_')
                        .remove(b'@');
                    return format!("[{}]({})", label.trim(), utf8_percent_encode(href, &SAFE));
                }
                label
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "figcaption" => {
                let content = self.children(node, path, depth);
                let target = self.anchor(node, path);
                let heading = tag
                    .as_bytes()
                    .get(1)
                    .filter(|_| tag.starts_with('h'))
                    .filter(|n| n.is_ascii_digit())
                    .map_or(String::new(), |n| {
                        format!("{} ", "#".repeat((n - b'0') as usize))
                    });
                let marker = target.map_or(String::new(), |id| format!(" {{#{id}}}"));
                let content = content.trim();
                if content.is_empty() {
                    target.map_or(String::new(), |id| format!("\n\n\u{200b} {{#{id}}}\n\n"))
                } else {
                    format!("\n\n{heading}{content}{marker}\n\n")
                }
            }
            "ul" | "ol" => {
                let mut list = String::from("\n\n");
                for (i, item) in node.children().filter(|n| named(*n, "li")).enumerate() {
                    if note(item)
                        || item
                            .attribute("id")
                            .is_some_and(|id| self.notes.contains_key(&(path.into(), id.into())))
                    {
                        continue;
                    }
                    let content = self.children(item, path, depth);
                    let marker = if tag == "ol" {
                        format!("{}. ", i + 1)
                    } else {
                        "- ".into()
                    };
                    for (line_index, line) in content.trim().lines().enumerate() {
                        let indent = if line_index == 0 {
                            marker.clone()
                        } else {
                            " ".repeat(marker.len())
                        };
                        let _ = writeln!(list, "{indent}{line}");
                    }
                }
                list.push('\n');
                list
            }
            "pre" => {
                let text = plain(node);
                let fence = "`".repeat(
                    text.split(|ch| ch != '`')
                        .map(str::len)
                        .max()
                        .unwrap_or(0)
                        .max(2)
                        + 1,
                );
                format!("\n\n{fence}\n{text}\n{fence}\n\n")
            }
            "table" => self
                .table(node, path, depth)
                .unwrap_or_else(|| self.children(node, path, depth)),
            "section" | "article" | "div" | "figure" | "blockquote" | "tr" | "td" | "th" => {
                let content = self.children(node, path, depth);
                self.anchor(node, path).map_or_else(
                    || format!("\n\n{}\n\n", content.trim()),
                    |id| format!("\n\n\u{200b} {{#{id}}}\n\n{}\n\n", content.trim()),
                )
            }
            _ => self.children(node, path, depth),
        }
    }
}

pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let zip = Archive::new(data)?;
    if zip.read_bounded("mimetype", 20)?.as_deref() != Some(b"application/epub+zip") {
        return Err(Error::new("Not an EPUB package"));
    }
    let mut remaining = MAX_TOTAL;
    let container = part(&zip, "META-INF/container.xml", &mut remaining)?;
    let container = xml(&container)?;
    let opf = container
        .descendants()
        .find(|n| named(*n, "rootfile"))
        .and_then(|n| n.attribute("full-path"))
        .ok_or_else(|| Error::new("Missing EPUB rootfile"))?;
    let opf_text = part(&zip, opf, &mut remaining)?;
    let package = xml(&opf_text)?;
    if !package
        .root_element()
        .attribute("version")
        .is_some_and(|v| v.starts_with("3."))
    {
        return Err(Error::new("Only EPUB 3 is supported"));
    }
    if package.descendants().any(|n| {
        (named(n, "meta")
            && n.attribute("property") == Some("rendition:layout")
            && n.text().is_some_and(|s| s.trim() == "pre-paginated"))
            || (named(n, "itemref")
                && n.attribute("properties").is_some_and(|p| {
                    p.split_whitespace()
                        .any(|p| p == "rendition:layout-pre-paginated")
                }))
    }) {
        return Err(Error::new("Fixed-layout EPUB is unsupported"));
    }
    let manifest: HashMap<_, _> = package
        .descendants()
        .filter(|n| named(*n, "item"))
        .filter_map(|n| Some((n.attribute("id")?, n)))
        .collect();
    let spine: Vec<_> = package
        .descendants()
        .filter(|n| named(*n, "itemref"))
        .collect();
    if spine.is_empty() || spine.len() > 1024 {
        return Err(Error::new("Invalid EPUB spine length"));
    }
    let mut chapters = Vec::new();
    for item in spine {
        let Some(entry) = item.attribute("idref").and_then(|id| manifest.get(id)) else {
            return Err(Error::new("EPUB spine refers to a missing manifest item"));
        };
        if entry.attribute("media-type") != Some("application/xhtml+xml") {
            if item.attribute("linear") == Some("no") {
                continue;
            }
            return Err(Error::new("EPUB spine item is not XHTML"));
        }
        let (path, _) = entry
            .attribute("href")
            .and_then(|href| local_target(opf, href))
            .ok_or_else(|| Error::new("Invalid EPUB spine path"))?;
        let chapter = part(&zip, &path, &mut remaining)?;
        chapters.push((path, chapter));
    }
    let docs: Vec<_> = chapters
        .iter()
        .map(|(_, source)| xml(source))
        .collect::<Result<_, _>>()?;
    let mut renderer = Renderer {
        zip: &zip,
        anchors: HashMap::new(),
        notes: HashMap::new(),
        block_anchors: HashMap::new(),
        images: HashMap::new(),
        remaining,
    };
    let mut note_nodes = Vec::new();
    let mut legacy_notes = HashMap::new();
    for ((path, _), doc) in chapters.iter().zip(&docs) {
        for node in doc.descendants().filter(|n| named(*n, "a")) {
            let legacy_ref = node
                .attribute("class")
                .is_some_and(|classes| classes.split_whitespace().any(|c| c == "note-d"));
            if legacy_ref
                && let Some(id) = node.attribute("id")
                && let Some(target) = node
                    .attribute("href")
                    .and_then(|href| local_target(path, href))
            {
                legacy_notes.insert(target, (path.clone(), id.to_owned()));
            }
        }
    }
    for ((path, _), doc) in chapters.iter().zip(&docs) {
        let index = renderer.anchors.len() + 1;
        renderer
            .anchors
            .insert((path.clone(), String::new()), format!("epub{index}"));
        for node in doc.descendants().filter(Node::is_element) {
            if let Some(id) = node.attribute("id") {
                let key = (path.clone(), id.into());
                let legacy_note = named(node, "li")
                    && legacy_notes.get(&key).is_some_and(|backlink| {
                        node.children()
                            .find(Node::is_element)
                            .filter(|n| named(*n, "a"))
                            .and_then(|n| n.attribute("href"))
                            .and_then(|href| local_target(path, href))
                            .as_ref()
                            == Some(backlink)
                    });
                if note(node) || legacy_note {
                    note_nodes.push((path, node, legacy_note));
                    renderer.notes.insert(key, note_nodes.len());
                } else if !node.ancestors().any(note)
                    && let Some(block) =
                        node.ancestors().find(|n| named(*n, "table")).or_else(|| {
                            node.ancestors().find(|n| {
                                n.is_element()
                                    && matches!(
                                        n.tag_name().name(),
                                        "h1" | "h2"
                                            | "h3"
                                            | "h4"
                                            | "h5"
                                            | "h6"
                                            | "p"
                                            | "figcaption"
                                            | "section"
                                            | "article"
                                            | "div"
                                            | "figure"
                                            | "blockquote"
                                            | "table"
                                    )
                            })
                        })
                {
                    let index = renderer.anchors.len() + 1;
                    let anchor = renderer
                        .block_anchors
                        .entry((path.clone(), block.id()))
                        .or_insert_with(|| format!("epub{index}"))
                        .clone();
                    renderer.anchors.insert(key, anchor);
                }
            }
        }
    }
    let mut output = String::from("---\n");
    for (tag, key) in [
        ("title", "title"),
        ("creator", "author"),
        ("language", "lang"),
    ] {
        if let Some(value) = package.descendants().find(|n| named(*n, tag)).map(plain) {
            let _ = writeln!(output, "{key}: {}", serde_json::Value::String(value));
        }
    }
    if vertical_book(&zip, &chapters, &docs, &mut renderer.remaining) {
        output.push_str("direction: vertical\n");
    }
    output.push_str("---\n\n");
    if let Some((path, _)) = manifest
        .values()
        .find(|n| {
            n.attribute("properties")
                .is_some_and(|p| p.split_whitespace().any(|p| p == "nav"))
        })
        .and_then(|n| n.attribute("href"))
        .and_then(|href| local_target(opf, href))
    {
        let source = part(&zip, &path, &mut renderer.remaining)?;
        let navigation = xml(&source)?;
        if let Some(toc) = navigation
            .descendants()
            .find(|n| named(*n, "nav") && epub_type(*n, "toc"))
        {
            output.push_str("## Contents\n\n");
            output.push_str(renderer.children(toc, &path, 0).trim());
            output.push_str("\n\n");
        }
    }
    for ((path, _), doc) in chapters.iter().zip(&docs) {
        if let Some(body) = doc.descendants().find(|n| named(*n, "body")) {
            let content = renderer.children(body, path, 0);
            if !content.trim().is_empty() {
                let anchor = &renderer.anchors[&(path.clone(), String::new())];
                let _ = write!(output, "\u{200b} {{#{anchor}}}\n\n{}\n\n", content.trim());
            }
        }
    }
    for (index, (path, node, legacy)) in note_nodes.into_iter().enumerate() {
        let backlink = legacy
            .then(|| node.children().find(Node::is_element))
            .flatten();
        let text: String = node
            .children()
            .filter(|child| Some(*child) != backlink)
            .map(|child| renderer.render(child, path, 1))
            .collect();
        let _ = writeln!(output, "[^{}]:", index + 1);
        for line in text.trim().lines() {
            let _ = writeln!(output, "    {line}");
        }
        output.push('\n');
    }
    Ok(output)
}
