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
    CompletionProvider, Editor, EditorState, Escape, FoldRange, HighlightStyleResolver, InputEdit,
    InputHighlighter, InputHighlighterFactory, Rope,
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

/// Automatically detected document kind for plain-text sources (`text` / `.md` / `.lawtext`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextContentKind {
    Markdown,
    Marp,
    Workbook,
    Lawtext,
}

impl TextContentKind {
    pub fn badge_label(self) -> &'static str {
        match self {
            Self::Markdown => "MD",
            Self::Marp => "Marp",
            Self::Workbook => "Sheet",
            Self::Lawtext => "Law",
        }
    }
}

/// Detect whether plain-text content is Marp presentation, Workbook Markdown, Lawtext, or standard Markdown.
pub fn detect_text_content_kind(data: &[u8]) -> TextContentKind {
    if ibis_formats::markdown::is_presentation_markdown(data) {
        TextContentKind::Marp
    } else if ibis_formats::markdown::is_workbook_markdown(data) {
        TextContentKind::Workbook
    } else if ibis_lawtext::is_lawtext(data) {
        TextContentKind::Lawtext
    } else {
        TextContentKind::Markdown
    }
}

pub const TEMPLATE_MARP: &str = "\
---
marp: true
theme: default
paginate: true
---

# タイトル

サブタイトル

---

## スライド 2

- 箇条書き 1
- 箇条書き 2
";

pub const TEMPLATE_WORKBOOK: &str = "\
---
workbook: true
---

<!-- xlsx-sheet \"Sheet1\" -->

## Sheet1

| 項目 | 値 | メモ |
| --- | ---: | --- |
| A | 100 | |
| B | 200 | |
| 合計 | =SUM(B2:B3) | |
";

pub const TEMPLATE_LAWTEXT: &str = "\
新規法令案（令和六年架空規則第一号）

  （目的）
第一条　この規則は、必要な事項を定めることを目的とする。

  附則
この規則は、公布の日から施行する。
";

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

/// Textual source dialect shown in the Split / Source editor pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SourceDialect {
    #[default]
    Markdown,
    Lawtext,
    LawXml,
}

impl SourceDialect {
    pub fn language_id(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Lawtext => "lawtext",
            Self::LawXml => "xml",
        }
    }

    pub fn badge_label(self) -> &'static str {
        match self {
            Self::Markdown => "MD",
            Self::Lawtext => "Lawtext",
            Self::LawXml => "e-Gov XML",
        }
    }
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

/// State for AI-powered Markdown inline completion (ghost text in EditorState).
#[derive(Clone, Debug, Default)]
pub struct InlineCompletionState {
    /// Whether automatic inline completion on typing is enabled.
    pub enabled: bool,
    /// Whether a completion request is currently in flight.
    pub loading: bool,
    /// Whether an inline suggestion is currently displayed in the editor.
    pub has_suggestion: bool,
    /// One-shot flag to skip debounce when triggered explicitly (e.g. Alt+] or toggle ON).
    pub immediate_once: Rc<Cell<bool>>,
    /// Last non-fatal error message from completion.
    pub error: Option<String>,
    /// Monotonic generation counter to track in-flight completion requests.
    pub generation: u64,
    pub auto_switch_generation: u64,
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
const IBIS_SHEET_MARKER_PREFIX: &str = "<!-- ibis-sheet:";

fn parse_xlsx_sheet_marker_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix(XLSX_SHEET_MARKER_PREFIX)?;
    let encoded = rest.strip_suffix(" -->")?;
    let val: serde_json::Value = serde_json::from_str(encoded).ok()?;
    if let Some(s) = val.as_str() {
        return Some(s.to_string());
    }
    val.get("name")?.as_str().map(ToString::to_string)
}

fn parse_ibis_sheet_marker_name(trimmed: &str) -> Option<String> {
    let inner = trimmed.strip_prefix(IBIS_SHEET_MARKER_PREFIX)?.strip_suffix("-->")?.trim();
    if inner.is_empty() {
        return None;
    }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(inner) {
        if let Some(s) = val.as_str() {
            return Some(s.to_string());
        }
        if let Some(s) = val.get("name").and_then(serde_json::Value::as_str) {
            return Some(s.to_string());
        }
    }
    Some(inner.to_string())
}

