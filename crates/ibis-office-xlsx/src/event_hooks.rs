// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! What the workbook does with the window's events (`FormatEditor`'s event hooks): the code was in `shell/editor.rs`.

use gpui_kit::*;

use ibis_doc_model::ShapeKind;

use crate::shell::editor::SlideEditor;
use crate::ext::*;

#[ibis_office_macros::editor_ext(EventHooks)]
impl SlideEditor {
    /// The pointer moved: the workbook's drags (the Draw panel's pen stroke first).
    pub(crate) fn xlsx_pointer_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.ink_move(event.position, event.pressed_button == Some(MouseButton::Left), cx) {
            return true;
        }
        if self.cell_edit().is_some_and(|e| e.dragging.is_some()) {
            if event.pressed_button == Some(MouseButton::Left) {
                self.drag_in_cell_text(event.position, window, cx);
            } else if let Some(edit) = self.sheet_state_mut().and_then(|s| s.cell_edit.as_mut()) {
                edit.dragging = None;
            }
            return true;
        }
        if self.sheet_state().is_some_and(|s| s.split_drag.is_some()) {
            if event.pressed_button == Some(MouseButton::Left) {
                self.on_split_move(event.position, cx);
            } else {
                self.on_split_up(event.position, cx);
            }
            return true;
        }
        if self.sheet_state().is_some_and(|s| s.timeline_drag.is_some()) {
            if event.pressed_button == Some(MouseButton::Left) {
                self.on_timeline_drag_move(event.position, cx);
            } else {
                self.on_timeline_drag_up(cx);
            }
            return true;
        }
        if self.sheet_state().is_some_and(|s| s.control_drag.is_some()) {
            if event.pressed_button == Some(MouseButton::Left) {
                self.on_control_drag_move(event.position, cx);
            } else {
                self.on_control_drag_up(cx);
            }
            return true;
        }
        if let Some(state) = self.sheet_state_mut()
            && state.fill_copy != event.modifiers.alt
        {
            state.fill_copy = event.modifiers.alt;
            cx.notify();
        }
        if self.sheet_state().is_some_and(|s| s.cell_move.is_some()) {
            if event.pressed_button == Some(MouseButton::Left) {
                let past = self.drag_pointer_moved(event.position, event.modifiers.alt, cx);
                let at = if past { self.drag_clamped(event.position) } else { event.position };
                self.update_cell_move(at, event.modifiers.alt, cx);
            } else {
                self.finish_cell_move(cx);
            }
            return true;
        }
        if self.sheet_state().is_some_and(|s| s.cell_drag) {
            if event.pressed_button == Some(MouseButton::Left) {
                // Past the edge of the cells the view scrolls on a timer (#186); the drag goes to the cell at the edge.
                let past = self.drag_pointer_moved(event.position, event.modifiers.alt, cx);
                let at = if past { self.drag_clamped(event.position) } else { event.position };
                self.extend_cells_to(at);
                cx.notify();
            } else if let Some(state) = self.sheet_state_mut() {
                state.cell_drag = false;
                state.deselect_drag = None;
            }
            return true;
        }
        // Dragging along the headers goes on past the sheet (over the status bar, the panes): the view scrolls (#186).
        if self.sheet_state().is_some_and(|s| s.header_select.is_some()) && event.pressed_button == Some(MouseButton::Left) {
            self.drag_pointer_moved(event.position, false, cx);
        }
        if self.drag_ruler_margin(event.position, event.pressed_button == Some(MouseButton::Left), cx) {
            return true;
        }
        false
    }

    /// A modifier key went down or up: a cell move and a fill that are being dragged copy while ⌥ is held.
    pub(crate) fn xlsx_modifiers_changed(&mut self, e: &ModifiersChangedEvent, cx: &mut Context<Self>) {
        if let Some(state) = self.sheet_state_mut() {
            let mut changed = false;
            if let Some(m) = &mut state.cell_move {
                if m.copy != e.modifiers.alt {
                    m.copy = e.modifiers.alt;
                    changed = true;
                }
            }
            if state.fill_copy != e.modifiers.alt {
                state.fill_copy = e.modifiers.alt;
                changed = true;
            }
            if changed {
                cx.notify();
            }
        }
    }

    /// The button went up: the workbook's drags end (the Draw panel's pen stroke first).
    pub(crate) fn xlsx_pointer_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.ink_release(cx) {
            return true;
        }
        if self.finish_sheet_insert(event, window, cx) {
            return true;
        }
        if self.end_ruler_margin_drag(cx) {
            return true;
        }
        if self.on_timeline_drag_up(cx) {
            return true;
        }
        if self.on_control_drag_up(cx) {
            return true;
        }
        if self.finish_cell_move(cx) {
            return true;
        }
        if self.sheet_state().is_some_and(|s| s.split_drag.is_some()) {
            self.on_split_up(event.position, cx);
            return true;
        }
        if let Some(edit) = self.sheet_state_mut().and_then(|s| s.cell_edit.as_mut())
            && edit.dragging.take().is_some()
        {
            { cx.notify(); return true; }
        }
        if let Some(state) = self.sheet_state_mut()
            && state.cell_drag
        {
            state.cell_drag = false;
            state.deselect_drag = None;
            { cx.notify(); return true; }
        }
        false
    }

    /// A press on a form control, timeline, slicer or chart data point.
    pub(crate) fn xlsx_shape_press(&mut self, shape: ibis_doc_model::ShapeId, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.selected_chart_bar = self.chart_bar(shape, event.position)
            .map(|(series, index)| (self.file_id, self.current_slide, shape, series, index));
        if event.modifiers.platform && !event.modifiers.shift && !event.modifiers.alt && !event.modifiers.control
            && self.selected_chart_bar.is_some()
            && self.chart_drill_down(shape, window, cx)
        {
            return true;
        }
        if self.slide().shape(shape).is_some_and(|s| matches!(s.kind, ShapeKind::Control(_))) {
            self.commit_edit(window, cx);
            self.focus_handle.focus(window, cx);
            if self.control_press(shape, event, window, cx) {
                { cx.notify(); return true; }
            }
        }
        if self.slide().shape(shape).is_some_and(|s| matches!(s.kind, ShapeKind::Timeline(_))) {
            self.commit_edit(window, cx);
            self.focus_handle.focus(window, cx);
            if self.timeline_press(shape, event, cx) {
                self.set_selection(vec![shape]);
                { cx.notify(); return true; }
            }
        }
        if self.slide().shape(shape).is_some_and(|s| matches!(s.kind, ShapeKind::Slicer(_))) {
            self.commit_edit(window, cx);
            self.focus_handle.focus(window, cx);
            if self.slicer_press(shape, event, cx) {
                self.set_selection(vec![shape]);
                { cx.notify(); return true; }
            }
        }
        false
    }

    fn chart_bar(&self, id: ibis_doc_model::ShapeId, position: Point<Pixels>) -> Option<(usize, usize)> {
        let shape = self.slide().shape(id)?;
        let ShapeKind::Chart(chart) = &shape.kind else { return None };
        if shape.rotation != 0 || shape.flip_h || shape.flip_v || chart.cell_refs.is_empty() || self.page_layout_shown() {
            return None;
        }
        let layout = self.slide_layout()?;
        let frame = self.display_frame(shape);
        let origin = layout.origin + self.main_pane_lead()
            + point(px(frame.x as f32 * layout.scale), px(frame.y as f32 * layout.scale));
        crate::render::chart_view::chart_bar_at(
            chart, ibis_doc_model::EmuRect { x: 0, y: 0, ..frame }, layout.scale, &self.resources, position - origin,
        )
    }

    fn chart_drill_down(&mut self, id: ibis_doc_model::ShapeId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let target = (|| {
            let (_, _, _, series, index) = self.selected_chart_bar?;
            let ShapeKind::Chart(chart) = &self.slide().shape(id)?.kind else { return None };
            let refs = chart.cell_refs.get(series)?;
            let source = |formula: &str| {
                let reference = crate::sheet_ui::parse_reference(formula)?;
                let (slide, range) = self.resolve_reference(&reference)?;
                let (column, row) = chart_source_cell(range, index)?;
                Some((slide, column, row))
            };
            let value = refs.values.as_deref().and_then(source);
            for (slide, column, row) in refs.categories.as_deref().and_then(source).into_iter().chain(value) {
                let (_, sheet) = crate::shell::editor::sheet_of(self.presentation().slide(slide)?)?;
                let location = sheet.hyperlinks.iter().find_map(|link| {
                    (link.target.is_none()
                        && (link.range.first_row..=link.range.last_row).contains(&row)
                        && (link.range.first_column..=link.range.last_column).contains(&column))
                        .then_some(link.location.as_deref()).flatten()
                });
                if let Some(location) = location
                    && let Some(reference) = crate::sheet_ui::parse_reference(location)
                    && let Some(target) = self.resolve_reference_from(&reference, slide)
                {
                    return Some(target);
                }
            }
            let (slide, column, row) = value?;
            Some((slide, ibis_doc_model::CellRange {
                first_column: column, last_column: column, first_row: row, last_row: row,
            }))
        })();
        let Some((slide, range)) = target else { return false };
        self.jump_to_cell_range(slide, range, window, cx);
        true
    }

    /// Escape, after the window's dialogs: a cell jump in progress is cancelled.
    pub(crate) fn xlsx_cancel_tool_late(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.cancel_cell_jump(window, cx) {
            return true;
        }
        // Esc while the box of the insert mode is dragged: the drag is dropped (the mode stays; a second Esc leaves it).
        if self.drag.is_some() && self.cancel_pointer_drag(cx) {
            return true;
        }
        // Esc takes the copied cells' dashed outline away (Excel's; K0011).
        if let Some(state) = self.sheet_state_mut()
            && state.copy_outline
        {
            state.copy_outline = false;
            cx.notify();
            return true;
        }
        false
    }

    /// The font size (points) chosen in the toolbar, for the cells selected.
    pub(crate) fn xlsx_set_font_size(&mut self, pt: f32, cx: &mut Context<Self>) {
        let hundredths = (pt * 100.0).round().clamp(100.0, 40_900.0) as u32;
        self.format_cells(ibis_doc_model::StyleChange::FontSize(hundredths), cx);
    }

    /// A workbook as text (.csv, and saved as .tsv: the sheet shown), as .ods (OpenDocument), as .xls (Excel 97-2003) and as a
    /// template (.xltx, .xltm). The file's password encrypts only the template. A copy named .tsv, .tab or .txt is the workbook's
    /// own package and a copy as .xls has no report of what was left out (as the shell did before the formats were cut apart).
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn xlsx_bytes_for_path(&self, path: &std::path::Path, source: &ibis_formats::Source, copy: bool) -> Option<Result<crate::shell::format::FileBytes, String>> {
        use crate::shell::format::FileBytes;
        let extension = path.extension()?;
        let is = |names: &[&str]| names.iter().any(|name| extension.eq_ignore_ascii_case(name));
        let plain = |bytes: Vec<u8>| FileBytes { bytes, protect: false, note: None };
        if is(&["csv", "tsv", "tab", "txt"])
            && self.path.as_deref().is_some_and(|old| crate::shell::save::same_file(old, path))
            && self.current_csv_format.is_some()
        {
            return Some(Err(crate::shell::i18n::text("file.csv-error.copy-original").to_string()));
        }
        if is(&["csv"]) {
            let index = self.presentation().slide_index(self.current_slide).unwrap_or(0);
            return Some(Ok(plain(ibis_formats::csv::sheet_to_csv(self.presentation(), index))));
        }
        if !copy && is(&["tsv", "tab", "txt"]) {
            let index = self.presentation().slide_index(self.current_slide).unwrap_or(0);
            return Some(Ok(plain(ibis_formats::csv::sheet_to_tsv(self.presentation(), index))));
        }
        if is(&["ods"]) {
            return Some(ibis_odf::write_ods_with_source(self.presentation(), source).map(plain).map_err(|e| e.to_string()));
        }
        if is(&["md", "markdown"]) {
            return Some(
                ibis_formats::markdown::export_presentation_to_markdown(self.presentation())
                    .map(|md| plain(md.into_bytes()))
                    .map_err(|e| e.to_string()),
            );
        }
        if is(&["xls"]) {
            if copy {
                return Some(ibis_formats::write_xls(self.presentation()).map(plain).map_err(|e| e.to_string()));
            }
            return Some(ibis_formats::write_xls_report(self.presentation()).map(|(bytes, report)| FileBytes { bytes, protect: false, note: Some(crate::shell::xls_note::note(&report)) }).map_err(|e| e.to_string()));
        }
        if is(&["xltx", "xltm"]) {
            return Some(source.save_template(self.presentation(), ibis_formats::SaveOptions::default()).map(|bytes| FileBytes { bytes, protect: true, note: None }).map_err(|e| e.to_string()));
        }
        None
    }

    /// What the PDF of the workbook is made from: the active sheet, the whole workbook or the selection (印刷する対象).
    pub(crate) fn xlsx_pdf_job(&self) -> Result<crate::shell::format::PdfJob, String> {
        let print_what = self.wb().print_what;
        let current_idx = self.presentation().slide_index(self.current_slide).unwrap_or(0);
        let (bytes, slides, _args): (Vec<u8>, Option<Vec<usize>>, Option<Vec<String>>) = match print_what {
            crate::PrintWhat::ActiveSheet => {
                let b = self.current_bytes()?;
                (b, Some(vec![current_idx + 1]), None)
            }
            crate::PrintWhat::EntireWorkbook => {
                let b = self.current_bytes()?;
                (b, None, None)
            }
            crate::PrintWhat::Selection => {
                let range = self.selected_range().or_else(|| self.current_cell().map(|(c, r)| ibis_doc_model::CellRange { first_row: r, first_column: c, last_row: r, last_column: c }));
                let mut p = self.presentation().clone();
                if let Some(r) = range {
                    if let Some(slide) = p.slide_mut(self.current_slide) {
                        for shape in &mut slide.shapes {
                            if let ibis_doc_model::ShapeKind::Sheet(sheet) = &mut shape.kind {
                                let sheet = std::sync::Arc::make_mut(sheet);
                                sheet.print_area = Some(r);
                            }
                        }
                    }
                }
                let source = self.source.clone().ok_or(crate::shell::i18n::text("_shell.this-file-cannot-be-saved-yet").to_string())?;
                let b = source.save(&p, ibis_formats::SaveOptions::default()).map_err(|e| e.to_string())?;
                (b, Some(vec![current_idx + 1]), None)
            }
        };
        Ok(crate::shell::format::PdfJob { bytes, slides, format: "pdf", args: None })
    }
}

pub(crate) fn chart_source_cell(range: ibis_doc_model::CellRange, index: usize) -> Option<(u32, u32)> {
    let index = u32::try_from(index).ok()?;
    if range.first_column == range.last_column {
        let row = range.first_row.checked_add(index)?;
        return (row <= range.last_row).then_some((range.first_column, row));
    }
    if range.first_row == range.last_row {
        let column = range.first_column.checked_add(index)?;
        return (column <= range.last_column).then_some((column, range.first_row));
    }
    None
}
