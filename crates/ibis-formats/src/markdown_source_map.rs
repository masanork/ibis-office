// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Block-level source map (`MarkdownSourceMap`) between Markdown source text and
//! `FlowDocument::body`, plus surgical WYSIWYG → Markdown patching that preserves
//! untouched Markdown formatting and minimizes Git diff noise.

use std::ops::Range;

use ibis_doc_model::{Block, FlowDocument, Media};

/// Mapping between one top-level `FlowDocument::body` block and its range in Markdown text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownBlockSpan {
    /// Index of the block inside `FlowDocument::body`.
    pub block_index: usize,
    /// Paragraph IDs contained in this block (one ID for `Block::Paragraph`,
    /// all cell paragraph IDs for `Block::Table`).
    pub para_ids: Vec<u32>,
    /// UTF-8 byte range in the Markdown source string (trimmed of trailing newlines).
    pub byte_range: Range<usize>,
    /// 0-based `[start_line, end_line)` line range in the Markdown source string.
    pub line_range: Range<usize>,
}

/// Bidirectional block-level map between a Markdown source string and `FlowDocument::body`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownSourceMap {
    /// Total number of lines in the mapped Markdown string.
    pub total_lines: usize,
    /// Ordered spans corresponding to `FlowDocument::body`.
    pub blocks: Vec<MarkdownBlockSpan>,
    /// True when `byte_range`s slice the exact source string and are safe for
    /// surgical block-level replacement during WYSIWYG → Source writeback.
    pub patchable: bool,
}

impl MarkdownSourceMap {
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn span_for_block(&self, block_index: usize) -> Option<&MarkdownBlockSpan> {
        self.blocks.get(block_index).filter(|s| s.block_index == block_index)
            .or_else(|| self.blocks.iter().find(|s| s.block_index == block_index))
    }

    pub fn span_for_para(&self, para_id: u32) -> Option<&MarkdownBlockSpan> {
        self.blocks.iter().find(|s| s.para_ids.contains(&para_id))
    }

    /// Find the block containing 0-based `line`, or the nearest preceding block
    /// when `line` falls on a blank separator line (falling back to the first block
    /// when `line` is inside frontmatter above the first body block).
    pub fn span_for_line(&self, line: usize) -> Option<&MarkdownBlockSpan> {
        if self.blocks.is_empty() {
            return None;
        }
        if let Some(exact) = self
            .blocks
            .iter()
            .find(|s| line >= s.line_range.start && line < s.line_range.end)
        {
            return Some(exact);
        }
        self.blocks
            .iter()
            .rev()
            .find(|s| s.line_range.start <= line)
            .or_else(|| self.blocks.first())
    }

    /// Build a [`MarkdownSourceMap`] from `(block_index, start_byte, end_byte)` records
    /// emitted against `text` (e.g. by the Markdown exporter).
    pub(crate) fn from_byte_spans(
        text: &str,
        doc: &FlowDocument,
        spans: Vec<(usize, usize, usize)>,
        patchable: bool,
    ) -> Self {
        let starts = line_starts(text);
        let total_lines = starts.len();
        let mut blocks = Vec::with_capacity(spans.len());
        for (block_index, raw_start, raw_end) in spans {
            let Some(block) = doc.body.get(block_index) else {
                continue;
            };
            let byte_range = trim_trailing_newlines(text, raw_start, raw_end);
            let line_range = byte_range_to_line_range(&starts, text, &byte_range);
            blocks.push(MarkdownBlockSpan {
                block_index,
                para_ids: collect_block_para_ids(block),
                byte_range,
                line_range,
            });
        }
        Self {
            total_lines,
            blocks,
            patchable,
        }
    }

