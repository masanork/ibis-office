// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Shared pure-Rust Markdown Split View foundation for Documents, Marp Presentations,
//! and Workbook Spreadsheets.
//!
//! Provides:
//! - [`MarkdownViewMode`] (`Visual` / `Split` / `Source`) and [`SyncAuthority`]
//! - Zero-C [`markdown_highlighter_factory`] (`InputHighlighter`) with Markdown, YAML frontmatter,
//!   GFM table, HTML comment marker, and fenced code block highlighting + folding
//! - Section span parsers for Marp slides ([`marp_slide_spans`]) and Workbook sheets
//!   ([`workbook_sheet_spans`]) to power slide/sheet-level bidirectional cursor & scroll sync
//! - [`MarkdownSplitHost`] state and shared UI render helpers (`[Source | Split | Visual]` bar,
//!   editable Markdown source pane, and draggable split divider container).

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{
    Editor, EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter,
    InputHighlighterFactory, Rope,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_doc_model::SlideId;

use crate::editor::SlideEditor;

pub const MARKDOWN_SOURCE_CONTEXT: &str = "MarkdownSource";
pub const SOURCE_TO_VISUAL_DEBOUNCE_MS: u64 = 250;
pub const VISUAL_TO_SOURCE_DEBOUNCE_MS: u64 = 200;
pub const DEFAULT_SPLIT_RATIO: f32 = 0.45;
pub const MIN_SPLIT_RATIO: f32 = 0.20;
pub const MAX_SPLIT_RATIO: f32 = 0.80;

/// View mode for bidirectional Markdown editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MarkdownViewMode {
    /// Rendered WYSIWYG editor only.
    #[default]
    Visual,
    /// Side-by-side: editable Markdown source (left) + live editable WYSIWYG editor (right).
    Split,
    /// Editable Markdown source only.
    Source,
}

/// Which side currently holds unsynced edits in bidirectional Split mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SyncAuthority {
    #[default]
    Idle,
    Source,
    Visual,
}

pub fn clamp_split_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
    } else {
        DEFAULT_SPLIT_RATIO
    }
}

/// A line/byte span corresponding to a logical slide (Marp) or worksheet (Workbook) in Markdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionSpan {
    /// 0-based slide or visible sheet index.
    pub index: usize,
    /// 0-based half-open line range (`start..end`).
    pub line_range: Range<usize>,
    /// UTF-8 byte range (`start..end`) in the Markdown source.
    pub byte_range: Range<usize>,
    /// Optional human-readable heading or sheet name extracted from this section.
    pub title: Option<String>,
}

/// Shared state for a presentation (Marp) or workbook Markdown Split / Source view.
pub struct MarkdownSplitHost {
    pub mode: MarkdownViewMode,
    pub editor: Entity<EditorState>,
    /// Cached latest UTF-8 text of `editor` so save paths (`bytes_for_path`) can write exact source.
    pub current_text: String,
    /// Logical section spans (slides for Marp, sheets for Workbook) for cursor/scroll sync.
    pub sections: Vec<SectionSpan>,
    /// True once the user modified the source since entering source view or last sync.
    pub dirty: bool,
    /// True while the Visual model has edits pending export back to `editor`.
    pub visual_dirty: bool,
    /// Which side currently owns pending synchronization.
    pub authority: SyncAuthority,
    /// `Document::revision()` value at the last synchronized state.
    pub last_synced_revision: u64,
    /// `Document::revision()` value for which a Visual -> Source debounce was already scheduled.
    pub last_seen_revision: u64,
    /// Monotonic counter used to supersede stale debounced sync tasks.
    pub sync_generation: u64,
    /// Guard flag to suppress `InputEvent::Change` during programmatic `Visual -> Source` updates.
    pub suppress_source_event: bool,
    /// Whether cursor/scroll synchronization across the Split divider is enabled.
    pub sync_scroll_enabled: bool,
    /// Last 0-based source line already synced to the Visual slide/sheet.
    pub last_synced_source_line: Option<usize>,
    /// Last Visual `SlideId` already synced to the Source editor.
    pub last_synced_slide_id: Option<SlideId>,
    /// Active 0-based slide/sheet section index under the current cursor.
    pub active_section_index: Option<usize>,
    /// Left pane width ratio in Split mode (`MIN_SPLIT_RATIO..=MAX_SPLIT_RATIO`).
    pub split_ratio: f32,
    /// Active divider drag state: `(start_pointer_x, start_ratio)`.
    pub split_drag: Option<(f32, f32)>,
    /// Last measured width of the split container in logical pixels.
    pub split_container_width: Rc<Cell<f32>>,
    /// Non-fatal parse error shown inline while preserving last valid Visual model.
    pub last_error: Option<SharedString>,
    /// Whether the source editor had focus on the previous frame (for focus-transition flushes).
    pub source_was_focused: bool,
    pub _subscription: Subscription,
}

