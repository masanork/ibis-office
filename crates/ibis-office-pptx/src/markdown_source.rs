// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Bidirectional Marp Presentation Markdown Split View (`Source | Split | Visual`).
//!
//! Left pane: pure-Rust syntax-highlighted Marp Markdown editor (`---` slides, YAML frontmatter,
//! Mermaid diagrams, GFM tables).
//! Right pane: live editable native PowerPoint slide canvas.
//!
//! - Source edits sync to live slides via 250ms debounce (or immediately on focus-leave) using
//!   [`Command::ReplacePresentation`], preserving active slide index and registering newly rendered
//!   media (including Mermaid SVGs).
//! - Visual slide edits sync back to Markdown via 200ms debounce (or immediately on focus-enter)
//!   by watching [`ibis_doc_model::Document::revision`].
//! - Slide-level cursor/scroll sync keeps the active slide and Markdown slide block aligned.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::input::{EditorState, InputEvent};
use gpui_kit::*;
use ibis_doc_model::Command;

use crate::ext::*;
use crate::shell::editor::SlideEditor;
use crate::shell::markdown_split::{
    DEFAULT_SPLIT_RATIO, MarkdownSplitHost, MarkdownViewMode, SOURCE_TO_VISUAL_DEBOUNCE_MS,
    SplitViewCallbacks, SyncAuthority, VISUAL_TO_SOURCE_DEBOUNCE_MS, clamp_split_ratio,
    markdown_highlighter_factory, marp_slide_spans, section_for_line,
};

#[ibis_office_macros::editor_ext(PptxMarkdown)]
impl SlideEditor {
    /// Current Markdown view mode for this presentation (`Visual`, `Split`, or `Source`).
    pub(crate) fn pptx_markdown_view_mode(&self) -> MarkdownViewMode {
        self.pptx()
            .and_then(|s| s.markdown_source.as_ref())
            .map_or(MarkdownViewMode::Visual, |ms| ms.mode)
    }

    /// True when the presentation is currently displaying the Markdown Split or Source editor.
    pub(crate) fn pptx_is_markdown_source_mode(&self) -> bool {
        self.pptx_markdown_view_mode() != MarkdownViewMode::Visual
    }

    /// True when the presentation is currently in side-by-side Split mode.
    pub(crate) fn pptx_is_markdown_split_mode(&self) -> bool {
        self.pptx_markdown_view_mode() == MarkdownViewMode::Split
    }

    /// True when the `[Source | Split | Visual]` mode bar should be displayed.
    pub(crate) fn pptx_shows_markdown_bar(&self) -> bool {
        self.is_markdown_path() || self.pptx_is_markdown_source_mode()
    }

    /// True when the Markdown source editor currently holds keyboard focus.
    pub(crate) fn pptx_is_markdown_source_focused(&self, window: &Window, cx: &App) -> bool {
        self.pptx()
            .and_then(|s| s.markdown_source.as_ref())
            .is_some_and(|ms| ms.editor.read(cx).focus_handle(cx).is_focused(window))
    }

    /// Exact UTF-8 Markdown source text to save when the user saves a `.md` presentation while
    /// Source/Split view is active and has no unexported visual edits.
    pub(crate) fn pptx_markdown_source_text(&self) -> Option<&str> {
        let ms = self.pptx().and_then(|s| s.markdown_source.as_ref())?;
        if ms.visual_dirty {
            None
        } else {
            Some(&ms.current_text)
        }
    }

