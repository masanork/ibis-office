// SPDX-License-Identifier: AGPL-3.0-only
//! Foldable syntax-tokenized line model for JSON, JSONL, XML, YAML, and TOML.

use serde_json::Value;

const MAX_CODE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TREE_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeFormat {
    Json,
    Jsonl,
    Xml,
    Yaml,
    Toml,
}

impl CodeFormat {
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "json" | "jsonld" => Some(Self::Json),
            "jsonl" | "ndjson" => Some(Self::Jsonl),
            "xml" | "rdf" | "owl" => Some(Self::Xml),
            "yaml" | "yml" => Some(Self::Yaml),
            "toml" => Some(Self::Toml),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Jsonl => "JSONL",
            Self::Xml => "XML",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Punctuation,
    Key,
    TagName,
    AttrName,
    AttrValue,
    String,
    Number,
    Boolean,
    Null,
    Text,
    Comment,
    Summary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeToken {
    pub text: String,
    pub kind: TokenKind,
}

impl CodeToken {
    fn new(kind: TokenKind, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodeLine {
    pub line_number: usize,
    pub depth: usize,
    pub path: String,
    pub node_id: Option<usize>,
    pub close_line_index: Option<usize>,
    pub summary: Option<String>,
    pub tokens: Vec<CodeToken>,
    pub plain_text: String,
}

#[derive(Debug, Clone)]
pub struct CodeDocument {
    pub format: CodeFormat,
    pub format_label: &'static str,
    pub byte_size: usize,
    pub node_count: usize,
    pub max_depth: usize,
    pub lines: Vec<CodeLine>,
    pub folded: Vec<bool>,
}

impl CodeDocument {
    pub fn parse(extension: &str, bytes: &[u8]) -> Result<Self, String> {
        let format = CodeFormat::from_extension(extension)
            .ok_or_else(|| format!("Unsupported structured code format: {extension}"))?;
        if bytes.len() > MAX_CODE_BYTES {
            return Err("Structured code view accepts files up to 8 MiB".into());
        }
        let byte_size = bytes.len();
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let mut builder = Builder::default();

        match format {
            CodeFormat::Json => {
                let value: Value = serde_json::from_slice(bytes).map_err(|e| format!("Invalid JSON: {e}"))?;
                builder.emit_json_value(None, &value, "$", 0, false);
            }
            CodeFormat::Jsonl => {
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| "Structured code view requires UTF-8 text".to_string())?;
                let records: Vec<Value> = text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .enumerate()
                    .map(|(index, line)| {
                        serde_json::from_str::<Value>(line)
                            .map_err(|e| format!("Invalid JSONL on line {}: {e}", index + 1))
                    })
                    .collect::<Result<_, _>>()?;
                let total = records.len();
                for (index, record) in records.iter().enumerate() {
                    let label = format!("[{index}]");
                    let path = format!("$[{index}]");
                    builder.emit_json_value(Some(&label), record, &path, 0, index + 1 < total);
                }
            }
            CodeFormat::Xml => {
                let doc = super::parse_xml(bytes)?;
                let root = doc.root_element();
                let root_name = xml_qname(root);
                let root_path = format!("/{root_name}");
                builder.emit_xml_node(root, &root_path, 0)?;
            }
            CodeFormat::Yaml => {
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| "Structured code view requires UTF-8 text".to_string())?;
                let value: Value =
                    serde_saphyr::from_str(text).map_err(|e| format!("Invalid YAML: {e}"))?;
                builder.emit_json_value(None, &value, "$", 0, false);
            }
            CodeFormat::Toml => {
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| "Structured code view requires UTF-8 text".to_string())?;
                let value = super::notebook_schema::parse_toml_to_value(text)?;
                builder.emit_json_value(None, &value, "$", 0, false);
            }
        }

        let node_count = builder.folded.len();
        Ok(Self {
            format,
            format_label: format.label(),
            byte_size,
            node_count,
            max_depth: builder.max_depth,
            lines: builder.lines,
            folded: builder.folded,
        })
    }

    pub fn is_folded(&self, node_id: usize) -> bool {
        self.folded.get(node_id).copied().unwrap_or(false)
    }

    pub fn toggle_fold(&mut self, node_id: usize) {
        if let Some(state) = self.folded.get_mut(node_id) {
            *state = !*state;
        }
    }

