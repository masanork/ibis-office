// SPDX-License-Identifier: AGPL-3.0-only
//! Read reflowable EPUB 3 into the editable document model through Markdown.

use std::collections::HashMap;
use std::fmt::Write;

use base64::{Engine, engine::general_purpose::STANDARD};
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use roxmltree::{Document, Node, ParsingOptions};

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
    images: HashMap<String, Option<String>>,
    remaining: usize,
}

impl Renderer<'_> {
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
        if !node.is_element() || note(node) {
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
                let target = node
                    .attribute("id")
                    .and_then(|id| self.anchors.get(&(path.into(), id.into())));
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
                    String::new()
                } else {
                    format!("\n\n{heading}{content}{marker}\n\n")
                }
            }
            "ul" | "ol" => {
                let mut list = String::from("\n\n");
                for (i, item) in node.children().filter(|n| named(*n, "li")).enumerate() {
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
            "section" | "article" | "div" | "figure" | "blockquote" | "table" | "tr" | "td"
            | "th" => {
                format!("\n\n{}\n\n", self.children(node, path, depth).trim())
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
        images: HashMap::new(),
        remaining,
    };
    let mut note_nodes = Vec::new();
    for ((path, _), doc) in chapters.iter().zip(&docs) {
        let index = renderer.anchors.len() + 1;
        renderer
            .anchors
            .insert((path.clone(), String::new()), format!("epub{index}"));
        for node in doc.descendants().filter(Node::is_element) {
            if let Some(id) = node.attribute("id") {
                let key = (path.clone(), id.into());
                if note(node) {
                    note_nodes.push((path, node));
                    renderer.notes.insert(key, note_nodes.len());
                } else if matches!(
                    node.tag_name().name(),
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "figcaption"
                ) {
                    let index = renderer.anchors.len() + 1;
                    renderer.anchors.insert(key, format!("epub{index}"));
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
    for (index, (path, node)) in note_nodes.into_iter().enumerate() {
        let text = renderer.children(node, path, 0);
        let _ = writeln!(output, "[^{}]:", index + 1);
        for line in text.trim().lines() {
            let _ = writeln!(output, "    {line}");
        }
        output.push('\n');
    }
    Ok(output)
}
