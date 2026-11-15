// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Shared AI Assistant sidebar for spreadsheets, presentations, and documents.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_doc_model::EmuRect;
use ibis_doc_model::ai_sheet::{
    format_range, read_sheet_range, resolve_sheet_slide, workbook_overview,
};
use ibis_doc_model::command::Command;
use ibis_doc_model::drawing::{Fill, Geometry};
use ibis_doc_model::flow::{Block, FlowParagraph, FlowRunKind};
use ibis_doc_model::flow_command::{FlowCommand, FlowPos};
use ibis_doc_model::machine_readability::{CandidateKind, candidates as mr_candidates};
use ibis_doc_model::model::{AutoShape, Shape, ShapeId, ShapeKind, Slide, SlideId};
use ibis_doc_model::sheet_calc::sheet_of_slide;
use ibis_doc_model::sheet_command::SheetCommand;
use ibis_doc_model::text::{TextBody, TextStyle};
use serde_json::json;

use crate::with_format;

use crate::ai_client::{AiConfig, ChatMessage, ToolCall, ToolDefinition, stream_chat_completion};
use crate::ai_tools::{
    AddSlideArgs, InsertTextArgs, MAX_READ_CELLS, ReadParagraphsArgs, ReplaceParagraphArgs,
    SetShapeTextArgs, SetSlideNotesArgs, SlideIndexOptionalArg, SlideIndexRequiredArg,
    build_replace_paragraph_command, docx_tool_definitions, parse_clear_cells_command,
    parse_create_sheet_arg, parse_delete_axis_command, parse_insert_axis_command,
    parse_merge_cells_command, parse_range_arg, parse_set_cells_command, parse_set_format_command,
    parse_set_paragraph_align_command, parse_sheet_arg, parse_switch_sheet_arg,
    pptx_tool_definitions, spreadsheet_tool_definitions,
};
use crate::chrome::SidebarPanel;
use crate::editor::SlideEditor;

const MAX_TOOL_STEPS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiDocKind {
    Xlsx,
    Pptx,
    Docx,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiMessageKind {
    User,
    Assistant,
    Tool,
    Error,
}

#[derive(Clone, Debug)]
pub struct UiMessage {
    pub kind: UiMessageKind,
    pub text: String,
    pub has_edits: bool,
}

impl UiMessage {
    fn new(kind: UiMessageKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            has_edits: false,
        }
    }
}

pub struct AiAssistantState {
    pub prompt_input: Entity<InputState>,
    pub base_url_input: Entity<InputState>,
    pub model_input: Entity<InputState>,
    pub api_key_input: Entity<InputState>,
    pub show_settings: bool,
    pub show_tool_logs: bool,
    pub messages: Vec<UiMessage>,
    pub history: Vec<ChatMessage>,
    pub streaming: bool,
    pub streaming_text: String,
    pub run_id: u64,
    pub scroll_handle: ScrollHandle,
    pub _prompt_subscription: Subscription,
}