// ---------------------------------------------------------------------------
// Marp Slide & Workbook Sheet Span Parsers
// ---------------------------------------------------------------------------

struct LineSlice {
    idx: usize,
    start: usize,
    end: usize,
    next: usize,
}

fn split_line_slices(text: &str) -> Vec<LineSlice> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut idx = 0usize;
    while pos <= len {
        let start = pos;
        while pos < len && bytes[pos] != b'\n' {
            pos += 1;
        }
        let end = if pos > start && bytes[pos - 1] == b'\r' {
            pos - 1
        } else {
            pos
        };
        let next = if pos < len { pos + 1 } else { len };
        out.push(LineSlice {
            idx,
            start,
            end,
            next,
        });
        idx += 1;
        if pos >= len {
            break;
        }
        pos += 1;
    }
    out
}

fn detect_frontmatter_end(text: &str, lines: &[LineSlice]) -> Option<usize> {
    let first = lines.first()?;
    let first_line = text[first.start..first.end].trim_start_matches('\u{feff}').trim();
    if first_line != "---" {
        return None;
    }
    for line in &lines[1..] {
        let t = text[line.start..line.end].trim();
        if t == "---" || t == "..." {
            return Some(line.idx);
        }
    }
    None
}

fn update_fence_state(trimmed: &str, leading: usize, in_fence: &mut Option<(u8, usize)>) -> bool {
    if let Some((fch, flen)) = *in_fence {
        let count = trimmed.bytes().take_while(|&b| b == fch).count();
        if count >= flen && trimmed[count..].trim().is_empty() {
            *in_fence = None;
        }
        return true;
    }
    if leading <= 3 && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
        let fch = trimmed.as_bytes()[0];
        let count = trimmed.bytes().take_while(|&b| b == fch).count();
        if count >= 3 {
            *in_fence = Some((fch, count));
            return true;
        }
    }
    false
}

fn extract_heading_title(trimmed: &str) -> Option<String> {
    if !trimmed.starts_with('#') {
        return None;
    }
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &trimmed[hashes..];
    if rest.is_empty() || matches!(rest.as_bytes()[0], b' ' | b'\t') {
        let title = rest.trim().trim_end_matches('#').trim();
        if !title.is_empty() {
            return Some(title.to_string());
        }
    }
    None
}

/// Split a Marp / slide Markdown document into 0-based slide [`SectionSpan`]s.
///
/// - Leading YAML frontmatter (`--- ... ---`) is kept inside slide 0 and does not create a slide break.
/// - `---` inside fenced code blocks (` ``` ` / `~~~`) does not split slides.
/// - Standalone `---` horizontal rules split slides.
pub fn marp_slide_spans(text: &str) -> Vec<SectionSpan> {
    let lines = split_line_slices(text);
    if lines.is_empty() {
        return vec![SectionSpan {
            index: 0,
            line_range: 0..1,
            byte_range: 0..0,
            title: None,
        }];
    }

    let fm_end = detect_frontmatter_end(text, &lines);
    let body_start_idx = fm_end.map_or(0, |end| (end + 1).min(lines.len()));
    let mut in_fence: Option<(u8, usize)> = None;

    let mut slide_starts: Vec<usize> = vec![0];
    for line in &lines[body_start_idx..] {
        let raw = &text[line.start..line.end];
        let trimmed = raw.trim_start();
        let leading = raw.len() - trimmed.len();
        if update_fence_state(trimmed, leading, &mut in_fence) {
            continue;
        }
        if trimmed.trim_end() == "---" {
            let next_idx = (line.idx + 1).min(lines.len().saturating_sub(1));
            slide_starts.push(next_idx);
        }
    }

    let total_lines = lines.len();
    let mut spans = Vec::with_capacity(slide_starts.len());
    for (slide_i, &start_line) in slide_starts.iter().enumerate() {
        let next_start = slide_starts
            .get(slide_i + 1)
            .copied()
            .unwrap_or(total_lines);
        // Exclude the `---` delimiter line from the previous slide's span when possible.
        let end_line = if slide_i + 1 < slide_starts.len() && next_start > start_line {
            (next_start - 1).max(start_line + 1)
        } else {
            total_lines.max(start_line + 1)
        };
        let scan_start = if slide_i == 0 {
            body_start_idx.min(end_line)
        } else {
            start_line
        };
        let mut title = None;
        let mut fence = None;
        for line in &lines[scan_start..end_line.min(total_lines)] {
            let raw = &text[line.start..line.end];
            let trimmed = raw.trim_start();
            let leading = raw.len() - trimmed.len();
            if update_fence_state(trimmed, leading, &mut fence) {
                continue;
            }
            if let Some(h) = extract_heading_title(trimmed) {
                title = Some(h);
                break;
            }
        }
        let byte_start = lines.get(start_line).map_or(text.len(), |l| l.start);
        let byte_end = lines
            .get(end_line.saturating_sub(1))
            .map_or(text.len(), |l| l.next);
        spans.push(SectionSpan {
            index: slide_i,
            line_range: start_line..end_line,
            byte_range: byte_start..byte_end,
            title,
        });
    }
    spans
}