    pub fn expand_all(&mut self) {
        self.folded.fill(false);
    }

    /// Collapse all foldable nodes at depth >= 1, keeping the root open.
    pub fn collapse_all(&mut self) {
        self.collapse_to_depth(1);
    }

    /// Collapse all foldable nodes at `depth >= max_open_depth`.
    pub fn collapse_to_depth(&mut self, max_open_depth: usize) {
        for line in &self.lines {
            if let Some(node_id) = line.node_id
                && let Some(state) = self.folded.get_mut(node_id)
            {
                *state = line.depth >= max_open_depth;
            }
        }
    }

    /// Return the indices in `self.lines` currently visible given `self.folded`.
    pub fn visible_lines(&self) -> Vec<usize> {
        let mut visible = Vec::with_capacity(self.lines.len());
        let mut idx = 0;
        while idx < self.lines.len() {
            visible.push(idx);
            let line = &self.lines[idx];
            if let (Some(node_id), Some(close_idx)) = (line.node_id, line.close_line_index)
                && self.is_folded(node_id)
                && close_idx > idx
            {
                idx = close_idx + 1;
                continue;
            }
            idx += 1;
        }
        visible
    }

    /// Search `all_lines` case-insensitively and return matching line indices.
    pub fn search_matches(&self, query: &str) -> Vec<usize> {
        let needle = query.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(idx, line)| {
                (line.plain_text.to_ascii_lowercase().contains(&needle)
                    || line.path.to_ascii_lowercase().contains(&needle))
                .then_some(idx)
            })
            .collect()
    }

    /// Unfold any ancestor containers enclosing `line_idx` so it becomes visible.
    pub fn reveal_line(&mut self, line_idx: usize) {
        for (idx, line) in self.lines.iter().enumerate() {
            if idx >= line_idx {
                break;
            }
            if let (Some(node_id), Some(close_idx)) = (line.node_id, line.close_line_index)
                && close_idx >= line_idx
                && let Some(state) = self.folded.get_mut(node_id)
            {
                *state = false;
            }
        }
    }
}

#[derive(Default)]
struct Builder {
    lines: Vec<CodeLine>,
    folded: Vec<bool>,
    max_depth: usize,
}

impl Builder {
    fn push_line(
        &mut self,
        depth: usize,
        path: String,
        node_id: Option<usize>,
        summary: Option<String>,
        tokens: Vec<CodeToken>,
    ) -> usize {
        self.max_depth = self.max_depth.max(depth);
        let idx = self.lines.len();
        let plain_text = tokens.iter().map(|t| t.text.as_str()).collect();
        self.lines.push(CodeLine {
            line_number: idx + 1,
            depth,
            path,
            node_id,
            close_line_index: None,
            summary,
            tokens,
            plain_text,
        });
        idx
    }

    fn alloc_node(&mut self) -> usize {
        let id = self.folded.len();
        self.folded.push(false);
        id
    }