fn column_letters(mut col: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push((b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.into_iter().rev().collect()
}

fn paragraph_plain_text(p: &FlowParagraph) -> String {
    let mut out = String::new();
    for run in &p.runs {
        match &run.kind {
            FlowRunKind::Text(s) => out.push_str(s),
            FlowRunKind::Tab => out.push('\t'),
            FlowRunKind::LineBreak => out.push('\n'),
            _ => {}
        }
    }
    out
}

fn shape_text(shape: &Shape) -> Option<String> {
    match &shape.kind {
        ShapeKind::Auto(auto) => auto.text.as_ref().map(|tb| tb.text()),
        _ => None,
    }
}

fn shape_kind_label(kind: &ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Auto(_) => "auto_shape",
        ShapeKind::Picture(_) => "picture",
        ShapeKind::Table(_) => "table",
        ShapeKind::Chart(_) => "chart",
        _ => "other",
    }
}

impl SlideEditor {
    fn ai_doc_kind(&self) -> AiDocKind {
        if self.presentation().workbook.is_some() {
            AiDocKind::Xlsx
        } else if self.presentation().flow.is_some() {
            AiDocKind::Docx
        } else {
            AiDocKind::Pptx
        }
    }

    /// Open or focus the Assistant sidebar.
    pub fn open_ai_assistant_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_assistant.is_none() {
            let cfg = AiConfig::load();
            let prompt_input =
                cx.new(|cx| InputState::new(window, cx).placeholder("指示を入力 (Enterで送信)"));
            let _prompt_subscription = cx.subscribe_in(
                &prompt_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                        this.send_ai_assistant_prompt(window, cx);
                    }
                },
            );
            let base_url_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(cfg.base_url)
                    .placeholder("https://api.openai.com/v1")
            });
            let model_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(cfg.model)
                    .placeholder("gpt-6.1-terra")
            });
            let api_key_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(cfg.api_key)
                    .placeholder("sk-... (ローカルLLMなら空でも可)")
                    .masked(true)
            });
            self.ai_assistant = Some(AiAssistantState {
                prompt_input: prompt_input.clone(),
                base_url_input,
                model_input,
                api_key_input,
                show_settings: false,
                show_tool_logs: false,
                messages: Vec::new(),
                history: Vec::new(),
                streaming: false,
                streaming_text: String::new(),
                run_id: 0,
                scroll_handle: ScrollHandle::new(),
                _prompt_subscription,
            });
            prompt_input.update(cx, |input, cx| input.focus(window, cx));
        }
        self.sidebar_open = true;
        cx.notify();
    }

    pub fn close_ai_assistant_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.ai_assistant.as_mut() {
            p.streaming = false;
            p.run_id = p.run_id.wrapping_add(1);
        }
        self.ai_assistant = None;
        self.sidebar_open = false;
        cx.notify();
    }

    fn toggle_ai_assistant_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.ai_assistant.as_mut() {
            p.show_settings = !p.show_settings;
        }
        cx.notify();
    }

    fn toggle_ai_tool_logs(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.ai_assistant.as_mut() {
            p.show_tool_logs = !p.show_tool_logs;
        }
        cx.notify();
    }

    fn save_ai_assistant_settings(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.ai_assistant.as_ref() else {
            return;
        };
        let cfg = AiConfig {
            base_url: p.base_url_input.read(cx).value().trim().to_string(),
            model: p.model_input.read(cx).value().trim().to_string(),
            api_key: p.api_key_input.read(cx).value().trim().to_string(),
        };
        cfg.save();
        if let Some(p) = self.ai_assistant.as_mut() {
            p.show_settings = false;
        }
        cx.notify();
    }

    fn stop_ai_assistant(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.ai_assistant.as_mut() {
            p.streaming = false;
            p.run_id = p.run_id.wrapping_add(1);
            if !p.streaming_text.is_empty() {
                let text = std::mem::take(&mut p.streaming_text);
                p.messages.push(UiMessage::new(UiMessageKind::Assistant, text));
            }
        }
        cx.notify();
    }

    fn clear_ai_assistant_history(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.ai_assistant.as_mut() {
            p.streaming = false;
            p.run_id = p.run_id.wrapping_add(1);
            p.streaming_text.clear();
            p.messages.clear();
            p.history.clear();
        }
        cx.notify();
    }

    fn send_ai_assistant_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = {
            let Some(p) = self.ai_assistant.as_ref() else {
                return;
            };
            if p.streaming {
                return;
            }
            let prompt = p.prompt_input.read(cx).value().trim().to_string();
            if prompt.is_empty() {
                return;
            }
            p.prompt_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            prompt
        };
        self.submit_ai_assistant_text(prompt, cx);
    }

    fn submit_ai_assistant_text(&mut self, prompt: String, cx: &mut Context<Self>) {
        let kind = self.ai_doc_kind();
        let (run_id, mut messages_to_send) = {
            let Some(p) = self.ai_assistant.as_mut() else {
                return;
            };
            if p.streaming {
                return;
            }
            p.run_id = p.run_id.wrapping_add(1);
            p.streaming = true;
            p.streaming_text.clear();
            p.messages.push(UiMessage::new(UiMessageKind::User, prompt.clone()));
            p.scroll_handle.scroll_to_bottom();
            if p.history.is_empty() {
                let sys = match kind {
                    AiDocKind::Xlsx => {
                        "You are an AI spreadsheet assistant inside ibisOffice. \
                         Use the provided tools to inspect the workbook and apply edits or formatting. \
                         Always inspect relevant cells with read_range before modifying existing data. \
                         Respond concisely in the user's language."
                    }
                    AiDocKind::Pptx => {
                        "You are an AI presentation assistant inside ibisOffice. \
                         Use the provided tools to inspect slides, edit shape text, update speaker notes, or add slides. \
                         Respond concisely in the user's language."
                    }
                    AiDocKind::Docx => {
                        "You are an AI document assistant inside ibisOffice. \
                         Use the provided tools to inspect paragraphs and edit or format document text. \
                         Respond concisely in the user's language."
                    }
                };
                p.history.push(ChatMessage::system(sys));
            }
            p.history.push(ChatMessage::user(prompt));
            (p.run_id, p.history.clone())
        };
        let overview_context = self.ai_overview_json(kind);
        if let Some(last) = messages_to_send.last_mut()
            && last.role == "user"
            && let Some(content) = &last.content
        {
            last.content = Some(format!("{content}\n\n[Document Context]\n{overview_context}"));
        }

        let cfg = AiConfig::load();
        let tools: Vec<ToolDefinition> = match kind {
            AiDocKind::Xlsx => spreadsheet_tool_definitions(),
            AiDocKind::Pptx => pptx_tool_definitions(),
            AiDocKind::Docx => docx_tool_definitions(),
        };

        cx.spawn(async move |this, cx| {
            let mut turn_commands: Vec<Command> = Vec::new();
            for _step in 0..MAX_TOOL_STEPS {
                let is_active = this
                    .read_with(cx, |editor, _| {
                        editor
                            .ai_assistant
                            .as_ref()
                            .is_some_and(|p| p.streaming && p.run_id == run_id)
                    })
                    .unwrap_or(false);
                if !is_active {
                    return;
                }

                let outcome = stream_chat_completion(
                    &cfg,
                    &messages_to_send,
                    tools.clone(),
                    |delta| {
                        let delta_owned = delta.to_string();
                        let _ = this.update(cx, |editor, cx| {
                            if let Some(p) = editor.ai_assistant.as_mut()
                                && p.streaming
                                && p.run_id == run_id
                            {
                                p.streaming_text.push_str(&delta_owned);
                                p.scroll_handle.scroll_to_bottom();
                                cx.notify();
                            }
                        });
                    },
                )
                .await;

                match outcome {
                    Err(err) => {
                        let _ = this.update(cx, |editor, cx| {
                            if let Some(p) = editor.ai_assistant.as_mut()
                                && p.run_id == run_id
                            {
                                p.streaming = false;
                                p.streaming_text.clear();
                                p.messages.push(UiMessage::new(UiMessageKind::Error, err));
                                p.scroll_handle.scroll_to_bottom();
                                cx.notify();
                            }
                        });
                        return;
                    }
                    Ok(out) => {
                        let mut should_continue = false;
                        let _ = this.update(cx, |editor, cx| {
                            let Some(p) = editor.ai_assistant.as_mut() else {
                                return;
                            };
                            if !p.streaming || p.run_id != run_id {
                                return;
                            }
                            p.streaming_text.clear();

                            let assistant_msg = ChatMessage::assistant(
                                if out.content.is_empty() {
                                    None
                                } else {
                                    Some(out.content.clone())
                                },
                                out.tool_calls.clone(),
                            );
                            p.history.push(assistant_msg.clone());
                            messages_to_send.push(assistant_msg);

                            if !out.content.is_empty() {
                                p.messages.push(UiMessage::new(
                                    UiMessageKind::Assistant,
                                    out.content.clone(),
                                ));
                            }

                            if out.tool_calls.is_empty() {
                                let applied_edits = !turn_commands.is_empty();
                                if applied_edits {
                                    let cmds = std::mem::take(&mut turn_commands);
                                    let grouped = if cmds.len() == 1 {
                                        cmds.into_iter().next().expect("single cmd")
                                    } else {
                                        Command::Group(cmds)
                                    };
                                    editor.run(vec![grouped], cx);
                                }
                                if let Some(p) = editor.ai_assistant.as_mut() {
                                    p.streaming = false;
                                    if applied_edits {
                                        if let Some(last) = p
                                            .messages
                                            .iter_mut()
                                            .rev()
                                            .find(|m| m.kind == UiMessageKind::Assistant)
                                        {
                                            last.has_edits = true;
                                        } else {
                                            let mut msg = UiMessage::new(
                                                UiMessageKind::Assistant,
                                                "変更を適用しました。",
                                            );
                                            msg.has_edits = true;
                                            p.messages.push(msg);
                                        }
                                    }
                                    p.scroll_handle.scroll_to_bottom();
                                }
                                cx.notify();
                                return;
                            }

                            for call in &out.tool_calls {
                                let (result_json, cmd_opt, log_text) =
                                    editor.execute_ai_tool(kind, call, cx);
                                if let Some(cmd) = cmd_opt {
                                    turn_commands.push(cmd);
                                }
                                let tool_msg = ChatMessage::tool_result(&call.id, result_json);
                                messages_to_send.push(tool_msg.clone());
                                if let Some(p) = editor.ai_assistant.as_mut() {
                                    p.history.push(tool_msg);
                                    p.messages.push(UiMessage::new(UiMessageKind::Tool, log_text));
                                    p.scroll_handle.scroll_to_bottom();
                                }
                            }
                            should_continue = true;
                            cx.notify();
                        });

                        if !should_continue {
                            return;
                        }
                    }
                }
            }

            let _ = this.update(cx, |editor, cx| {
                if !turn_commands.is_empty() {
                    let grouped = if turn_commands.len() == 1 {
                        turn_commands.into_iter().next().expect("single cmd")
                    } else {
                        Command::Group(turn_commands)
                    };
                    editor.run(vec![grouped], cx);
                }
                if let Some(p) = editor.ai_assistant.as_mut()
                    && p.run_id == run_id
                {
                    p.streaming = false;
                    p.messages.push(UiMessage::new(
                        UiMessageKind::Error,
                        format!("ツール実行が上限 ({MAX_TOOL_STEPS} 回) に達しました。"),
                    ));
                    p.scroll_handle.scroll_to_bottom();
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn ai_overview_json(&self, kind: AiDocKind) -> String {
        match kind {
            AiDocKind::Xlsx => self.ai_workbook_overview_json(),
            AiDocKind::Pptx => self.ai_presentation_overview_json(),
            AiDocKind::Docx => self.ai_document_overview_json(),
        }
    }

    fn ai_workbook_overview_json(&self) -> String {
        let presentation = self.presentation();
        let Some(wb) = presentation.workbook.as_ref() else {
            return json!({ "error": "no active workbook" }).to_string();
        };
        let active_slide = self.current_slide;
        let selection = with_format!(self, ai_sheet_selection(self, active_slide));
        let ov = workbook_overview(
            wb,
            active_slide,
            &|id| presentation.slide(id).and_then(sheet_of_slide).map(|v| &**v),
            selection,
        );
        json!({
            "active_sheet": ov.active_sheet,
            "selection": ov.selection,
            "sheets": ov.sheets.iter().map(|s| json!({
                "name": s.name,
                "active": s.active,
                "used_range": s.used_range,
                "cell_count": s.cell_count,
            })).collect::<Vec<_>>()
        })
        .to_string()
    }

    fn ai_presentation_overview_json(&self) -> String {
        let p = self.presentation();
        let active_idx = p.slide_index(self.current_slide).map(|i| i + 1);
        let slides: Vec<_> = p
            .slides
            .iter()
            .enumerate()
            .map(|(i, slide)| {
                let title = slide
                    .shapes
                    .iter()
                    .find_map(shape_text)
                    .map(|t| t.lines().next().unwrap_or("").chars().take(60).collect::<String>());
                json!({
                    "index": i + 1,
                    "slide_id": slide.id.0,
                    "active": slide.id == self.current_slide,
                    "title": title,
                    "shape_count": slide.shapes.len(),
                    "has_notes": !slide.notes.trim().is_empty(),
                })
            })
            .collect();
        let selected_shape_id = self.selection.map(|id| id.0);
        let selected_shape_text = self
            .selection
            .and_then(|sid| p.slide(self.current_slide).and_then(|s| s.shape(sid)))
            .and_then(shape_text);
        json!({
            "slide_count": p.slides.len(),
            "active_slide": active_idx,
            "selected_shape_id": selected_shape_id,
            "selected_shape_text": selected_shape_text,
            "slides": slides,
        })
        .to_string()
    }

    fn ai_document_overview_json(&self) -> String {
        let Some(flow) = self.presentation().flow.as_ref() else {
            return json!({ "error": "no active document" }).to_string();
        };
        let paragraphs: Vec<_> = flow
            .body
            .iter()
            .enumerate()
            .take(100)
            .map(|(idx, block)| match block {
                Block::Paragraph(p) => {
                    let preview: String = paragraph_plain_text(p).chars().take(120).collect();
                    json!({ "index": idx, "kind": "paragraph", "text": preview })
                }
                Block::Table(t) => {
                    json!({ "index": idx, "kind": "table", "rows": t.rows.len() })
                }
            })
            .collect();
        let cursor_paragraph = with_format!(self, ai_docx_cursor_paragraph(self));
        json!({
            "block_count": flow.body.len(),
            "cursor_paragraph": cursor_paragraph,
            "blocks": paragraphs,
        })
        .to_string()
    }

    fn resolve_ai_target_slide(&self, args: &str) -> Result<SlideId, String> {
        let sheet_name = parse_sheet_arg(args)?;
        let Some(wb) = self.presentation().workbook.as_ref() else {
            return Err("no active workbook".to_string());
        };
        resolve_sheet_slide(wb, sheet_name.as_deref(), self.current_slide)
    }

    fn resolve_pptx_slide_id(&self, slide_index: Option<usize>) -> Result<SlideId, String> {
        let Some(idx) = slide_index else {
            return Ok(self.current_slide);
        };
        if idx == 0 {
            return Err("slide_index must be >= 1".to_string());
        }
        self.presentation()
            .slides
            .get(idx - 1)
            .map(|s| s.id)
            .ok_or_else(|| format!("slide {idx} not found"))
    }

    fn execute_ai_tool(
        &mut self,
        kind: AiDocKind,
        call: &ToolCall,
        cx: &mut Context<Self>,
    ) -> (String, Option<Command>, String) {
        match kind {
            AiDocKind::Xlsx => self.execute_xlsx_tool(call, cx),
            AiDocKind::Pptx => self.execute_pptx_tool(call, cx),
            AiDocKind::Docx => self.execute_docx_tool(call),
        }
    }

    fn execute_xlsx_tool(
        &mut self,
        call: &ToolCall,
        cx: &mut Context<Self>,
    ) -> (String, Option<Command>, String) {
        let name = call.function.name.as_str();
        let args = call.function.arguments.as_str();
        match name {
            "get_workbook_overview" => {
                let json = self.ai_workbook_overview_json();
                (json, None, "✓ get_workbook_overview".to_string())
            }
            "read_range" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ read_range: {e}"),
                        );
                    }
                };
                match parse_range_arg(args) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ read_range: {e}"),
                    ),
                    Ok(range) => {
                        let Some(sheet) = self.presentation().slide(slide).and_then(sheet_of_slide)
                        else {
                            return (
                                json!({ "error": "target sheet not found" }).to_string(),
                                None,
                                "✗ read_range: target sheet not found".to_string(),
                            );
                        };
                        match read_sheet_range(sheet, range, MAX_READ_CELLS) {
                            Err(e) => (
                                json!({ "error": e }).to_string(),
                                None,
                                format!("✗ read_range: {e}"),
                            ),
                            Ok(cells) => {
                                let label = format_range(&range);
                                let payload = json!({
                                    "range": label,
                                    "cells": cells.iter().map(|c| json!({
                                        "cell": c.cell,
                                        "text": c.text,
                                        "formula": c.formula,
                                    })).collect::<Vec<_>>()
                                })
                                .to_string();
                                (
                                    payload,
                                    None,
                                    format!("✓ read_range ({label}, {} cells)", cells.len()),
                                )
                            }
                        }
                    }
                }
            }
            "set_cells" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ set_cells: {e}"),
                        );
                    }
                };
                match parse_set_cells_command(slide, args) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ set_cells: {e}"),
                    ),
                    Ok(cmd) => (
                        json!({ "ok": true }).to_string(),
                        Some(cmd),
                        "✓ set_cells".to_string(),
                    ),
                }
            }
            "set_format" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ set_format: {e}"),
                        );
                    }
                };
                match parse_set_format_command(slide, args) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ set_format: {e}"),
                    ),
                    Ok(cmd) => (
                        json!({ "ok": true }).to_string(),
                        Some(cmd),
                        "✓ set_format".to_string(),
                    ),
                }
            }
            "clear_cells" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ clear_cells: {e}"),
                        );
                    }
                };
                match parse_clear_cells_command(slide, args) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ clear_cells: {e}"),
                    ),
                    Ok(cmd) => (
                        json!({ "ok": true }).to_string(),
                        Some(cmd),
                        "✓ clear_cells".to_string(),
                    ),
                }
            }
            "select_range" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ select_range: {e}"),
                        );
                    }
                };
                match parse_range_arg(args) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ select_range: {e}"),
                    ),
                    Ok(range) => {
                        with_format!(self, set_ai_sheet_selection(self, slide, range));
                        cx.notify();
                        let label = format_range(&range);
                        (
                            json!({ "ok": true, "selection": label }).to_string(),
                            None,
                            format!("✓ select_range ({label})"),
                        )
                    }
                }
            }
            "switch_sheet" => match parse_switch_sheet_arg(args) {
                Err(e) => (
                    json!({ "error": e }).to_string(),
                    None,
                    format!("✗ switch_sheet: {e}"),
                ),
                Ok(sheet_name) => {
                    let target = self
                        .presentation()
                        .workbook
                        .as_ref()
                        .and_then(|wb| {
                            resolve_sheet_slide(wb, Some(&sheet_name), self.current_slide).ok()
                        });
                    let Some(slide) = target else {
                        return (
                            json!({ "error": format!("sheet '{sheet_name}' not found") })
                                .to_string(),
                            None,
                            format!("✗ switch_sheet: '{sheet_name}' not found"),
                        );
                    };
                    self.current_slide = slide;
                    self.selection = None;
                    cx.notify();
                    (
                        json!({ "ok": true, "active_sheet": sheet_name }).to_string(),
                        None,
                        format!("✓ switch_sheet ({sheet_name})"),
                    )
                }
            },
            "create_sheet" => match parse_create_sheet_arg(args) {
                Err(e) => (
                    json!({ "error": e }).to_string(),
                    None,
                    format!("✗ create_sheet: {e}"),
                ),
                Ok(name_opt) => {
                    let index = self
                        .presentation()
                        .workbook
                        .as_ref()
                        .map_or(0, |w| w.sheets.len());
                    let (slide, shape) = (
                        self.document.ids().new_slide_id(),
                        self.document.ids().new_shape_id(),
                    );
                    let sheet_name = name_opt.unwrap_or_else(|| format!("Sheet{}", index + 1));
                    let cmd = Command::Sheet(SheetCommand::AddSheet {
                        index,
                        name: sheet_name,
                        slide,
                        shape,
                    });
                    self.run(vec![cmd], cx);
                    if self.presentation().slide(slide).is_some() {
                        self.current_slide = slide;
                        self.selection = None;
                        cx.notify();
                    }
                    let actual_name = self
                        .presentation()
                        .workbook
                        .as_ref()
                        .and_then(|wb| {
                            wb.sheets
                                .iter()
                                .find(|s| s.slide == Some(slide))
                                .map(|s| s.name.clone())
                        })
                        .unwrap_or_default();
                    (
                        json!({ "ok": true, "sheet": actual_name }).to_string(),
                        None,
                        format!("✓ create_sheet ({actual_name})"),
                    )
                }
            },
            "insert_rows_or_cols" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ insert_rows_or_cols: {e}"),
                        );
                    }
                };
                match parse_insert_axis_command(slide, args) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ insert_rows_or_cols: {e}"),
                    ),
                    Ok(cmd) => (
                        json!({ "ok": true }).to_string(),
                        Some(cmd),
                        "✓ insert_rows_or_cols".to_string(),
                    ),
                }
            }
            "delete_rows_or_cols" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ delete_rows_or_cols: {e}"),
                        );
                    }
                };
                match parse_delete_axis_command(slide, args) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ delete_rows_or_cols: {e}"),
                    ),
                    Ok(cmd) => (
                        json!({ "ok": true }).to_string(),
                        Some(cmd),
                        "✓ delete_rows_or_cols".to_string(),
                    ),
                }
            }
            "merge_cells" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ merge_cells: {e}"),
                        );
                    }
                };
                match parse_merge_cells_command(slide, args) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ merge_cells: {e}"),
                    ),
                    Ok(cmd) => (
                        json!({ "ok": true }).to_string(),
                        Some(cmd),
                        "✓ merge_cells".to_string(),
                    ),
                }
            }
            "check_machine_readability" => {
                let filter_sheet = match parse_sheet_arg(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ check_machine_readability: {e}"),
                        );
                    }
                };
                let mut findings = Vec::new();
                for (index, slide) in self.presentation().slides.iter().enumerate() {
                    let Some(sheet) = sheet_of_slide(slide) else {
                        continue;
                    };
                    let sheet_name = slide
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("Sheet{}", index + 1));
                    if let Some(ref target) = filter_sheet
                        && &sheet_name != target
                    {
                        continue;
                    }
                    for c in mr_candidates(sheet) {
                        let r = c.range;
                        let a = format!("{}{}", column_letters(r.first_column), r.first_row + 1);
                        let b = format!("{}{}", column_letters(r.last_column), r.last_row + 1);
                        let address = if a == b { a } else { format!("{a}:{b}") };
                        let kind = match c.kind {
                            CandidateKind::MergedCells => "merged_cells",
                            CandidateKind::HiddenRow => "hidden_row",
                            CandidateKind::HiddenColumn => "hidden_column",
                            CandidateKind::Formula => "formula",
                            CandidateKind::PlatformDependentCharacters => {
                                "platform_dependent_characters"
                            }
                            CandidateKind::PrivateUseCharacters => "private_use_characters",
                        };
                        findings.push(json!({
                            "sheet": sheet_name,
                            "kind": kind,
                            "location": address,
                        }));
                    }
                }
                let count = findings.len();
                (
                    json!({ "count": count, "findings": findings }).to_string(),
                    None,
                    format!("✓ check_machine_readability ({count} findings)"),
                )
            }
            other => (
                json!({ "error": format!("unknown tool: {other}") }).to_string(),
                None,
                format!("✗ {other}: unknown tool"),
            ),
        }
    }

    fn execute_pptx_tool(
        &mut self,
        call: &ToolCall,
        cx: &mut Context<Self>,
    ) -> (String, Option<Command>, String) {
        let name = call.function.name.as_str();
        let args = call.function.arguments.as_str();
        match name {
            "get_presentation_overview" => (
                self.ai_presentation_overview_json(),
                None,
                "✓ get_presentation_overview".to_string(),
            ),
            "read_slide" => {
                let parsed: Result<SlideIndexOptionalArg, _> = serde_json::from_str(args);
                let slide_id = match parsed
                    .map_err(|e| format!("invalid arguments: {e}"))
                    .and_then(|a| self.resolve_pptx_slide_id(a.slide_index))
                {
                    Ok(id) => id,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ read_slide: {e}"),
                        );
                    }
                };
                let Some(slide) = self.presentation().slide(slide_id) else {
                    return (
                        json!({ "error": "slide not found" }).to_string(),
                        None,
                        "✗ read_slide: not found".to_string(),
                    );
                };
                let slide_idx = self.presentation().slide_index(slide_id).map(|i| i + 1);
                let shapes: Vec<_> = slide
                    .shapes
                    .iter()
                    .map(|sh| {
                        json!({
                            "shape_id": sh.id.0,
                            "kind": shape_kind_label(&sh.kind),
                            "text": shape_text(sh),
                        })
                    })
                    .collect();
                (
                    json!({
                        "slide_index": slide_idx,
                        "notes": slide.notes,
                        "shapes": shapes,
                    })
                    .to_string(),
                    None,
                    format!("✓ read_slide ({} shapes)", shapes.len()),
                )
            }
            "set_shape_text" => {
                let parsed: Result<SetShapeTextArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ set_shape_text: {e}"),
                    ),
                    Ok(a) => match self.resolve_pptx_slide_id(a.slide_index) {
                        Err(e) => (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ set_shape_text: {e}"),
                        ),
                        Ok(slide_id) => {
                            let shape_id = ShapeId(a.shape_id);
                            let exists = self
                                .presentation()
                                .slide(slide_id)
                                .and_then(|s| s.shape(shape_id))
                                .is_some();
                            if !exists {
                                return (
                                    json!({ "error": format!("shape {} not found", a.shape_id) })
                                        .to_string(),
                                    None,
                                    format!("✗ set_shape_text: shape {} not found", a.shape_id),
                                );
                            }
                            (
                                json!({ "ok": true }).to_string(),
                                Some(Command::SetText {
                                    slide: slide_id,
                                    shape: shape_id,
                                    text: a.text,
                                }),
                                format!("✓ set_shape_text (shape {})", a.shape_id),
                            )
                        }
                    },
                }
            }
            "set_slide_notes" => {
                let parsed: Result<SetSlideNotesArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ set_slide_notes: {e}"),
                    ),
                    Ok(a) => match self.resolve_pptx_slide_id(a.slide_index) {
                        Err(e) => (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ set_slide_notes: {e}"),
                        ),
                        Ok(slide_id) => (
                            json!({ "ok": true }).to_string(),
                            Some(Command::SetNotes {
                                slide: slide_id,
                                notes: a.notes,
                            }),
                            "✓ set_slide_notes".to_string(),
                        ),
                    },
                }
            }
            "add_slide" => {
                let parsed: Result<AddSlideArgs, _> = if args.trim().is_empty() {
                    Ok(AddSlideArgs {
                        title: None,
                        body: None,
                    })
                } else {
                    serde_json::from_str(args)
                };
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ add_slide: {e}"),
                    ),
                    Ok(a) => {
                        let slide_id = self.document.ids().new_slide_id();
                        let index = self
                            .presentation()
                            .slide_index(self.current_slide)
                            .map_or(self.presentation().slides.len(), |i| i + 1);
                        let size = self.presentation().slide_size;
                        let mut slide = Slide::new(slide_id);
                        if let Some(title) = a.title.filter(|s| !s.trim().is_empty()) {
                            let shape_id = self.document.ids().new_shape_id();
                            let frame = EmuRect::new(
                                size.width / 12,
                                size.height / 10,
                                size.width * 10 / 12,
                                size.height / 6,
                            );
                            let mut auto = AutoShape::new(Geometry::Rect, Fill::None, None);
                            let mut style = TextStyle::default();
                            style.font_size = 3200;
                            style.bold = true;
                            auto.text = Some(TextBody::plain(&title, style));
                            slide
                                .shapes
                                .push(Shape::new(shape_id, frame, ShapeKind::Auto(auto)));
                        }
                        if let Some(body) = a.body.filter(|s| !s.trim().is_empty()) {
                            let shape_id = self.document.ids().new_shape_id();
                            let frame = EmuRect::new(
                                size.width / 12,
                                size.height * 3 / 10,
                                size.width * 10 / 12,
                                size.height * 6 / 10,
                            );
                            let mut auto = AutoShape::new(Geometry::Rect, Fill::None, None);
                            let mut style = TextStyle::default();
                            style.font_size = 1800;
                            auto.text = Some(TextBody::plain(&body, style));
                            slide
                                .shapes
                                .push(Shape::new(shape_id, frame, ShapeKind::Auto(auto)));
                        }
                        self.run(vec![Command::InsertSlide { index, slide }], cx);
                        self.current_slide = slide_id;
                        self.selection = None;
                        cx.notify();
                        (
                            json!({ "ok": true, "slide_index": index + 1 }).to_string(),
                            None,
                            format!("✓ add_slide (slide {})", index + 1),
                        )
                    }
                }
            }
            "select_slide" => {
                let parsed: Result<SlideIndexRequiredArg, _> = serde_json::from_str(args);
                match parsed
                    .map_err(|e| format!("invalid arguments: {e}"))
                    .and_then(|a| {
                        self.resolve_pptx_slide_id(Some(a.slide_index))
                            .map(|id| (a.slide_index, id))
                    }) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ select_slide: {e}"),
                    ),
                    Ok((idx, slide_id)) => {
                        self.current_slide = slide_id;
                        self.selection = None;
                        cx.notify();
                        (
                            json!({ "ok": true, "slide_index": idx }).to_string(),
                            None,
                            format!("✓ select_slide ({idx})"),
                        )
                    }
                }
            }
            other => (
                json!({ "error": format!("unknown tool: {other}") }).to_string(),
                None,
                format!("✗ {other}: unknown tool"),
            ),
        }
    }

    fn execute_docx_tool(&mut self, call: &ToolCall) -> (String, Option<Command>, String) {
        let name = call.function.name.as_str();
        let args = call.function.arguments.as_str();
        match name {
            "get_document_overview" => (
                self.ai_document_overview_json(),
                None,
                "✓ get_document_overview".to_string(),
            ),
            "read_paragraphs" => {
                let parsed: Result<ReadParagraphsArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ read_paragraphs: {e}"),
                    ),
                    Ok(a) => {
                        let Some(flow) = self.presentation().flow.as_ref() else {
                            return (
                                json!({ "error": "no active document" }).to_string(),
                                None,
                                "✗ read_paragraphs: no active document".to_string(),
                            );
                        };
                        let count = a.count.clamp(1, 50);
                        let items: Vec<_> = flow
                            .body
                            .iter()
                            .enumerate()
                            .skip(a.start)
                            .take(count)
                            .map(|(idx, block)| match block {
                                Block::Paragraph(p) => json!({
                                    "index": idx,
                                    "kind": "paragraph",
                                    "text": paragraph_plain_text(p),
                                }),
                                Block::Table(t) => json!({
                                    "index": idx,
                                    "kind": "table",
                                    "rows": t.rows.len(),
                                }),
                            })
                            .collect();
                        (
                            json!({ "paragraphs": items }).to_string(),
                            None,
                            format!("✓ read_paragraphs ({} items)", items.len()),
                        )
                    }
                }
            }
            "insert_text" => {
                let parsed: Result<InsertTextArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ insert_text: {e}"),
                    ),
                    Ok(a) => {
                        let Some(flow) = self.presentation().flow.as_ref() else {
                            return (
                                json!({ "error": "no active document" }).to_string(),
                                None,
                                "✗ insert_text: no active document".to_string(),
                            );
                        };
                        let Some(Block::Paragraph(p)) = flow.body.get(a.paragraph as usize) else {
                            return (
                                json!({ "error": format!("paragraph {} not found", a.paragraph) })
                                    .to_string(),
                                None,
                                format!("✗ insert_text: paragraph {} not found", a.paragraph),
                            );
                        };
                        let offset = a.offset.unwrap_or_else(|| p.len()).min(p.len());
                        (
                            json!({ "ok": true }).to_string(),
                            Some(Command::Flow(FlowCommand::InsertText {
                                at: FlowPos::new(a.paragraph, offset),
                                text: a.text,
                                format: None,
                            })),
                            format!("✓ insert_text (p{})", a.paragraph),
                        )
                    }
                }
            }
            "replace_paragraph_text" => {
                let parsed: Result<ReplaceParagraphArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ replace_paragraph_text: {e}"),
                    ),
                    Ok(a) => {
                        let Some(flow) = self.presentation().flow.as_ref() else {
                            return (
                                json!({ "error": "no active document" }).to_string(),
                                None,
                                "✗ replace_paragraph_text: no active document".to_string(),
                            );
                        };
                        let Some(Block::Paragraph(p)) = flow.body.get(a.paragraph as usize) else {
                            return (
                                json!({ "error": format!("paragraph {} not found", a.paragraph) })
                                    .to_string(),
                                None,
                                format!(
                                    "✗ replace_paragraph_text: paragraph {} not found",
                                    a.paragraph
                                ),
                            );
                        };
                        let old_len = p.len();
                        let cmd = build_replace_paragraph_command(a.paragraph, old_len, a.text);
                        (
                            json!({ "ok": true }).to_string(),
                            Some(cmd),
                            format!("✓ replace_paragraph_text (p{})", a.paragraph),
                        )
                    }
                }
            }
            "set_paragraph_align" => match parse_set_paragraph_align_command(args) {
                Err(e) => (
                    json!({ "error": e }).to_string(),
                    None,
                    format!("✗ set_paragraph_align: {e}"),
                ),
                Ok(cmd) => (
                    json!({ "ok": true }).to_string(),
                    Some(cmd),
                    "✓ set_paragraph_align".to_string(),
                ),
            },
            other => (
                json!({ "error": format!("unknown tool: {other}") }).to_string(),
                None,
                format!("✗ {other}: unknown tool"),
            ),
        }
    }

    /// Render the Assistant right sidebar panel.
    pub fn ai_assistant_panel(&mut self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        let kind = self.ai_doc_kind();
        let panel = self.ai_assistant.as_ref()?;
        let cfg = AiConfig::load();
        let show_settings = panel.show_settings;
        let show_tool_logs = panel.show_tool_logs;
        let streaming = panel.streaming;
        let streaming_text = panel.streaming_text.clone();
        let messages = panel.messages.clone();
        let tool_log_count = messages
            .iter()
            .filter(|m| m.kind == UiMessageKind::Tool)
            .count();
        let scroll_handle = panel.scroll_handle.clone();
        let prompt_input = panel.prompt_input.clone();
        let base_url_input = panel.base_url_input.clone();
        let model_input = panel.model_input.clone();
        let api_key_input = panel.api_key_input.clone();

        let header = h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Model: {}", cfg.model)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .when(tool_log_count > 0, |row| {
                        row.child(
                            Button::new("ai-assistant-logs")
                                .ghost()
                                .xsmall()
                                .label(if show_tool_logs {
                                    format!("ログ非表示 ({tool_log_count})")
                                } else {
                                    format!("ログ ({tool_log_count})")
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_ai_tool_logs(cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("ai-assistant-clear")
                            .ghost()
                            .xsmall()
                            .label("クリア")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_ai_assistant_history(cx);
                            })),
                    )
                    .child(
                        Button::new("ai-assistant-settings")
                            .ghost()
                            .xsmall()
                            .label(if show_settings { "閉じる" } else { "設定" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_ai_assistant_settings(cx);
                            })),
                    ),
            );

        let settings_box = show_settings.then(|| {
            v_flex()
                .gap_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary.opacity(0.4))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("OpenAI 互換 API 設定"),
                )
                .child(Input::new(&base_url_input).small())
                .child(Input::new(&model_input).small())
                .child(Input::new(&api_key_input).small())
                .child(
                    Button::new("ai-assistant-save-settings")
                        .primary()
                        .small()
                        .label("設定を保存")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.save_ai_assistant_settings(cx);
                        })),
                )
        });

        let (intro, quick_actions): (&str, Vec<(&'static str, &'static str, &'static str)>) =
            match kind {
                AiDocKind::Xlsx => (
                    "セル範囲の集計、数式の作成、書式設定などを指示できます。",
                    vec![
                        (
                            "ai-quick-1",
                            "選択範囲を要約",
                            "現在の選択範囲（なければ使用範囲）を読み取って内容を要約してください。",
                        ),
                        (
                            "ai-quick-2",
                            "表を見やすく整形",
                            "アクティブシートの表を確認し、見出しを太字・背景色付きにし、罫線と数値書式を整えてください。",
                        ),
                        (
                            "ai-quick-3",
                            "機械可読性チェック",
                            "このブックの機械可読性をチェックし、改善点があれば報告してください。",
                        ),
                    ],
                ),
                AiDocKind::Pptx => (
                    "スライドの要約、テキスト編集、ノート作成、新規スライド追加などを指示できます。",
                    vec![
                        (
                            "ai-quick-1",
                            "プレゼン全体を要約",
                            "プレゼンテーション全体の構成と要点を要約してください。",
                        ),
                        (
                            "ai-quick-2",
                            "現在のスライドを整理",
                            "現在のスライドの内容を読み取り、より簡潔で分かりやすい表現に整えてください。",
                        ),
                        (
                            "ai-quick-3",
                            "スピーカーノートを作成",
                            "現在のスライドの内容に基づいて発表用のスピーカーノートを作成し、設定してください。",
                        ),
                    ],
                ),
                AiDocKind::Docx => (
                    "文書の要約、文章の校正・書き換え、段落の整形などを指示できます。",
                    vec![
                        (
                            "ai-quick-1",
                            "文書全体を要約",
                            "文書全体の段落を読み取って要点を要約してください。",
                        ),
                        (
                            "ai-quick-2",
                            "文章を校正・推敲",
                            "先頭の段落を読み取り、誤字脱字や読みやすさの改善案を提案してください。",
                        ),
                        (
                            "ai-quick-3",
                            "見出し・構成を提案",
                            "文書の内容に基づいて見出しや章立ての改善案を提案してください。",
                        ),
                    ],
                ),
            };

        let mut msg_list = v_flex().gap_2();
        if messages.is_empty() && streaming_text.is_empty() {
            let mut quick_col = v_flex().gap_1();
            for (id, label, prompt_text) in quick_actions {
                let prompt_owned = prompt_text.to_string();
                quick_col = quick_col.child(
                    Button::new(id)
                        .outline()
                        .xsmall()
                        .label(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.submit_ai_assistant_text(prompt_owned.clone(), cx);
                        })),
                );
            }
            msg_list = msg_list
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(intro),
                )
                .child(quick_col);
        }
        for (idx, msg) in messages.iter().enumerate() {
            if msg.kind == UiMessageKind::Tool && !show_tool_logs {
                continue;
            }
            let (bg, fg) = match msg.kind {
                UiMessageKind::User => (cx.theme().primary.opacity(0.12), cx.theme().foreground),
                UiMessageKind::Assistant => {
                    (cx.theme().secondary.opacity(0.5), cx.theme().foreground)
                }
                UiMessageKind::Tool => (
                    cx.theme().secondary.opacity(0.25),
                    cx.theme().muted_foreground,
                ),
                UiMessageKind::Error => (cx.theme().danger.opacity(0.15), cx.theme().danger),
            };
            let copy_text = msg.text.clone();
            let show_copy = matches!(msg.kind, UiMessageKind::Error | UiMessageKind::Assistant);
            let show_undo = msg.has_edits;
            msg_list = msg_list.child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .rounded_md()
                    .bg(bg)
                    .child(div().text_xs().text_color(fg).child(msg.text.clone()))
                    .when(show_copy || show_undo, |box_el| {
                        box_el.child(
                            h_flex()
                                .w_full()
                                .justify_end()
                                .gap_1()
                                .when(show_undo, |row| {
                                    row.child(
                                        Button::new(("ai-assistant-undo", idx))
                                            .ghost()
                                            .xsmall()
                                            .label("元に戻す")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.undo(&crate::actions::Undo, window, cx);
                                            })),
                                    )
                                })
                                .when(show_copy, |row| {
                                    row.child(
                                        Button::new(("ai-assistant-copy", idx))
                                            .ghost()
                                            .xsmall()
                                            .label("コピー")
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    copy_text.clone(),
                                                ));
                                            })),
                                    )
                                }),
                        )
                    }),
            );
        }
        if !streaming_text.is_empty() {
            msg_list = msg_list.child(
                div()
                    .p_2()
                    .rounded_md()
                    .bg(cx.theme().secondary.opacity(0.5))
                    .text_xs()
                    .text_color(cx.theme().foreground)
                    .child(format!("{streaming_text}▌")),
            );
        }

        let scrollable_msgs = div()
            .id("ai-assistant-msg-scroll")
            .max_h(px(460.0))
            .overflow_y_scroll()
            .track_scroll(&scroll_handle)
            .child(msg_list);

        let content = v_flex()
            .gap_2()
            .child(header)
            .children(settings_box)
            .child(scrollable_msgs);

        let footer = v_flex()
            .gap_2()
            .w_full()
            .child(Input::new(&prompt_input).small())
            .child(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_2()
                    .when(streaming, |row| {
                        row.child(
                            Button::new("ai-assistant-stop")
                                .danger()
                                .small()
                                .label("停止")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.stop_ai_assistant(cx);
                                })),
                        )
                    })
                    .when(!streaming, |row| {
                        row.child(
                            Button::new("ai-assistant-send")
                                .primary()
                                .small()
                                .label("送信")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.send_ai_assistant_prompt(window, cx);
                                })),
                        )
                    }),
            );

        let body = v_flex().gap_3().child(content).child(footer);

        Some(SidebarPanel {
            title: "Assistant".into(),
            body: body.into_any_element(),
        })
    }
}
