// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Bidirectional Workbook Markdown Split View (`Source | Split | Visual`).
//!
//! Left pane: pure-Rust syntax-highlighted Workbook Markdown editor (`<!-- xlsx-sheet ... -->`,
//! `## SheetName`, GFM tables, formula markers).
//! Right pane: live editable native Excel grid with formula bar and sheet tabs.
//!
//! - Source edits sync to live worksheets via 250ms debounce (or immediately on focus-leave) using
//!   [`Command::ReplacePresentation`], preserving active sheet index.
//! - Visual cell/sheet edits sync back to Markdown via 200ms debounce (or immediately on
//!   focus-enter) by watching [`ibis_doc_model::Document::revision`].
//! - Sheet-level cursor/scroll sync keeps the active worksheet tab and Markdown sheet section aligned.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::input::{EditorState, InputEvent};
use gpui_kit::*;
use ibis_doc_model::{Command, SheetState, SlideId};

use crate::ext::*;
use crate::shell::editor::SlideEditor;
use crate::shell::markdown_split::{
    DEFAULT_SPLIT_RATIO, MarkdownSplitHost, MarkdownViewMode, SOURCE_TO_VISUAL_DEBOUNCE_MS,
    SplitViewCallbacks, SyncAuthority, VISUAL_TO_SOURCE_DEBOUNCE_MS, clamp_split_ratio,
    markdown_highlighter_factory, section_for_line, workbook_sheet_spans,
};

#[ibis_office_macros::editor_ext(XlsxMarkdown)]
impl SlideEditor {
    /// Current Markdown view mode for this workbook (`Visual`, `Split`, or `Source`).
    pub(crate) fn xlsx_markdown_view_mode(&self) -> MarkdownViewMode {
        self.sheet_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map_or(MarkdownViewMode::Visual, |ms| ms.mode)
    }

    /// True when the workbook is currently displaying the Markdown Split or Source editor.
    pub(crate) fn xlsx_is_markdown_source_mode(&self) -> bool {
        self.xlsx_markdown_view_mode() != MarkdownViewMode::Visual
    }

    /// True when the workbook is currently in side-by-side Split mode.
    pub(crate) fn xlsx_is_markdown_split_mode(&self) -> bool {
        self.xlsx_markdown_view_mode() == MarkdownViewMode::Split
    }

    /// True when the `[Source | Split | Visual]` mode bar should be displayed.
    pub(crate) fn xlsx_shows_markdown_bar(&self) -> bool {
        !self.grid_only && (self.is_markdown_path() || self.xlsx_is_markdown_source_mode())
    }

    /// True when the Markdown source editor currently holds keyboard focus.
    pub(crate) fn xlsx_is_markdown_source_focused(&self, window: &Window, cx: &App) -> bool {
        self.sheet_state()
            .and_then(|s| s.markdown_source.as_ref())
            .is_some_and(|ms| ms.editor.read(cx).focus_handle(cx).is_focused(window))
    }

    /// Exact UTF-8 Markdown source text to save when the user saves a `.md` workbook while
    /// Source/Split view is active and has no unexported visual edits.
    pub(crate) fn xlsx_markdown_source_text(&self) -> Option<&str> {
        let ms = self.sheet_state().and_then(|s| s.markdown_source.as_ref())?;
        if ms.visual_dirty {
            None
        } else {
            Some(&ms.current_text)
        }
    }