const XLSX_SHEET_MARKER_PREFIX: &str = "<!-- xlsx-sheet ";

fn parse_xlsx_sheet_marker_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix(XLSX_SHEET_MARKER_PREFIX)?;
    let encoded = rest.strip_suffix(" -->")?;
    let val: serde_json::Value = serde_json::from_str(encoded).ok()?;
    if let Some(s) = val.as_str() {
        return Some(s.to_string());
    }
    val.get("name")?.as_str().map(ToString::to_string)
}

/// Split a Workbook Markdown document into 0-based visible sheet [`SectionSpan`]s.
///
/// - Recognizes `<!-- xlsx-sheet ... -->` markers outside fenced code blocks.
/// - Falls back to a single sheet span when no `<!-- xlsx-sheet ... -->` markers are present.
pub fn workbook_sheet_spans(text: &str) -> Vec<SectionSpan> {
    let lines = split_line_slices(text);
    if lines.is_empty() {
        return vec![SectionSpan {
            index: 0,
            line_range: 0..1,
            byte_range: 0..0,
            title: None,
        }];
    }

    let fm_end = detect_frontmatter_end(text, &lines);
    let body_start_idx = fm_end.map_or(0, |end| (end + 1).min(lines.len()));
    let mut in_fence: Option<(u8, usize)> = None;

    let mut markers: Vec<(usize, Option<String>)> = Vec::new();
    for line in &lines[body_start_idx..] {
        let raw = &text[line.start..line.end];
        let trimmed = raw.trim_start();
        let leading = raw.len() - trimmed.len();
        if update_fence_state(trimmed, leading, &mut in_fence) {
            continue;
        }
        let trimmed_end = trimmed.trim_end();
        if trimmed_end.starts_with(XLSX_SHEET_MARKER_PREFIX) {
            markers.push((line.idx, parse_xlsx_sheet_marker_name(trimmed_end)));
        }
    }

    let total_lines = lines.len();
    if markers.is_empty() {
        let mut title = None;
        let mut fence = None;
        for line in &lines[body_start_idx..] {
            let raw = &text[line.start..line.end];
            let trimmed = raw.trim_start();
            let leading = raw.len() - trimmed.len();
            if update_fence_state(trimmed, leading, &mut fence) {
                continue;
            }
            if let Some(h) = extract_heading_title(trimmed) {
                title = Some(h);
                break;
            }
        }
        let byte_start = lines.get(body_start_idx).map_or(0, |l| l.start);
        return vec![SectionSpan {
            index: 0,
            line_range: 0..total_lines.max(1),
            byte_range: byte_start..text.len(),
            title,
        }];
    }

    let mut spans = Vec::with_capacity(markers.len());
    for (sheet_i, (start_line, marker_name)) in markers.iter().enumerate() {
        let span_start = if sheet_i == 0 { 0 } else { *start_line };
        let end_line = markers
            .get(sheet_i + 1)
            .map_or(total_lines, |(next_start, _)| *next_start)
            .max(span_start + 1);
        let title = marker_name.clone().or_else(|| {
            let mut fence = None;
            for line in &lines[*start_line..end_line.min(total_lines)] {
                let raw = &text[line.start..line.end];
                let trimmed = raw.trim_start();
                let leading = raw.len() - trimmed.len();
                if update_fence_state(trimmed, leading, &mut fence) {
                    continue;
                }
                if let Some(h) = extract_heading_title(trimmed) {
                    return Some(h);
                }
            }
            None
        });
        let byte_start = lines.get(*start_line).map_or(text.len(), |l| l.start);
        let byte_end = lines
            .get(end_line.saturating_sub(1))
            .map_or(text.len(), |l| l.next);
        spans.push(SectionSpan {
            index: sheet_i,
            line_range: span_start..end_line,
            byte_range: byte_start..byte_end,
            title,
        });
    }
    spans
}

