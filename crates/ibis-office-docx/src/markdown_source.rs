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
use ibis_doc_model::{Block, FlowCommand, FlowDocument, FlowRunKind, MediaId};

use crate::ext::*;
use crate::shell::editor::SlideEditor;

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

/// Active state while the document is shown in Markdown Split or Source view.
pub(crate) struct MarkdownSourceState {
    pub(crate) mode: MarkdownViewMode,
    pub(crate) editor: Entity<EditorState>,
    /// Cached latest UTF-8 text of `editor` so save paths without `App` context
    /// (`FormatEditor::bytes_for_path`) can write the exact source directly.
    pub(crate) current_text: String,
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

    /// Cached Markdown source text when the source editor is authoritative or synchronized
    /// with the current `FlowDocument`. Returns `None` if the WYSIWYG side has newer
    /// unsynchronized edits so save paths fall back to exporting the latest `FlowDocument`.
    pub(crate) fn markdown_source_text(&self) -> Option<&str> {
        let ms = self.doc_state()?.markdown_source.as_ref()?;
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

    /// Toggle between Visual view and Markdown Split/Source view (⌘⇧M / View > Markdown Source).
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

    /// Switch from rendered view to Markdown source view (defaults to Split).
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
        if mode == MarkdownViewMode::Visual || self.is_markdown_source_mode() {
            return;
        }
        let Some(flow_arc) = self.presentation().flow.clone() else {
            return;
        };
        let initial = ibis_formats::markdown::export_markdown(&flow_arc, &self.presentation().media);
        let readonly = self.is_read_only()
            || self.is_jtd_read_only()
            || self.is_viewing_mode()
            || self.is_protection_read_only();

        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("markdown")
                .line_number(true)
                .soft_wrap(true)
                .folding(true)
                .searchable(true)
                .default_value(initial.clone());
            state.set_highlighter_factory(markdown_highlighter_factory(), cx);
            state.set_readonly(readonly, cx);
            state
        });

        let subscription = cx.subscribe(&editor, |this, ed, event: &InputEvent, cx| {
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
                cx.notify();
            }
        });

        editor.update(cx, |s, cx| s.focus(window, cx));

        if let Some(state) = self.doc_state_mut() {
            state.markdown_source = Some(MarkdownSourceState {
                mode,
                editor,
                current_text: initial,
                dirty: false,
                visual_dirty: false,
                authority: SyncAuthority::Idle,
                last_synced_flow: Some(flow_arc.clone()),
                last_seen_flow: Some(flow_arc),
                sync_generation: 0,
                suppress_source_event: false,
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

    /// Called from `Document::before_render`: flushes focus-transition syncs and detects
    /// WYSIWYG `FlowDocument` mutations to keep the left Markdown source pane in sync.
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
        }
    }

    /// Export the current WYSIWYG `FlowDocument` back into the Markdown `EditorState`,
    /// preserving the source editor's scroll offset and suppressing recursive change events.
    pub(crate) fn sync_visual_to_markdown_source(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(flow_arc) = self.presentation().flow.clone() else {
            return;
        };
        let Some((editor_entity, prev_text)) = self
            .doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|ms| (ms.editor.clone(), ms.current_text.clone()))
        else {
            return;
        };

        let exported = ibis_formats::markdown::export_markdown(&flow_arc, &self.presentation().media);
        if exported == prev_text {
            if let Some(ms) = self.doc_state_mut().and_then(|s| s.markdown_source.as_mut()) {
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
            ms.dirty = false;
            ms.visual_dirty = false;
            ms.authority = SyncAuthority::Idle;
            ms.last_error = None;
            ms.last_synced_flow = Some(flow_arc.clone());
            ms.last_seen_flow = Some(flow_arc);
            ms.suppress_source_event = false;
        }
        cx.notify();
    }

    /// Sync dirty Markdown source edits back into `FlowDocument` without leaving source view.
    /// Returns `true` if synced (or nothing to sync), `false` on parse error.
    pub(crate) fn sync_markdown_source_if_dirty(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((text, dirty)) = self
            .doc_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|m| (m.current_text.clone(), m.dirty))
        else {
            return true;
        };
        if !dirty {
            return true;
        }

        let applied = self.replace_markdown_text(&text, cx);
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

        let parsed = ibis_formats::markdown::reparse_markdown_document(
            text,
            &current_doc,
            &mut media,
            source.as_deref(),
            Some(&baseline_pres),
            base_dir,
        );

        let mut new_doc = match parsed {
            Ok(doc) => doc,
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

        self.run_flow(vec![FlowCommand::ReplaceDocument(Box::new(new_doc))], cx)
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

    /// Compact mode strip shown for Markdown source/split views and `.md` files:
    /// displays live sync status and `[Source | Split | Visual]` 3-way switcher.
    pub(crate) fn render_markdown_mode_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let mode = self.markdown_view_mode();
        let ms = self.doc_state().and_then(|s| s.markdown_source.as_ref());

        let status_label = match ms {
            Some(ms) if ms.mode == MarkdownViewMode::Split && (ms.dirty || ms.visual_dirty) => {
                Some(crate::shell::i18n::text("docx.markdown-source.syncing"))
            }
            Some(ms) if ms.dirty => Some(crate::shell::i18n::text("docx.markdown-source.modified")),
            Some(_) => Some(crate::shell::i18n::text("docx.markdown-source.synced")),
            None => None,
        };

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
                    }),
            )
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
                    .child(mode_btn(
                        "markdown-mode-split",
                        "docx.markdown-source.mode-split",
                        MarkdownViewMode::Split,
                    ))
                    .child(mode_btn(
                        "markdown-mode-visual",
                        "docx.markdown-source.mode-visual",
                        MarkdownViewMode::Visual,
                    )),
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

        pane.child(
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
}