    /// Build a [`MarkdownSourceMap`] from per-block 0-based line ranges relative to
    /// `line_offset` inside `text` (emitted by the Markdown parser).
    pub(crate) fn from_line_spans(
        text: &str,
        doc: &FlowDocument,
        line_offset: usize,
        raw_line_spans: Vec<Range<usize>>,
        patchable: bool,
    ) -> Self {
        let starts = line_starts(text);
        let total_lines = starts.len();
        let mut blocks = Vec::with_capacity(raw_line_spans.len().min(doc.body.len()));
        for (block_index, raw_lines) in raw_line_spans.into_iter().enumerate() {
            let Some(block) = doc.body.get(block_index) else {
                break;
            };
            let start_line = (line_offset + raw_lines.start).min(total_lines.saturating_sub(1));
            let end_line = (line_offset + raw_lines.end)
                .max(start_line + 1)
                .min(total_lines);
            let start_byte = starts.get(start_line).copied().unwrap_or(text.len());
            let raw_end_byte = if end_line < total_lines {
                starts[end_line]
            } else {
                text.len()
            };
            let byte_range = trim_trailing_newlines(text, start_byte, raw_end_byte);
            let line_range = start_line..end_line;
            blocks.push(MarkdownBlockSpan {
                block_index,
                para_ids: collect_block_para_ids(block),
                byte_range,
                line_range,
            });
        }
        let is_patchable = patchable && blocks.len() == doc.body.len();
        Self {
            total_lines,
            blocks,
            patchable: is_patchable,
        }
    }
}

pub(crate) fn collect_block_para_ids(block: &Block) -> Vec<u32> {
    let mut ids = Vec::new();
    fn push_block(block: &Block, ids: &mut Vec<u32>) {
        match block {
            Block::Paragraph(p) => ids.push(p.id),
            Block::Table(t) => {
                for row in &t.rows {
                    for cell in &row.cells {
                        for inner in &cell.blocks {
                            push_block(inner, ids);
                        }
                    }
                }
            }
        }
    }
    push_block(block, &mut ids);
    ids
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (idx, b) in text.bytes().enumerate() {
        if b == b'\n' && idx + 1 < text.len() {
            starts.push(idx + 1);
        }
    }
    starts
}