/// Find the [`SectionSpan`] containing the 0-based `line` (clamping to the nearest span).
pub fn section_for_line(spans: &[SectionSpan], line: usize) -> Option<&SectionSpan> {
    if spans.is_empty() {
        return None;
    }
    if let Some(exact) = spans
        .iter()
        .find(|s| line >= s.line_range.start && line < s.line_range.end)
    {
        return Some(exact);
    }
    spans
        .iter()
        .rfind(|s| s.line_range.start <= line)
        .or_else(|| spans.first())
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
    HtmlComment,
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
            TokenKind::Quote | TokenKind::HtmlComment => {
                resolver.style("comment").unwrap_or_default()
            }
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
                resolver.style("keyword").or_else(|| resolver.style("comment")).unwrap_or_default()
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

/// Pure-Rust Markdown highlighter (zero C / tree-sitter deps) shared across
/// Documents, Marp Presentations, and Workbook Spreadsheets.
pub struct PureRustMarkdownHighlighter {
    spans: Vec<Span>,
    folds: Vec<FoldRange>,
}

impl PureRustMarkdownHighlighter {
    pub fn new() -> Self {
        Self {
            spans: Vec::new(),
            folds: Vec::new(),
        }
    }

    pub fn reparse(&mut self, text: &str) {
        self.spans.clear();
        self.folds.clear();

        let line_starts = split_line_slices(text);
        let mut in_frontmatter = false;
        let mut frontmatter_start_line: Option<usize> = None;
        let mut in_fence: Option<(u8, usize, usize)> = None;
        let mut heading_stack: Vec<(usize, usize)> = Vec::new();

        for slice in &line_starts {
            let line_idx = slice.idx;
            let start = slice.start;
            let end = slice.end;
            let line = &text[start..end];
            let trimmed = line.trim_start();
            let leading = line.len() - trimmed.len();

            // 1. YAML frontmatter at top of document
            if line_idx == 0 && line.trim_start_matches('\u{feff}').trim() == "---" {
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

            // 2. Fenced code blocks (``` or ~~~, including Mermaid diagrams)
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

            // 3. Standalone HTML comment lines (e.g. `<!-- xlsx-sheet ... -->`, Marp directives)
            if trimmed.starts_with("<!--") && trimmed.trim_end().ends_with("-->") {
                self.spans.push(Span {
                    range: start..end,
                    kind: TokenKind::HtmlComment,
                });
                continue;
            }

            // 4. ATX Headings (# .. ######)
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

            // 5. Horizontal rule / Marp slide separator (---, ***, ___)
            if is_horizontal_rule(trimmed) {
                self.spans.push(Span {
                    range: start..end,
                    kind: TokenKind::HorizontalRule,
                });
                continue;
            }

            // 6. Blockquote (> ...)
            if trimmed.starts_with('>') {
                self.spans.push(Span {
                    range: start..end,
                    kind: TokenKind::Quote,
                });
                continue;
            }

            // 7. Table separator row (|---|---|)
            if is_table_separator(trimmed) {
                self.spans.push(Span {
                    range: start..end,
                    kind: TokenKind::TablePipe,
                });
                continue;
            }

            // 8. List marker (- , * , + , 1. , plus optional [ ] / [x])
            let mut inline_offset = start + leading;
            if let Some(marker_len) = list_marker_len(trimmed) {
                self.spans.push(Span {
                    range: inline_offset..inline_offset + marker_len,
                    kind: TokenKind::ListMarker,
                });
                inline_offset += marker_len;
            }

            // 9. Inline tokens (code, bold, italic, strike, links, table pipes, inline comments)
            if inline_offset < end {
                scan_inline(&text[inline_offset..end], inline_offset, &mut self.spans);
            }
        }

        let last_line = line_starts.len().saturating_sub(1);
        for (_, prev_line) in heading_stack.into_iter().rev() {
            if last_line > prev_line {
                self.folds.push(FoldRange::new(prev_line, last_line));
            }
        }
        self.folds.sort_by_key(|f| (f.start_line, f.end_line));
    }
}

impl Default for PureRustMarkdownHighlighter {
    fn default() -> Self {
        Self::new()
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
        if bytes[i..].starts_with(b"<!--")
            && let Some(rel_end) = s[i + 4..].find("-->")
        {
            let end = i + 4 + rel_end + 3;
            out.push(Span {
                range: base + i..base + end,
                kind: TokenKind::HtmlComment,
            });
            i = end;
            continue;
        }
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

pub fn markdown_highlighter_factory() -> InputHighlighterFactory {
    Rc::new(|lang| {
        if lang.eq_ignore_ascii_case("markdown") || lang.eq_ignore_ascii_case("md") {
            Some(Box::new(PureRustMarkdownHighlighter::new()))
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// SlideEditor Shared Helpers & Split UI Components
// ---------------------------------------------------------------------------

/// Callbacks for rendering a shared Markdown Split view on top of any format's `MarkdownSplitHost`.
pub struct SplitViewCallbacks {
    pub on_select_mode: Rc<dyn Fn(&mut SlideEditor, MarkdownViewMode, &mut Window, &mut Context<SlideEditor>)>,
    pub on_toggle_sync_scroll: Rc<dyn Fn(&mut SlideEditor, &mut Context<SlideEditor>)>,
    pub on_reset_split_ratio: Rc<dyn Fn(&mut SlideEditor, &mut Context<SlideEditor>)>,
    pub on_start_split_drag: Rc<dyn Fn(&mut SlideEditor, f32, &mut Context<SlideEditor>)>,
    pub on_move_split_drag: Rc<dyn Fn(&mut SlideEditor, f32, &mut Context<SlideEditor>)>,
    pub on_end_split_drag: Rc<dyn Fn(&mut SlideEditor, &mut Context<SlideEditor>)>,
}

/// Extract a leading YAML frontmatter block (`---\n...\n---\n`) including trailing blank lines,
/// if present at the top of `text`.
pub fn extract_leading_frontmatter(text: &str) -> Option<&str> {
    let lines = split_line_slices(text);
    let fm_end = detect_frontmatter_end(text, &lines)?;
    let mut end_idx = fm_end + 1;
    while let Some(line) = lines.get(end_idx) {
        if text[line.start..line.end].trim().is_empty() {
            end_idx += 1;
        } else {
            break;
        }
    }
    let byte_end = lines.get(end_idx).map_or(text.len(), |l| l.start);
    Some(&text[..byte_end])
}

/// Preserve any leading YAML frontmatter from `prev_text` (such as `type: workbook` or Obsidian
/// tags/properties) when exporting a workbook back to Markdown.
pub fn preserve_workbook_frontmatter(prev_text: &str, exported: String) -> String {
    if exported.trim_start_matches('\u{feff}').starts_with("---\n")
        || exported.trim_start_matches('\u{feff}').starts_with("---\r\n")
    {
        return exported;
    }
    let Some(prefix) = extract_leading_frontmatter(prev_text) else {
        return exported;
    };
    let mut out = String::with_capacity(prefix.len() + exported.len() + 2);
    out.push_str(prefix);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.ends_with("\n\n") && !out.ends_with("\r\n\r\n") {
        out.push('\n');
    }
    out.push_str(exported.trim_start_matches(['\r', '\n']));
    out
}

impl SlideEditor {
    /// Current file path on native platforms (`None` on wasm where `SlideEditor::path` does not exist).
    pub fn current_path(&self) -> Option<&std::path::Path> {
        #[cfg(not(target_family = "wasm"))]
        {
            self.path.as_deref()
        }
        #[cfg(target_family = "wasm")]
        {
            None
        }
    }

    /// True when this editor's open file path or file name has a `.md` / `.markdown` extension.
    pub fn is_markdown_path(&self) -> bool {
        let from_path = self
            .current_path()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str());
        let from_name = self
            .file_name
            .as_deref()
            .and_then(|n| std::path::Path::new(n).extension())
            .and_then(|e| e.to_str());
        from_path
            .or(from_name)
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
    }

    /// True when this embedded editor should automatically enter bidirectional Markdown `Split`
    /// view on initial open (e.g. `.md` notes, Marp decks, and workbooks inside `SoraneWorkspace`).
    pub fn should_auto_open_markdown_split(&self) -> bool {
        self.embedded_chrome
            .is_some_and(|policy| policy.default_markdown_split)
            && self.is_markdown_path()
    }

    /// Ensure all raster/SVG media embedded in `self.presentation().media` (including Marp Mermaid
    /// diagrams regenerated during live Markdown sync) are registered in `self.resources.images`.
    pub fn register_missing_media_images(&mut self) {
        let missing: Vec<(ibis_doc_model::MediaId, ImageFormat, Vec<u8>)> = self
            .presentation()
            .media
            .iter()
            .filter(|m| !self.resources.images.contains_key(&m.id))
            .filter_map(|m| {
                let format = match m.format {
                    ibis_doc_model::MediaFormat::Png => ImageFormat::Png,
                    ibis_doc_model::MediaFormat::Jpeg => ImageFormat::Jpeg,
                    ibis_doc_model::MediaFormat::Gif => ImageFormat::Gif,
                    ibis_doc_model::MediaFormat::Bmp => ImageFormat::Bmp,
                    ibis_doc_model::MediaFormat::Tiff => ImageFormat::Tiff,
                    ibis_doc_model::MediaFormat::Svg => ImageFormat::Svg,
                    _ => return None,
                };
                Some((m.id, format, m.data.to_vec()))
            })
            .collect();
        for (id, format, bytes) in missing {
            self.resources
                .images
                .insert(id, Arc::new(Image::from_bytes(format, bytes)));
        }
    }

    /// Render the shared `[Source | Split | Visual]` mode bar with sync status, active section badge,
    /// and `⇅` cursor/scroll sync toggle.
    pub fn render_shared_markdown_mode_bar(
        &self,
        id_prefix: &'static str,
        format_badge: &'static str,
        mode: MarkdownViewMode,
        host: Option<&MarkdownSplitHost>,
        section_badge: Option<String>,
        callbacks: &SplitViewCallbacks,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let status_label = match host {
            Some(ms) if ms.mode == MarkdownViewMode::Split && (ms.dirty || ms.visual_dirty) => {
                Some(crate::i18n::text("docx.markdown-source.syncing"))
            }
            Some(ms) if ms.dirty => Some(crate::i18n::text("docx.markdown-source.modified")),
            Some(_) => Some(crate::i18n::text("docx.markdown-source.synced")),
            None => None,
        };
        let sync_scroll_enabled = host.is_some_and(|ms| ms.sync_scroll_enabled);

        let on_select_source = callbacks.on_select_mode.clone();
        let on_select_split = callbacks.on_select_mode.clone();
        let on_select_visual = callbacks.on_select_mode.clone();
        let on_toggle_sync = callbacks.on_toggle_sync_scroll.clone();

        let mode_btn = |suffix: &'static str,
                        label_key: &'static str,
                        target: MarkdownViewMode,
                        handler: Rc<dyn Fn(&mut SlideEditor, MarkdownViewMode, &mut Window, &mut Context<SlideEditor>)>| {
            let active = mode == target;
            div()
                .id(SharedString::from(format!("{id_prefix}-{suffix}")))
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
                .child(crate::i18n::text(label_key))
                .on_click(cx.listener(move |this, _, window, cx| {
                    handler(this, target, window, cx);
                }))
        };

        let title = self
            .file_name
            .clone()
            .unwrap_or_else(|| crate::i18n::text("docx.view.markdown-source"));

        div()
            .id(SharedString::from(format!("{id_prefix}-mode-bar")))
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
                            .px(px(5.))
                            .py(px(1.))
                            .rounded(px(4.))
                            .bg(theme.secondary)
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child(format_badge),
                    )
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
                    .when_some(section_badge, |row, badge| {
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
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .when(mode == MarkdownViewMode::Split, |row| {
                        row.child(
                            div()
                                .id(SharedString::from(format!("{id_prefix}-sync-scroll")))
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
                                        "スライド／シート連動 (スクロール・カーソル同期)",
                                    )
                                    .build(window, cx)
                                })
                                .child("⇅")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    on_toggle_sync(this, cx);
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
                                "mode-source",
                                "docx.markdown-source.mode-source",
                                MarkdownViewMode::Source,
                                on_select_source,
                            ))
                            .child(mode_btn(
                                "mode-split",
                                "docx.markdown-source.mode-split",
                                MarkdownViewMode::Split,
                                on_select_split,
                            ))
                            .child(mode_btn(
                                "mode-visual",
                                "docx.markdown-source.mode-visual",
                                MarkdownViewMode::Visual,
                                on_select_visual,
                            )),
                    ),
            )
            .into_any_element()
    }

    /// Render the shared Markdown source editor pane (with inline non-blocking error banner).
    pub fn render_shared_markdown_source_pane(
        &self,
        id_prefix: &'static str,
        host: &MarkdownSplitHost,
        readonly: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let last_error = host.last_error.clone();
        let editor_entity = host.editor.clone();

        let mut pane = div()
            .id(SharedString::from(format!("{id_prefix}-source-workspace")))
            .key_context(MARKDOWN_SOURCE_CONTEXT)
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

    /// Render the shared Split container: left Markdown source pane + draggable divider + right Visual pane.
    pub fn render_shared_markdown_split_container(
        &self,
        id_prefix: &'static str,
        host: &MarkdownSplitHost,
        source_pane: AnyElement,
        visual_pane: AnyElement,
        callbacks: &SplitViewCallbacks,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let ratio = clamp_split_ratio(host.split_ratio);
        let dragging = host.split_drag.is_some();
        let width_cell = host.split_container_width.clone();
        let measured_width = width_cell.get();
        let divider_w = 5.0_f32;

        let left_wrapper = if measured_width > 200.0 {
            let left_px = ((measured_width - divider_w) * ratio).clamp(180.0, measured_width - 180.0);
            div().w(px(left_px)).flex_none()
        } else {
            div().flex_1()
        };

        let weak = cx.entity().downgrade();
        let on_reset = callbacks.on_reset_split_ratio.clone();
        let on_start = callbacks.on_start_split_drag.clone();
        let on_move = callbacks.on_move_split_drag.clone();
        let on_end = callbacks.on_end_split_drag.clone();

        let divider = div()
            .id(SharedString::from(format!("{id_prefix}-split-divider")))
            .relative()
            .flex_none()
            .w(px(divider_w))
            .h_full()
            .bg(theme.border)
            .hover(|h| h.bg(theme.accent))
            .cursor_col_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if event.click_count == 2 {
                        on_reset(this, cx);
                    } else {
                        on_start(this, f32::from(event.position.x), cx);
                    }
                }),
            )
            .when(dragging, |el| {
                let tracker = weak.clone();
                let on_move = on_move.clone();
                let on_end = on_end.clone();
                el.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            let moving = tracker.clone();
                            let on_move = on_move.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture {
                                    let _ = moving.update(cx, |this, cx| {
                                        on_move(this, f32::from(event.position.x), cx);
                                    });
                                }
                            });
                            let released = tracker.clone();
                            let on_end = on_end.clone();
                            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture {
                                    let _ = released.update(cx, |this, cx| {
                                        on_end(this, cx);
                                    });
                                }
                            });
                        },
                    )
                    .size_0()
                    .absolute(),
                )
            });

        let measure_cell = width_cell.clone();
        let measure_canvas = canvas(
            move |bounds, _, _| {
                let w = f32::from(bounds.size.width);
                if w > 0.0 {
                    measure_cell.set(w);
                }
            },
            |_, _, _, _| {},
        )
        .size_full()
        .absolute();

        div()
            .id(SharedString::from(format!("{id_prefix}-split-container")))
            .relative()
            .flex()
            .flex_row()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(measure_canvas)
            .child(
                left_wrapper
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .child(source_pane),
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
                    .child(visual_pane),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn marp_slide_spans_handles_frontmatter_fences_and_slide_breaks() {
        let md = "\
---
marp: true
theme: default
---

# Title Slide

Intro text

---

## Architecture

```yaml
---
not_a_slide: true
---
```

---

## Summary

- Done
";
        let spans = marp_slide_spans(md);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].title.as_deref(), Some("Title Slide"));
        assert_eq!(spans[1].title.as_deref(), Some("Architecture"));
        assert_eq!(spans[2].title.as_deref(), Some("Summary"));

        // Line 0 (frontmatter) maps to slide 0
        assert_eq!(section_for_line(&spans, 0).map(|s| s.index), Some(0));
        // Inside fenced block maps to slide 1 (not split by `---` inside fence!)
        let arch_line = md.lines().position(|l| l == "not_a_slide: true").unwrap();
        assert_eq!(section_for_line(&spans, arch_line).map(|s| s.index), Some(1));
        // Last line maps to slide 2
        let done_line = md.lines().position(|l| l == "- Done").unwrap();
        assert_eq!(section_for_line(&spans, done_line).map(|s| s.index), Some(2));
    }

    #[::core::prelude::v1::test]
    fn workbook_sheet_spans_maps_multi_sheet_markers_and_single_sheet_fallback() {
        let multi = "\
<!-- xlsx-sheet \"Sales\" -->

## Sales

| Q | Rev |
| --- | ---: |
| Q1 | 100 |

<!-- xlsx-sheet {\"name\":\"Summary\",\"origin\":\"B2\"} -->

## Summary

| Total |
| --- |
| 100 |
";
        let spans = workbook_sheet_spans(multi);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].title.as_deref(), Some("Sales"));
        assert_eq!(spans[1].title.as_deref(), Some("Summary"));
        let q1_line = multi.lines().position(|l| l.starts_with("| Q1")).unwrap();
        let total_line = multi.lines().position(|l| l.starts_with("| Total")).unwrap();
        assert_eq!(section_for_line(&spans, q1_line).map(|s| s.index), Some(0));
        assert_eq!(section_for_line(&spans, total_line).map(|s| s.index), Some(1));

        let single = "---\ntype: workbook\n---\n\n## Budget\n\n| Item | Cost |\n| --- | --- |\n| A | 10 |\n";
        let single_spans = workbook_sheet_spans(single);
        assert_eq!(single_spans.len(), 1);
        assert_eq!(single_spans[0].title.as_deref(), Some("Budget"));
    }

    #[::core::prelude::v1::test]
    fn highlighter_parses_frontmatter_mermaid_and_xlsx_comments() {
        let mut h = PureRustMarkdownHighlighter::new();
        let sample = "---\nmarp: true\n---\n\n<!-- xlsx-sheet \"S1\" -->\n\n# Slide\n\n```mermaid\ngraph TD;\nA-->B;\n```\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        h.reparse(sample);
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::FrontmatterKey));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::HtmlComment));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::Heading));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::CodeFence));
        assert!(h.spans.iter().any(|s| s.kind == TokenKind::TablePipe));
        assert!(!h.folds.is_empty());
    }

    #[::core::prelude::v1::test]
    fn clamp_split_ratio_guards_bounds_and_nan() {
        assert_eq!(clamp_split_ratio(0.05), MIN_SPLIT_RATIO);
        assert_eq!(clamp_split_ratio(0.95), MAX_SPLIT_RATIO);
        assert_eq!(clamp_split_ratio(f32::NAN), DEFAULT_SPLIT_RATIO);
        assert_eq!(clamp_split_ratio(0.5), 0.5);
    }

    #[::core::prelude::v1::test]
    fn preserve_workbook_frontmatter_keeps_obsidian_yaml_properties() {
        let prev = "---\ntype: workbook\ntags: [budget, 2026]\n---\n\n| Item | Cost |\n| --- | --- |\n| A | 10 |\n";
        let exported = "<!-- xlsx-sheet \"Sheet1\" -->\n\n## Sheet1\n\n| Item | Cost |\n| --- | --- |\n| A | 20 |\n".to_string();
        let merged = preserve_workbook_frontmatter(prev, exported);
        assert!(merged.starts_with("---\ntype: workbook\ntags: [budget, 2026]\n---\n\n"));
        assert!(merged.contains("| A | 20 |"));

        let no_fm = "<!-- xlsx-sheet \"Sheet1\" -->\n\n## Sheet1\n";
        let unchanged = preserve_workbook_frontmatter(no_fm, "## Sheet1\n".to_string());
        assert_eq!(unchanged, "## Sheet1\n");
    }
}