    /// Toggle between Visual and Split (or back to Visual from Source/Split) via `⌘⇧M`.
    pub(crate) fn pptx_toggle_markdown_source_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pptx_is_markdown_source_mode() {
            self.pptx_set_markdown_view_mode(MarkdownViewMode::Visual, window, cx);
        } else {
            self.pptx_set_markdown_view_mode(MarkdownViewMode::Split, window, cx);
        }
    }

    /// Switch to the requested [`MarkdownViewMode`] (`Visual`, `Split`, or `Source`).
    pub(crate) fn pptx_set_markdown_view_mode(
        &mut self,
        target: MarkdownViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            MarkdownViewMode::Visual => {
                self.pptx_exit_markdown_source_mode(window, cx);
            }
            MarkdownViewMode::Split | MarkdownViewMode::Source => {
                if self.pptx_is_markdown_source_mode() {
                    let current_mode = self.pptx_markdown_view_mode();
                    if current_mode == target {
                        return;
                    }
                    if target == MarkdownViewMode::Split {
                        let _ = self.pptx_sync_markdown_source_if_dirty(cx);
                    } else if target == MarkdownViewMode::Source {
                        self.pptx_sync_visual_to_markdown_source(window, cx);
                    }
                    let editor_to_focus = self
                        .pptx_mut()
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
                    self.pptx_enter_markdown_view_mode(target, window, cx);
                }
            }
        }
    }

    fn pptx_enter_markdown_view_mode(
        &mut self,
        mode: MarkdownViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode == MarkdownViewMode::Visual || self.pptx_is_markdown_source_mode() {
            return;
        }
        // Finish any active inline text edit before capturing Markdown source.
        self.commit_edit(window, cx);

        let initial = self.pptx_initial_markdown_text();
        let sections = marp_slide_spans(&initial);
        let readonly = self.is_read_only();
        let revision = self.document.revision();
        let active_slide_idx = self.presentation().slide_index(self.current_slide);

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
                    .pptx()
                    .and_then(|s| s.markdown_source.as_ref())
                    .is_some_and(|ms| ms.suppress_source_event);
                if suppressed {
                    return;
                }
                let text = ed.read(cx).value().to_string();
                let mut should_schedule = false;
                if let Some(ms) = this.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
                    ms.sections = marp_slide_spans(&text);
                    ms.current_text = text;
                    ms.dirty = true;
                    ms.visual_dirty = false;
                    ms.authority = SyncAuthority::Source;
                    ms.last_error = None;
                    ms.sync_generation = ms.sync_generation.wrapping_add(1);
                    should_schedule = ms.mode == MarkdownViewMode::Split;
                }
                if should_schedule {
                    this.pptx_schedule_source_to_visual_sync(cx);
                }
                cx.notify();
            }
        });

        editor.update(cx, |s, cx| s.focus(window, cx));

        let current_slide = self.current_slide;
        if let Some(state) = self.pptx_mut() {
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
                active_section_index: active_slide_idx,
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

    fn pptx_initial_markdown_text(&self) -> String {
        // When opened from an unmodified `.md` file on disk, prefer the exact file bytes
        // so hand-authored Marp frontmatter, themes, and comments are preserved verbatim.
        #[cfg(not(target_family = "wasm"))]
        if !self.document.is_dirty()
            && self.is_markdown_path()
            && let Some(path) = self.path.as_deref()
            && let Ok(raw) = std::fs::read_to_string(path)
        {
            return raw;
        }
        ibis_formats::markdown::export_presentation_markdown(self.presentation())
    }

    fn pptx_schedule_source_to_visual_sync(&mut self, cx: &mut Context<Self>) {
        let Some(expected_gen) = self
            .pptx()
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
                    .pptx()
                    .and_then(|s| s.markdown_source.as_ref())
                    .is_some_and(|ms| {
                        ms.mode == MarkdownViewMode::Split
                            && ms.dirty
                            && ms.sync_generation == expected_gen
                    });
                if should_sync {
                    let _ = editor.pptx_sync_markdown_source_if_dirty(cx);
                }
            });
        })
        .detach();
    }

    fn pptx_schedule_visual_to_source_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(expected_gen) = self
            .pptx()
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
                    .pptx()
                    .and_then(|s| s.markdown_source.as_ref())
                    .is_some_and(|ms| {
                        ms.mode == MarkdownViewMode::Split
                            && ms.visual_dirty
                            && !ms.dirty
                            && ms.sync_generation == expected_gen
                    });
                if should_sync {
                    editor.pptx_sync_visual_to_markdown_source(window, cx);
                }
            });
        })
        .detach();
    }

    /// Called from `Presentation::before_render`: flushes focus-transition syncs, detects
    /// visual slide mutations via `Document::revision()`, and synchronizes slide selection
    /// with the Markdown cursor line.
    pub(crate) fn poll_pptx_markdown_bidirectional_sync(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pptx_is_markdown_split_mode() {
            return;
        }
        let source_focused = self.pptx_is_markdown_source_focused(window, cx);
        let current_rev = self.document.revision();
        let (was_focused, source_dirty, visual_dirty, rev_changed, rev_newly_mutated) = {
            let Some(ms) = self.pptx().and_then(|s| s.markdown_source.as_ref()) else {
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

        if let Some(ms) = self.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
            ms.source_was_focused = source_focused;
        }

        // Focus left Source -> Visual while Source has unsynced edits: flush immediately.
        if was_focused && !source_focused && source_dirty {
            let _ = self.pptx_sync_markdown_source_if_dirty(cx);
            return;
        }

        // Focus entered Visual -> Source while Visual has newer edits: flush immediately.
        if !was_focused && source_focused && !source_dirty && (visual_dirty || rev_changed) {
            self.pptx_sync_visual_to_markdown_source(window, cx);
            return;
        }

        // Visual side mutated while Source is clean: schedule debounced Visual -> Source export.
        if !source_dirty && rev_changed && rev_newly_mutated {
            let scheduled = if let Some(ms) =
                self.pptx_mut().and_then(|s| s.markdown_source.as_mut())
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
                self.pptx_schedule_visual_to_source_sync(window, cx);
            }
            return;
        }

        // Keep active slide and Markdown slide cursor aligned across the Split divider.
        self.pptx_sync_split_cursor_and_slide(source_focused, cx);
    }

    fn pptx_sync_split_cursor_and_slide(
        &mut self,
        source_focused: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((sync_enabled, dirty, visual_dirty, editor_entity, last_line, last_slide)) = self
            .pptx()
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

        if source_focused {
            let line = editor_entity.read(cx).cursor_position().line as usize;
            if Some(line) == last_line {
                return;
            }
            let target_idx = self
                .pptx()
                .and_then(|s| s.markdown_source.as_ref())
                .and_then(|ms| section_for_line(&ms.sections, line))
                .map(|span| span.index);
            if let Some(ms) = self.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
                ms.last_synced_source_line = Some(line);
                ms.active_section_index = target_idx;
            }
            if let Some(idx) = target_idx
                && let Some(slide_id) = self.presentation().slides.get(idx).map(|s| s.id)
                && slide_id != self.current_slide
            {
                self.set_current_slide(slide_id);
                self.set_selection(Vec::new());
                self.list_scroll.scroll_to_item(idx);
                if let Some(ms) = self.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
                    ms.last_synced_slide_id = Some(slide_id);
                }
                cx.notify();
            }
        } else {
            let current_slide = self.current_slide;
            if Some(current_slide) == last_slide {
                return;
            }
            let Some(slide_idx) = self.presentation().slide_index(current_slide) else {
                return;
            };
            let target = self
                .pptx()
                .and_then(|s| s.markdown_source.as_ref())
                .and_then(|ms| ms.sections.get(slide_idx))
                .map(|span| (span.index, span.byte_range.start, span.line_range.start));
            if let Some(ms) = self.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
                ms.last_synced_slide_id = Some(current_slide);
                ms.active_section_index = target.map(|(idx, _, _)| idx).or(Some(slide_idx));
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

    /// Export the current slides back to Marp Markdown and update the Source editor while
    /// preserving scroll offset and suppressing recursive change events.
    pub(crate) fn pptx_sync_visual_to_markdown_source(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((editor_entity, prev_text)) = self
            .pptx()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|ms| (ms.editor.clone(), ms.current_text.clone()))
        else {
            return;
        };

        let exported = ibis_formats::markdown::export_presentation_markdown(self.presentation());
        let rev = self.document.revision();
        let next_sections = marp_slide_spans(&exported);

        if exported == prev_text {
            if let Some(ms) = self.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
                ms.sections = next_sections;
                ms.last_synced_revision = rev;
                ms.last_seen_revision = rev;
                ms.visual_dirty = false;
                ms.authority = SyncAuthority::Idle;
            }
            return;
        }

        let prev_scroll = editor_entity.read(cx).scroll_offset();
        if let Some(ms) = self.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
            ms.suppress_source_event = true;
        }
        editor_entity.update(cx, |state, cx| {
            state.set_value(exported.clone(), window, cx);
            state.set_scroll_offset(prev_scroll, cx);
        });
        if let Some(ms) = self.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
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

    /// Re-parse dirty Marp Markdown source edits into the live presentation model without leaving
    /// Split/Source view. Returns `true` on success, `false` on parse error (preserving last valid slides).
    pub(crate) fn pptx_sync_markdown_source_if_dirty(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((text, dirty)) = self
            .pptx()
            .and_then(|s| s.markdown_source.as_ref())
            .map(|ms| (ms.current_text.clone(), ms.dirty))
        else {
            return true;
        };
        if !dirty {
            return true;
        }

        let applied = self.pptx_replace_markdown_text(&text, cx);
        if applied {
            let rev = self.document.revision();
            let sections = marp_slide_spans(&text);
            let current_slide = self.current_slide;
            if let Some(ms) = self.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
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

    pub(crate) fn pptx_replace_markdown_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        #[cfg(not(target_family = "wasm"))]
        let parsed = {
            let base_dir = self.path.as_deref().and_then(|p| p.parent());
            ibis_formats::markdown::open_markdown_presentation_with_base_dir(
                text.as_bytes().to_vec(),
                base_dir,
            )
        };
        #[cfg(target_family = "wasm")]
        let parsed = ibis_formats::markdown::open_markdown_presentation(text.as_bytes().to_vec());

        let opened = match parsed {
            Ok(opened) => opened,
            Err(err) => {
                let msg: SharedString = err.to_string().into();
                if let Some(ms) = self.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
                    ms.last_error = Some(msg.clone());
                }
                self.set_status(msg.to_string(), cx);
                cx.notify();
                return false;
            }
        };

        let prev_slide_idx = self
            .presentation()
            .slide_index(self.current_slide)
            .unwrap_or(0);

        if let Err(err) = self
            .document
            .execute(vec![Command::ReplacePresentation(Box::new(opened.presentation))])
        {
            let msg: SharedString = format!("{err:?}").into();
            if let Some(ms) = self.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
                ms.last_error = Some(msg);
            }
            cx.notify();
            return false;
        }

        let slides_len = self.presentation().slides.len();
        if slides_len > 0 {
            let next_idx = prev_slide_idx.min(slides_len - 1);
            let next_id = self.presentation().slides[next_idx].id;
            self.set_current_slide(next_id);
        }
        self.set_selection(Vec::new());
        self.register_missing_media_images();
        cx.notify();
        true
    }

    /// Exit Markdown Split/Source view and return to the rendered Visual presentation view.
    pub(crate) fn pptx_exit_markdown_source_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.pptx_is_markdown_source_mode() {
            return true;
        }
        if !self.pptx_sync_markdown_source_if_dirty(cx) {
            return false;
        }
        if let Some(state) = self.pptx_mut() {
            state.markdown_source = None;
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
        window.refresh();
        true
    }

    fn pptx_split_callbacks() -> SplitViewCallbacks {
        SplitViewCallbacks {
            on_select_mode: Rc::new(|this, target, window, cx| {
                this.pptx_set_markdown_view_mode(target, window, cx);
            }),
            on_toggle_sync_scroll: Rc::new(|this, cx| {
                if let Some(ms) = this.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
                    ms.sync_scroll_enabled = !ms.sync_scroll_enabled;
                    ms.last_synced_source_line = None;
                    ms.last_synced_slide_id = None;
                }
                cx.notify();
            }),
            on_reset_split_ratio: Rc::new(|this, cx| {
                if let Some(ms) = this.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
                    ms.split_ratio = DEFAULT_SPLIT_RATIO;
                    ms.split_drag = None;
                }
                cx.notify();
            }),
            on_start_split_drag: Rc::new(|this, x, cx| {
                if let Some(ms) = this.pptx_mut().and_then(|s| s.markdown_source.as_mut()) {
                    ms.split_drag = Some((x, ms.split_ratio));
                }
                cx.notify();
            }),
            on_move_split_drag: Rc::new(|this, x, cx| {
                if let Some(ms) = this.pptx_mut().and_then(|s| s.markdown_source.as_mut())
                    && let Some((x0, r0)) = ms.split_drag
                {
                    let total = ms.split_container_width.get().max(300.0);
                    let dx = x - x0;
                    ms.split_ratio = clamp_split_ratio(r0 + dx / total);
                    cx.notify();
                }
            }),
            on_end_split_drag: Rc::new(|this, cx| {
                if let Some(ms) = this.pptx_mut().and_then(|s| s.markdown_source.as_mut())
                    && ms.split_drag.take().is_some()
                {
                    cx.notify();
                }
            }),
        }
    }

    /// Wrap the normal presentation body with the `[Source | Split | Visual]` mode bar and
    /// Marp Markdown Split/Source panes when applicable.
    pub(crate) fn render_pptx_markdown_body(
        &self,
        visual_body: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.pptx_shows_markdown_bar() {
            return visual_body;
        }
        let mode = self.pptx_markdown_view_mode();
        let host = self.pptx().and_then(|s| s.markdown_source.as_ref());
        let total_slides = self.presentation().slides.len();
        let section_badge = host
            .filter(|ms| ms.mode == MarkdownViewMode::Split)
            .and_then(|ms| {
                let idx = ms
                    .active_section_index
                    .or_else(|| self.presentation().slide_index(self.current_slide))?;
                let span = ms.sections.get(idx)?;
                let start_l = span.line_range.start + 1;
                let end_l = span.line_range.end.max(start_l);
                let line_label = if start_l == end_l {
                    format!("L{start_l}")
                } else {
                    format!("L{start_l}–{end_l}")
                };
                Some(format!("{line_label} ↔ スライド {}/{total_slides}", idx + 1))
            });

        let callbacks = Self::pptx_split_callbacks();
        let mode_bar = self.render_shared_markdown_mode_bar(
            "pptx-markdown",
            "Marp",
            mode,
            host,
            section_badge,
            &callbacks,
            cx,
        );

        let body = match (mode, host) {
            (MarkdownViewMode::Source, Some(ms)) => {
                self.render_shared_markdown_source_pane("pptx-markdown", ms, self.is_read_only(), cx)
            }
            (MarkdownViewMode::Split, Some(ms)) => {
                let source_pane = self.render_shared_markdown_source_pane(
                    "pptx-markdown",
                    ms,
                    self.is_read_only(),
                    cx,
                );
                self.render_shared_markdown_split_container(
                    "pptx-markdown",
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
    use ibis_doc_model::{Document, ShapeKind};

    #[::core::prelude::v1::test]
    fn marp_bidirectional_roundtrip_with_replace_presentation_and_visual_edit() {
        let initial_md = "\
---
marp: true
---

# Project Kickoff

Welcome to Sorane

---

## Architecture

- Rust + GPUI
- Bidirectional Markdown
";
        let opened = ibis_formats::markdown::open_markdown_presentation(
            initial_md.as_bytes().to_vec(),
        )
        .expect("open initial marp");
        assert_eq!(opened.presentation.slides.len(), 2);
        let spans = marp_slide_spans(initial_md);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].title.as_deref(), Some("Project Kickoff"));
        assert_eq!(spans[1].title.as_deref(), Some("Architecture"));

        // Source -> Visual via Command::ReplacePresentation
        let mut doc = Document::new(opened.presentation);
        let rev0 = doc.revision();

        let edited_md = format!("{initial_md}\n---\n\n## Next Steps\n\n- Ship Phase 6\n");
        let reparsed = ibis_formats::markdown::open_markdown_presentation(
            edited_md.as_bytes().to_vec(),
        )
        .expect("reparse edited marp");
        assert_eq!(reparsed.presentation.slides.len(), 3);

        doc.execute(vec![Command::ReplacePresentation(Box::new(
            reparsed.presentation,
        ))])
        .expect("replace presentation");
        assert_eq!(doc.presentation().slides.len(), 3);
        let rev1 = doc.revision();
        assert!(rev1 > rev0);

        // Visual -> Source: edit first text shape on slide 3 via Command::SetText
        let slide3 = &doc.presentation().slides[2];
        let slide3_id = slide3.id;
        let shape_id = slide3
            .shapes
            .iter()
            .find(|sh| matches!(sh.kind, ShapeKind::TextBox(_) | ShapeKind::Auto(_)))
            .expect("slide 3 has text shape")
            .id;
        doc.execute(vec![Command::SetText {
            slide: slide3_id,
            shape: shape_id,
            text: "Updated Visual Title".to_string(),
        }])
        .expect("visual edit");
        assert!(doc.revision() > rev1);

        let exported = ibis_formats::markdown::export_presentation_markdown(doc.presentation());
        assert!(
            exported.contains("Updated Visual Title"),
            "exported Marp Markdown should contain visual edit: {exported}"
        );
        assert_eq!(marp_slide_spans(&exported).len(), 3);

        // Undo restores pre-visual-edit state, second undo restores 2-slide presentation
        assert!(doc.undo());
        assert!(doc.undo());
        assert_eq!(doc.presentation().slides.len(), 2);
    }
}
