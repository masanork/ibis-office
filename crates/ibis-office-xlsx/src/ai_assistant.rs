// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Assistant sidebar for spreadsheets (OpenAI-compatible SSE + Tool Calling).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_doc_model::ai_sheet::{format_range, read_sheet_range, workbook_overview};
use ibis_doc_model::command::Command;
use ibis_doc_model::model::ShapeKind;
use ibis_doc_model::sheet::Sheet;
use serde_json::json;

use crate::ext::*;
use crate::shell::ai_client::{AiConfig, ChatMessage, ToolCall, stream_chat_completion};
use crate::shell::ai_tools::{
    MAX_READ_CELLS, parse_clear_cells_command, parse_range_arg, parse_set_cells_command,
    parse_set_format_command, spreadsheet_tool_definitions,
};
use crate::shell::chrome::SidebarPanel;
use crate::shell::editor::SlideEditor;

const MAX_TOOL_STEPS: usize = 8;

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
}

pub struct AiAssistantPanel {
    pub prompt_input: Entity<InputState>,
    pub base_url_input: Entity<InputState>,
    pub model_input: Entity<InputState>,
    pub api_key_input: Entity<InputState>,
    pub show_settings: bool,
    pub messages: Vec<UiMessage>,
    pub history: Vec<ChatMessage>,
    pub streaming: bool,
    pub streaming_text: String,
    pub run_id: u64,
}