    fn emit_json_value(
        &mut self,
        key: Option<&str>,
        value: &Value,
        path: &str,
        depth: usize,
        trailing_comma: bool,
    ) {
        let comma = if trailing_comma { "," } else { "" };
        let mut prefix = Vec::new();
        if let Some(k) = key {
            prefix.push(CodeToken::new(TokenKind::Key, serde_json::to_string(k).unwrap_or_default()));
            prefix.push(CodeToken::new(TokenKind::Punctuation, ": "));
        }

        match value {
            Value::Object(map) => {
                if map.is_empty() || depth >= MAX_TREE_DEPTH {
                    prefix.push(CodeToken::new(TokenKind::Punctuation, format!("{{}}{comma}")));
                    self.push_line(depth, path.to_string(), None, None, prefix);
                    return;
                }
                let node_id = self.alloc_node();
                let summary = format!(" … {} keys }}{comma}", map.len());
                prefix.push(CodeToken::new(TokenKind::Punctuation, "{"));
                let open_idx = self.push_line(
                    depth,
                    path.to_string(),
                    Some(node_id),
                    Some(summary),
                    prefix,
                );
                let len = map.len();
                for (i, (child_key, child_val)) in map.iter().enumerate() {
                    let child_path = format!("{path}.{child_key}");
                    self.emit_json_value(
                        Some(child_key),
                        child_val,
                        &child_path,
                        depth + 1,
                        i + 1 < len,
                    );
                }
                let close_idx = self.push_line(
                    depth,
                    path.to_string(),
                    None,
                    None,
                    vec![CodeToken::new(TokenKind::Punctuation, format!("}}{comma}"))],
                );
                self.lines[open_idx].close_line_index = Some(close_idx);
            }
            Value::Array(items) => {
                if items.is_empty() || depth >= MAX_TREE_DEPTH {
                    prefix.push(CodeToken::new(TokenKind::Punctuation, format!("[]{comma}")));
                    self.push_line(depth, path.to_string(), None, None, prefix);
                    return;
                }
                let node_id = self.alloc_node();
                let summary = format!(" … {} items ]{comma}", items.len());
                prefix.push(CodeToken::new(TokenKind::Punctuation, "["));
                let open_idx = self.push_line(
                    depth,
                    path.to_string(),
                    Some(node_id),
                    Some(summary),
                    prefix,
                );
                let len = items.len();
                for (i, item) in items.iter().enumerate() {
                    let child_path = format!("{path}[{i}]");
                    self.emit_json_value(None, item, &child_path, depth + 1, i + 1 < len);
                }
                let close_idx = self.push_line(
                    depth,
                    path.to_string(),
                    None,
                    None,
                    vec![CodeToken::new(TokenKind::Punctuation, format!("]{comma}"))],
                );
                self.lines[open_idx].close_line_index = Some(close_idx);
            }
            Value::String(s) => {
                let serialized = serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""));
                prefix.push(CodeToken::new(TokenKind::String, serialized));
                if !comma.is_empty() {
                    prefix.push(CodeToken::new(TokenKind::Punctuation, comma));
                }
                self.push_line(depth, path.to_string(), None, None, prefix);
            }
            Value::Number(n) => {
                prefix.push(CodeToken::new(TokenKind::Number, n.to_string()));
                if !comma.is_empty() {
                    prefix.push(CodeToken::new(TokenKind::Punctuation, comma));
                }
                self.push_line(depth, path.to_string(), None, None, prefix);
            }
            Value::Bool(b) => {
                prefix.push(CodeToken::new(TokenKind::Boolean, if *b { "true" } else { "false" }));
                if !comma.is_empty() {
                    prefix.push(CodeToken::new(TokenKind::Punctuation, comma));
                }
                self.push_line(depth, path.to_string(), None, None, prefix);
            }
            Value::Null => {
                prefix.push(CodeToken::new(TokenKind::Null, "null"));
                if !comma.is_empty() {
                    prefix.push(CodeToken::new(TokenKind::Punctuation, comma));
                }
                self.push_line(depth, path.to_string(), None, None, prefix);
            }
        }
    }

    fn emit_xml_node(
        &mut self,
        node: roxmltree::Node<'_, '_>,
        path: &str,
        depth: usize,
    ) -> Result<(), String> {
        if depth > MAX_TREE_DEPTH {
            return Ok(());
        }
        let qname = xml_qname(node);
        let mut open_tokens = vec![
            CodeToken::new(TokenKind::Punctuation, "<"),
            CodeToken::new(TokenKind::TagName, qname.clone()),
        ];
        for attr in node.attributes() {
            open_tokens.push(CodeToken::new(TokenKind::Punctuation, " "));
            open_tokens.push(CodeToken::new(TokenKind::AttrName, attr.name()));
            open_tokens.push(CodeToken::new(TokenKind::Punctuation, "=\""));
            open_tokens.push(CodeToken::new(TokenKind::AttrValue, attr.value()));
            open_tokens.push(CodeToken::new(TokenKind::Punctuation, "\""));
        }

        let child_elements: Vec<_> = node
            .children()
            .filter(|c| c.is_element() || c.is_comment())
            .collect();
        let text_content: String = node
            .children()
            .filter(|c| c.is_text())
            .filter_map(|c| c.text())
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_string();

        if child_elements.is_empty() && text_content.is_empty() {
            open_tokens.push(CodeToken::new(TokenKind::Punctuation, " />"));
            self.push_line(depth, path.to_string(), None, None, open_tokens);
            return Ok(());
        }

        if child_elements.is_empty() && !text_content.contains('\n') && text_content.len() <= 160 {
            open_tokens.push(CodeToken::new(TokenKind::Punctuation, ">"));
            open_tokens.push(CodeToken::new(TokenKind::Text, text_content));
            open_tokens.push(CodeToken::new(TokenKind::Punctuation, "</"));
            open_tokens.push(CodeToken::new(TokenKind::TagName, qname));
            open_tokens.push(CodeToken::new(TokenKind::Punctuation, ">"));
            self.push_line(depth, path.to_string(), None, None, open_tokens);
            return Ok(());
        }

        open_tokens.push(CodeToken::new(TokenKind::Punctuation, ">"));
        let node_id = self.alloc_node();
        let elem_count = child_elements.iter().filter(|c| c.is_element()).count();
        let summary = if elem_count > 0 {
            format!(" … {elem_count} children </{qname}>")
        } else {
            format!(" … </{qname}>")
        };
        let open_idx = self.push_line(
            depth,
            path.to_string(),
            Some(node_id),
            Some(summary),
            open_tokens,
        );

        if !text_content.is_empty() {
            for line in text_content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    self.push_line(
                        depth + 1,
                        path.to_string(),
                        None,
                        None,
                        vec![CodeToken::new(TokenKind::Text, trimmed)],
                    );
                }
            }
        }

        let mut tag_counts = std::collections::BTreeMap::<String, usize>::new();
        for child in child_elements {
            if child.is_comment() {
                let comment = child.text().unwrap_or_default().trim();
                self.push_line(
                    depth + 1,
                    path.to_string(),
                    None,
                    None,
                    vec![CodeToken::new(TokenKind::Comment, format!("<!-- {comment} -->"))],
                );
                continue;
            }
            let child_qname = xml_qname(child);
            let idx = tag_counts.entry(child_qname.clone()).or_insert(0);
            let child_path = format!("{path}/{child_qname}[{idx}]");
            *idx += 1;
            self.emit_xml_node(child, &child_path, depth + 1)?;
        }

        let close_idx = self.push_line(
            depth,
            path.to_string(),
            None,
            None,
            vec![
                CodeToken::new(TokenKind::Punctuation, "</"),
                CodeToken::new(TokenKind::TagName, qname),
                CodeToken::new(TokenKind::Punctuation, ">"),
            ],
        );
        self.lines[open_idx].close_line_index = Some(close_idx);
        Ok(())
    }
}