/// Split a Workbook Markdown document into 0-based visible sheet [`SectionSpan`]s.
///
/// - Recognizes `<!-- xlsx-sheet ... -->` and `<!-- ibis-sheet: ... -->` markers outside fenced code blocks.
/// - Falls back to a single sheet span when no sheet markers are present.
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
        } else if trimmed_end.starts_with(IBIS_SHEET_MARKER_PREFIX) {
            markers.push((line.idx, parse_ibis_sheet_marker_name(trimmed_end)));
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

/// Multi-dialect pure-Rust syntax highlighter factory supporting Markdown, Lawtext, and e-Gov XML.
pub fn multi_source_highlighter_factory() -> InputHighlighterFactory {
    Rc::new(|lang| {
        if lang.eq_ignore_ascii_case("markdown") || lang.eq_ignore_ascii_case("md") {
            Some(Box::new(PureRustMarkdownHighlighter::new()))
        } else if lang.eq_ignore_ascii_case("lawtext") {
            Some(Box::new(PureRustLawtextHighlighter::new()))
        } else if lang.eq_ignore_ascii_case("xml") || lang.eq_ignore_ascii_case("lawxml") {
            Some(Box::new(PureRustXmlHighlighter::new()))
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Lawtext & e-Gov XML Section Spans + Pure-Rust Highlighters
// ---------------------------------------------------------------------------

fn is_kanji_or_digit_num(ch: char) -> bool {
    matches!(
        ch,
        '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十' | '百' | '千' | '〇'
            | '0'..='9'
            | '０'..='９'
    )
}

fn lawtext_article_title(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix('第')?;
    let mut chars = rest.char_indices().peekable();
    let mut saw_num = false;
    let mut end = 0usize;
    while let Some((idx, ch)) = chars.peek().copied() {
        if is_kanji_or_digit_num(ch) {
            saw_num = true;
            end = idx + ch.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    if !saw_num {
        return None;
    }
    let after = &rest[end..];
    if !after.starts_with('条') {
        return None;
    }
    let mut title_len = '第'.len_utf8() + end + '条'.len_utf8();
    // Optional branch suffix: `の二`, `の三の二`, etc.
    let mut tail = &trimmed[title_len..];
    while let Some(after_no) = tail.strip_prefix('の') {
        let mut branch_len = 0usize;
        for (idx, ch) in after_no.char_indices() {
            if is_kanji_or_digit_num(ch) {
                branch_len = idx + ch.len_utf8();
            } else {
                break;
            }
        }
        if branch_len == 0 {
            break;
        }
        title_len += 'の'.len_utf8() + branch_len;
        tail = &trimmed[title_len..];
    }
    if tail.is_empty()
        || tail.starts_with([' ', '\t', '　', '（', '('])
    {
        Some(trimmed[..title_len].to_string())
    } else {
        None
    }
}

fn lawtext_structural_heading(trimmed: &str) -> Option<String> {
    if trimmed.starts_with("附則") || trimmed.starts_with("別表") {
        let label = trimmed.split_whitespace().next().unwrap_or(trimmed);
        return Some(label.to_string());
    }
    if let Some(rest) = trimmed.strip_prefix('第') {
        let mut num_len = 0usize;
        for (idx, ch) in rest.char_indices() {
            if is_kanji_or_digit_num(ch) {
                num_len = idx + ch.len_utf8();
            } else {
                break;
            }
        }
        if num_len > 0 {
            let after = &rest[num_len..];
            if after.starts_with(['編', '章', '節', '款', '目']) {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn lawtext_is_caption(trimmed: &str) -> bool {
    (trimmed.starts_with('（') && trimmed.ends_with('）') && trimmed.chars().count() >= 3)
        || (trimmed.starts_with('(') && trimmed.ends_with(')') && trimmed.chars().count() >= 3)
}

fn xml_attr_value<'a>(tag_line: &'a str, attr: &str) -> Option<&'a str> {
    let needle = format!("{attr}=\"");
    let pos = tag_line.find(&needle)?;
    let rest = &tag_line[pos + needle.len()..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn xml_inline_text<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}");
    let start_tag = line.find(&open)?;
    let gt = line[start_tag..].find('>')? + start_tag + 1;
    let close = format!("</{tag}>");
    let end = line[gt..].find(&close)? + gt;
    let inner = line[gt..end].trim();
    (!inner.is_empty()).then_some(inner)
}

/// Split a Lawtext or e-Gov Law XML document into article/chapter/suppl [`SectionSpan`]s
/// for bidirectional article-level cursor & scroll sync.
pub fn law_section_spans(text: &str, dialect: SourceDialect) -> Vec<SectionSpan> {
    let lines = split_line_slices(text);
    if lines.is_empty() {
        return Vec::new();
    }

    let mut anchors: Vec<(usize, String)> = Vec::new();
    match dialect {
        SourceDialect::Lawtext => {
            for (idx, slice) in lines.iter().enumerate() {
                let trimmed = text[slice.start..slice.end].trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(title) = lawtext_article_title(trimmed) {
                    // Attach an immediately preceding `(見出し)` caption line to this article span.
                    let start_line = if idx > 0 {
                        let prev = text[lines[idx - 1].start..lines[idx - 1].end].trim();
                        if lawtext_is_caption(prev) {
                            idx - 1
                        } else {
                            idx
                        }
                    } else {
                        idx
                    };
                    anchors.push((start_line, title));
                } else if let Some(heading) = lawtext_structural_heading(trimmed) {
                    anchors.push((idx, heading));
                }
            }
        }
        SourceDialect::LawXml => {
            for (idx, slice) in lines.iter().enumerate() {
                let trimmed = text[slice.start..slice.end].trim();
                if trimmed.starts_with("<Article")
                    && trimmed
                        .as_bytes()
                        .get("<Article".len())
                        .is_some_and(|b| matches!(b, b' ' | b'>' | b'\t'))
                {
                    // Prefer the inner `<ArticleTitle>` text on the next few lines when present.
                    let mut title = None;
                    for look in &lines[idx..(idx + 5).min(lines.len())] {
                        if let Some(t) = xml_inline_text(&text[look.start..look.end], "ArticleTitle") {
                            title = Some(t.to_string());
                            break;
                        }
                    }
                    let label = title.unwrap_or_else(|| {
                        xml_attr_value(trimmed, "Num")
                            .map(|n| format!("第{n}条"))
                            .unwrap_or_else(|| "条文".to_string())
                    });
                    anchors.push((idx, label));
                } else if trimmed.starts_with("<Chapter")
                    && trimmed
                        .as_bytes()
                        .get("<Chapter".len())
                        .is_some_and(|b| matches!(b, b' ' | b'>' | b'\t'))
                {
                    let mut title = None;
                    for look in &lines[idx..(idx + 4).min(lines.len())] {
                        if let Some(t) = xml_inline_text(&text[look.start..look.end], "ChapterTitle") {
                            title = Some(t.to_string());
                            break;
                        }
                    }
                    let label = title.unwrap_or_else(|| {
                        xml_attr_value(trimmed, "Num")
                            .map(|n| format!("第{n}章"))
                            .unwrap_or_else(|| "章".to_string())
                    });
                    anchors.push((idx, label));
                } else if trimmed.starts_with("<SupplProvision")
                    && trimmed
                        .as_bytes()
                        .get("<SupplProvision".len())
                        .is_some_and(|b| matches!(b, b' ' | b'>' | b'\t'))
                {
                    let mut title = None;
                    for look in &lines[idx..(idx + 4).min(lines.len())] {
                        if let Some(t) =
                            xml_inline_text(&text[look.start..look.end], "SupplProvisionLabel")
                        {
                            title = Some(t.to_string());
                            break;
                        }
                    }
                    anchors.push((idx, title.unwrap_or_else(|| "附則".to_string())));
                } else if trimmed.starts_with("<AppdxTable")
                    && trimmed
                        .as_bytes()
                        .get("<AppdxTable".len())
                        .is_some_and(|b| matches!(b, b' ' | b'>' | b'\t'))
                {
                    anchors.push((idx, "別表".to_string()));
                }
            }
        }
        SourceDialect::Markdown => return Vec::new(),
    }

    if anchors.is_empty() {
        return vec![SectionSpan {
            index: 0,
            line_range: 0..lines.len().max(1),
            byte_range: 0..text.len(),
            title: None,
        }];
    }

    let total_lines = lines.len();
    let mut spans = Vec::with_capacity(anchors.len());
    for (i, (start_line, title)) in anchors.iter().enumerate() {
        let end_line = anchors
            .get(i + 1)
            .map_or(total_lines, |(next_start, _)| *next_start)
            .max(*start_line + 1);
        let byte_start = lines.get(*start_line).map_or(text.len(), |l| l.start);
        let byte_end = lines
            .get(end_line.saturating_sub(1))
            .map_or(text.len(), |l| l.next);
        spans.push(SectionSpan {
            index: i,
            line_range: *start_line..end_line,
            byte_range: byte_start..byte_end,
            title: Some(title.clone()),
        });
    }
    spans
}

/// Pure-Rust Lawtext syntax highlighter and article/chapter folder.
pub struct PureRustLawtextHighlighter {
    spans: Vec<Span>,
    folds: Vec<FoldRange>,
}

impl PureRustLawtextHighlighter {
    pub fn new() -> Self {
        Self {
            spans: Vec::new(),
            folds: Vec::new(),
        }
    }

    pub fn reparse(&mut self, text: &str) {
        self.spans.clear();
        self.folds.clear();

        let lines = split_line_slices(text);
        let mut active_article_start: Option<usize> = None;

        for slice in &lines {
            let line = &text[slice.start..slice.end];
            let trimmed = line.trim_start();
            let leading = line.len() - trimmed.len();
            let content_start = slice.start + leading;
            let trimmed_end = trimmed.trim_end();
            if trimmed_end.is_empty() {
                continue;
            }

            // First non-empty line with LawNum parentheses (e.g. `テスト法（令和七年法律第一号）`)
            if slice.idx == 0 && trimmed_end.contains('（') && trimmed_end.ends_with('）') {
                self.spans.push(Span {
                    range: content_start..slice.end,
                    kind: TokenKind::Heading,
                });
                continue;
            }

            // Structural divisions (編 / 章 / 節 / 附則 / 別表)
            if lawtext_structural_heading(trimmed_end).is_some() {
                if let Some(prev_start) = active_article_start.take()
                    && slice.idx.saturating_sub(1) > prev_start
                {
                    self.folds
                        .push(FoldRange::new(prev_start, slice.idx.saturating_sub(1)));
                }
                self.spans.push(Span {
                    range: content_start..slice.end,
                    kind: TokenKind::Heading,
                });
                continue;
            }

            // Article caption `(見出し)`
            if lawtext_is_caption(trimmed_end) {
                self.spans.push(Span {
                    range: content_start..slice.end,
                    kind: TokenKind::FrontmatterKey,
                });
                continue;
            }

            // Article header (`第一条 ...`)
            if let Some(title) = lawtext_article_title(trimmed_end) {
                if let Some(prev_start) = active_article_start.replace(slice.idx)
                    && slice.idx.saturating_sub(1) > prev_start
                {
                    self.folds
                        .push(FoldRange::new(prev_start, slice.idx.saturating_sub(1)));
                }
                let title_end = content_start + title.len();
                self.spans.push(Span {
                    range: content_start..title_end,
                    kind: TokenKind::Strong,
                });
                if title_end < slice.end {
                    scan_lawtext_inline(&text[title_end..slice.end], title_end, &mut self.spans);
                }
                continue;
            }

            // Paragraph / Item / Subitem leading number marker (`２ `, `一 `, `イ `)
            if let Some((first_tok, _)) = trimmed_end.split_once([' ', '\t', '　'])
                && is_lawtext_item_marker(first_tok)
            {
                let marker_end = content_start + first_tok.len();
                self.spans.push(Span {
                    range: content_start..marker_end,
                    kind: TokenKind::ListMarker,
                });
                if marker_end < slice.end {
                    scan_lawtext_inline(&text[marker_end..slice.end], marker_end, &mut self.spans);
                }
                continue;
            }

            scan_lawtext_inline(&text[content_start..slice.end], content_start, &mut self.spans);
        }

        let last_line = lines.len().saturating_sub(1);
        if let Some(prev_start) = active_article_start
            && last_line > prev_start
        {
            self.folds.push(FoldRange::new(prev_start, last_line));
        }
        self.folds.sort_by_key(|f| (f.start_line, f.end_line));
    }
}

impl Default for PureRustLawtextHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

fn is_lawtext_item_marker(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if token.chars().all(is_kanji_or_digit_num) {
        return true;
    }
    matches!(
        token,
        "イ" | "ロ" | "ハ" | "ニ" | "ホ" | "ヘ" | "ト" | "チ" | "リ" | "ヌ"
    ) || (token.starts_with('（') && token.ends_with('）') && token.chars().count() <= 5)
}

fn scan_lawtext_inline(s: &str, base: usize, out: &mut Vec<Span>) {
    let mut idx = 0usize;
    while idx < s.len() {
        let rest = &s[idx..];
        if rest.starts_with('「')
            && let Some(close_rel) = rest['「'.len_utf8()..].find('」')
        {
            let end = idx + '「'.len_utf8() + close_rel + '」'.len_utf8();
            out.push(Span {
                range: base + idx..base + end,
                kind: TokenKind::LinkText,
            });
            idx = end;
            continue;
        }
        if rest.starts_with('第') {
            let after_dai = &rest['第'.len_utf8()..];
            let mut num_len = 0usize;
            for (i, ch) in after_dai.char_indices() {
                if is_kanji_or_digit_num(ch) {
                    num_len = i + ch.len_utf8();
                } else {
                    break;
                }
            }
            if num_len > 0 {
                let after_num = &after_dai[num_len..];
                if let Some(unit) = ['条', '項', '号', '章', '節']
                    .into_iter()
                    .find(|&u| after_num.starts_with(u))
                {
                    let end = idx + '第'.len_utf8() + num_len + unit.len_utf8();
                    out.push(Span {
                        range: base + idx..base + end,
                        kind: TokenKind::CodeFence,
                    });
                    idx = end;
                    continue;
                }
            }
        }
        let ch_len = rest.chars().next().map_or(1, char::len_utf8);
        idx += ch_len;
    }
}

impl InputHighlighter for PureRustLawtextHighlighter {
    fn language(&self) -> SharedString {
        "lawtext".into()
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
        resolve_spans(&self.spans, range, resolver)
    }

    fn fold_ranges(&self, _text: &Rope) -> Vec<FoldRange> {
        self.folds.clone()
    }
}

/// Pure-Rust e-Gov Law XML syntax highlighter and element folder.
pub struct PureRustXmlHighlighter {
    spans: Vec<Span>,
    folds: Vec<FoldRange>,
}

impl PureRustXmlHighlighter {
    pub fn new() -> Self {
        Self {
            spans: Vec::new(),
            folds: Vec::new(),
        }
    }

    pub fn reparse(&mut self, text: &str) {
        self.spans.clear();
        self.folds.clear();

        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut i = 0usize;
        while i < len {
            if bytes[i..].starts_with(b"<!--") {
                let end = text[i + 4..]
                    .find("-->")
                    .map_or(len, |rel| i + 4 + rel + 3);
                self.spans.push(Span {
                    range: i..end,
                    kind: TokenKind::HtmlComment,
                });
                i = end;
                continue;
            }
            if bytes[i..].starts_with(b"<?") {
                let end = text[i + 2..]
                    .find("?>")
                    .map_or(len, |rel| i + 2 + rel + 2);
                self.spans.push(Span {
                    range: i..end,
                    kind: TokenKind::FrontmatterDelimiter,
                });
                i = end;
                continue;
            }
            if bytes[i] == b'<' {
                let tag_start = i;
                i += 1;
                if i < len && bytes[i] == b'/' {
                    i += 1;
                }
                let name_start = i;
                while i < len
                    && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/')
                {
                    i += 1;
                }
                if i > name_start {
                    self.spans.push(Span {
                        range: tag_start..i,
                        kind: TokenKind::CodeFence,
                    });
                }
                while i < len && bytes[i] != b'>' {
                    if matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'/') {
                        i += 1;
                        continue;
                    }
                    if bytes[i] == b'"' || bytes[i] == b'\'' {
                        let q = bytes[i];
                        let val_start = i;
                        i += 1;
                        while i < len && bytes[i] != q {
                            i += 1;
                        }
                        if i < len {
                            i += 1;
                        }
                        self.spans.push(Span {
                            range: val_start..i,
                            kind: TokenKind::LinkText,
                        });
                        continue;
                    }
                    let attr_start = i;
                    while i < len
                        && !matches!(bytes[i], b'=' | b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/')
                    {
                        i += 1;
                    }
                    if i > attr_start {
                        self.spans.push(Span {
                            range: attr_start..i,
                            kind: TokenKind::FrontmatterKey,
                        });
                    }
                    if i < len && bytes[i] == b'=' {
                        i += 1;
                    }
                }
                if i < len && bytes[i] == b'>' {
                    self.spans.push(Span {
                        range: i..i + 1,
                        kind: TokenKind::CodeFence,
                    });
                    i += 1;
                }
                continue;
            }
            i += 1;
        }

        // Line-level folds for major e-Gov XML container elements.
        let lines = split_line_slices(text);
        let mut stack: Vec<(&'static str, usize)> = Vec::new();
        for slice in &lines {
            let trimmed = text[slice.start..slice.end].trim();
            for tag in ["Chapter", "Article", "SupplProvision", "AppdxTable"] {
                let open_prefix = format!("<{tag}");
                let close_tag = format!("</{tag}>");
                let opens = trimmed.starts_with(&open_prefix)
                    && trimmed
                        .as_bytes()
                        .get(open_prefix.len())
                        .is_some_and(|b| matches!(b, b' ' | b'>' | b'\t'))
                    && !trimmed.contains(&close_tag)
                    && !trimmed.ends_with("/>");
                if opens {
                    stack.push((tag, slice.idx));
                } else if trimmed.starts_with(&close_tag)
                    && let Some(pos) = stack.iter().rposition(|(t, _)| *t == tag)
                {
                    let (_, start_line) = stack.remove(pos);
                    if slice.idx > start_line {
                        self.folds.push(FoldRange::new(start_line, slice.idx));
                    }
                }
            }
        }
        self.folds.sort_by_key(|f| (f.start_line, f.end_line));
    }
}

impl Default for PureRustXmlHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl InputHighlighter for PureRustXmlHighlighter {
    fn language(&self) -> SharedString {
        "xml".into()
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
        resolve_spans(&self.spans, range, resolver)
    }

    fn fold_ranges(&self, _text: &Rope) -> Vec<FoldRange> {
        self.folds.clone()
    }
}

fn resolve_spans(
    spans: &[Span],
    range: &Range<usize>,
    resolver: &dyn HighlightStyleResolver,
) -> Vec<(Range<usize>, HighlightStyle)> {
    if range.start >= range.end {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = range.start;
    let first = spans.partition_point(|s| s.range.end <= range.start);
    for span in &spans[first..] {
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

    /// Raw Lawtext / e-Gov XML bytes loaded in the browser on wasm (`None` on native platforms).
    pub fn web_law_source_bytes(&self) -> Option<&[u8]> {
        #[cfg(target_family = "wasm")]
        {
            self.web.law_source.as_deref().map(Vec::as_slice)
        }
        #[cfg(not(target_family = "wasm"))]
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

    /// True when this editor's open file represents a Japanese statute (Lawtext, e-Gov Law XML,
    /// or Lawtext-flavored Markdown).
    pub fn is_law_document(&self) -> bool {
        if self.kind() != ibis_formats::Kind::Document {
            return false;
        }
        let name_lower = self
            .current_path()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .or(self.file_name.as_deref())
            .map(|n| n.to_ascii_lowercase());
        if name_lower
            .as_deref()
            .is_some_and(|n| n.ends_with(".lawtext") || n.ends_with(".law.txt"))
        {
            return true;
        }
        #[cfg(target_family = "wasm")]
        if self.web.law_source.is_some() {
            return true;
        }
        if let Some((orig_md, _, _)) = self.source.as_ref().and_then(|s| s.original_markdown())
            && ibis_formats::markdown::roundtrip::document_frontmatter(
                orig_md.trim_start_matches(['\u{feff}', '\r', '\n']),
            )
            .is_some_and(|frame| frame.lawtext)
        {
            return true;
        }
        self.presentation().flow.as_ref().is_some_and(|flow| {
            flow.body.iter().take(8).any(|block| match block {
                ibis_doc_model::flow::Block::Paragraph(p) => {
                    p.text().trim_start().starts_with("法令番号:")
                }
                _ => false,
            })
        })
    }

    /// True when the open file path is a `.lawtext` / `.law.txt` file, or an e-Gov Law `.xml` file.
    pub fn is_law_path(&self) -> bool {
        let name_lower = self
            .current_path()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .or(self.file_name.as_deref())
            .map(|n| n.to_ascii_lowercase());
        match name_lower.as_deref() {
            Some(n) if n.ends_with(".lawtext") || n.ends_with(".law.txt") => true,
            Some(n) if n.ends_with(".xml") => self.is_law_document(),
            _ => false,
        }
    }

    /// True when this editor should automatically enter bidirectional Source `Split`
    /// view on initial open (e.g. `.md` notes, Marp decks, workbooks, and `.lawtext`/e-Gov `.xml`
    /// inside `SoraneWorkspace` or standalone Web mode).
    pub fn should_auto_open_markdown_split(&self) -> bool {
        let auto_by_chrome = self
            .embedded_chrome
            .is_some_and(|policy| policy.default_markdown_split);
        #[cfg(target_family = "wasm")]
        let auto_on_web = !crate::host::is_connected();
        #[cfg(not(target_family = "wasm"))]
        let auto_on_web = false;
        (auto_by_chrome || auto_on_web) && (self.is_markdown_path() || self.is_law_path())
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

        let compact_mobile = self.compact_viewport;
        let mode_btn = |suffix: &'static str,
                        label_key: &'static str,
                        target: MarkdownViewMode,
                        handler: Rc<dyn Fn(&mut SlideEditor, MarkdownViewMode, &mut Window, &mut Context<SlideEditor>)>| {
            let active = mode == target;
            div()
                .id(SharedString::from(format!("{id_prefix}-{suffix}")))
                .px(px(if compact_mobile { 12. } else { 9. }))
                .py(px(if compact_mobile { 6. } else { 3. }))
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(if compact_mobile { 12. } else { 11.5 }))
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
            .py(px(if compact_mobile { 6. } else { 4. }))
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
                    .when(!compact_mobile, |row| {
                        row.child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.foreground)
                                .truncate()
                                .child(title),
                        )
                    })
                    .when_some(status_label, |row, label| {
                        row.child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .child(label),
                        )
                    })
                    .when_some(section_badge.filter(|_| !compact_mobile), |row, badge| {
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
                    .when(!compact_mobile && mode == MarkdownViewMode::Split, |row| {
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
                            .when(!compact_mobile, |bar| {
                                bar.child(mode_btn(
                                    "mode-split",
                                    "docx.markdown-source.mode-split",
                                    MarkdownViewMode::Split,
                                    on_select_split,
                                ))
                            })
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

        pane.child(self.render_inline_completion_bar(id_prefix, editor_entity.clone(), readonly, cx))
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

    fn trigger_inline_completion_via_public_api(
        immediate_once: &Rc<Cell<bool>>,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        immediate_once.set(true);
        editor.update(cx, |state, cx| {
            if let Some(sel) = EntityInputHandler::selected_text_range(state, false, window, cx) {
                let end = sel.range.end;
                EntityInputHandler::replace_text_in_range(state, Some(end..end), "", window, cx);
            } else {
                EntityInputHandler::replace_text_in_range(state, None, "", window, cx);
            }
        });
    }

    pub fn toggle_inline_completion(
        &mut self,
        editor: Option<Entity<EditorState>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inline_completion.enabled = !self.inline_completion.enabled;
        self.inline_completion.generation = self.inline_completion.generation.wrapping_add(1);
        self.inline_completion.loading = false;
        self.inline_completion.error = None;
        if let Some(ed) = editor {
            if self.inline_completion.enabled {
                Self::trigger_inline_completion_via_public_api(
                    &self.inline_completion.immediate_once,
                    &ed,
                    window,
                    cx,
                );
            } else if self.inline_completion.has_suggestion {
                self.inline_completion.has_suggestion = false;
                ed.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
                window.dispatch_action(Box::new(Escape), cx);
            }
        } else {
            self.inline_completion.has_suggestion = false;
        }
        cx.notify();
    }

    pub fn handle_inline_completion_key(
        &mut self,
        editor: Entity<EditorState>,
        readonly: bool,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if readonly {
            return false;
        }
        let ks = &event.keystroke;
        if ks.key.as_str() == "]"
            && ks.modifiers.alt
            && !ks.modifiers.control
            && !ks.modifiers.platform
        {
            self.inline_completion.enabled = true;
            Self::trigger_inline_completion_via_public_api(
                &self.inline_completion.immediate_once,
                &editor,
                window,
                cx,
            );
            cx.notify();
            return true;
        }
        if self.inline_completion.has_suggestion
            && !ks.modifiers.control
            && !ks.modifiers.platform
            && !ks.modifiers.alt
        {
            self.inline_completion.has_suggestion = false;
            cx.notify();
        }
        false
    }

    pub fn attach_inline_completion_provider(
        state: &mut EditorState,
        slide_editor: WeakEntity<Self>,
        immediate_once: Rc<Cell<bool>>,
    ) {
        state.lsp_mut().completion_provider = Some(Rc::new(CopilotInlineCompletionProvider {
            slide_editor,
            immediate_once,
        }));
    }

    pub fn render_inline_completion_bar(
        &self,
        id_prefix: &'static str,
        editor: Entity<EditorState>,
        readonly: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use gpui_kit::component::Disableable as _;
        use gpui_kit::component::button::{Button, ButtonVariants as _};

        let theme = cx.theme().clone();
        let enabled = self.inline_completion.enabled;
        let tooltip_text: SharedString = if let Some(err) = &self.inline_completion.error {
            err.clone().into()
        } else {
            "AIインライン補完 (Alt+]: 今すぐ補完 / Tab: 確定 / Esc: 却下)".into()
        };

        let toggle_label = if enabled {
            "AI補完: ON"
        } else {
            "AI補完: OFF"
        };

        let mut bar = div()
            .id(SharedString::from(format!("{id_prefix}-inline-completion-bar")))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(4.))
            .bg(theme.muted)
            .border_b_1()
            .border_color(theme.border)
            .text_size(px(12.))
            .child(
                Button::new(SharedString::from(format!("{id_prefix}-inline-toggle")))
                    .label(toggle_label)
                    .compact()
                    .disabled(readonly)
                    .when(enabled, |btn| btn.primary())
                    .when(!enabled, |btn| btn.ghost())
                    .tooltip(tooltip_text)
                    .on_click(cx.listener({
                        let editor = editor.clone();
                        move |this, _ev, window, cx| {
                            this.toggle_inline_completion(Some(editor.clone()), window, cx);
                        }
                    })),
            )
            .child(div().flex_1());

        let current_kind = self.current_text_content_kind();
        bar = bar.child(
            div()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(4.))
                .bg(theme.background)
                .border_1()
                .border_color(theme.border)
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(format!("自動判定: {}", current_kind.badge_label())),
        );

        if !readonly {
            let templates: [(&'static str, &'static str, &'static str); 3] = [
                ("marp", "Marp", TEMPLATE_MARP),
                ("sheet", "Sheet", TEMPLATE_WORKBOOK),
                ("law", "Law", TEMPLATE_LAWTEXT),
            ];
            for (slug, label, body) in templates {
                let ed = editor.clone();
                bar = bar.child(
                    Button::new(SharedString::from(format!("{id_prefix}-tpl-{slug}")))
                        .label(label)
                        .compact()
                        .ghost()
                        .on_click(cx.listener(move |this, _ev, window, cx| {
                            ed.update(cx, |state, cx| {
                                state.set_value(body.to_string(), window, cx);
                            });
                            let _ = this.reopen_text_source_as_detected(
                                body.to_string(),
                                window,
                                cx,
                            );
                        })),
                );
            }
        }

        bar.into_any_element()
    }

    pub fn current_text_content_kind(&self) -> TextContentKind {
        match self.kind() {
            ibis_formats::Kind::Presentation => TextContentKind::Marp,
            ibis_formats::Kind::Workbook => TextContentKind::Workbook,
            ibis_formats::Kind::Document if self.is_law_document() => TextContentKind::Lawtext,
            _ => TextContentKind::Markdown,
        }
    }

    /// Schedule a debounced check to auto-switch between Markdown / Marp / Workbook / Lawtext
    /// when the user edits or pastes content whose format differs from the active editor kind.
    pub fn schedule_text_content_auto_switch(
        &mut self,
        editor: Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inline_completion.auto_switch_generation =
            self.inline_completion.auto_switch_generation.wrapping_add(1);
        let expected_gen = self.inline_completion.auto_switch_generation;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(450))
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.inline_completion.auto_switch_generation != expected_gen {
                    return;
                }
                let text = editor.read(cx).value().to_string();
                let _ = this.reopen_text_source_as_detected(text, window, cx);
            });
        })
        .detach();
    }

    /// If `text` is detected as a different [`TextContentKind`] than the current editor,
    /// re-open it in-place in that format while preserving the text source and split view.
    pub fn reopen_text_source_as_detected(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        let detected = detect_text_content_kind(text.as_bytes());
        if detected == self.current_text_content_kind() {
            return false;
        }
        let base_dir = self.current_path().and_then(|p| p.parent()).map(std::path::Path::to_path_buf);
        let opened_result: Result<(ibis_formats::Opened, bool), String> = match detected {
            TextContentKind::Marp => {
                crate::read_file::open_markdown_bytes(text.as_bytes().to_vec(), base_dir.as_deref(), true)
                    .map(|o| (o, false))
            }
            TextContentKind::Workbook => {
                ibis_formats::markdown::open_markdown_workbook(text.as_bytes().to_vec())
                    .map(|o| (o, false))
                    .map_err(|e| e.to_string())
            }
            TextContentKind::Lawtext => match ibis_lawtext::parse_lawtext(&text) {
                Ok(law) => {
                    let law_md = ibis_lawtext::to_markdown(&law);
                    crate::read_file::open_markdown_bytes(law_md.into_bytes(), base_dir.as_deref(), false)
                        .map(|o| (o, true))
                }
                Err(_) => return false,
            },
            TextContentKind::Markdown => {
                crate::read_file::open_markdown_bytes(text.as_bytes().to_vec(), base_dir.as_deref(), false)
                    .map(|o| (o, false))
            }
        };
        let Ok((opened, is_law)) = opened_result else {
            return false;
        };
        let next_name: SharedString = match self.file_name.as_deref() {
            Some(current) => {
                let lower = current.to_ascii_lowercase();
                let is_untitled = lower.starts_with("untitled")
                    || current.starts_with("文書 ")
                    || current.starts_with("無題");
                if is_untitled {
                    if is_law {
                        "Untitled.lawtext".into()
                    } else {
                        "Untitled.md".into()
                    }
                } else {
                    current.to_string().into()
                }
            }
            None => {
                if is_law {
                    "Untitled.lawtext".into()
                } else {
                    "Untitled.md".into()
                }
            }
        };
        self.load(opened.presentation, next_name, window, cx);
        self.source = Some(std::sync::Arc::new(opened.source));
        #[cfg(target_family = "wasm")]
        {
            self.web.law_source = if is_law {
                Some(std::sync::Arc::new(text.into_bytes()))
            } else {
                None
            };
        }
        crate::format::with_format!(self, new_file_shown(self, cx));
        if !self.should_auto_open_markdown_split() {
            window.dispatch_action(Box::new(crate::actions::ToggleMarkdownSource), cx);
        }
        cx.notify();
        true
    }
}

struct CopilotInlineCompletionProvider {
    slide_editor: WeakEntity<SlideEditor>,
    immediate_once: Rc<Cell<bool>>,
}

impl CompletionProvider for CopilotInlineCompletionProvider {
    fn completions(
        &self,
        _text: &Rope,
        _offset: usize,
        _trigger: lsp_types::CompletionContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<anyhow::Result<lsp_types::CompletionResponse>> {
        Task::ready(Ok(lsp_types::CompletionResponse::Array(vec![])))
    }

    fn is_completion_trigger(&self, _offset: usize, _new_text: &str, _cx: &mut App) -> bool {
        false
    }

    fn inline_completion_debounce(&self) -> std::time::Duration {
        if self.immediate_once.replace(false) {
            std::time::Duration::ZERO
        } else {
            std::time::Duration::from_millis(700)
        }
    }

    fn inline_completion(
        &self,
        rope: &Rope,
        offset: usize,
        trigger: lsp_types::InlineCompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<lsp_types::InlineCompletionResponse>> {
        let invoked = trigger.trigger_kind == lsp_types::InlineCompletionTriggerKind::Invoked;
        let text = rope.to_string();
        if text.trim().is_empty() {
            return Task::ready(Ok(lsp_types::InlineCompletionResponse::Array(vec![])));
        }

        let Some(editor_entity) = self.slide_editor.upgrade() else {
            return Task::ready(Ok(lsp_types::InlineCompletionResponse::Array(vec![])));
        };

        let completion_gen = editor_entity.update(cx, |this, cx| {
            if invoked {
                this.inline_completion.enabled = true;
            } else if !this.inline_completion.enabled {
                return None;
            }
            this.inline_completion.generation = this.inline_completion.generation.wrapping_add(1);
            this.inline_completion.loading = true;
            this.inline_completion.has_suggestion = false;
            this.inline_completion.error = None;
            cx.notify();
            Some(this.inline_completion.generation)
        });

        let Some(completion_gen) = completion_gen else {
            return Task::ready(Ok(lsp_types::InlineCompletionResponse::Array(vec![])));
        };

        let (prefix, suffix) = crate::ai_client::slice_prefix_suffix_by_offset(&text, offset);
        let weak_editor = self.slide_editor.clone();

        cx.spawn(async move |cx: &mut AsyncApp| {
            let result = crate::ai_client::complete_inline_suggestion(&prefix, &suffix).await;
            let has_text = matches!(&result, Ok(s) if !s.trim().is_empty());
            let _ = weak_editor.update(cx, |this, cx| {
                if this.inline_completion.generation != completion_gen {
                    return;
                }
                this.inline_completion.loading = false;
                this.inline_completion.has_suggestion = has_text;
                if let Err(err) = &result {
                    let explained = crate::ai_client::explain_ai_error(err);
                    this.inline_completion.error = if explained.is_empty() {
                        None
                    } else {
                        Some(explained)
                    };
                } else {
                    this.inline_completion.error = None;
                }
                cx.notify();
            });

            match result {
                Ok(suggestion) if !suggestion.trim().is_empty() => {
                    Ok(lsp_types::InlineCompletionResponse::Array(vec![
                        lsp_types::InlineCompletionItem {
                            insert_text: suggestion,
                            filter_text: None,
                            range: None,
                            command: None,
                            insert_text_format: None,
                        },
                    ]))
                }
                _ => Ok(lsp_types::InlineCompletionResponse::Array(vec![])),
            }
        })
    }
}

impl SlideEditor {
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
    fn detect_text_content_kind_classifies_all_four_formats() {
        assert_eq!(
            detect_text_content_kind(b"# Plain Note\n\nHello world\n"),
            TextContentKind::Markdown
        );
        assert_eq!(
            detect_text_content_kind(TEMPLATE_MARP.as_bytes()),
            TextContentKind::Marp
        );
        assert_eq!(
            detect_text_content_kind(TEMPLATE_WORKBOOK.as_bytes()),
            TextContentKind::Workbook
        );
        assert_eq!(
            detect_text_content_kind(TEMPLATE_LAWTEXT.as_bytes()),
            TextContentKind::Lawtext
        );
    }

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

        let ibis_multi = include_str!("../../../assets/help/samples/workbook-sheet.md");
        let ibis_spans = workbook_sheet_spans(ibis_multi);
        assert_eq!(ibis_spans.len(), 2);
        assert_eq!(ibis_spans[0].title.as_deref(), Some("売上計画"));
        assert_eq!(ibis_spans[1].title.as_deref(), Some("構成メモ"));
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

    #[::core::prelude::v1::test]
    fn law_section_spans_maps_lawtext_and_egov_xml_articles() {
        let lawtext = "\
テスト法（令和七年法律第一号）

第一章 総則

（目的）
第一条 この法律は、第一条第二項に定める「基準」を示す。
２ 前項の規定を補う。

（定義）
第二条 用語を定める。
一 第一号の事項

附則
この法律は、公布の日から施行する。
";
        let spans = law_section_spans(lawtext, SourceDialect::Lawtext);
        let titles: Vec<&str> = spans
            .iter()
            .filter_map(|s| s.title.as_deref())
            .collect();
        assert_eq!(titles, vec!["第一章 総則", "第一条", "第二条", "附則"]);

        let art2_item = lawtext.lines().position(|l| l.starts_with("一 ")).unwrap();
        assert_eq!(
            section_for_line(&spans, art2_item).and_then(|s| s.title.as_deref()),
            Some("第二条")
        );

        let mut lh = PureRustLawtextHighlighter::new();
        lh.reparse(lawtext);
        assert!(lh.spans.iter().any(|s| s.kind == TokenKind::Heading));
        assert!(lh.spans.iter().any(|s| s.kind == TokenKind::Strong));
        assert!(lh.spans.iter().any(|s| s.kind == TokenKind::FrontmatterKey));
        assert!(lh.spans.iter().any(|s| s.kind == TokenKind::ListMarker));
        assert!(lh.spans.iter().any(|s| s.kind == TokenKind::LinkText));
        assert!(!lh.folds.is_empty());

        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Law Era="Reiwa" Lang="ja" LawType="Act" Num="001" Year="07">
  <LawBody>
    <MainProvision>
      <Chapter Num="1">
        <ChapterTitle>第一章 総則</ChapterTitle>
        <Article Num="1">
          <ArticleTitle>第一条</ArticleTitle>
          <Paragraph Num="1"><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence></Paragraph>
        </Article>
      </Chapter>
    </MainProvision>
    <SupplProvision>
      <SupplProvisionLabel>附則</SupplProvisionLabel>
    </SupplProvision>
  </LawBody>
</Law>"#;
        let xml_spans = law_section_spans(xml, SourceDialect::LawXml);
        let xml_titles: Vec<&str> = xml_spans
            .iter()
            .filter_map(|s| s.title.as_deref())
            .collect();
        assert_eq!(xml_titles, vec!["第一章 総則", "第一条", "附則"]);

        let mut xh = PureRustXmlHighlighter::new();
        xh.reparse(xml);
        assert!(xh.spans.iter().any(|s| s.kind == TokenKind::FrontmatterDelimiter));
        assert!(xh.spans.iter().any(|s| s.kind == TokenKind::CodeFence));
        assert!(xh.spans.iter().any(|s| s.kind == TokenKind::FrontmatterKey));
        assert!(xh.spans.iter().any(|s| s.kind == TokenKind::LinkText));
        assert!(!xh.folds.is_empty());
    }
}