fn trim_trailing_newlines(text: &str, start: usize, end: usize) -> Range<usize> {
    let start = start.min(text.len());
    let mut end = end.clamp(start, text.len());
    while end > start && matches!(text.as_bytes()[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    start..end
}

fn byte_to_line(starts: &[usize], offset: usize) -> usize {
    match starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

fn byte_range_to_line_range(starts: &[usize], text: &str, byte_range: &Range<usize>) -> Range<usize> {
    if starts.is_empty() {
        return 0..0;
    }
    let start_line = byte_to_line(starts, byte_range.start.min(text.len()));
    let end_probe = if byte_range.end > byte_range.start {
        byte_range.end - 1
    } else {
        byte_range.start
    };
    let end_line = (byte_to_line(starts, end_probe.min(text.len())) + 1).min(starts.len());
    start_line..end_line.max(start_line + 1).min(starts.len())
}

fn metadata_unchanged(prev: &FlowDocument, next: &FlowDocument) -> bool {
    prev.page == next.page
        && prev.header == next.header
        && prev.footer == next.footer
        && prev.header_first == next.header_first
        && prev.footer_first == next.footer_first
        && prev.header_even == next.header_even
        && prev.footer_even == next.footer_even
        && prev.title_page == next.title_page
        && prev.even_and_odd == next.even_and_odd
        && prev.restart_numbering == next.restart_numbering
        && prev.linked == next.linked
        && prev.footnotes == next.footnotes
        && prev.endnotes == next.endnotes
        && prev.comments == next.comments
        && prev.watermark == next.watermark
        && prev.section_watermarks == next.section_watermarks
        && prev.page_color == next.page_color
        && prev.properties == next.properties
        && prev.bibliography_sources == next.bibliography_sources
        && prev.styles == next.styles
}

fn spans_valid_and_monotonic(text: &str, blocks: &[MarkdownBlockSpan]) -> bool {
    let mut prev_start = 0usize;
    for (idx, span) in blocks.iter().enumerate() {
        if span.block_index != idx {
            return false;
        }
        let Range { start, end } = span.byte_range;
        if start > end
            || end > text.len()
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
            || start < prev_start
        {
            return false;
        }
        prev_start = start;
    }
    true
}

fn detect_newline(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

fn apply_newline_style(fragment: &str, nl: &str) -> String {
    if nl == "\r\n" {
        fragment.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        fragment.to_owned()
    }
}

/// Attempt to surgically patch only the blocks modified between `prev_doc` and `new_doc`
/// onto `prev_source`, preserving untouched formatting (frontmatter, `*` bullets, custom
/// blank lines, Mermaid blocks, tables) byte-for-byte.
///
/// Returns `None` when a safe surgical patch cannot be guaranteed (e.g., non-patchable
/// source map, document metadata changed, or whole-document structural rewrite), so the
/// caller can fall back to full export.
pub fn try_patch_markdown_from_visual_edit(
    prev_source: &str,
    prev_doc: &FlowDocument,
    prev_map: &MarkdownSourceMap,
    new_doc: &FlowDocument,
    media: &[Media],
) -> Option<(String, MarkdownSourceMap)> {
    if !metadata_unchanged(prev_doc, new_doc) {
        return None;
    }
    if prev_doc.body == new_doc.body {
        return Some((prev_source.to_owned(), prev_map.clone()));
    }
    if !prev_map.patchable
        || prev_map.blocks.len() != prev_doc.body.len()
        || !spans_valid_and_monotonic(prev_source, &prev_map.blocks)
    {
        return None;
    }

    let (fresh_md, fresh_map) = crate::markdown::export_markdown_with_source_map(new_doc, media);
    if fresh_map.blocks.len() != new_doc.body.len()
        || !spans_valid_and_monotonic(&fresh_md, &fresh_map.blocks)
    {
        return None;
    }

    let nl = detect_newline(prev_source);

    // Case A: Same block count — patch each changed contiguous byte-range group independently.
    if prev_doc.body.len() == new_doc.body.len() {
        // Partition block indices into groups that share the exact same `prev_map` byte_range.
        let mut groups: Vec<(usize, usize, Range<usize>)> = Vec::new();
        let mut idx = 0;
        while idx < prev_map.blocks.len() {
            let range = prev_map.blocks[idx].byte_range.clone();
            let mut end_idx = idx + 1;
            while end_idx < prev_map.blocks.len() && prev_map.blocks[end_idx].byte_range == range {
                end_idx += 1;
            }
            if let Some((_, _, prev_range)) = groups.last()
                && prev_range.end > range.start
            {
                return None;
            }
            groups.push((idx, end_idx, range));
            idx = end_idx;
        }

        let mut out = String::with_capacity(prev_source.len() + 64);
        let mut cursor = 0usize;
        let mut new_byte_spans: Vec<(usize, usize, usize)> = Vec::with_capacity(new_doc.body.len());

        for (g_start, g_end, old_range) in groups {
            let changed = (g_start..g_end).any(|k| prev_doc.body[k] != new_doc.body[k]);
            out.push_str(&prev_source[cursor..old_range.start]);
            let dst_start = out.len();

            if !changed {
                out.push_str(&prev_source[old_range.start..old_range.end]);
                let dst_end = out.len();
                for k in g_start..g_end {
                    new_byte_spans.push((k, dst_start, dst_end));
                }
            } else {
                let f_start = fresh_map.blocks[g_start].byte_range.start;
                let f_end = fresh_map.blocks[g_end - 1].byte_range.end;
                let replacement = apply_newline_style(&fresh_md[f_start..f_end], nl);
                out.push_str(&replacement);
                let dst_end = out.len();
                if g_end - g_start == 1 {
                    new_byte_spans.push((g_start, dst_start, dst_end));
                } else {
                    for k in g_start..g_end {
                        // Multi-block construct from a single source group: map each to the spliced group span.
                        new_byte_spans.push((k, dst_start, dst_end));
                    }
                }
            }
            cursor = old_range.end;
        }
        out.push_str(&prev_source[cursor..]);
        let updated_map = MarkdownSourceMap::from_byte_spans(&out, new_doc, new_byte_spans, true);
        return Some((out, updated_map));
    }

    // Case B: Structural edit (block insertion, split, merge, or deletion).
    // Preserve longest common prefix and longest common suffix byte-for-byte.
    let prev_len = prev_doc.body.len();
    let new_len = new_doc.body.len();

    let mut prefix = 0usize;
    while prefix < prev_len && prefix < new_len && prev_doc.body[prefix] == new_doc.body[prefix] {
        prefix += 1;
    }
    while prefix > 0
        && prefix < prev_len
        && prev_map.blocks[prefix - 1].byte_range == prev_map.blocks[prefix].byte_range
    {
        prefix -= 1;
    }

    let mut suffix = 0usize;
    while suffix < (prev_len - prefix)
        && suffix < (new_len - prefix)
        && prev_doc.body[prev_len - 1 - suffix] == new_doc.body[new_len - 1 - suffix]
    {
        suffix += 1;
    }
    while suffix > 0
        && prev_len - suffix > prefix
        && prev_map.blocks[prev_len - suffix - 1].byte_range
            == prev_map.blocks[prev_len - suffix].byte_range
    {
        suffix -= 1;
    }

    if prefix == 0 && suffix == 0 {
        return None;
    }

    let prev_mid = prefix..(prev_len - suffix);
    let new_mid = prefix..(new_len - suffix);

    // Ensure non-overlapping boundaries between prefix, middle, and suffix.
    if prefix > 0 && !prev_mid.is_empty()
        && prev_map.blocks[prefix - 1].byte_range.end > prev_map.blocks[prev_mid.start].byte_range.start
    {
        return None;
    }
    if suffix > 0 && !prev_mid.is_empty()
        && prev_map.blocks[prev_mid.end - 1].byte_range.end > prev_map.blocks[prev_len - suffix].byte_range.start
    {
        return None;
    }
    if prefix > 0 && suffix > 0 && prev_mid.is_empty()
        && prev_map.blocks[prefix - 1].byte_range.end > prev_map.blocks[prev_len - suffix].byte_range.start
    {
        return None;
    }

    let mut out = String::with_capacity(prev_source.len() + 128);
    let mut new_byte_spans: Vec<(usize, usize, usize)> = Vec::with_capacity(new_len);

    // Helper: copy prefix block spans unchanged.
    for k in 0..prefix {
        let r = &prev_map.blocks[k].byte_range;
        new_byte_spans.push((k, r.start, r.end));
    }

    let suffix_old_slice_start: usize;

    if !prev_mid.is_empty() && !new_mid.is_empty() {
        // Replace `prev_mid` bytes with `new_mid` bytes from `fresh_md`.
        let old_start = prev_map.blocks[prev_mid.start].byte_range.start;
        let old_end = prev_map.blocks[prev_mid.end - 1].byte_range.end;
        let f_start = fresh_map.blocks[new_mid.start].byte_range.start;
        let f_end = fresh_map.blocks[new_mid.end - 1].byte_range.end;

        out.push_str(&prev_source[..old_start]);
        let dst_mid_start = out.len();
        let replacement = apply_newline_style(&fresh_md[f_start..f_end], nl);
        out.push_str(&replacement);
        let dst_mid_end = out.len();

        if nl == "\n" {
            for k in new_mid.clone() {
                let r = &fresh_map.blocks[k].byte_range;
                let s = dst_mid_start + (r.start - f_start);
                let e = dst_mid_start + (r.end - f_start);
                new_byte_spans.push((k, s, e));
            }
        } else {
            for k in new_mid.clone() {
                new_byte_spans.push((k, dst_mid_start, dst_mid_end));
            }
        }
        suffix_old_slice_start = old_end;
    } else if prev_mid.is_empty() && !new_mid.is_empty() {
        // Pure insertion of `new_mid` blocks.
        let f_start = fresh_map.blocks[new_mid.start].byte_range.start;
        let f_end = fresh_map.blocks[new_mid.end - 1].byte_range.end;
        let inserted = apply_newline_style(&fresh_md[f_start..f_end], nl);

        if prefix > 0 {
            // Determine separator from fresh_md between block `prefix - 1` and `prefix`.
            let fresh_sep = &fresh_md
                [fresh_map.blocks[prefix - 1].byte_range.end..fresh_map.blocks[prefix].byte_range.start];
            let sep = if fresh_sep.is_empty() {
                format!("{nl}{nl}")
            } else {
                apply_newline_style(fresh_sep, nl)
            };
            let anchor = prev_map.blocks[prefix - 1].byte_range.end;
            out.push_str(&prev_source[..anchor]);
            out.push_str(&sep);
            let dst_mid_start = out.len();
            out.push_str(&inserted);
            let dst_mid_end = out.len();
            if nl == "\n" {
                for k in new_mid.clone() {
                    let r = &fresh_map.blocks[k].byte_range;
                    new_byte_spans.push((k, dst_mid_start + (r.start - f_start), dst_mid_start + (r.end - f_start)));
                }
            } else {
                for k in new_mid.clone() {
                    new_byte_spans.push((k, dst_mid_start, dst_mid_end));
                }
            }
            suffix_old_slice_start = anchor;
        } else {
            // Inserting before the first block (`suffix > 0`).
            let fresh_sep = &fresh_md
                [fresh_map.blocks[new_mid.end - 1].byte_range.end..fresh_map.blocks[new_mid.end].byte_range.start];
            let sep = if fresh_sep.is_empty() {
                format!("{nl}{nl}")
            } else {
                apply_newline_style(fresh_sep, nl)
            };
            let anchor = prev_map.blocks[0].byte_range.start;
            out.push_str(&prev_source[..anchor]);
            let dst_mid_start = out.len();
            out.push_str(&inserted);
            let dst_mid_end = out.len();
            out.push_str(&sep);
            if nl == "\n" {
                for k in new_mid.clone() {
                    let r = &fresh_map.blocks[k].byte_range;
                    new_byte_spans.push((k, dst_mid_start + (r.start - f_start), dst_mid_start + (r.end - f_start)));
                }
            } else {
                for k in new_mid.clone() {
                    new_byte_spans.push((k, dst_mid_start, dst_mid_end));
                }
            }
            suffix_old_slice_start = anchor;
        }
    } else {
        // Pure deletion of `prev_mid` blocks.
        if prefix > 0 {
            let keep_end = prev_map.blocks[prefix - 1].byte_range.end;
            out.push_str(&prev_source[..keep_end]);
            suffix_old_slice_start = prev_map.blocks[prev_mid.end - 1].byte_range.end;
        } else {
            // Deleting leading blocks while `suffix > 0`.
            let keep_start = prev_map.blocks[0].byte_range.start;
            out.push_str(&prev_source[..keep_start]);
            suffix_old_slice_start = prev_map.blocks[prev_len - suffix].byte_range.start;
        }
    }

    let suffix_dst_offset = out.len();
    out.push_str(&prev_source[suffix_old_slice_start..]);

    for s_idx in 0..suffix {
        let old_k = prev_len - suffix + s_idx;
        let new_k = new_len - suffix + s_idx;
        let old_r = &prev_map.blocks[old_k].byte_range;
        if old_r.start < suffix_old_slice_start {
            return None;
        }
        let s = suffix_dst_offset + (old_r.start - suffix_old_slice_start);
        let e = suffix_dst_offset + (old_r.end - suffix_old_slice_start);
        new_byte_spans.push((new_k, s, e));
    }

    let updated_map = MarkdownSourceMap::from_byte_spans(&out, new_doc, new_byte_spans, true);
    Some((out, updated_map))
}

/// Patch `prev_source` with WYSIWYG edits from `prev_doc` to `new_doc`, falling back to
/// full deterministic Markdown export when surgical patching is not applicable.
pub fn patch_markdown_from_visual_edit(
    prev_source: &str,
    prev_doc: &FlowDocument,
    prev_map: &MarkdownSourceMap,
    new_doc: &FlowDocument,
    media: &[Media],
) -> (String, MarkdownSourceMap) {
    try_patch_markdown_from_visual_edit(prev_source, prev_doc, prev_map, new_doc, media)
        .unwrap_or_else(|| crate::markdown::export_markdown_with_source_map(new_doc, media))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{
        export_markdown_with_source_map, open_markdown, reparse_markdown_document_with_source_map,
    };
    use ibis_doc_model::{Block, FlowDocument, FlowParagraph, FlowRun, FlowRunKind, FlowRunStyle};

    #[test]
    fn parse_source_map_tracks_frontmatter_headings_mermaid_tables_and_lists() {
        let md = "\
---
title: Phase 4 Demo
---

# Overview

* bullet one
* bullet two

Intro paragraph.

| Col A | Col B |
| --- | --- |
| v1 | v2 |

```mermaid
graph LR
  A --> B
```
";
        let mut media = Vec::new();
        let (doc, map) = reparse_markdown_document_with_source_map(
            md,
            &FlowDocument::default(),
            &mut media,
            None,
            None,
            None,
        )
        .expect("parse with source map");

        assert!(map.patchable, "direct parse should produce a patchable source map");
        assert_eq!(map.blocks.len(), doc.body.len());

        // Line 4 is `# Overview`
        let heading_span = map.span_for_line(4).expect("heading span");
        assert_eq!(&md[heading_span.byte_range.clone()], "# Overview");
        assert_eq!(heading_span.line_range, 4..5);

        // Blank line 5 falls back to nearest preceding block (`# Overview`)
        assert_eq!(map.span_for_line(5).unwrap().block_index, heading_span.block_index);

        // Frontmatter line 1 falls back to first body block
        assert_eq!(map.span_for_line(1).unwrap().block_index, 0);

        // Table block maps all its cell paragraph IDs back to the table lines (11..14)
        let table_idx = doc
            .body
            .iter()
            .position(|b| matches!(b, Block::Table(_)))
            .expect("table block");
        let table_span = map.span_for_block(table_idx).expect("table span");
        assert!(!table_span.para_ids.is_empty());
        assert_eq!(table_span.line_range, 11..14);
        for &pid in &table_span.para_ids {
            assert_eq!(map.span_for_para(pid).unwrap().block_index, table_idx);
        }

        // Mermaid block spans lines 15..19
        let mermaid_span = map.span_for_line(16).expect("mermaid span");
        assert_eq!(mermaid_span.line_range, 15..19);
        assert!(md[mermaid_span.byte_range.clone()].starts_with("```mermaid"));
    }

    #[test]
    fn surgical_patch_preserves_untouched_star_lists_frontmatter_and_mermaid() {
        let original = "\
---
title:   Custom Spacing
author:  masanork
---

# Architecture


* first star item
* second star item

Target paragraph to edit.

```mermaid
graph TD
  Left --> Right
```
";
        let opened = open_markdown(original.as_bytes().to_vec()).expect("open markdown");
        let (stored_text, stored_map, stored_doc) =
            opened.source.original_markdown().expect("source retains original markdown");
        assert_eq!(stored_text, original);
        assert!(stored_map.patchable);

        // Edit only the "Target paragraph to edit." block in WYSIWYG.
        let mut edited = (**stored_doc).clone();
        let mut changed = false;
        for block in &mut edited.body {
            if let Block::Paragraph(p) = block {
                for run in &mut p.runs {
                    if let FlowRunKind::Text(t) = &mut run.kind
                        && t.contains("Target paragraph")
                    {
                        *t = "Surgically updated paragraph.".to_owned();
                        changed = true;
                    }
                }
            }
        }
        assert!(changed);

        let (patched, updated_map) = try_patch_markdown_from_visual_edit(
            stored_text,
            stored_doc,
            stored_map,
            &edited,
            &opened.presentation.media,
        )
        .expect("surgical patch should succeed");

        let expected = original.replace(
            "Target paragraph to edit.",
            "Surgically updated paragraph.",
        );
        assert_eq!(
            patched, expected,
            "untouched frontmatter, double blank lines, '*' bullets, and mermaid fence must be byte-exact"
        );
        assert!(updated_map.patchable);
        assert_eq!(updated_map.blocks.len(), edited.body.len());

        // Verify updated map slices match the patched document accurately.
        let last_span = updated_map.blocks.last().unwrap();
        assert!(patched[last_span.byte_range.clone()].starts_with("```mermaid"));
    }

    #[test]
    fn surgical_patch_handles_paragraph_insertion_and_deletion_preserving_neighbors() {
        let original = "\
# Heading

* star alpha
* star beta

Middle block.

Footer block.
";
        let opened = open_markdown(original.as_bytes().to_vec()).expect("open markdown");
        let (stored_text, stored_map, stored_doc) = opened.source.original_markdown().unwrap();

        // 1. Insert a new paragraph right after "Middle block."
        let mut with_insert = (**stored_doc).clone();
        let middle_idx = with_insert
            .body
            .iter()
            .position(|b| matches!(b, Block::Paragraph(p) if p.text().contains("Middle block.")))
            .expect("middle block");
        let mut inserted_para = FlowParagraph {
            id: with_insert.new_id(),
            style: with_insert.styles.default_paragraph.clone(),
            runs: vec![FlowRun::new(
                FlowRunKind::Text("Inserted paragraph.".into()),
                FlowRunStyle::default(),
            )],
            ..FlowParagraph::default()
        };
        inserted_para.resolve(&with_insert.styles);
        with_insert.body.insert(middle_idx + 1, Block::Paragraph(inserted_para));

        let (patched_insert, map_after_insert) = try_patch_markdown_from_visual_edit(
            stored_text,
            stored_doc,
            stored_map,
            &with_insert,
            &opened.presentation.media,
        )
        .expect("insertion surgical patch");

        assert!(patched_insert.contains("* star alpha\n* star beta"), "prefix star bullets preserved:\n{patched_insert}");
        assert!(patched_insert.contains("Middle block.\n\nInserted paragraph.\n\nFooter block."), "inserted cleanly:\n{patched_insert}");
        assert_eq!(map_after_insert.blocks.len(), with_insert.body.len());

        // 2. Now delete the inserted paragraph back — should return to original!
        let (patched_delete, map_after_delete) = try_patch_markdown_from_visual_edit(
            &patched_insert,
            &with_insert,
            &map_after_insert,
            stored_doc,
            &opened.presentation.media,
        )
        .expect("deletion surgical patch");

        assert_eq!(patched_delete, original);
        assert_eq!(map_after_delete.blocks.len(), stored_doc.body.len());
    }

    #[test]
    fn surgical_patch_preserves_crlf_line_endings() {
        let original = "# Title\r\n\r\n* star item\r\n\r\nOld body.\r\n";
        let opened = open_markdown(original.as_bytes().to_vec()).expect("open crlf");
        let (stored_text, stored_map, stored_doc) = opened.source.original_markdown().unwrap();

        let mut edited = (**stored_doc).clone();
        for block in &mut edited.body {
            if let Block::Paragraph(p) = block {
                for run in &mut p.runs {
                    if let FlowRunKind::Text(t) = &mut run.kind
                        && t == "Old body."
                    {
                        *t = "New body.".to_owned();
                    }
                }
            }
        }

        let (patched, _) = try_patch_markdown_from_visual_edit(
            stored_text,
            stored_doc,
            stored_map,
            &edited,
            &opened.presentation.media,
        )
        .expect("crlf patch");
        assert_eq!(patched, "# Title\r\n\r\n* star item\r\n\r\nNew body.\r\n");
    }

    #[test]
    fn surgical_patch_falls_back_when_document_metadata_changes() {
        let original = "---\ntitle: Before\n---\n\nBody.\n";
        let opened = open_markdown(original.as_bytes().to_vec()).expect("open");
        let (stored_text, stored_map, stored_doc) = opened.source.original_markdown().unwrap();

        let mut edited = (**stored_doc).clone();
        edited.properties.title = Some("After".into());

        assert!(
            try_patch_markdown_from_visual_edit(
                stored_text,
                stored_doc,
                stored_map,
                &edited,
                &opened.presentation.media,
            )
            .is_none(),
            "metadata edits must fall back to full export so frontmatter updates"
        );

        let (full, full_map) =
            patch_markdown_from_visual_edit(stored_text, stored_doc, stored_map, &edited, &opened.presentation.media);
        assert!(full.contains("title: After"));
        assert!(!full_map.is_empty());

        let (exported, exp_map) = export_markdown_with_source_map(&edited, &opened.presentation.media);
        assert_eq!(full, exported);
        assert_eq!(full_map, exp_map);
    }
}