fn xml_qname(node: roxmltree::Node<'_, '_>) -> String {
    let name = node.tag_name().name();
    match node.tag_name().namespace() {
        Some(ns) => {
            if let Some(ns_decl) = node.namespaces().find(|n| n.uri() == ns)
                && let Some(prefix) = ns_decl.name()
                && !prefix.is_empty()
            {
                return format!("{prefix}:{name}");
            }
            name.to_string()
        }
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_folding_hides_children_and_reveals_on_search() {
        let raw = br#"{"name":"demo","nested":{"a":1,"b":[true,null]}}"#;
        let mut doc = CodeDocument::parse("json", raw).unwrap();
        let total = doc.lines.len();
        assert!(total > 5);
        assert_eq!(doc.visible_lines().len(), total);

        // Collapse depth >= 1 (folds "nested" and "b").
        doc.collapse_all();
        let collapsed = doc.visible_lines();
        assert!(collapsed.len() < total);

        let hits = doc.search_matches("true");
        assert_eq!(hits.len(), 1);
        doc.reveal_line(hits[0]);
        assert!(doc.visible_lines().contains(&hits[0]));
    }

    #[test]
    fn xml_folding_preserves_attributes_and_children() {
        let raw = br#"<bom xmlns="http://cyclonedx.org/schema/bom/1.6" version="1"><components><component type="library"><name>serde</name></component></components></bom>"#;
        let mut doc = CodeDocument::parse("xml", raw).unwrap();
        let total = doc.lines.len();
        assert!(total >= 7);
        assert!(doc.lines[0].plain_text.contains("version=\"1\""));

        // Fold <components> (depth 1)
        doc.collapse_to_depth(1);
        let visible = doc.visible_lines();
        // Root open + <components> + Root close = 3 lines
        assert_eq!(visible.len(), 3);
    }
}
