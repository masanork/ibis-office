// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust Markdown source code view for flowing documents (`.md` / `.docx`).
//!
//! Allows toggling back and forth between the rendered page view and an editable
//! Markdown source view powered by `gpui-kit`'s `Editor` with a zero-C, pure-Rust
//! syntax highlighter and section folder (`InputHighlighter`).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{
    Editor, EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputEvent,
    InputHighlighter, InputHighlighterFactory, Rope,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_doc_model::{Block, FlowCommand, FlowDocument, FlowPos, FlowRunKind, MediaId};
use ibis_formats::markdown::MarkdownSourceMap;

use crate::ext::*;
use crate::shell::editor::SlideEditor;
use crate::shell::markdown_split::{
    SectionSpan, SourceDialect, law_section_spans, multi_source_highlighter_factory, section_for_line,
};

pub(crate) const MARKDOWN_SOURCE_CONTEXT: &str = "MarkdownSource";
const SOURCE_TO_VISUAL_DEBOUNCE_MS: u64 = 250;
const VISUAL_TO_SOURCE_DEBOUNCE_MS: u64 = 200;
const DEFAULT_SPLIT_RATIO: f32 = 0.45;
const MIN_SPLIT_RATIO: f32 = 0.20;
const MAX_SPLIT_RATIO: f32 = 0.80;

/// View mode for Markdown / flowing document source editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MarkdownViewMode {
    /// Rendered WYSIWYG document only.
    #[default]
    Visual,
    /// Side-by-side: editable Markdown source (left) + editable WYSIWYG document (right).
    Split,
    /// Editable Markdown source only.
    Source,
}

/// Which side currently holds unsynced edits in bidirectional Split mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum SyncAuthority {
    #[default]
    Idle,
    Source,
    Visual,
}

pub(crate) fn clamp_split_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
    } else {
        DEFAULT_SPLIT_RATIO
    }
}

/// Active state while the document is shown in Markdown / Lawtext / e-Gov XML Split or Source view.
pub(crate) struct MarkdownSourceState {
    pub(crate) mode: MarkdownViewMode,
    /// Active source dialect (`Markdown`, `Lawtext`, or `LawXml`).
    pub(crate) dialect: SourceDialect,
    pub(crate) editor: Entity<EditorState>,
    /// Cached latest UTF-8 text of `editor` so save paths without `App` context
    /// (`FormatEditor::bytes_for_path`) can write the exact source directly.
    pub(crate) current_text: String,
    /// Block-level source map linking `current_text` lines/bytes to `last_synced_flow.body` (Markdown).
    pub(crate) source_map: MarkdownSourceMap,
    /// Article/chapter/suppl section spans for Lawtext and e-Gov XML cursor/scroll sync.
    pub(crate) law_sections: Vec<SectionSpan>,
    /// Live legislative lint summary (`ibis_lawtext::lint_law`) for Lawtext / e-Gov XML documents.
    pub(crate) law_lint: Option<ibis_lawtext::LintReport>,
    /// Original e-Gov Law XML baseline for surgical `apply_law_source_edits` writeback.
    pub(crate) original_law_xml: Option<String>,
    /// True once the user has modified the source since entering source view or last sync.
    pub(crate) dirty: bool,
    /// True while the WYSIWYG `FlowDocument` has edits pending export back to `editor`.
    pub(crate) visual_dirty: bool,
    /// Which side currently owns pending synchronization.
    pub(crate) authority: SyncAuthority,
    /// Last `FlowDocument` snapshot synchronized with `current_text` (compared via `Arc::ptr_eq`).
    pub(crate) last_synced_flow: Option<Arc<FlowDocument>>,
    /// Last `FlowDocument` snapshot for which a debounced Visual->Source sync was scheduled.
    pub(crate) last_seen_flow: Option<Arc<FlowDocument>>,
    /// Monotonic counter used to supersede stale debounced sync tasks.
    pub(crate) sync_generation: u64,
    /// Guard flag to suppress `InputEvent::Change` emitted during programmatic `Visual -> Source` sync.
    pub(crate) suppress_source_event: bool,
    /// Whether cursor/scroll synchronization across the Split divider is enabled.
    pub(crate) sync_scroll_enabled: bool,
    /// Last 0-based source line already synced to the WYSIWYG viewport.
    pub(crate) last_synced_source_line: Option<usize>,
    /// Last WYSIWYG caret paragraph ID already synced to the Source editor.
    pub(crate) last_synced_visual_para: Option<u32>,
    /// Active top-level `FlowDocument::body` block index (Markdown) or law section index (Lawtext/XML).
    pub(crate) active_block_index: Option<usize>,
    /// Left pane width ratio in Split mode (`MIN_SPLIT_RATIO..=MAX_SPLIT_RATIO`).
    pub(crate) split_ratio: f32,
    /// Active divider drag state: `(start_pointer_x, start_ratio)`.
    pub(crate) split_drag: Option<(f32, f32)>,
    /// Last measured width of the split container in logical pixels.
    pub(crate) split_container_width: Rc<Cell<f32>>,
    /// Non-fatal parse error shown inline when switching back or syncing fails.
    pub(crate) last_error: Option<SharedString>,
    /// Whether the source editor had focus on the previous frame (for focus-transition flushes).
    pub(crate) source_was_focused: bool,
    pub(crate) _subscription: Subscription,
}

// ---------------------------------------------------------------------------
// Pure-Rust Markdown Syntax Highlighter & Folding (`InputHighlighter`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenKind {
    Heading,
    Strong,
    Emphasis,
    Strike,
    Code,
    CodeFence,
    Quote,
    ListMarker,
    LinkText,
    LinkUri,
    FrontmatterDelimiter,
    FrontmatterKey,
    TablePipe,
    HorizontalRule,
}

impl TokenKind {
    fn resolve(self, resolver: &dyn HighlightStyleResolver) -> HighlightStyle {
        match self {
            TokenKind::Heading => resolver.style("title").unwrap_or_else(|| HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..HighlightStyle::default()
            }),
            TokenKind::Strong => resolver.style("emphasis.strong").unwrap_or_else(|| HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..HighlightStyle::default()
            }),
            TokenKind::Emphasis => resolver.style("emphasis").unwrap_or_else(|| HighlightStyle {
                font_style: Some(FontStyle::Italic),
                ..HighlightStyle::default()
            }),
            TokenKind::Strike => {
                let mut s = resolver.style("comment").unwrap_or_default();
                s.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.0),
                    color: s.color,
                });
                s
            }
            TokenKind::Code => resolver
                .style("text.literal")
                .or_else(|| resolver.style("string"))
                .unwrap_or_default(),
            TokenKind::CodeFence => resolver
                .style("keyword")
                .or_else(|| resolver.style("comment"))
                .unwrap_or_default(),
            TokenKind::Quote => resolver.style("comment").unwrap_or_default(),
            TokenKind::ListMarker => resolver
                .style("punctuation.list_marker")
                .or_else(|| resolver.style("keyword"))
                .unwrap_or_default(),
            TokenKind::LinkText => resolver
                .style("link_text")
                .or_else(|| resolver.style("string"))
                .unwrap_or_default(),
            TokenKind::LinkUri => resolver
                .style("link_uri")
                .or_else(|| resolver.style("comment"))
                .unwrap_or_default(),
            TokenKind::FrontmatterDelimiter | TokenKind::HorizontalRule => {
                resolver.style("comment").unwrap_or_default()
            }
            TokenKind::FrontmatterKey => resolver
                .style("property")
                .or_else(|| resolver.style("keyword"))
                .unwrap_or_default(),
            TokenKind::TablePipe => resolver
                .style("punctuation")
                .or_else(|| resolver.style("comment"))
                .unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Span {
    range: Range<usize>,
    kind: TokenKind,
}

/// Pure-Rust incremental-friendly Markdown highlighter (zero C / tree-sitter deps).
struct PureRustMarkdownHighlighter {
    spans: Vec<Span>,
    folds: Vec<FoldRange>,
}

impl PureRustMarkdownHighlighter {
    fn new() -> Self {
        Self {
            spans: Vec::new(),
            folds: Vec::new(),
        }
    }