    /// Toggle between Visual and Split (or back to Visual from Source/Split) via `⌘⇧M`.
    pub(crate) fn xlsx_toggle_markdown_source_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.xlsx_is_markdown_source_mode() {
            self.xlsx_set_markdown_view_mode(MarkdownViewMode::Visual, window, cx);
        } else {
            self.xlsx_set_markdown_view_mode(MarkdownViewMode::Split, window, cx);
        }
    }

    /// Switch to the requested [`MarkdownViewMode`] (`Visual`, `Split`, or `Source`).
    pub(crate) fn xlsx_set_markdown_view_mode(
        &mut self,
        target: MarkdownViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            MarkdownViewMode::Visual => {
                self.xlsx_exit_markdown_source_mode(window, cx);
            }
            MarkdownViewMode::Split | MarkdownViewMode::Source => {
                if self.xlsx_is_markdown_source_mode() {
                    let current_mode = self.xlsx_markdown_view_mode();
                    if current_mode == target {
                        return;
                    }
                    if target == MarkdownViewMode::Split {
                        let _ = self.xlsx_sync_markdown_source_if_dirty(cx);
                    } else if target == MarkdownViewMode::Source {
                        self.xlsx_sync_visual_to_markdown_source(window, cx);
                    }
                    let editor_to_focus = self
                        .sheet_state_mut()
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
                    self.xlsx_enter_markdown_view_mode(target, window, cx);
                }
            }
        }
    }

    fn xlsx_visible_sheet_slides(&self) -> Vec<(SlideId, String)> {
        if let Some(book) = self.presentation().workbook.as_deref() {
            let mut out = Vec::new();
            for sheet in &book.sheets {
                if !sheet.other
                    && sheet.state == SheetState::Visible
                    && let Some(id) = sheet.slide
                {
                    out.push((id, sheet.name.clone()));
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
        self.presentation()
            .slides
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id, format!("Sheet{}", i + 1)))
            .collect()
    }

    fn xlsx_active_visible_sheet_index(&self) -> Option<usize> {
        self.xlsx_visible_sheet_slides()
            .iter()
            .position(|(id, _)| *id == self.current_slide)
    }

    fn xlsx_enter_markdown_view_mode(
        &mut self,
        mode: MarkdownViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode == MarkdownViewMode::Visual || self.xlsx_is_markdown_source_mode() {
            return;
        }
        if self.cell_edit().is_some() {
            self.commit_cell_edit(cx);
        }

        let initial = self.xlsx_initial_markdown_text();
        let sections = workbook_sheet_spans(&initial);
        let readonly = self.is_read_only();
        let revision = self.document.revision();
        let active_sheet_idx = self.xlsx_active_visible_sheet_index();

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

        let subscription = cx.subscribe_in(&editor, window, |this, ed, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                let suppressed = this
                    .sheet_state()
                    .and_then(|s| s.markdown_source.as_ref())
                    .is_some_and(|ms| ms.suppress_source_event);
                if suppressed {
                    return;
                }
                let text = ed.read(cx).value().to_string();
                let mut should_schedule = false;
                if let Some(ms) = this
                    .sheet_state_mut()
                    .and_then(|s| s.markdown_source.as_mut())
                {
                    ms.sections = workbook_sheet_spans(&text);
                    ms.current_text = text;
                    ms.dirty = true;
                    ms.visual_dirty = false;
                    ms.authority = SyncAuthority::Source;
                    ms.last_error = None;
                    ms.sync_generation = ms.sync_generation.wrapping_add(1);
                    should_schedule = ms.mode == MarkdownViewMode::Split;
                }
                this.schedule_inline_completion(ed.clone(), cx);
                this.schedule_text_content_auto_switch(ed.clone(), window, cx);
                if should_schedule {
                    this.xlsx_schedule_source_to_visual_sync(cx);
                }
                cx.notify();
            }
        });

        editor.update(cx, |s, cx| s.focus(window, cx));

        let current_slide = self.current_slide;
        if let Some(state) = self.sheet_state_mut() {
            state.markdown_source = Some(MarkdownSplitHost {
                mode,
                editor,
                current_text: initial,
                sections,
                dirty: false,
                visual_dirty: false,
                authority: SyncAuthority::Idle,
                last_synced_revision: revision,
                last_seen_revision: revision,
                sync_generation: 0,
                suppress_source_event: false,
                sync_scroll_enabled: true,
                last_synced_source_line: None,
                last_synced_slide_id: Some(current_slide),
                active_section_index: active_sheet_idx,
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

    fn xlsx_initial_markdown_text(&self) -> String {
        #[cfg(not(target_family = "wasm"))]
        if !self.document.is_dirty()
            && self.is_markdown_path()
            && let Some(path) = self.path.as_deref()
            && let Ok(raw) = std::fs::read_to_string(path)
        {
            return raw;
        }
        ibis_formats::markdown::export_workbook_markdown(self.presentation()).unwrap_or_default()
    }

    fn xlsx_schedule_source_to_visual_sync(&mut self, cx: &mut Context<Self>) {
        let Some(expected_gen) = self
            .sheet_state()
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
                    .sheet_state()
                    .and_then(|s| s.markdown_source.as_ref())
                    .is_some_and(|ms| {
                        ms.mode == MarkdownViewMode::Split
                            && ms.dirty
                            && ms.sync_generation == expected_gen
                    });
                if should_sync {
                    let _ = editor.xlsx_sync_markdown_source_if_dirty(cx);
                }
            });
        })
        .detach();
    }

    fn xlsx_schedule_visual_to_source_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(expected_gen) = self
            .sheet_state()
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
                    .sheet_state()
                    .and_then(|s| s.markdown_source.as_ref())
                    .is_some_and(|ms| {
                        ms.mode == MarkdownViewMode::Split
                            && ms.visual_dirty
                            && !ms.dirty
                            && ms.sync_generation == expected_gen
                    });
                if should_sync {
                    editor.xlsx_sync_visual_to_markdown_source(window, cx);
                }
            });
        })
        .detach();
    }

    /// Called from `Workbook::before_render`: flushes focus-transition syncs, detects
    /// worksheet edits via `Document::revision()`, and synchronizes active sheet tabs with
    /// the Markdown cursor line.
    pub(crate) fn poll_xlsx_markdown_bidirectional_sync(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.xlsx_is_markdown_split_mode() {
            return;
        }
        let source_focused = self.xlsx_is_markdown_source_focused(window, cx);
        let current_rev = self.document.revision();
        let (was_focused, source_dirty, visual_dirty, rev_changed, rev_newly_mutated) = {
            let Some(ms) = self.sheet_state().and_then(|s| s.markdown_source.as_ref()) else {
                return;
            };
            (
                ms.source_was_focused,
                ms.dirty,
                ms.visual_dirty,
                current_rev != ms.last_synced_revision,
                current_rev != ms.last_seen_revision,
            )
        };

        if let Some(ms) = self
            .sheet_state_mut()
            .and_then(|s| s.markdown_source.as_mut())
        {
            ms.source_was_focused = source_focused;
        }

        // Focus left Source -> Visual while Source has unsynced edits: flush immediately.
        if was_focused && !source_focused && source_dirty {
            let _ = self.xlsx_sync_markdown_source_if_dirty(cx);
            return;
        }

        // Focus entered Visual -> Source while Visual has newer edits: flush immediately.
        if !was_focused && source_focused && !source_dirty && (visual_dirty || rev_changed) {
            self.xlsx_sync_visual_to_markdown_source(window, cx);
            return;
        }

        // Visual side mutated while Source is clean: schedule debounced Visual -> Source export.
        if !source_dirty && rev_changed && rev_newly_mutated {
            let scheduled = if let Some(ms) = self
                .sheet_state_mut()
                .and_then(|s| s.markdown_source.as_mut())
            {
                ms.visual_dirty = true;
                ms.authority = SyncAuthority::Visual;
                ms.last_seen_revision = current_rev;
                ms.sync_generation = ms.sync_generation.wrapping_add(1);
                true
            } else {
                false
            };
            if scheduled {
                self.xlsx_schedule_visual_to_source_sync(window, cx);
            }
            return;
        }

        // Keep active worksheet tab and Markdown sheet section aligned across the Split divider.
        self.xlsx_sync_split_cursor_and_sheet(source_focused, cx);
    }

    fn xlsx_sync_split_cursor_and_sheet(
        &mut self,
        source_focused: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((sync_enabled, dirty, visual_dirty, editor_entity, last_line, last_slide)) = self
            .sheet_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|ms| {
                (
                    ms.sync_scroll_enabled,
                    ms.dirty,
                    ms.visual_dirty,
                    ms.editor.clone(),
                    ms.last_synced_source_line,
                    ms.last_synced_slide_id,
                )
            })
        else {
            return;
        };
        if !sync_enabled || dirty || visual_dirty {
            return;
        }

        let visible_sheets = self.xlsx_visible_sheet_slides();
        if source_focused {
            let line = editor_entity.read(cx).cursor_position().line as usize;
            if Some(line) == last_line {
                return;
            }
            let target_idx = self
                .sheet_state()
                .and_then(|s| s.markdown_source.as_ref())
                .and_then(|ms| section_for_line(&ms.sections, line))
                .map(|span| span.index);
            if let Some(ms) = self
                .sheet_state_mut()
                .and_then(|s| s.markdown_source.as_mut())
            {
                ms.last_synced_source_line = Some(line);
                ms.active_section_index = target_idx;
            }
            if let Some(idx) = target_idx
                && let Some((slide_id, _)) = visible_sheets.get(idx)
                && *slide_id != self.current_slide
            {
                if self.cell_edit().is_some() {
                    self.commit_cell_edit(cx);
                }
                self.set_current_slide(*slide_id);
                self.set_selection(Vec::new());
                if let Some(ms) = self
                    .sheet_state_mut()
                    .and_then(|s| s.markdown_source.as_mut())
                {
                    ms.last_synced_slide_id = Some(*slide_id);
                }
                cx.notify();
            }
        } else {
            let current_slide = self.current_slide;
            if Some(current_slide) == last_slide {
                return;
            }
            let Some(sheet_idx) = visible_sheets
                .iter()
                .position(|(id, _)| *id == current_slide)
            else {
                return;
            };
            let target = self
                .sheet_state()
                .and_then(|s| s.markdown_source.as_ref())
                .and_then(|ms| ms.sections.get(sheet_idx))
                .map(|span| (span.index, span.byte_range.start, span.line_range.start));
            if let Some(ms) = self
                .sheet_state_mut()
                .and_then(|s| s.markdown_source.as_mut())
            {
                ms.last_synced_slide_id = Some(current_slide);
                ms.active_section_index = target.map(|(idx, _, _)| idx).or(Some(sheet_idx));
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

    /// Export the current workbook back to Markdown tables and update the Source editor while
    /// preserving scroll offset and suppressing recursive change events.
    pub(crate) fn xlsx_sync_visual_to_markdown_source(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((editor_entity, prev_text)) = self
            .sheet_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|ms| (ms.editor.clone(), ms.current_text.clone()))
        else {
            return;
        };

        let exported = match ibis_formats::markdown::export_workbook_markdown(self.presentation()) {
            Ok(md) => crate::shell::markdown_split::preserve_workbook_frontmatter(&prev_text, md),
            Err(err) => {
                let msg: SharedString = err.to_string().into();
                if let Some(ms) = self
                    .sheet_state_mut()
                    .and_then(|s| s.markdown_source.as_mut())
                {
                    ms.last_error = Some(msg);
                    ms.visual_dirty = false;
                    ms.authority = SyncAuthority::Idle;
                }
                cx.notify();
                return;
            }
        };
        let rev = self.document.revision();
        let next_sections = workbook_sheet_spans(&exported);

        if exported == prev_text {
            if let Some(ms) = self
                .sheet_state_mut()
                .and_then(|s| s.markdown_source.as_mut())
            {
                ms.sections = next_sections;
                ms.last_synced_revision = rev;
                ms.last_seen_revision = rev;
                ms.visual_dirty = false;
                ms.authority = SyncAuthority::Idle;
            }
            return;
        }

        let prev_scroll = editor_entity.read(cx).scroll_offset();
        if let Some(ms) = self
            .sheet_state_mut()
            .and_then(|s| s.markdown_source.as_mut())
        {
            ms.suppress_source_event = true;
        }
        editor_entity.update(cx, |state, cx| {
            state.set_value(exported.clone(), window, cx);
            state.set_scroll_offset(prev_scroll, cx);
        });
        if let Some(ms) = self
            .sheet_state_mut()
            .and_then(|s| s.markdown_source.as_mut())
        {
            ms.current_text = exported;
            ms.sections = next_sections;
            ms.dirty = false;
            ms.visual_dirty = false;
            ms.authority = SyncAuthority::Idle;
            ms.last_error = None;
            ms.last_synced_revision = rev;
            ms.last_seen_revision = rev;
            ms.last_synced_slide_id = None;
            ms.suppress_source_event = false;
        }
        cx.notify();
    }

    /// Re-parse dirty Workbook Markdown source edits into the live workbook model without leaving
    /// Split/Source view. Returns `true` on success, `false` on parse error (preserving last valid workbook).
    pub(crate) fn xlsx_sync_markdown_source_if_dirty(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((text, dirty)) = self
            .sheet_state()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|ms| (ms.current_text.clone(), ms.dirty))
        else {
            return true;
        };
        if !dirty {
            return true;
        }

        let applied = self.xlsx_replace_markdown_text(&text, cx);
        if applied {
            let rev = self.document.revision();
            let sections = workbook_sheet_spans(&text);
            let current_slide = self.current_slide;
            if let Some(ms) = self
                .sheet_state_mut()
                .and_then(|s| s.markdown_source.as_mut())
            {
                ms.sections = sections;
                ms.dirty = false;
                ms.visual_dirty = false;
                ms.authority = SyncAuthority::Idle;
                ms.last_synced_revision = rev;
                ms.last_seen_revision = rev;
                ms.last_error = None;
                ms.last_synced_source_line = None;
                ms.last_synced_slide_id = Some(current_slide);
            }
        }
        applied
    }

    pub(crate) fn xlsx_replace_markdown_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if self.cell_edit().is_some() {
            self.commit_cell_edit(cx);
        }

        let opened = match ibis_formats::markdown::open_markdown_workbook(text.as_bytes().to_vec()) {
            Ok(opened) => opened,
            Err(err) => {
                let msg: SharedString = err.to_string().into();
                if let Some(ms) = self
                    .sheet_state_mut()
                    .and_then(|s| s.markdown_source.as_mut())
                {
                    ms.last_error = Some(msg.clone());
                }
                self.set_status(msg.to_string(), cx);
                cx.notify();
                return false;
            }
        };

        let prev_sheet_idx = self.xlsx_active_visible_sheet_index().unwrap_or(0);

        if let Err(err) = self
            .document
            .execute(vec![Command::ReplacePresentation(Box::new(opened.presentation))])
        {
            let msg: SharedString = format!("{err:?}").into();
            if let Some(ms) = self
                .sheet_state_mut()
                .and_then(|s| s.markdown_source.as_mut())
            {
                ms.last_error = Some(msg);
            }
            cx.notify();
            return false;
        }

        let visible = self.xlsx_visible_sheet_slides();
        if !visible.is_empty() {
            let next_idx = prev_sheet_idx.min(visible.len() - 1);
            let next_id = visible[next_idx].0;
            self.set_current_slide(next_id);
        } else if let Some(first) = self.presentation().slides.first().map(|s| s.id) {
            self.set_current_slide(first);
        }

        let valid_ids: std::collections::HashSet<SlideId> =
            self.presentation().slides.iter().map(|s| s.id).collect();
        if let Some(state) = self.sheet_state_mut() {
            state.cells.retain(|id, _| valid_ids.contains(id));
            state.cell_ends.retain(|id, _| valid_ids.contains(id));
            state.sheet_scrolls.retain(|id, _| valid_ids.contains(id));
            state.group.retain(|id| valid_ids.contains(id));
        }

        self.set_selection(Vec::new());
        self.register_missing_media_images();
        cx.notify();
        true
    }

    /// Exit Markdown Split/Source view and return to the rendered Visual workbook grid.
    pub(crate) fn xlsx_exit_markdown_source_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.xlsx_is_markdown_source_mode() {
            return true;
        }
        if !self.xlsx_sync_markdown_source_if_dirty(cx) {
            return false;
        }
        if let Some(state) = self.sheet_state_mut() {
            state.markdown_source = None;
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
        window.refresh();
        true
    }

    fn xlsx_split_callbacks() -> SplitViewCallbacks {
        SplitViewCallbacks {
            on_select_mode: Rc::new(|this, target, window, cx| {
                this.xlsx_set_markdown_view_mode(target, window, cx);
            }),
            on_toggle_sync_scroll: Rc::new(|this, cx| {
                if let Some(ms) = this
                    .sheet_state_mut()
                    .and_then(|s| s.markdown_source.as_mut())
                {
                    ms.sync_scroll_enabled = !ms.sync_scroll_enabled;
                    ms.last_synced_source_line = None;
                    ms.last_synced_slide_id = None;
                }
                cx.notify();
            }),
            on_reset_split_ratio: Rc::new(|this, cx| {
                if let Some(ms) = this
                    .sheet_state_mut()
                    .and_then(|s| s.markdown_source.as_mut())
                {
                    ms.split_ratio = DEFAULT_SPLIT_RATIO;
                    ms.split_drag = None;
                }
                cx.notify();
            }),
            on_start_split_drag: Rc::new(|this, x, cx| {
                if let Some(ms) = this
                    .sheet_state_mut()
                    .and_then(|s| s.markdown_source.as_mut())
                {
                    ms.split_drag = Some((x, ms.split_ratio));
                }
                cx.notify();
            }),
            on_move_split_drag: Rc::new(|this, x, cx| {
                if let Some(ms) = this
                    .sheet_state_mut()
                    .and_then(|s| s.markdown_source.as_mut())
                    && let Some((x0, r0)) = ms.split_drag
                {
                    let total = ms.split_container_width.get().max(300.0);
                    let dx = x - x0;
                    ms.split_ratio = clamp_split_ratio(r0 + dx / total);
                    cx.notify();
                }
            }),
            on_end_split_drag: Rc::new(|this, cx| {
                if let Some(ms) = this
                    .sheet_state_mut()
                    .and_then(|s| s.markdown_source.as_mut())
                    && ms.split_drag.take().is_some()
                {
                    cx.notify();
                }
            }),
        }
    }

    /// Wrap the workbook grid body with the `[Source | Split | Visual]` mode bar and
    /// Workbook Markdown Split/Source panes when applicable.
    pub(crate) fn render_xlsx_markdown_body(
        &self,
        visual_body: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.xlsx_shows_markdown_bar() {
            return visual_body;
        }
        let mode = self.xlsx_markdown_view_mode();
        let host = self.sheet_state().and_then(|s| s.markdown_source.as_ref());
        let visible_sheets = self.xlsx_visible_sheet_slides();
        let section_badge = host
            .filter(|ms| ms.mode == MarkdownViewMode::Split)
            .and_then(|ms| {
                let idx = ms
                    .active_section_index
                    .or_else(|| self.xlsx_active_visible_sheet_index())?;
                let span = ms.sections.get(idx)?;
                let start_l = span.line_range.start + 1;
                let end_l = span.line_range.end.max(start_l);
                let line_label = if start_l == end_l {
                    format!("L{start_l}")
                } else {
                    format!("L{start_l}–{end_l}")
                };
                let sheet_name = visible_sheets
                    .get(idx)
                    .map(|(_, name)| name.as_str())
                    .or(span.title.as_deref())
                    .unwrap_or("Sheet");
                Some(format!("{line_label} ↔ {sheet_name}"))
            });

        let callbacks = Self::xlsx_split_callbacks();
        let mode_bar = self.render_shared_markdown_mode_bar(
            "xlsx-markdown",
            "Workbook",
            mode,
            host,
            section_badge,
            &callbacks,
            cx,
        );

        let body = match (mode, host) {
            (MarkdownViewMode::Source, Some(ms)) => {
                self.render_shared_markdown_source_pane("xlsx-markdown", ms, self.is_read_only(), cx)
            }
            (MarkdownViewMode::Split, Some(ms)) if !self.compact_viewport => {
                let source_pane = self.render_shared_markdown_source_pane(
                    "xlsx-markdown",
                    ms,
                    self.is_read_only(),
                    cx,
                );
                self.render_shared_markdown_split_container(
                    "xlsx-markdown",
                    ms,
                    source_pane,
                    visual_body,
                    &callbacks,
                    cx,
                )
            }
            _ => visual_body,
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(mode_bar)
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibis_doc_model::{CellEdit, CellInput, CellValue, Document, SheetCommand};

    #[::core::prelude::v1::test]
    fn workbook_markdown_bidirectional_roundtrip_with_formulas_and_cell_edits() {
        let initial_md = r#"<!-- xlsx-sheet "Sales" -->

## Sales

| Product | Price | Qty | Total |
| --- | ---: | ---: | ---: |
| Apple | 100 | 3 | 300 <!-- xlsx-formula:"=B2*C2" --> |

<!-- xlsx-sheet "Summary" -->

## Summary

| Metric | Value |
| --- | ---: |
| Grand Total | 300 <!-- xlsx-formula:"=Sales!D2" --> |
"#;
        let opened = ibis_formats::markdown::open_markdown_workbook(initial_md.as_bytes().to_vec())
            .expect("open initial workbook markdown");
        assert_eq!(opened.presentation.slides.len(), 2);
        let spans = workbook_sheet_spans(initial_md);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].title.as_deref(), Some("Sales"));
        assert_eq!(spans[1].title.as_deref(), Some("Summary"));

        let mut doc = Document::new(opened.presentation);
        let rev0 = doc.revision();

        // Source -> Visual: add a second product row via Markdown reparse + ReplacePresentation
        let edited_md = initial_md.replace(
            "| Apple | 100 | 3 | 300 <!-- xlsx-formula:\"=B2*C2\" --> |",
            "| Apple | 100 | 3 | 300 <!-- xlsx-formula:\"=B2*C2\" --> |\n| Orange | 80 | 5 | 400 <!-- xlsx-formula:\"=B3*C3\" --> |",
        );
        let reparsed = ibis_formats::markdown::open_markdown_workbook(edited_md.as_bytes().to_vec())
            .expect("reparse edited workbook markdown");
        doc.execute(vec![Command::ReplacePresentation(Box::new(
            reparsed.presentation,
        ))])
        .expect("replace workbook presentation");
        let rev1 = doc.revision();
        assert!(rev1 > rev0);

        // Visual -> Source: edit cell B2 (Apple price 100 -> 150) on first sheet
        let sales_slide = doc.presentation().slides[0].id;
        doc.execute(vec![Command::Sheet(SheetCommand::SetCells {
            sheet: sales_slide,
            edits: vec![CellEdit {
                row: 1,
                column: 1,
                input: CellInput::Value(CellValue::Number(150.0)),
            }],
        })])
        .expect("edit cell B2");
        assert!(doc.revision() > rev1);

        let exported = ibis_formats::markdown::export_workbook_markdown(doc.presentation())
            .expect("export workbook markdown");
        assert!(exported.contains("| Apple | 150 |"), "exported:\n{exported}");
        assert!(exported.contains("| Orange | 80 |"), "exported:\n{exported}");
        assert_eq!(workbook_sheet_spans(&exported).len(), 2);

        // Undo reverts cell edit, second undo reverts source replacement
        assert!(doc.undo());
        let after_undo1 = ibis_formats::markdown::export_workbook_markdown(doc.presentation())
            .expect("export after undo 1");
        assert!(after_undo1.contains("| Apple | 100 |"));
        assert!(after_undo1.contains("| Orange | 80 |"));

        assert!(doc.undo());
        let after_undo2 = ibis_formats::markdown::export_workbook_markdown(doc.presentation())
            .expect("export after undo 2");
        assert!(!after_undo2.contains("Orange"));
    }
}