#[ibis_office_macros::editor_ext(AiAssistantPanelExt)]
impl SlideEditor {
    /// Open or focus the Assistant sidebar.
    pub fn open_ai_assistant_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wb().panels.ai_assistant.is_none() {
            let cfg = AiConfig::load();
            let prompt_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("指示を入力 (例: B2:B10の合計をB11に入れて太字に)")
            });
            let base_url_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(cfg.base_url)
                    .placeholder("https://api.openai.com/v1")
            });
            let model_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(cfg.model)
                    .placeholder("gpt-4o-mini")
            });
            let api_key_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(cfg.api_key)
                    .placeholder("sk-... (ローカルLLMなら空でも可)")
                    .masked(true)
            });
            let show_settings = false;
            self.wb_mut().panels.ai_assistant = Some(AiAssistantPanel {
                prompt_input: prompt_input.clone(),
                base_url_input,
                model_input,
                api_key_input,
                show_settings,
                messages: Vec::new(),
                history: Vec::new(),
                streaming: false,
                streaming_text: String::new(),
                run_id: 0,
            });
            prompt_input.update(cx, |input, cx| input.focus(window, cx));
        }
        self.sidebar_open = true;
        cx.notify();
    }

    pub fn close_ai_assistant_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.wb_mut().panels.ai_assistant.as_mut() {
            p.streaming = false;
            p.run_id = p.run_id.wrapping_add(1);
        }
        self.wb_mut().panels.ai_assistant = None;
        self.sidebar_open = false;
        cx.notify();
    }

    fn toggle_ai_assistant_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.wb_mut().panels.ai_assistant.as_mut() {
            p.show_settings = !p.show_settings;
        }
        cx.notify();
    }

    fn save_ai_assistant_settings(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.wb().panels.ai_assistant.as_ref() else {
            return;
        };
        let cfg = AiConfig {
            base_url: p.base_url_input.read(cx).value().trim().to_string(),
            model: p.model_input.read(cx).value().trim().to_string(),
            api_key: p.api_key_input.read(cx).value().trim().to_string(),
        };
        cfg.save();
        if let Some(p) = self.wb_mut().panels.ai_assistant.as_mut() {
            p.show_settings = false;
        }
        cx.notify();
    }

    fn stop_ai_assistant(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.wb_mut().panels.ai_assistant.as_mut() {
            p.streaming = false;
            p.run_id = p.run_id.wrapping_add(1);
            if !p.streaming_text.is_empty() {
                let text = std::mem::take(&mut p.streaming_text);
                p.messages.push(UiMessage {
                    kind: UiMessageKind::Assistant,
                    text,
                });
            }
        }
        cx.notify();
    }

    fn clear_ai_assistant_history(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.wb_mut().panels.ai_assistant.as_mut() {
            p.streaming = false;
            p.run_id = p.run_id.wrapping_add(1);
            p.streaming_text.clear();
            p.messages.clear();
            p.history.clear();
        }
        cx.notify();
    }

    fn send_ai_assistant_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (prompt, run_id, mut messages_to_send) = {
            let Some(p) = self.wb_mut().panels.ai_assistant.as_mut() else {
                return;
            };
            if p.streaming {
                return;
            }
            let prompt = p.prompt_input.read(cx).value().trim().to_string();
            if prompt.is_empty() {
                return;
            }
            p.prompt_input.update(cx, |input, cx| input.set_value("", window, cx));
            p.run_id = p.run_id.wrapping_add(1);
            p.streaming = true;
            p.streaming_text.clear();
            p.messages.push(UiMessage {
                kind: UiMessageKind::User,
                text: prompt.clone(),
            });
            if p.history.is_empty() {
                p.history.push(ChatMessage::system(
                    "You are an AI spreadsheet assistant inside ibisOffice. \
                     Use the provided tools to inspect the workbook and apply edits or formatting. \
                     Always inspect relevant cells with read_range before modifying existing data. \
                     Respond concisely in the user's language.",
                ));
            }
            p.history.push(ChatMessage::user(prompt.clone()));
            (prompt, p.run_id, p.history.clone())
        };
        let _ = prompt;
        let overview_context = self.ai_workbook_overview_json();
        if let Some(last) = messages_to_send.last_mut() {
            if last.role == "user" {
                let original = last.content.take().unwrap_or_default();
                last.content = Some(format!(
                    "[Current Workbook Context]\n{overview_context}\n\n[User Request]\n{original}"
                ));
            }
        }
        let config = {
            let mut cfg = AiConfig::load();
            if let Some(p) = self.wb().panels.ai_assistant.as_ref() {
                let base_url = p.base_url_input.read(cx).value().trim().to_string();
                let model = p.model_input.read(cx).value().trim().to_string();
                let api_key = p.api_key_input.read(cx).value().trim().to_string();
                if !base_url.is_empty() {
                    cfg.base_url = base_url;
                }
                if !model.is_empty() {
                    cfg.model = model;
                }
                if !api_key.is_empty() {
                    cfg.api_key = api_key;
                }
            }
            cfg.save();
            cfg
        };
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut turn_commands: Vec<Command> = Vec::new();
            for _step in 0..MAX_TOOL_STEPS {
                let mut delta_cx = cx.clone();
                let weak_for_delta = this.clone();
                let outcome = stream_chat_completion(
                    &config,
                    &messages_to_send,
                    spreadsheet_tool_definitions(),
                    move |delta| {
                        let _ = weak_for_delta.update(&mut delta_cx, |editor, cx| {
                            if let Some(p) = editor.wb_mut().panels.ai_assistant.as_mut() {
                                if p.run_id == run_id && p.streaming {
                                    p.streaming_text.push_str(delta);
                                    cx.notify();
                                }
                            }
                        });
                    },
                )
                .await;

                let mut cancelled = false;
                let _ = this.update(cx, |editor, cx| {
                    let Some(p) = editor.wb_mut().panels.ai_assistant.as_mut() else {
                        cancelled = true;
                        return;
                    };
                    if p.run_id != run_id || !p.streaming {
                        cancelled = true;
                        return;
                    }
                    p.streaming_text.clear();
                    cx.notify();
                });
                if cancelled {
                    return;
                }

                match outcome {
                    Err(err) => {
                        let _ = this.update(cx, |editor, cx| {
                            if let Some(p) = editor.wb_mut().panels.ai_assistant.as_mut() {
                                if p.run_id == run_id {
                                    p.streaming = false;
                                    p.messages.push(UiMessage {
                                        kind: UiMessageKind::Error,
                                        text: err,
                                    });
                                }
                            }
                            cx.notify();
                        });
                        return;
                    }
                    Ok(out) => {
                        let assistant_text = if out.content.is_empty() {
                            None
                        } else {
                            Some(out.content.clone())
                        };
                        let assistant_msg =
                            ChatMessage::assistant(assistant_text.clone(), out.tool_calls.clone());
                        messages_to_send.push(assistant_msg.clone());

                        let mut should_continue = false;
                        let _ = this.update(cx, |editor, cx| {
                            if let Some(text) = assistant_text {
                                if let Some(p) = editor.wb_mut().panels.ai_assistant.as_mut() {
                                    p.messages.push(UiMessage {
                                        kind: UiMessageKind::Assistant,
                                        text,
                                    });
                                }
                            }
                            if let Some(p) = editor.wb_mut().panels.ai_assistant.as_mut() {
                                p.history.push(assistant_msg);
                            }

                            if out.tool_calls.is_empty() {
                                if !turn_commands.is_empty() {
                                    let cmds = std::mem::take(&mut turn_commands);
                                    let grouped = if cmds.len() == 1 {
                                        cmds.into_iter().next().unwrap()
                                    } else {
                                        Command::Group(cmds)
                                    };
                                    editor.run(vec![grouped], cx);
                                }
                                if let Some(p) = editor.wb_mut().panels.ai_assistant.as_mut() {
                                    p.streaming = false;
                                }
                                cx.notify();
                                return;
                            }

                            for call in &out.tool_calls {
                                let (result_json, cmd_opt, log_text) =
                                    editor.execute_ai_tool(call, cx);
                                if let Some(cmd) = cmd_opt {
                                    turn_commands.push(cmd);
                                }
                                let tool_msg = ChatMessage::tool_result(&call.id, result_json);
                                messages_to_send.push(tool_msg.clone());
                                if let Some(p) = editor.wb_mut().panels.ai_assistant.as_mut() {
                                    p.history.push(tool_msg);
                                    p.messages.push(UiMessage {
                                        kind: UiMessageKind::Tool,
                                        text: log_text,
                                    });
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
                    let cmds = std::mem::take(&mut turn_commands);
                    let grouped = if cmds.len() == 1 {
                        cmds.into_iter().next().unwrap()
                    } else {
                        Command::Group(cmds)
                    };
                    editor.run(vec![grouped], cx);
                }
                if let Some(p) = editor.wb_mut().panels.ai_assistant.as_mut() {
                    if p.run_id == run_id {
                        p.streaming = false;
                        p.messages.push(UiMessage {
                            kind: UiMessageKind::Error,
                            text: format!("Stopped after reaching {MAX_TOOL_STEPS} tool steps."),
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn ai_workbook_overview_json(&self) -> String {
        let Some(wb) = self.presentation().workbook.as_ref() else {
            return "{}".to_string();
        };
        let active_slide = self.current_slide;
        let slides = &self.presentation().slides;
        let sheet_for_slide = |slide_id| -> Option<&Sheet> {
            slides.iter().find(|s| s.id == slide_id).and_then(|slide| {
                slide.shapes.iter().find_map(|shape| match &shape.kind {
                    ShapeKind::Sheet(sheet) => Some(sheet.as_ref()),
                    _ => None,
                })
            })
        };
        let overview =
            workbook_overview(wb, active_slide, &sheet_for_slide, self.selected_range());
        let sheets: Vec<_> = overview
            .sheets
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "active": s.active,
                    "used_range": s.used_range,
                    "cell_count": s.cell_count,
                })
            })
            .collect();
        json!({
            "active_sheet": overview.active_sheet,
            "selection": overview.selection,
            "sheets": sheets,
        })
        .to_string()
    }

    fn execute_ai_tool(
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
            "read_range" => match parse_range_arg(args) {
                Err(e) => (json!({ "error": e }).to_string(), None, format!("✗ read_range: {e}")),
                Ok(range) => {
                    let Some((_, sheet)) = self.current_sheet() else {
                        return (
                            json!({ "error": "no active sheet" }).to_string(),
                            None,
                            "✗ read_range: no active sheet".to_string(),
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
            },
            "set_cells" => {
                let slide = self.current_slide;
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
                let slide = self.current_slide;
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
                let slide = self.current_slide;
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
            "select_range" => match parse_range_arg(args) {
                Err(e) => (
                    json!({ "error": e }).to_string(),
                    None,
                    format!("✗ select_range: {e}"),
                ),
                Ok(range) => {
                    let slide = self.current_slide;
                    let wb = self.wb_mut();
                    wb.cells.insert(slide, (range.first_column, range.first_row));
                    if range.first_column != range.last_column || range.first_row != range.last_row
                    {
                        wb.cell_ends
                            .insert(slide, (range.last_column, range.last_row));
                    } else {
                        wb.cell_ends.remove(&slide);
                    }
                    cx.notify();
                    let label = format_range(&range);
                    (
                        json!({ "ok": true, "selection": label }).to_string(),
                        None,
                        format!("✓ select_range ({label})"),
                    )
                }
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
        let panel = self.wb().panels.ai_assistant.as_ref()?;
        let cfg = AiConfig::load();
        let show_settings = panel.show_settings;
        let streaming = panel.streaming;
        let streaming_text = panel.streaming_text.clone();
        let messages = panel.messages.clone();
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
                .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child("OpenAI 互換 API 設定"))
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

        let mut msg_list = v_flex().gap_2();
        if messages.is_empty() && streaming_text.is_empty() {
            msg_list = msg_list.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("セル範囲の集計、数式の作成、書式設定などを指示できます。"),
            );
        }
        for (idx, msg) in messages.iter().enumerate() {
            let (bg, fg) = match msg.kind {
                UiMessageKind::User => (cx.theme().primary.opacity(0.12), cx.theme().foreground),
                UiMessageKind::Assistant => (cx.theme().secondary.opacity(0.5), cx.theme().foreground),
                UiMessageKind::Tool => (cx.theme().secondary.opacity(0.25), cx.theme().muted_foreground),
                UiMessageKind::Error => (cx.theme().danger.opacity(0.15), cx.theme().danger),
            };
            let copy_text = msg.text.clone();
            let show_copy = matches!(msg.kind, UiMessageKind::Error | UiMessageKind::Assistant);
            msg_list = msg_list.child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .rounded_md()
                    .bg(bg)
                    .child(
                        div()
                            .text_xs()
                            .text_color(fg)
                            .child(msg.text.clone()),
                    )
                    .when(show_copy, |box_el| {
                        box_el.child(
                            h_flex().w_full().justify_end().child(
                                Button::new(("ai-assistant-copy", idx))
                                    .ghost()
                                    .xsmall()
                                    .label("コピー")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_text.clone(),
                                        ));
                                    })),
                            ),
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

        let content = v_flex()
            .gap_2()
            .child(header)
            .children(settings_box)
            .child(msg_list);

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

        let body = v_flex()
            .gap_3()
            .child(content)
            .child(footer);

        Some(SidebarPanel {
            title: "Assistant".into(),
            body: body.into_any_element(),
        })
    }
}