    fn reparse(&mut self, text: &str) {
        self.spans.clear();
        self.folds.clear();

        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut line_starts: Vec<(usize, usize)> = Vec::new();
        let mut pos = 0usize;
        while pos <= len {
            let line_start = pos;
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            let line_end = if pos > line_start && bytes[pos - 1] == b'\r' {
                pos - 1
            } else {
                pos
            };
            line_starts.push((line_start, line_end));
            if pos >= len {
                break;
            }
            pos += 1; // skip '\n'
        }

        let mut in_frontmatter = false;
        let mut frontmatter_start_line: Option<usize> = None;
        let mut in_fence: Option<(u8, usize, usize)> = None; // (fence_char, fence_len, start_line)
        let mut heading_stack: Vec<(usize, usize)> = Vec::new(); // (level, line_idx)

        for (line_idx, &(start, end)) in line_starts.iter().enumerate() {
            let line = &text[start..end];
            let trimmed = line.trim_start();
            let leading = line.len() - trimmed.len();

            // 1. YAML frontmatter at top of document
            if line_idx == 0 && line.trim() == "---" {
                in_frontmatter = true;
                frontmatter_start_line = Some(0);
                self.spans.push(Span {
                    range: start..end,
                    kind: TokenKind::FrontmatterDelimiter,
                });
                continue;
            }
            if in_frontmatter {
                let t = line.trim();
                if t == "---" || t == "..." {
                    self.spans.push(Span {
                        range: start..end,
                        kind: TokenKind::FrontmatterDelimiter,
                    });
                    if let Some(fm_start) = frontmatter_start_line.take()
                        && line_idx > fm_start
                    {
                        self.folds.push(FoldRange::new(fm_start, line_idx));
                    }
                    in_frontmatter = false;
                } else if let Some(colon) = line.find(':') {
                    self.spans.push(Span {
                        range: start..start + colon,
                        kind: TokenKind::FrontmatterKey,
                    });
                }
                continue;
            }

            // 2. Fenced code blocks (``` or ~~~)
            if let Some((fch, flen, fstart)) = in_fence {
                let count = trimmed.bytes().take_while(|&b| b == fch).count();
                if count >= flen && trimmed[count..].trim().is_empty() {
                    self.spans.push(Span {
                        range: start..end,
                        kind: TokenKind::CodeFence,
                    });
                    if line_idx > fstart {
                        self.folds.push(FoldRange::new(fstart, line_idx));
                    }
                    in_fence = None;
                } else if start < end {
                    self.spans.push(Span {
                        range: start..end,
                        kind: TokenKind::Code,
                    });
                }
                continue;
            }

            if leading <= 3 && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
                let fch = trimmed.as_bytes()[0];
                let count = trimmed.bytes().take_while(|&b| b == fch).count();
                if count >= 3 {
                    self.spans.push(Span {
                        range: start..end,
                        kind: TokenKind::CodeFence,
                    });
                    in_fence = Some((fch, count, line_idx));
                    continue;
                }
            }

            // 3. ATX Headings (# .. ######)
            if leading <= 3 && trimmed.starts_with('#') {
                let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
                if (1..=6).contains(&hashes)
                    && (trimmed.len() == hashes
                        || matches!(trimmed.as_bytes()[hashes], b' ' | b'\t'))
                {
                    while let Some(&(prev_level, prev_line)) = heading_stack.last() {
                        if prev_level >= hashes {
                            heading_stack.pop();
                            let fold_end = line_idx.saturating_sub(1);
                            if fold_end > prev_line {
                                self.folds.push(FoldRange::new(prev_line, fold_end));
                            }
                        } else {
                            break;
                        }
                    }
                    heading_stack.push((hashes, line_idx));
                    self.spans.push(Span {
                        range: start..end,
                        kind: TokenKind::Heading,
                    });
                    continue;
                }
            }

            // 4. Horizontal rule (---, ***, ___)
            if is_horizontal_rule(trimmed) {
                self.spans.push(Span {
                    range: start..end,
                    kind: TokenKind::HorizontalRule,
                });
                continue;
            }

            // 5. Blockquote (> ...)
            if trimmed.starts_with('>') {
                self.spans.push(Span {
                    range: start..end,
                    kind: TokenKind::Quote,
                });
                continue;
            }

            // 6. Table separator row (|---|---|)
            if is_table_separator(trimmed) {
                self.spans.push(Span {
                    range: start..end,
                    kind: TokenKind::TablePipe,
                });
                continue;
            }

            // 7. List marker (- , * , + , 1. , plus optional [ ] / [x])
            let mut inline_offset = start + leading;
            if let Some(marker_len) = list_marker_len(trimmed) {
                self.spans.push(Span {
                    range: inline_offset..inline_offset + marker_len,
                    kind: TokenKind::ListMarker,
                });
                inline_offset += marker_len;
            }

            // 8. Inline tokens (code, bold, italic, strike, links, table pipes)
            if inline_offset < end {
                scan_inline(&text[inline_offset..end], inline_offset, &mut self.spans);
            }
        }

        // Close any remaining heading fold ranges at EOF.
        let last_line = line_starts.len().saturating_sub(1);
        for (_, prev_line) in heading_stack.into_iter().rev() {
            if last_line > prev_line {
                self.folds.push(FoldRange::new(prev_line, last_line));
            }
        }
        self.folds.sort_by_key(|f| (f.start_line, f.end_line));
    }
}

fn is_horizontal_rule(trimmed: &str) -> bool {
    let mut marker: Option<u8> = None;
    let mut count = 0usize;
    for b in trimmed.bytes() {
        if b == b' ' || b == b'\t' {
            continue;
        }
        if matches!(b, b'-' | b'*' | b'_') {
            if let Some(m) = marker {
                if m != b {
                    return false;
                }
            } else {
                marker = Some(b);
            }
            count += 1;
        } else {
            return false;
        }
    }
    count >= 3
}

fn is_table_separator(trimmed: &str) -> bool {
    if !trimmed.contains('-') || !trimmed.contains('|') {
        return false;
    }
    trimmed
        .bytes()
        .all(|b| matches!(b, b'|' | b'-' | b':' | b' ' | b'\t'))
}

fn list_marker_len(trimmed: &str) -> Option<usize> {
    let bytes = trimmed.as_bytes();
    let base = if bytes.len() >= 2
        && matches!(bytes[0], b'-' | b'*' | b'+')
        && matches!(bytes[1], b' ' | b'\t')
    {
        2
    } else {
        let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits > 0
            && bytes.len() >= digits + 2
            && matches!(bytes[digits], b'.' | b')')
            && matches!(bytes[digits + 1], b' ' | b'\t')
        {
            digits + 2
        } else {
            return None;
        }
    };
    let rest = &trimmed[base..];
    if rest.starts_with("[ ] ") || rest.starts_with("[x] ") || rest.starts_with("[X] ") {
        Some(base + 4)
    } else {
        Some(base)
    }
}

fn scan_inline(s: &str, base: usize, out: &mut Vec<Span>) {
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        match bytes[i] {
            b'\\' if i + 1 < n => {
                i += 2;
            }
            b'|' => {
                out.push(Span {
                    range: base + i..base + i + 1,
                    kind: TokenKind::TablePipe,
                });
                i += 1;
            }
            b'`' => {
                let tick_len = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                if let Some(close_rel) = find_closing_ticks(&bytes[i + tick_len..], tick_len) {
                    let end = i + tick_len + close_rel + tick_len;
                    out.push(Span {
                        range: base + i..base + end,
                        kind: TokenKind::Code,
                    });
                    i = end;
                } else {
                    i += tick_len;
                }
            }
            b'*' | b'_' => {
                let ch = bytes[i];
                if i + 1 < n && bytes[i + 1] == ch {
                    if let Some(close_rel) = find_double_delim(&bytes[i + 2..], ch) {
                        let end = i + 2 + close_rel + 2;
                        out.push(Span {
                            range: base + i..base + end,
                            kind: TokenKind::Strong,
                        });
                        i = end;
                        continue;
                    }
                } else if let Some(close_rel) = find_single_delim(&bytes[i + 1..], ch) {
                    let end = i + 1 + close_rel + 1;
                    out.push(Span {
                        range: base + i..base + end,
                        kind: TokenKind::Emphasis,
                    });
                    i = end;
                    continue;
                }
                i += 1;
            }
            b'~' if i + 1 < n && bytes[i + 1] == b'~' => {
                if let Some(close_rel) = find_double_delim(&bytes[i + 2..], b'~') {
                    let end = i + 2 + close_rel + 2;
                    out.push(Span {
                        range: base + i..base + end,
                        kind: TokenKind::Strike,
                    });
                    i = end;
                } else {
                    i += 2;
                }
            }
            b'!' if i + 1 < n && bytes[i + 1] == b'[' => {
                if let Some((text_end, uri_start, uri_end)) = parse_link_at(bytes, i + 1) {
                    out.push(Span {
                        range: base + i..base + text_end + 1,
                        kind: TokenKind::LinkText,
                    });
                    out.push(Span {
                        range: base + uri_start..base + uri_end + 1,
                        kind: TokenKind::LinkUri,
                    });
                    i = uri_end + 1;
                } else {
                    i += 1;
                }
            }
            b'[' => {
                if let Some((text_end, uri_start, uri_end)) = parse_link_at(bytes, i) {
                    out.push(Span {
                        range: base + i..base + text_end + 1,
                        kind: TokenKind::LinkText,
                    });
                    out.push(Span {
                        range: base + uri_start..base + uri_end + 1,
                        kind: TokenKind::LinkUri,
                    });
                    i = uri_end + 1;
                } else {
                    i += 1;
                }
            }
            _ => {
                i += 1;
            }
        }
    }
}

fn find_closing_ticks(bytes: &[u8], tick_len: usize) -> Option<usize> {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let count = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            if count == tick_len {
                return Some(i);
            }
            i += count;
        } else {
            i += 1;
        }
    }
    None
}

fn find_double_delim(bytes: &[u8], ch: u8) -> Option<usize> {
    if bytes.is_empty() || bytes[0] == b' ' {
        return None;
    }
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == ch && bytes[i + 1] == ch && i > 0 && bytes[i - 1] != b' ' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_single_delim(bytes: &[u8], ch: u8) -> Option<usize> {
    if bytes.is_empty() || bytes[0] == b' ' || bytes[0] == ch {
        return None;
    }
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == ch && i > 0 && bytes[i - 1] != b' ' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_link_at(bytes: &[u8], open_bracket: usize) -> Option<(usize, usize, usize)> {
    let mut i = open_bracket + 1;
    while i < bytes.len() && bytes[i] != b']' {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'[' {
            return None;
        }
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b']' {
        return None;
    }
    let text_end = i;
    if text_end + 1 >= bytes.len() || bytes[text_end + 1] != b'(' {
        return None;
    }
    let uri_start = text_end + 1;
    let mut j = uri_start + 1;
    let mut depth = 1usize;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'\\' => j += 2,
            b'(' => {
                depth += 1;
                j += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((text_end, uri_start, j));
                }
                j += 1;
            }
            _ => j += 1,
        }
    }
    None
}

impl InputHighlighter for PureRustMarkdownHighlighter {
    fn language(&self) -> SharedString {
        "markdown".into()
    }

    fn update(
        &mut self,
        _edit: Option<InputEdit>,
        text: &Rope,
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        self.reparse(&text.to_string());
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        if range.start >= range.end {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut cursor = range.start;
        let first = self.spans.partition_point(|s| s.range.end <= range.start);
        for span in &self.spans[first..] {
            if span.range.start >= range.end {
                break;
            }
            let s_start = span.range.start.max(cursor);
            let s_end = span.range.end.min(range.end);
            if s_start >= s_end {
                continue;
            }
            if s_start > cursor {
                out.push((cursor..s_start, HighlightStyle::default()));
            }
            out.push((s_start..s_end, span.kind.resolve(resolver)));
            cursor = s_end;
        }
        if cursor < range.end {
            out.push((cursor..range.end, HighlightStyle::default()));
        }
        out
    }

    fn fold_ranges(&self, _text: &Rope) -> Vec<FoldRange> {
        self.folds.clone()
    }
}

#[allow(dead_code)]
fn markdown_highlighter_factory() -> InputHighlighterFactory {
    Rc::new(|lang| {
        if lang.eq_ignore_ascii_case("markdown") || lang.eq_ignore_ascii_case("md") {
            Some(Box::new(PureRustMarkdownHighlighter::new()))
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Media remapping when re-parsing Markdown into FlowDocument
// ---------------------------------------------------------------------------

fn remap_blocks_media(blocks: &mut [Block], remap: &BTreeMap<MediaId, MediaId>) {
    for block in blocks {
        match block {
            Block::Paragraph(p) => {
                for run in &mut p.runs {
                    if let FlowRunKind::Picture { media, .. }
                    | FlowRunKind::PictureControl { media, .. } = &mut run.kind
                        && let Some(&new_id) = remap.get(media)
                    {
                        *media = new_id;
                    }
                }
            }
            Block::Table(t) => {
                for row in &mut t.rows {
                    for cell in &mut row.cells {
                        remap_blocks_media(&mut cell.blocks, remap);
                    }
                }
            }
        }
    }
}

fn remap_document_media(doc: &mut FlowDocument, remap: &BTreeMap<MediaId, MediaId>) {
    if remap.is_empty() {
        return;
    }
    remap_blocks_media(&mut doc.body, remap);
    remap_blocks_media(&mut doc.header, remap);
    remap_blocks_media(&mut doc.footer, remap);
    for n in &mut doc.footnotes {
        remap_blocks_media(&mut n.blocks, remap);
    }
    for n in &mut doc.endnotes {
        remap_blocks_media(&mut n.blocks, remap);
    }
}

// ---------------------------------------------------------------------------
// SlideEditor extension methods
// ---------------------------------------------------------------------------

#[ibis_office_macros::editor_ext(MarkdownSource)]
impl SlideEditor {
    /// Current Markdown view mode (`Visual`, `Split`, or `Source`).
    pub fn markdown_view_mode(&self) -> MarkdownViewMode {
        self.doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map_or(MarkdownViewMode::Visual, |m| m.mode)
    }

    /// Whether the document has an active Markdown source pane (`Split` or `Source`).
    pub(crate) fn is_markdown_source_mode(&self) -> bool {
        self.markdown_view_mode() != MarkdownViewMode::Visual
    }

    /// Whether the document is showing only the Markdown source pane (no WYSIWYG canvas).
    pub(crate) fn is_markdown_source_only(&self) -> bool {
        self.markdown_view_mode() == MarkdownViewMode::Source
    }

    /// Whether the document is showing the side-by-side bidirectional Markdown Split view.
    pub(crate) fn is_markdown_split_mode(&self) -> bool {
        self.markdown_view_mode() == MarkdownViewMode::Split
    }

    /// Whether the Markdown source editor currently holds unsynced edits.
    pub(crate) fn is_markdown_source_dirty(&self) -> bool {
        self.doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .is_some_and(|ms| ms.dirty)
    }

    /// Whether the currently open file has a `.md` or `.markdown` extension.
    pub(crate) fn is_markdown_file(&self) -> bool {
        self.file_name.as_ref().is_some_and(|name| {
            std::path::Path::new(name.as_ref())
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
                })
        })
    }

    /// Whether the Markdown source `EditorState` currently holds keyboard focus.
    pub(crate) fn is_markdown_source_focused(&self, window: &Window, cx: &App) -> bool {
        self.doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .is_some_and(|ms| ms.editor.read(cx).focus_handle(cx).is_focused(window))
    }

    /// Default source dialect for the currently open document (`.lawtext`/`.law.txt` -> `Lawtext`,
    /// e-Gov Law `.xml` -> `LawXml`, otherwise `Markdown`).
    pub(crate) fn default_source_dialect(&self) -> SourceDialect {
        let name_lower = self
            .current_path()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .or(self.file_name.as_deref())
            .map(|n| n.to_ascii_lowercase());
        match name_lower.as_deref() {
            Some(n) if n.ends_with(".lawtext") || n.ends_with(".law.txt") => SourceDialect::Lawtext,
            Some(n) if n.ends_with(".xml") && self.is_law_document() => SourceDialect::LawXml,
            _ => SourceDialect::Markdown,
        }
    }

    /// Active source dialect in the Split/Source editor (or default dialect when in Visual mode).
    pub(crate) fn source_dialect(&self) -> SourceDialect {
        self.doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map_or_else(|| self.default_source_dialect(), |ms| ms.dialect)
    }

    /// Cached Markdown source text when the source editor is in `Markdown` dialect and authoritative
    /// or synchronized with the current `FlowDocument`. Returns `None` if the WYSIWYG side has newer
    /// unsynchronized edits or if the pane is displaying Lawtext/XML.
    pub(crate) fn markdown_source_text(&self) -> Option<&str> {
        let ms = self.doc_state()?.markdown_source.as_ref()?;
        if ms.dialect != SourceDialect::Markdown {
            return None;
        }
        if ms.dirty || ms.mode == MarkdownViewMode::Source {
            return Some(ms.current_text.as_str());
        }
        if ms.visual_dirty {
            return None;
        }
        let flow_matches = self
            .presentation()
            .flow
            .as_ref()
            .zip(ms.last_synced_flow.as_ref())
            .is_some_and(|(cur, synced)| Arc::ptr_eq(cur, synced));
        flow_matches.then_some(ms.current_text.as_str())
    }

    /// Exact UTF-8 Lawtext or e-Gov XML source text when the source editor is showing `dialect`
    /// and is authoritative or synchronized with the current `FlowDocument`.
    pub(crate) fn law_source_text(&self, dialect: SourceDialect) -> Option<&str> {
        let ms = self.doc_state()?.markdown_source.as_ref()?;
        if ms.dialect != dialect {
            return None;
        }
        if ms.dirty || ms.mode == MarkdownViewMode::Source {
            return Some(ms.current_text.as_str());
        }
        if ms.visual_dirty {
            return None;
        }
        let flow_matches = self
            .presentation()
            .flow
            .as_ref()
            .zip(ms.last_synced_flow.as_ref())
            .is_some_and(|(cur, synced)| Arc::ptr_eq(cur, synced));
        flow_matches.then_some(ms.current_text.as_str())
    }

    /// Original e-Gov Law XML baseline for surgical writeback when saving `.xml`.
    pub(crate) fn law_original_xml(&self) -> Option<&str> {
        self.doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .and_then(|ms| ms.original_law_xml.as_deref())
    }

    /// Switch the source pane dialect (`Lawtext` / `e-Gov XML` / `Markdown`) on a law document.
    pub(crate) fn set_source_dialect(
        &mut self,
        target: SourceDialect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_markdown_source_mode() {
            self.enter_markdown_view_mode_with_dialect(MarkdownViewMode::Split, target, window, cx);
            return;
        }
        let current = self.source_dialect();
        if current == target {
            return;
        }
        // Flush any dirty edits from the current dialect into FlowDocument first.
        if !self.sync_markdown_source_if_dirty(cx) {
            return;
        }
        let Some(flow_arc) = self.presentation().flow.clone() else {
            return;
        };
        let prev_orig_xml = self
            .doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .and_then(|ms| ms.original_law_xml.clone());
        let (rendered, source_map, law_sections, law_lint, orig_xml) =
            self.render_initial_source_for_dialect(target, &flow_arc, prev_orig_xml);

        let editor_entity = self
            .doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|ms| ms.editor.clone());
        if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
            ms.suppress_source_event = true;
            ms.dialect = target;
            ms.current_text = rendered.clone();
            ms.source_map = source_map;
            ms.law_sections = law_sections;
            ms.law_lint = law_lint;
            ms.original_law_xml = orig_xml;
            ms.dirty = false;
            ms.visual_dirty = false;
            ms.authority = SyncAuthority::Idle;
            ms.last_error = None;
            ms.last_synced_flow = Some(flow_arc.clone());
            ms.last_seen_flow = Some(flow_arc);
            ms.last_synced_source_line = None;
            ms.last_synced_visual_para = None;
            ms.active_block_index = None;
        }
        if let Some(editor) = editor_entity {
            editor.update(cx, |state, cx| {
                state.set_highlighter(target.language_id(), cx);
                state.set_value(rendered, window, cx);
            });
        }
        if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
            ms.suppress_source_event = false;
        }
        cx.notify();
        window.refresh();
    }

    /// Toggle between Visual view and Markdown/Lawtext Split/Source view (⌘⇧M / View > Markdown Source).
    pub(crate) fn toggle_markdown_source_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_markdown_source_mode() {
            self.exit_markdown_source_mode(window, cx);
        } else {
            self.set_markdown_view_mode(MarkdownViewMode::Split, window, cx);
        }
    }

    /// Switch to the requested [`MarkdownViewMode`] (`Visual`, `Split`, or `Source`).
    pub(crate) fn set_markdown_view_mode(
        &mut self,
        target: MarkdownViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            MarkdownViewMode::Visual => {
                self.exit_markdown_source_mode(window, cx);
            }
            MarkdownViewMode::Split | MarkdownViewMode::Source => {
                if self.is_markdown_source_mode() {
                    let current_mode = self.markdown_view_mode();
                    if current_mode == target {
                        return;
                    }
                    // Entering Split from Source: push any pending source edits to WYSIWYG first.
                    if target == MarkdownViewMode::Split {
                        let _ = self.sync_markdown_source_if_dirty(cx);
                    } else if target == MarkdownViewMode::Source {
                        // Entering Source-only from Split: flush any Visual edits back to source.
                        self.sync_visual_to_markdown_source(window, cx);
                    }
                    let editor_to_focus = self
                        .doc_state_mut()
                        .and_then(|s| s.markdown_source.as_mut())
                        .map(|ms| {
                            ms.mode = target;
                            ms.editor.clone()
                        });
                    if target == MarkdownViewMode::Source
                        && let Some(ed) = editor_to_focus
                    {
                        ed.update(cx, |s, cx| s.focus(window, cx));
                    }
                    cx.notify();
                    window.refresh();
                } else {
                    self.enter_markdown_view_mode(target, window, cx);
                }
            }
        }
    }

    /// Switch from rendered view to Markdown / Lawtext / e-Gov XML source view (defaults to Split).
    pub(crate) fn enter_markdown_source_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.enter_markdown_view_mode(MarkdownViewMode::Split, window, cx);
    }

    fn enter_markdown_view_mode(
        &mut self,
        mode: MarkdownViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialect = self.default_source_dialect();
        self.enter_markdown_view_mode_with_dialect(mode, dialect, window, cx);
    }

    fn render_initial_source_for_dialect(
        &self,
        dialect: SourceDialect,
        flow_arc: &Arc<FlowDocument>,
        existing_orig_xml: Option<String>,
    ) -> (
        String,
        MarkdownSourceMap,
        Vec<SectionSpan>,
        Option<ibis_lawtext::LintReport>,
        Option<String>,
    ) {
        let disk_raw = self.read_unmodified_law_raw_text();
        let orig_xml = existing_orig_xml.or_else(|| {
            disk_raw
                .as_deref()
                .filter(|raw| ibis_lawtext::is_law_xml(raw.as_bytes()))
                .map(ToOwned::to_owned)
        });

        match dialect {
            SourceDialect::Markdown => {
                let (initial, source_map) = if let Some((orig_text, orig_map, orig_doc)) = self
                    .source
                    .as_ref()
                    .and_then(|s| s.original_markdown())
                {
                    ibis_formats::markdown::patch_markdown_from_visual_edit(
                        orig_text,
                        orig_doc,
                        orig_map,
                        flow_arc,
                        &self.presentation().media,
                    )
                } else {
                    ibis_formats::markdown::export_markdown_with_source_map(
                        flow_arc,
                        &self.presentation().media,
                    )
                };
                let lint = if self.is_law_document() {
                    ibis_lawtext::parse_markdown_law(&initial)
                        .ok()
                        .map(|law| ibis_lawtext::lint_law(&law))
                } else {
                    None
                };
                (initial, source_map, Vec::new(), lint, orig_xml)
            }
            SourceDialect::Lawtext => {
                let initial = if !self.document.is_dirty()
                    && let Some(raw) = disk_raw.as_deref()
                    && !ibis_lawtext::is_law_xml(raw.as_bytes())
                {
                    raw.to_owned()
                } else {
                    ibis_lawtext::export_presentation_to_lawtext(self.presentation()).unwrap_or_else(
                        |_| ibis_formats::markdown::export_markdown(flow_arc, &self.presentation().media),
                    )
                };
                let lint = ibis_lawtext::parse_lawtext(&initial)
                    .ok()
                    .map(|law| ibis_lawtext::lint_law(&law));
                let sections = law_section_spans(&initial, SourceDialect::Lawtext);
                (initial, MarkdownSourceMap::default(), sections, lint, orig_xml)
            }
            SourceDialect::LawXml => {
                let initial = if !self.document.is_dirty()
                    && let Some(xml) = orig_xml.as_deref()
                {
                    xml.to_owned()
                } else if let Some(xml) = orig_xml.as_deref()
                    && let Ok(md) = ibis_formats::markdown::export_presentation_to_markdown(self.presentation())
                    && let Ok(patch) = ibis_lawtext::apply_law_source_edits(
                        xml,
                        &md,
                        ibis_lawtext::LawEditFormat::Markdown,
                    )
                    && let Some(patched_xml) = patch.xml
                {
                    patched_xml
                } else {
                    ibis_lawtext::export_presentation_to_law_xml(self.presentation()).unwrap_or_else(
                        |_| ibis_formats::markdown::export_markdown(flow_arc, &self.presentation().media),
                    )
                };
                let lint = ibis_lawtext::parse_law_xml(&initial)
                    .ok()
                    .map(|law| ibis_lawtext::lint_law(&law));
                let sections = law_section_spans(&initial, SourceDialect::LawXml);
                let xml_baseline = orig_xml.or_else(|| {
                    ibis_lawtext::is_law_xml(initial.as_bytes()).then(|| initial.clone())
                });
                (initial, MarkdownSourceMap::default(), sections, lint, xml_baseline)
            }
        }
    }

    fn read_unmodified_law_raw_text(&self) -> Option<String> {
        if let Some(bytes) = self.web_law_source_bytes()
            && let Ok(s) = std::str::from_utf8(bytes)
        {
            return Some(s.to_owned());
        }
        #[cfg(not(target_family = "wasm"))]
        if self.is_law_path()
            && let Some(path) = self.current_path()
            && let Ok(raw) = std::fs::read_to_string(path)
        {
            return Some(raw);
        }
        None
    }

    fn enter_markdown_view_mode_with_dialect(
        &mut self,
        mode: MarkdownViewMode,
        dialect: SourceDialect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode == MarkdownViewMode::Visual || self.is_markdown_source_mode() {
            return;
        }
        let Some(flow_arc) = self.presentation().flow.clone() else {
            return;
        };
        let (initial, source_map, law_sections, law_lint, original_law_xml) =
            self.render_initial_source_for_dialect(dialect, &flow_arc, None);
        let readonly = self.is_read_only()
            || self.is_jtd_read_only()
            || self.is_viewing_mode()
            || self.is_protection_read_only();

        let weak_editor = cx.weak_entity();
        let immediate_once = self.inline_completion.immediate_once.clone();
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language(dialect.language_id())
                .line_number(true)
                .soft_wrap(true)
                .folding(true)
                .searchable(true)
                .default_value(initial.clone());
            state.set_highlighter_factory(multi_source_highlighter_factory(), cx);
            state.set_readonly(readonly, cx);
            Self::attach_inline_completion_provider(&mut state, weak_editor, immediate_once);
            state
        });

        let subscription = cx.subscribe_in(&editor, window, |this, ed, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                let suppressed = this
                    .doc_state()
                    .and_then(|s| s.markdown_source.as_ref())
                    .is_some_and(|ms| ms.suppress_source_event);
                if suppressed {
                    return;
                }
                let text = ed.read(cx).value().to_string();
                let chars = text.chars().filter(|c| !c.is_whitespace()).count();
                let mut should_schedule = false;
                if let Some(doc_state) = this.doc_state_mut() {
                    doc_state.characters = chars;
                    if let Some(ms) = doc_state.markdown_source.as_mut() {
                        if ms.dialect != SourceDialect::Markdown {
                            ms.law_sections = law_section_spans(&text, ms.dialect);
                        }
                        ms.current_text = text;
                        ms.dirty = true;
                        ms.visual_dirty = false;
                        ms.authority = SyncAuthority::Source;
                        ms.last_error = None;
                        ms.sync_generation = ms.sync_generation.wrapping_add(1);
                        should_schedule = ms.mode == MarkdownViewMode::Split;
                    }
                }
                if should_schedule {
                    this.schedule_source_to_visual_sync(cx);
                }
                this.schedule_text_content_auto_switch(ed.clone(), window, cx);
                cx.notify();
            }
        });

        editor.update(cx, |s, cx| s.focus(window, cx));

        let active_block_index = if dialect == SourceDialect::Markdown {
            self.doc_state()
                .and_then(|s| s.caret)
                .and_then(|c| source_map.span_for_para(c.para))
                .map(|s| s.block_index)
        } else {
            None
        };

        if let Some(state) = self.doc_state_mut() {
            state.markdown_source = Some(MarkdownSourceState {
                mode,
                dialect,
                editor,
                current_text: initial,
                source_map,
                law_sections,
                law_lint,
                original_law_xml,
                dirty: false,
                visual_dirty: false,
                authority: SyncAuthority::Idle,
                last_synced_flow: Some(flow_arc.clone()),
                last_seen_flow: Some(flow_arc),
                sync_generation: 0,
                suppress_source_event: false,
                sync_scroll_enabled: true,
                last_synced_source_line: None,
                last_synced_visual_para: None,
                active_block_index,
                split_ratio: DEFAULT_SPLIT_RATIO,
                split_drag: None,
                split_container_width: Rc::new(Cell::new(0.0)),
                last_error: None,
                source_was_focused: true,
                _subscription: subscription,
            });
        }
        cx.notify();
        window.refresh();
    }

    fn schedule_source_to_visual_sync(&mut self, cx: &mut Context<Self>) {
        let Some(expected_gen) = self
            .doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|ms| ms.sync_generation)
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SOURCE_TO_VISUAL_DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |editor, cx| {
                let should_sync = editor
                    .doc_state()
                    .and_then(|s| s.markdown_source.as_ref())
                    .is_some_and(|ms| {
                        ms.mode == MarkdownViewMode::Split
                            && ms.dirty
                            && ms.sync_generation == expected_gen
                    });
                if should_sync {
                    let _ = editor.sync_markdown_source_if_dirty(cx);
                }
            });
        })
        .detach();
    }

    fn schedule_visual_to_source_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(expected_gen) = self
            .doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|ms| ms.sync_generation)
        else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(VISUAL_TO_SOURCE_DEBOUNCE_MS))
                .await;
            let _ = this.update_in(cx, |editor, window, cx| {
                let should_sync = editor
                    .doc_state()
                    .and_then(|s| s.markdown_source.as_ref())
                    .is_some_and(|ms| {
                        ms.mode == MarkdownViewMode::Split
                            && ms.visual_dirty
                            && !ms.dirty
                            && ms.sync_generation == expected_gen
                    });
                if should_sync {
                    editor.sync_visual_to_markdown_source(window, cx);
                }
            });
        })
        .detach();
    }

    fn scroll_visual_to_paragraph_preserving_focus(
        &mut self,
        para_id: u32,
        cx: &mut Context<Self>,
    ) {
        self.relayout_if_changed();
        let lines = self.flow_lines();
        if let Some((page, _, y, h)) = self.caret_box(&lines, FlowPos::new(para_id, 0)) {
            self.scroll_into_view(page, y, h, cx);
        }
    }

    fn find_law_paragraph_for_title(&self, title: &str) -> Option<u32> {
        let flow = self.presentation().flow.as_ref()?;
        let normalized = title.trim();
        if normalized.is_empty() {
            return None;
        }
        for block in &flow.body {
            if let Block::Paragraph(p) = block {
                let text = p.text();
                let t = text.trim();
                if t == normalized
                    || t.starts_with(normalized)
                    || (normalized.starts_with('第') && t.contains(normalized))
                {
                    return Some(p.id);
                }
            }
        }
        None
    }

    fn find_law_section_for_paragraph(&self, para_id: u32, sections: &[SectionSpan]) -> Option<usize> {
        let flow = self.presentation().flow.as_ref()?;
        let mut target_idx = None;
        for (idx, block) in flow.body.iter().enumerate() {
            if let Block::Paragraph(p) = block
                && p.id == para_id
            {
                target_idx = Some(idx);
                break;
            }
        }
        let target_idx = target_idx?;
        // Walk backwards from the caret paragraph to find the nearest article/chapter heading.
        for block in flow.body[..=target_idx].iter().rev() {
            if let Block::Paragraph(p) = block {
                let text = p.text();
                let t = text.trim();
                if t.is_empty() {
                    continue;
                }
                if let Some(span) = sections.iter().find(|s| {
                    s.title.as_deref().is_some_and(|title| {
                        t == title || t.starts_with(title) || (title.starts_with('第') && t.contains(title))
                    })
                }) {
                    return Some(span.index);
                }
            }
        }
        None
    }

    fn sync_split_cursor_and_scroll(
        &mut self,
        source_focused: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((dialect, sync_enabled, dirty, visual_dirty, editor_entity, last_line, last_para)) =
            self.doc_state()
                .and_then(|s| s.markdown_source.as_ref())
                .map(|ms| {
                    (
                        ms.dialect,
                        ms.sync_scroll_enabled,
                        ms.dirty,
                        ms.visual_dirty,
                        ms.editor.clone(),
                        ms.last_synced_source_line,
                        ms.last_synced_visual_para,
                    )
                })
        else {
            return;
        };

        if !sync_enabled || dirty || visual_dirty {
            return;
        }

        if dialect != SourceDialect::Markdown {
            if source_focused {
                let line = editor_entity.read(cx).cursor_position().line as usize;
                if Some(line) == last_line {
                    return;
                }
                let matched = self
                    .doc_state()
                    .and_then(|s| s.markdown_source.as_ref())
                    .and_then(|ms| section_for_line(&ms.law_sections, line))
                    .map(|span| (span.index, span.title.clone()));
                if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
                    ms.last_synced_source_line = Some(line);
                    ms.active_block_index = matched.as_ref().map(|(idx, _)| *idx);
                }
                if let Some((_, Some(title))) = matched
                    && let Some(para_id) = self.find_law_paragraph_for_title(&title)
                {
                    self.scroll_visual_to_paragraph_preserving_focus(para_id, cx);
                }
            } else {
                let Some(para_id) = self.doc_state().and_then(|s| s.caret).map(|p| p.para) else {
                    return;
                };
                if Some(para_id) == last_para {
                    return;
                }
                let sections = self
                    .doc_state()
                    .and_then(|s| s.markdown_source.as_ref())
                    .map(|ms| ms.law_sections.clone())
                    .unwrap_or_default();
                let target = self
                    .find_law_section_for_paragraph(para_id, &sections)
                    .and_then(|idx| sections.get(idx))
                    .map(|span| (span.index, span.byte_range.start, span.line_range.start));
                if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
                    ms.last_synced_visual_para = Some(para_id);
                    ms.active_block_index = target.map(|(idx, _, _)| idx);
                    if let Some((_, _, start_line)) = target {
                        ms.last_synced_source_line = Some(start_line);
                    }
                }
                if let Some((_, start_byte, _)) = target {
                    editor_entity.update(cx, |state, cx| {
                        state.set_selected_range(start_byte..start_byte, cx);
                    });
                }
            }
            return;
        }

        if source_focused {
            let line = editor_entity.read(cx).cursor_position().line as usize;
            if Some(line) == last_line {
                return;
            }
            let target = self
                .doc_state()
                .and_then(|s| s.markdown_source.as_ref())
                .and_then(|ms| ms.source_map.span_for_line(line))
                .map(|span| (span.block_index, span.para_ids.first().copied()));
            if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
                ms.last_synced_source_line = Some(line);
                ms.active_block_index = target.map(|(idx, _)| idx);
            }
            if let Some((_, Some(para_id))) = target {
                self.scroll_visual_to_paragraph_preserving_focus(para_id, cx);
            }
        } else {
            let Some(para_id) = self.doc_state().and_then(|s| s.caret).map(|p| p.para) else {
                return;
            };
            if Some(para_id) == last_para {
                return;
            }
            let target = self
                .doc_state()
                .and_then(|s| s.markdown_source.as_ref())
                .and_then(|ms| ms.source_map.span_for_para(para_id))
                .map(|span| (span.block_index, span.byte_range.start, span.line_range.start));
            if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
                ms.last_synced_visual_para = Some(para_id);
                ms.active_block_index = target.map(|(idx, _, _)| idx);
                if let Some((_, _, start_line)) = target {
                    ms.last_synced_source_line = Some(start_line);
                }
            }
            if let Some((_, start_byte, _)) = target {
                editor_entity.update(cx, |state, cx| {
                    state.set_selected_range(start_byte..start_byte, cx);
                });
            }
        }
    }

    /// Called from `Document::before_render`: flushes focus-transition syncs, detects
    /// WYSIWYG `FlowDocument` mutations, and synchronizes cursor/scroll position across panes.
    pub(crate) fn poll_markdown_bidirectional_sync(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_markdown_split_mode() {
            return;
        }
        let source_focused = self.is_markdown_source_focused(window, cx);
        let current_flow = self.presentation().flow.clone();
        let (was_focused, source_dirty, visual_dirty, flow_changed, flow_newly_mutated) = {
            let Some(ms) = self.doc_state().and_then(|s| s.markdown_source.as_ref()) else {
                return;
            };
            let flow_changed = match (&current_flow, &ms.last_synced_flow) {
                (Some(cur), Some(synced)) => !Arc::ptr_eq(cur, synced),
                (Some(_), None) => true,
                _ => false,
            };
            let flow_newly_mutated = match (&current_flow, &ms.last_seen_flow) {
                (Some(cur), Some(seen)) => !Arc::ptr_eq(cur, seen),
                (Some(_), None) => true,
                _ => false,
            };
            (
                ms.source_was_focused,
                ms.dirty,
                ms.visual_dirty,
                flow_changed,
                flow_newly_mutated,
            )
        };

        if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
            ms.source_was_focused = source_focused;
        }

        // Focus left Source -> Visual while Source has unsynced edits: flush immediately.
        if was_focused && !source_focused && source_dirty {
            let _ = self.sync_markdown_source_if_dirty(cx);
            return;
        }

        // Focus entered Visual -> Source while Visual has newer edits: flush immediately.
        if !was_focused && source_focused && !source_dirty && (visual_dirty || flow_changed) {
            self.sync_visual_to_markdown_source(window, cx);
            return;
        }

        // Visual side mutated to a new snapshot while Source is clean: schedule debounced Visual -> Source export.
        if !source_dirty && flow_changed && flow_newly_mutated {
            let scheduled = if let Some(ms) =
                self.doc_state_mut().and_then(|s| s.markdown_source.as_mut())
            {
                ms.visual_dirty = true;
                ms.authority = SyncAuthority::Visual;
                ms.last_seen_flow = current_flow;
                ms.sync_generation = ms.sync_generation.wrapping_add(1);
                true
            } else {
                false
            };
            if scheduled {
                self.schedule_visual_to_source_sync(window, cx);
            }
            return;
        }

        // When both sides are clean and synchronized, keep cursor & viewport aligned via SourceMap/LawSections.
        self.sync_split_cursor_and_scroll(source_focused, cx);
    }

    /// Export or surgically patch the current WYSIWYG `FlowDocument` back into the source
    /// `EditorState` (Markdown, Lawtext, or e-Gov XML), preserving untouched source formatting,
    /// scroll offset, and suppressing recursive change events.
    pub(crate) fn sync_visual_to_markdown_source(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(flow_arc) = self.presentation().flow.clone() else {
            return;
        };
        let Some((dialect, editor_entity, prev_text, prev_map, prev_synced_flow, orig_xml)) = self
            .doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|ms| {
                (
                    ms.dialect,
                    ms.editor.clone(),
                    ms.current_text.clone(),
                    ms.source_map.clone(),
                    ms.last_synced_flow.clone(),
                    ms.original_law_xml.clone(),
                )
            })
        else {
            return;
        };

        let (exported, next_map, next_sections, next_lint) = match dialect {
            SourceDialect::Markdown => {
                let (md, map) = if let Some(prev_flow) = prev_synced_flow.as_deref() {
                    ibis_formats::markdown::patch_markdown_from_visual_edit(
                        &prev_text,
                        prev_flow,
                        &prev_map,
                        &flow_arc,
                        &self.presentation().media,
                    )
                } else {
                    ibis_formats::markdown::export_markdown_with_source_map(
                        &flow_arc,
                        &self.presentation().media,
                    )
                };
                let lint = if self.is_law_document() {
                    ibis_lawtext::parse_markdown_law(&md)
                        .ok()
                        .map(|law| ibis_lawtext::lint_law(&law))
                } else {
                    None
                };
                (md, map, Vec::new(), lint)
            }
            SourceDialect::Lawtext | SourceDialect::LawXml => {
                let md = ibis_formats::markdown::export_markdown(
                    &flow_arc,
                    &self.presentation().media,
                );
                let law = match ibis_lawtext::parse_markdown_law(&md) {
                    Ok(law) => law,
                    Err(err) => {
                        let msg: SharedString = err.to_string().into();
                        if let Some(ms) =
                            self.doc_state_mut().and_then(|s| s.markdown_source.as_mut())
                        {
                            ms.last_error = Some(msg);
                            ms.visual_dirty = false;
                            ms.authority = SyncAuthority::Idle;
                        }
                        cx.notify();
                        return;
                    }
                };
                let lint = Some(ibis_lawtext::lint_law(&law));
                let rendered = if dialect == SourceDialect::Lawtext {
                    ibis_lawtext::to_lawtext(&law)
                } else {
                    let xml_base = orig_xml.as_deref().or_else(|| {
                        ibis_lawtext::is_law_xml(prev_text.as_bytes()).then_some(prev_text.as_str())
                    });
                    if let Some(base) = xml_base
                        && let Ok(patch) = ibis_lawtext::apply_law_source_edits(
                            base,
                            &md,
                            ibis_lawtext::LawEditFormat::Markdown,
                        )
                        && let Some(patched_xml) = patch.xml
                    {
                        patched_xml
                    } else {
                        ibis_lawtext::to_law_xml(&law)
                    }
                };
                let sections = law_section_spans(&rendered, dialect);
                (rendered, MarkdownSourceMap::default(), sections, lint)
            }
        };

        if exported == prev_text {
            if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
                ms.source_map = next_map;
                ms.law_sections = next_sections;
                ms.law_lint = next_lint;
                ms.last_synced_flow = Some(flow_arc.clone());
                ms.last_seen_flow = Some(flow_arc);
                ms.visual_dirty = false;
                ms.authority = SyncAuthority::Idle;
            }
            return;
        }

        let prev_scroll = editor_entity.read(cx).scroll_offset();
        if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
            ms.suppress_source_event = true;
        }
        editor_entity.update(cx, |state, cx| {
            state.set_value(exported.clone(), window, cx);
            state.set_scroll_offset(prev_scroll, cx);
        });
        if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
            ms.current_text = exported;
            ms.source_map = next_map;
            ms.law_sections = next_sections;
            ms.law_lint = next_lint;
            ms.dirty = false;
            ms.visual_dirty = false;
            ms.authority = SyncAuthority::Idle;
            ms.last_error = None;
            ms.last_synced_flow = Some(flow_arc.clone());
            ms.last_seen_flow = Some(flow_arc);
            ms.last_synced_visual_para = None;
            ms.suppress_source_event = false;
        }
        cx.notify();
    }

    /// Sync dirty Markdown / Lawtext / e-Gov XML source edits back into `FlowDocument` without
    /// leaving source view. Returns `true` if synced (or nothing to sync), `false` on parse error.
    pub(crate) fn sync_markdown_source_if_dirty(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((dialect, text, dirty)) = self
            .doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|m| (m.dialect, m.current_text.clone(), m.dirty))
        else {
            return true;
        };
        if !dirty {
            return true;
        }

        let (markdown_to_apply, law_lint, law_sections, updated_xml) = match dialect {
            SourceDialect::Markdown => {
                let lint = if self.is_law_document() {
                    ibis_lawtext::parse_markdown_law(&text)
                        .ok()
                        .map(|law| ibis_lawtext::lint_law(&law))
                } else {
                    None
                };
                (text.clone(), lint, Vec::new(), None)
            }
            SourceDialect::Lawtext => match ibis_lawtext::parse_lawtext(&text) {
                Ok(law) => {
                    let lint = Some(ibis_lawtext::lint_law(&law));
                    let md = ibis_lawtext::to_markdown(&law);
                    let sections = law_section_spans(&text, SourceDialect::Lawtext);
                    (md, lint, sections, None)
                }
                Err(err) => {
                    let msg: SharedString = err.to_string().into();
                    if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
                        ms.last_error = Some(msg.clone());
                    }
                    self.set_status(msg.to_string(), cx);
                    cx.notify();
                    return false;
                }
            },
            SourceDialect::LawXml => match ibis_lawtext::parse_law_xml(&text) {
                Ok(law) => {
                    let lint = Some(ibis_lawtext::lint_law(&law));
                    let md = ibis_lawtext::to_markdown(&law);
                    let sections = law_section_spans(&text, SourceDialect::LawXml);
                    (md, lint, sections, Some(text.clone()))
                }
                Err(err) => {
                    let msg: SharedString = err.to_string().into();
                    if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
                        ms.last_error = Some(msg.clone());
                    }
                    self.set_status(msg.to_string(), cx);
                    cx.notify();
                    return false;
                }
            },
        };

        let applied = self.replace_markdown_text(&markdown_to_apply, cx);
        if applied {
            self.relayout_now(cx);
            let synced_flow = self.presentation().flow.clone();
            if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
                ms.dirty = false;
                ms.visual_dirty = false;
                ms.authority = SyncAuthority::Idle;
                ms.last_synced_flow = synced_flow.clone();
                ms.last_seen_flow = synced_flow;
                ms.last_error = None;
                ms.last_synced_source_line = None;
                ms.last_synced_visual_para = None;
                ms.law_lint = law_lint;
                if dialect != SourceDialect::Markdown {
                    ms.law_sections = law_sections;
                }
                if let Some(xml) = updated_xml {
                    ms.original_law_xml = Some(xml);
                }
            }
        }
        applied
    }

    pub(crate) fn replace_markdown_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let Some(current_doc) = self.flow().cloned() else {
            return false;
        };
        let mut media = self.presentation().media.clone();
        let prev_media_len = media.len();
        #[cfg(not(target_family = "wasm"))]
        let base_dir = self.path.as_deref().and_then(|p| p.parent());
        #[cfg(target_family = "wasm")]
        let base_dir: Option<&std::path::Path> = None;
        let source = self.source.clone();
        let baseline_pres = self.presentation().clone();

        let parsed = ibis_formats::markdown::reparse_markdown_document_with_source_map(
            text,
            &current_doc,
            &mut media,
            source.as_deref(),
            Some(&baseline_pres),
            base_dir,
        );

        let (mut new_doc, new_map) = match parsed {
            Ok(pair) => pair,
            Err(err) => {
                let msg: SharedString = err.to_string().into();
                if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
                    ms.last_error = Some(msg.clone());
                }
                self.set_status(msg.to_string(), cx);
                cx.notify();
                return false;
            }
        };

        let mut media_remap = BTreeMap::new();
        for new_m in media.iter().skip(prev_media_len) {
            let new_id = self.document.ids().add_media(new_m.format, new_m.data.clone());
            let format = match new_m.format {
                ibis_doc_model::MediaFormat::Png => Some(ImageFormat::Png),
                ibis_doc_model::MediaFormat::Jpeg => Some(ImageFormat::Jpeg),
                ibis_doc_model::MediaFormat::Gif => Some(ImageFormat::Gif),
                ibis_doc_model::MediaFormat::Bmp => Some(ImageFormat::Bmp),
                ibis_doc_model::MediaFormat::Tiff => Some(ImageFormat::Tiff),
                ibis_doc_model::MediaFormat::Svg => Some(ImageFormat::Svg),
                _ => None,
            };
            if let Some(format) = format {
                self.resources
                    .images
                    .insert(new_id, Arc::new(Image::from_bytes(format, new_m.data.to_vec())));
            }
            if new_m.id != new_id {
                media_remap.insert(new_m.id, new_id);
            }
        }
        remap_document_media(&mut new_doc, &media_remap);

        let applied = self.run_flow(vec![FlowCommand::ReplaceDocument(Box::new(new_doc))], cx);
        if applied
            && let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut())
        {
            ms.source_map = new_map;
        }
        applied
    }

    /// Exit Markdown source / split view and return to the rendered Visual document view.
    /// Re-parses only when the source text was modified.
    pub(crate) fn exit_markdown_source_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_markdown_source_mode() {
            return true;
        }
        if !self.sync_markdown_source_if_dirty(cx) {
            return false;
        }
        if let Some(state) = self.doc_state_mut() {
            state.markdown_source = None;
        }
        if let Some(flow) = self.flow() {
            let chars = crate::doc_ui::count_characters(flow);
            if let Some(state) = self.doc_state_mut() {
                state.characters = chars;
            }
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
        window.refresh();
        true
    }

    /// Compact mode strip shown for Markdown / Lawtext / e-Gov XML source/split views:
    /// displays live sync status, active SourceMap/article indicator, live 法制執務 Lint badge,
    /// dialect switcher (`Lawtext | XML | MD`) for statutes, sync-scroll toggle,
    /// and `[Source | Split | Visual]` 3-way switcher.
    pub(crate) fn render_markdown_mode_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let mode = self.markdown_view_mode();
        let ms = self.doc_state().and_then(|s| s.markdown_source.as_ref());
        let dialect = self.source_dialect();
        let is_law = self.is_law_document() || dialect != SourceDialect::Markdown;

        let status_label = match ms {
            Some(ms) if ms.mode == MarkdownViewMode::Split && (ms.dirty || ms.visual_dirty) => {
                Some(crate::shell::i18n::text("docx.markdown-source.syncing"))
            }
            Some(ms) if ms.dirty => Some(crate::shell::i18n::text("docx.markdown-source.modified")),
            Some(_) => Some(crate::shell::i18n::text("docx.markdown-source.synced")),
            None => None,
        };

        let block_badge = ms
            .filter(|ms| ms.mode == MarkdownViewMode::Split)
            .and_then(|ms| {
                let idx = ms.active_block_index?;
                if ms.dialect != SourceDialect::Markdown {
                    let span = ms.law_sections.get(idx)?;
                    let start_l = span.line_range.start + 1;
                    let end_l = span.line_range.end.max(start_l);
                    let line_part = if start_l == end_l {
                        format!("L{start_l}")
                    } else {
                        format!("L{start_l}–{end_l}")
                    };
                    let label = span.title.as_deref().unwrap_or("条文");
                    Some(format!("{line_part} ↔ {label}"))
                } else {
                    let span = ms.source_map.span_for_block(idx)?;
                    let start_l = span.line_range.start + 1;
                    let end_l = span.line_range.end.max(start_l);
                    if start_l == end_l {
                        Some(format!("L{start_l} ↔ #{}", idx + 1))
                    } else {
                        Some(format!("L{start_l}–{end_l} ↔ #{}", idx + 1))
                    }
                }
            });

        let lint_badge = ms.and_then(|ms| ms.law_lint.as_ref()).map(|report| {
            let total = report.findings.len();
            let (label, is_err, is_warn) = if report.error_count > 0 {
                (format!("Lint ✕ {}", report.error_count), true, false)
            } else if report.warning_count > 0 {
                (format!("Lint ⚠ {}", report.warning_count), false, true)
            } else if total > 0 {
                (format!("Lint ℹ {total}"), false, false)
            } else {
                ("Lint ✓".to_string(), false, false)
            };
            let tooltip_text = if report.findings.is_empty() {
                "法制執務 Lint: 指摘事項なし".to_string()
            } else {
                report
                    .findings
                    .iter()
                    .take(6)
                    .map(|f| format!("[{}] {}: {}", f.code, f.location, f.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            (label, is_err, is_warn, tooltip_text)
        });

        let sync_scroll_enabled = ms.is_some_and(|ms| ms.sync_scroll_enabled);

        let mode_btn = |id: &'static str, label_key: &'static str, target: MarkdownViewMode| {
            let active = mode == target;
            div()
                .id(id)
                .px(px(9.))
                .py(px(3.))
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(11.5))
                .when(active, |d| {
                    d.bg(theme.background)
                        .border_1()
                        .border_color(theme.border)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                })
                .when(!active, |d| {
                    d.text_color(theme.muted_foreground)
                        .hover(|h| h.bg(theme.secondary).text_color(theme.foreground))
                })
                .child(crate::shell::i18n::text(label_key))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_markdown_view_mode(target, window, cx);
                }))
        };

        let dialect_btn = |id: &'static str, label: &'static str, target: SourceDialect| {
            let active = dialect == target;
            div()
                .id(id)
                .px(px(7.))
                .py(px(2.))
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(10.5))
                .when(active, |d| {
                    d.bg(theme.background)
                        .border_1()
                        .border_color(theme.border)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                })
                .when(!active, |d| {
                    d.text_color(theme.muted_foreground)
                        .hover(|h| h.bg(theme.secondary).text_color(theme.foreground))
                })
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_source_dialect(target, window, cx);
                }))
        };

        let title = self
            .file_name
            .clone()
            .unwrap_or_else(|| crate::shell::i18n::text("docx.view.markdown-source"));

        div()
            .id("markdown-mode-bar")
            .flex()
            .flex_row()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .px(px(12.))
            .py(px(4.))
            .bg(theme.title_bar)
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .when(is_law, |row| {
                        row.child(
                            div()
                                .px(px(5.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(theme.secondary)
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.foreground)
                                .child(dialect.badge_label()),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .truncate()
                            .child(title),
                    )
                    .when_some(status_label, |row, label| {
                        row.child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .child(label),
                        )
                    })
                    .when_some(block_badge, |row, badge| {
                        row.child(
                            div()
                                .px(px(6.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(theme.muted)
                                .text_size(px(10.5))
                                .text_color(theme.muted_foreground)
                                .child(badge),
                        )
                    })
                    .when_some(lint_badge, |row, (label, is_err, is_warn, tip)| {
                        row.child(
                            div()
                                .id("law-lint-badge")
                                .px(px(6.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .when(is_err, |d| {
                                    d.bg(theme.danger).text_color(theme.danger_foreground)
                                })
                                .when(!is_err && is_warn, |d| {
                                    d.bg(theme.secondary).text_color(theme.foreground)
                                })
                                .when(!is_err && !is_warn, |d| {
                                    d.bg(theme.muted).text_color(theme.muted_foreground)
                                })
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(tip.clone())
                                        .build(window, cx)
                                })
                                .child(label),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .when(is_law, |row| {
                        row.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(2.))
                                .p(px(2.))
                                .rounded(px(6.))
                                .bg(theme.muted)
                                .child(dialect_btn(
                                    "law-dialect-lawtext",
                                    "Lawtext",
                                    SourceDialect::Lawtext,
                                ))
                                .child(dialect_btn(
                                    "law-dialect-xml",
                                    "XML",
                                    SourceDialect::LawXml,
                                ))
                                .child(dialect_btn(
                                    "law-dialect-md",
                                    "MD",
                                    SourceDialect::Markdown,
                                )),
                        )
                    })
                    .when(mode == MarkdownViewMode::Split, |row| {
                        row.child(
                            div()
                                .id("markdown-sync-scroll-toggle")
                                .px(px(6.))
                                .py(px(3.))
                                .rounded(px(4.))
                                .cursor_pointer()
                                .text_size(px(11.5))
                                .when(sync_scroll_enabled, |d| {
                                    d.bg(theme.accent)
                                        .text_color(theme.accent_foreground)
                                        .font_weight(FontWeight::SEMIBOLD)
                                })
                                .when(!sync_scroll_enabled, |d| {
                                    d.text_color(theme.muted_foreground)
                                        .hover(|h| h.bg(theme.secondary).text_color(theme.foreground))
                                })
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "スクロール・カーソル連動 (SourceMap / 条文)",
                                    )
                                    .build(window, cx)
                                })
                                .child("⇅")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(ms) = this
                                        .doc_state_mut()
                                        .and_then(|s| s.markdown_source.as_mut())
                                    {
                                        ms.sync_scroll_enabled = !ms.sync_scroll_enabled;
                                        ms.last_synced_source_line = None;
                                        ms.last_synced_visual_para = None;
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(2.))
                            .p(px(2.))
                            .rounded(px(6.))
                            .bg(theme.muted)
                            .child(mode_btn(
                                "markdown-mode-source",
                                "docx.markdown-source.mode-source",
                                MarkdownViewMode::Source,
                            ))
                            .when(!self.compact_viewport, |bar| {
                                bar.child(mode_btn(
                                    "markdown-mode-split",
                                    "docx.markdown-source.mode-split",
                                    MarkdownViewMode::Split,
                                ))
                            })
                            .child(mode_btn(
                                "markdown-mode-visual",
                                "docx.markdown-source.mode-visual",
                                MarkdownViewMode::Visual,
                            )),
                    ),
            )
            .into_any_element()
    }

    /// Render only the Markdown source `Editor` pane (plus inline non-blocking error banner).
    pub(crate) fn render_markdown_source_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(ms) = self.doc_state().and_then(|s| s.markdown_source.as_ref()) else {
            return div().into_any_element();
        };
        let theme = cx.theme().clone();
        let readonly = self.is_read_only()
            || self.is_jtd_read_only()
            || self.is_viewing_mode()
            || self.is_protection_read_only();
        let last_error = ms.last_error.clone();
        let editor_entity = ms.editor.clone();

        let mut pane = div()
            .id("markdown-source-workspace")
            .key_context(MARKDOWN_SOURCE_CONTEXT)
            .on_action(cx.listener(|this, _: &crate::editing::ToggleMarkdownSource, window, cx| {
                this.toggle_markdown_source_mode(window, cx);
            }))
            .capture_key_down(cx.listener({
                let editor = editor_entity.clone();
                move |this, event: &KeyDownEvent, window, cx| {
                    if this.handle_inline_completion_key(editor.clone(), readonly, event, window, cx)
                    {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                }
            }))
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .bg(theme.background);

        if let Some(err) = last_error {
            pane = pane.child(
                div()
                    .px(px(12.))
                    .py(px(6.))
                    .bg(theme.danger)
                    .text_color(theme.danger_foreground)
                    .text_size(px(12.))
                    .child(err),
            );
        }

        pane.child(self.render_inline_completion_bar(
            "docx-markdown",
            editor_entity.clone(),
            readonly,
            cx,
        ))
        .child(
            div().flex_1().min_w_0().min_h_0().child(
                Editor::new(&editor_entity)
                    .h(relative(1.0))
                    .bordered(false)
                    .readonly(readonly),
            ),
        )
        .into_any_element()
    }

    /// Render Source-only view (mode bar + Markdown source editor).
    pub(crate) fn render_markdown_source_view(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(self.render_markdown_mode_bar(cx))
            .child(self.render_markdown_source_pane(cx))
            .into_any_element()
    }

    /// Render the bidirectional Split view: left editable Markdown source + draggable divider
    /// + right editable WYSIWYG document pane.
    pub(crate) fn render_markdown_split_view(
        &self,
        visual_pane: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(ms) = self.doc_state().and_then(|s| s.markdown_source.as_ref()) else {
            return visual_pane;
        };
        let theme = cx.theme().clone();
        let ratio = clamp_split_ratio(ms.split_ratio);
        let dragging = ms.split_drag.is_some();
        let width_cell = ms.split_container_width.clone();
        let measured_width = width_cell.get();
        let divider_w = 5.0_f32;

        let left_wrapper = if measured_width > 200.0 {
            let left_px = ((measured_width - divider_w) * ratio).clamp(180.0, measured_width - 180.0);
            div().w(px(left_px)).flex_none()
        } else {
            div().flex_1()
        };

        let weak = cx.entity().downgrade();
        let divider = div()
            .id("markdown-split-divider")
            .relative()
            .flex_none()
            .w(px(divider_w))
            .h_full()
            .bg(theme.border)
            .hover(|h| h.bg(theme.accent))
            .cursor_col_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if event.click_count == 2 {
                        if let Some(ms) = this.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
                            ms.split_ratio = DEFAULT_SPLIT_RATIO;
                            ms.split_drag = None;
                        }
                        cx.notify();
                    } else if let Some(ms) =
                        this.doc_state_mut().and_then(|s| s.markdown_source.as_mut())
                    {
                        ms.split_drag = Some((f32::from(event.position.x), ms.split_ratio));
                        cx.notify();
                    }
                }),
            )
            .when(dragging, |el| {
                let tracker = weak.clone();
                el.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            let moving = tracker.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture {
                                    let _ = moving.update(cx, |this, cx| {
                                        if let Some(ms) = this
                                            .doc_state_mut()
                                            .and_then(|s| s.markdown_source.as_mut())
                                            && let Some((x0, r0)) = ms.split_drag
                                        {
                                            let total = ms.split_container_width.get().max(300.0);
                                            let dx = f32::from(event.position.x) - x0;
                                            ms.split_ratio = clamp_split_ratio(r0 + dx / total);
                                            cx.notify();
                                        }
                                    });
                                }
                            });
                            let released = tracker.clone();
                            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture {
                                    let _ = released.update(cx, |this, cx| {
                                        if let Some(ms) = this
                                            .doc_state_mut()
                                            .and_then(|s| s.markdown_source.as_mut())
                                            && ms.split_drag.take().is_some()
                                        {
                                            cx.notify();
                                        }
                                    });
                                }
                            });
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            });

        div()
            .id("markdown-split-container")
            .relative()
            .flex()
            .flex_row()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(crate::shell::editor::bounds_observer(move |bounds, _, _| {
                width_cell.set(f32::from(bounds.size.width));
            }))
            .child(
                left_wrapper
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .overflow_hidden()
                    .child(self.render_markdown_source_pane(cx)),
            )
            .child(divider)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .overflow_hidden()
                    .child(visual_pane),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SPLIT_RATIO, MAX_SPLIT_RATIO, MIN_SPLIT_RATIO, MarkdownViewMode,
        PureRustMarkdownHighlighter, TokenKind, clamp_split_ratio,
    };
    use gpui_kit::component::input::{HighlightStyleResolver, InputHighlighter as _};
    use gpui_kit::{FontWeight, HighlightStyle};
    use ibis_doc_model::{Block, FlowDocument, FlowRunKind};

    #[test]
    fn highlighter_identifies_frontmatter_headings_fences_and_inline() {
        let md = "\
---
title: Demo
---

# Heading 1

Normal **bold** and *italic* and `code` and ~~strike~~ and [link](https://example.com).

```mermaid
graph TD
  A --> B
```

> Quoted line

- [x] Done item
";
        let mut h = PureRustMarkdownHighlighter::new();
        h.reparse(md);

        assert!(h.spans.iter().any(|s| s.kind == TokenKind::FrontmatterDelimiter));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::FrontmatterKey));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::Heading));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::Strong));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::Emphasis));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::Code));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::Strike));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::LinkText));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::LinkUri));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::CodeFence));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::Quote));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::ListMarker));

        // Frontmatter (0..2), fenced code block (8..11), heading section (4..last)
        assert!(h.folds.iter().any(|f| f.start_line == 0 && f.end_line == 2));
        assert!(h.folds.iter().any(|f| f.start_line == 8 && f.end_line == 11));
        assert!(h.folds.iter().any(|f| f.start_line == 4));
    }

    #[test]
    fn highlighter_styles_cover_entire_requested_range_without_overlap() {
        struct DummyResolver;
        impl HighlightStyleResolver for DummyResolver {
            fn style(&self, _: &str) -> Option<HighlightStyle> {
                Some(HighlightStyle {
                    font_weight: Some(FontWeight::BOLD),
                    ..HighlightStyle::default()
                })
            }
        }

        let md = "# Title\n\nHello **world** `ok`!\n";
        let mut h = PureRustMarkdownHighlighter::new();
        h.reparse(md);

        let full = 0..md.len();
        let runs = h.styles(&full, &DummyResolver);
        assert!(!runs.is_empty());
        assert_eq!(runs.first().unwrap().0.start, 0);
        assert_eq!(runs.last().unwrap().0.end, md.len());
        for w in runs.windows(2) {
            assert_eq!(w[0].0.end, w[1].0.start);
        }
    }

    #[test]
    fn split_ratio_clamps_bounds_and_handles_nan() {
        assert_eq!(clamp_split_ratio(0.05), MIN_SPLIT_RATIO);
        assert_eq!(clamp_split_ratio(0.95), MAX_SPLIT_RATIO);
        assert_eq!(clamp_split_ratio(0.5), 0.5);
        assert_eq!(clamp_split_ratio(f32::NAN), DEFAULT_SPLIT_RATIO);
        assert_eq!(MarkdownViewMode::default(), MarkdownViewMode::Visual);
    }

    #[test]
    fn bidirectional_markdown_roundtrip_preserves_mermaid_and_visual_edits() {
        let initial_md = "\
# Architecture

Initial note.

```mermaid
graph LR
  Source --> Preview
```
";
        let mut media = Vec::new();
        let mut doc = ibis_formats::markdown::reparse_markdown_document(
            initial_md,
            &FlowDocument::default(),
            &mut media,
            None,
            None,
            None,
        )
        .expect("reparse markdown with mermaid");

        // Simulate a WYSIWYG edit on the paragraph block.
        let mut edited = false;
        for block in &mut doc.body {
            if let Block::Paragraph(p) = block {
                for run in &mut p.runs {
                    if let FlowRunKind::Text(text) = &mut run.kind
                        && text.contains("Initial note.")
                    {
                        *text = "Edited from WYSIWYG.".to_owned();
                        edited = true;
                    }
                }
            }
        }
        assert!(edited, "expected paragraph run to edit");

        // Export back to Markdown source (Visual -> Source direction).
        let exported = ibis_formats::markdown::export_markdown(&doc, &media);
        assert!(exported.contains("# Architecture"), "heading preserved: {exported}");
        assert!(exported.contains("Edited from WYSIWYG."), "WYSIWYG edit exported: {exported}");
        assert!(exported.contains("```mermaid"), "mermaid fence preserved: {exported}");
        assert!(exported.contains("Source --> Preview"), "mermaid body preserved: {exported}");
    }

    #[test]
    fn source_map_bidirectional_cursor_and_surgical_patch_roundtrip() {
        let initial_md = "\
# Spec

* keep star bullet

Editable paragraph.

```mermaid
graph LR
  A --> B
```
";
        let mut media = Vec::new();
        let (doc, map) = ibis_formats::markdown::reparse_markdown_document_with_source_map(
            initial_md,
            &FlowDocument::default(),
            &mut media,
            None,
            None,
            None,
        )
        .expect("reparse with source map");

        // Source line 4 ("Editable paragraph.") resolves to a paragraph ID, and back to line 4.
        let para_span = map.span_for_line(4).expect("line 4 span");
        assert_eq!(&initial_md[para_span.byte_range.clone()], "Editable paragraph.");
        let para_id = para_span.para_ids[0];
        assert_eq!(map.span_for_para(para_id).unwrap().line_range, 4..5);

        // Edit only that paragraph in WYSIWYG and patch back to Markdown.
        let mut edited = doc.clone();
        for block in &mut edited.body {
            if let Block::Paragraph(p) = block
                && p.id == para_id
            {
                for run in &mut p.runs {
                    if let FlowRunKind::Text(t) = &mut run.kind {
                        *t = "Edited cleanly.".to_owned();
                    }
                }
            }
        }
        let (patched, patched_map) = ibis_formats::markdown::patch_markdown_from_visual_edit(
            initial_md,
            &doc,
            &map,
            &edited,
            &media,
        );
        assert!(patched.contains("* keep star bullet"), "untouched star bullet preserved:\n{patched}");
        assert!(patched.contains("Edited cleanly."), "visual edit applied:\n{patched}");
        assert!(patched.contains("```mermaid\ngraph LR\n  A --> B\n```"), "mermaid preserved:\n{patched}");
        let new_span = patched_map.span_for_para(para_id).expect("updated para span");
        assert_eq!(&patched[new_span.byte_range.clone()], "Edited cleanly.");
    }

    #[test]
    fn lawtext_and_egov_xml_bidirectional_roundtrip_with_surgical_xml_patch_and_lint() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Law Era="Reiwa" Lang="ja" LawType="Act" Num="001" Year="07">
  <LawNum>令和七年法律第一号</LawNum>
  <LawBody>
    <LawTitle>デジタル手続モデル法</LawTitle>
    <MainProvision>
      <Article Num="1">
        <ArticleCaption>（目的）</ArticleCaption>
        <ArticleTitle>第一条</ArticleTitle>
        <Paragraph Num="1">
          <ParagraphNum></ParagraphNum>
          <ParagraphSentence><Sentence>この法律は、行政手続の標準化を定める。</Sentence></ParagraphSentence>
        </Paragraph>
      </Article>
      <Article Num="2">
        <ArticleCaption>（適用）</ArticleCaption>
        <ArticleTitle>第二条</ArticleTitle>
        <Paragraph Num="1">
          <ParagraphNum></ParagraphNum>
          <ParagraphSentence><Sentence>第一条の規定を準用する。</Sentence></ParagraphSentence>
        </Paragraph>
      </Article>
    </MainProvision>
  </LawBody>
</Law>"#;

        // 1. e-Gov XML -> LawDoc -> Markdown -> FlowDocument
        let law = ibis_lawtext::parse_law_xml(xml).expect("parse law xml");
        let lint0 = ibis_lawtext::lint_law(&law);
        assert_eq!(lint0.error_count, 0);
        let md = ibis_lawtext::to_markdown(&law);
        let mut media = Vec::new();
        let (doc, _) = ibis_formats::markdown::reparse_markdown_document_with_source_map(
            &md,
            &FlowDocument::default(),
            &mut media,
            None,
            None,
            None,
        )
        .expect("reparse law markdown");

        // 2. Visual edit on FlowDocument -> Markdown -> Lawtext & surgical XML patch
        let mut edited = doc.clone();
        let mut changed = false;
        for block in &mut edited.body {
            if let Block::Paragraph(p) = block {
                for run in &mut p.runs {
                    if let FlowRunKind::Text(t) = &mut run.kind
                        && t.contains("行政手続の標準化")
                    {
                        *t = t.replace("行政手続の標準化", "電子手続の標準化");
                        changed = true;
                    }
                }
            }
        }
        assert!(changed, "expected article 1 text in FlowDocument");

        let edited_md = ibis_formats::markdown::export_markdown(&edited, &media);
        let edited_law = ibis_lawtext::parse_markdown_law(&edited_md).expect("parse edited law md");
        let exported_lawtext = ibis_lawtext::to_lawtext(&edited_law);
        assert!(exported_lawtext.contains("電子手続の標準化"));
        assert!(exported_lawtext.contains("第一条"));
        assert!(exported_lawtext.contains("第二条"));

        let patch = ibis_lawtext::apply_law_source_edits(
            xml,
            &edited_md,
            ibis_lawtext::LawEditFormat::Markdown,
        )
        .expect("surgical xml patch");
        let patched_xml = patch.xml.expect("surgical patch xml produced");
        assert!(patched_xml.contains("電子手続の標準化"));
        assert!(patched_xml.contains("Era=\"Reiwa\""));
        assert!(patched_xml.contains("<ArticleTitle>第二条</ArticleTitle>"));
    }
}
