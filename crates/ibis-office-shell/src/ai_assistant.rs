// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Shared AI Assistant sidebar for spreadsheets, presentations, and documents.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_doc_model::EmuRect;
use ibis_doc_model::ai_sheet::{
    audit_sheet_errors, find_sheet_cells, format_range, parse_cell_range, parse_hex_rgb,
    read_sheet_range, resolve_sheet_slide, workbook_overview,
};
use ibis_doc_model::command::Command;
use ibis_doc_model::drawing::{Color, Fill, Geometry};
use ibis_doc_model::flow::{Block, FlowParagraph, FlowRunKind};
use ibis_doc_model::flow_command::{FlowCommand, FlowPos, ParaChange};
use ibis_doc_model::machine_readability::{CandidateKind, candidates as mr_candidates};
use ibis_doc_model::model::{AutoShape, Shape, ShapeId, ShapeKind, Slide, SlideId};
use ibis_doc_model::sd_jwt::{Oid4vpRequest, SdJwtOptions, issue_sd_jwt};
use ibis_doc_model::sd_jwt_binding::{
    BindingSource, bind_range_as_sd_claim_commands, build_sd_jwt_template_commands,
    extract_sheet_claims,
};
use ibis_doc_model::sd_jwt_import::{ParsedCredentialFormat, parse_mdoc_text, parse_sd_jwt};
use ibis_doc_model::sd_jwt_mdoc::{MdocOptions, issue_mdoc};
use ibis_doc_model::sd_jwt_schema::SdJwtSchema;
use ibis_doc_model::sheet_calc::sheet_of_slide;
use ibis_doc_model::sheet_command::SheetCommand;
use ibis_doc_model::text::{TextBody, TextStyle};
use serde_json::json;

use crate::with_format;

use crate::ai_client::{AiConfig, BrowserModel, ChatMessage, ToolCall, stream_chat_completion};
#[cfg(not(target_family = "wasm"))]
use crate::ai_client::ToolDefinition;
#[cfg(target_family = "wasm")]
use crate::ai_client::{
    abort_browser_stream, browser_csrf, explain_ai_error, fit_browser_messages, load_browser_catalog, set_browser_csrf,
    utf16_units,
};
use crate::ai_tools::{
    AddSlideArgs, ApplyVcTemplateArgs, BindVcClaimArgs, FindCellsArgs, FindTextArgs,
    InferVcSchemaArgs, InsertTableArgs, InsertTextArgs, InsertTextBoxArgs, IssueVcPreviewArgs,
    FetchEgovLawArgs, GetVcPublicKeysArgs, GenerateLawtext5SetArgs, LintLawtextArgs, MAX_READ_CELLS,
    MoveSlideArgs, OpenMarkdownDocumentArgs, ParseVcTokenArgs, PrepareOid4vpArgs, ReadParagraphsArgs,
    VerifyLawtextAratameArgs, claim_value_to_json, resolve_vc_holder_key,
    resolve_vc_issuer_key, vc_public_keys_json,
    ReplaceParagraphArgs, SetParagraphStyleArgs, SetRunFormatArgs, SetShapeStyleArgs,
    SetShapeTextArgs, SetSlideNotesArgs, SlideIndexOptionalArg, SlideIndexRequiredArg,
    build_replace_paragraph_command, build_set_run_format_command,
    parse_clear_cells_command, parse_create_sheet_arg, parse_delete_axis_command,
    parse_insert_axis_command, parse_merge_cells_command, parse_range_arg,
    parse_set_axis_size_command, parse_set_cell_comment_command, parse_set_cells_command,
    parse_set_format_command, parse_set_paragraph_align_command, parse_sheet_arg,
    parse_switch_sheet_arg, parse_vc_template_kind,
};
#[cfg(not(target_family = "wasm"))]
use crate::ai_tools::{docx_tool_definitions, pptx_tool_definitions, spreadsheet_tool_definitions};
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
pub struct AiReplacementTarget {
    pub flow: std::sync::Weak<ibis_doc_model::flow::FlowDocument>,
    pub range: (FlowPos, FlowPos),
    pub before: String,
}

impl AiReplacementTarget {
    pub fn matches(&self, other: &Self) -> bool {
        self.flow.ptr_eq(&other.flow) && self.range == other.range && self.before == other.before
    }
}

#[derive(Clone, Debug)]
pub struct UiMessage {
    pub kind: UiMessageKind,
    pub text: String,
    pub has_edits: bool,
    #[cfg(target_family = "wasm")]
    replacement: Option<AiReplacementTarget>,
    #[cfg(target_family = "wasm")]
    preview_replacement: bool,
}

impl UiMessage {
    fn new(kind: UiMessageKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            has_edits: false,
            #[cfg(target_family = "wasm")]
            replacement: None,
            #[cfg(target_family = "wasm")]
            preview_replacement: false,
        }
    }
}

pub struct AiAssistantState {
    pub prompt_input: Entity<TextareaState>,
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
    pub pending_old_lawtext: Option<(String, String)>,
    pub _prompt_subscription: Subscription,
    /// Allowlist from `GET /api/ai/models`. Desktop leaves this empty.
    #[allow(dead_code)]
    pub browser_models: Vec<BrowserModel>,
    /// Login or catalog error for the browser pane. Desktop leaves this empty.
    #[allow(dead_code)]
    pub browser_status: String,
    /// The open document is over the selected model's cap, and the user has not chosen a range.
    #[allow(dead_code)]
    browser_pending: Option<BrowserPendingSend>,
}

/// A browser send that is waiting for the user to choose which text to include.
#[derive(Clone, Debug)]
#[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
struct BrowserPendingSend {
    prompt: String,
    budget: usize,
    prefix_ok: bool,
    selection_fits: bool,
    selection_over: bool,
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

pub(crate) fn parse_law_input_str(text: &str) -> Result<ibis_pptx::lawtext::LawDoc, String> {
    if ibis_pptx::lawtext::is_law_xml(text.as_bytes()) {
        ibis_pptx::lawtext::parse_law_xml(text).map_err(|e| e.to_string())
    } else if text.trim_start().starts_with("---") && text.contains("type: lawtext") {
        ibis_pptx::lawtext::parse_markdown_law(text).map_err(|e| e.to_string())
    } else {
        ibis_pptx::lawtext::parse_lawtext(text).map_err(|e| e.to_string())
    }
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

#[cfg(target_family = "wasm")]
const BROWSER_DOCUMENT_MARKER: &str = "\n\n[文書]\n";

#[cfg(target_family = "wasm")]
enum BrowserScope {
    Document,
    Selection,
}

#[cfg(target_family = "wasm")]
struct TextBudget {
    text: String,
    units: usize,
    limit: usize,
    overflow: bool,
}

#[cfg(target_family = "wasm")]
impl TextBudget {
    fn new(limit: usize) -> Self {
        Self { text: String::new(), units: 0, limit, overflow: false }
    }

    fn push_char(&mut self, ch: char) {
        if self.overflow {
            return;
        }
        let next = self.units.saturating_add(ch.len_utf16());
        if next > self.limit {
            self.overflow = true;
            return;
        }
        self.text.push(ch);
        self.units = next;
    }

    fn push_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.push_char(ch);
            if self.overflow {
                return;
            }
        }
    }
}

#[cfg(target_family = "wasm")]
fn push_flow_table(table: &ibis_doc_model::flow::FlowTable, depth: usize, out: &mut TextBudget) {
    if depth > 8 || out.overflow {
        return;
    }
    for row in &table.rows {
        if out.overflow {
            return;
        }
        for (index, cell) in row.cells.iter().enumerate() {
            if index > 0 {
                out.push_char('\t');
            }
            push_cell_text(&cell.blocks, depth + 1, out);
        }
        out.push_char('\n');
    }
}

#[cfg(target_family = "wasm")]
fn push_cell_text(blocks: &[Block], depth: usize, out: &mut TextBudget) {
    if depth > 8 || out.overflow {
        return;
    }
    let mut first = true;
    for block in blocks {
        if out.overflow {
            return;
        }
        match block {
            Block::Paragraph(paragraph) => {
                if !first {
                    out.push_char(' ');
                }
                first = false;
                out.push_str(&paragraph_plain_text(paragraph));
            }
            Block::Table(table) => {
                out.push_char('\n');
                push_flow_table(table, depth + 1, out);
            }
        }
    }
}

#[cfg(target_family = "wasm")]
fn push_flow_blocks(blocks: &[Block], depth: usize, out: &mut TextBudget) {
    if depth > 8 || out.overflow {
        return;
    }
    for block in blocks {
        if out.overflow {
            return;
        }
        match block {
            Block::Paragraph(paragraph) => {
                out.push_str(&paragraph_plain_text(paragraph));
                out.push_char('\n');
            }
            Block::Table(table) => push_flow_table(table, depth + 1, out),
        }
    }
}

#[cfg(target_family = "wasm")]
fn push_shape_body(shape: &Shape, out: &mut TextBudget) {
    let Some(body) = shape.text_body() else { return };
    for paragraph in &body.paragraphs {
        if out.overflow {
            return;
        }
        let text = paragraph.text();
        if text.is_empty() {
            continue;
        }
        out.push_str(&text);
        out.push_char('\n');
    }
}

#[cfg(target_family = "wasm")]
fn document_room(prompt: &str, budget: usize) -> usize {
    let overhead = utf16_units(prompt) + utf16_units(BROWSER_DOCUMENT_MARKER) + utf16_units("\n…");
    budget.saturating_sub(overhead)
}

#[cfg(target_family = "wasm")]
fn wrap_browser_user(prompt: &str, document: &str, truncated: bool) -> String {
    if document.is_empty() {
        return prompt.to_string();
    }
    let mut out = String::with_capacity(prompt.len() + document.len() + BROWSER_DOCUMENT_MARKER.len() + 2);
    out.push_str(prompt);
    out.push_str(BROWSER_DOCUMENT_MARKER);
    out.push_str(document);
    if truncated {
        out.push_str("\n…");
    }
    out
}

#[cfg(target_family = "wasm")]
fn open_browser_login() {
    use wasm_bindgen::JsCast as _;
    let global = js_sys::global();
    let Ok(location) = js_sys::Reflect::get(&global, &wasm_bindgen::JsValue::from_str("location")) else { return };
    let Ok(assign) = js_sys::Reflect::get(&location, &wasm_bindgen::JsValue::from_str("assign")) else { return };
    let Ok(assign) = assign.dyn_into::<js_sys::Function>() else { return };
    let _ = assign.call1(&location, &wasm_bindgen::JsValue::from_str("/files"));
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
            let prompt_input = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(2, 8)
                    .submit_on_enter(true)
                    .placeholder("指示を入力（Enterで送信、Shift+Enterで改行）")
            });
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
                pending_old_lawtext: None,
                _prompt_subscription,
                browser_models: Vec::new(),
                browser_status: String::new(),
                browser_pending: None,
            });
            prompt_input.update(cx, |input, cx| input.focus(window, cx));
        }
        #[cfg(target_family = "wasm")]
        if browser_csrf().is_empty() {
            self.load_ai_browser_catalog(cx);
        }
        self.sidebar_open = true;
        cx.notify();
    }

    pub fn close_ai_assistant_panel(&mut self, cx: &mut Context<Self>) {
        #[cfg(target_family = "wasm")]
        abort_browser_stream();
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

    #[cfg(not(target_family = "wasm"))]
    fn pick_old_lawtext_for_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            extensions: vec![
                "xml".to_string(),
                "lawtext".to_string(),
                "txt".to_string(),
                "md".to_string(),
            ],
            multiple: false,
            prompt: Some("比較対象の現行法令 (.xml / .lawtext / .md) を選択".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                return;
            };
            let Ok(text) = String::from_utf8(bytes) else {
                return;
            };
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("現行法令")
                .to_string();
            let _ = this.update(cx, |editor, cx| {
                if let Err(err) = parse_law_input_str(&text) {
                    if let Some(p) = editor.ai_assistant.as_mut() {
                        p.messages.push(UiMessage::new(
                            UiMessageKind::Error,
                            format!("現行法令「{file_name}」の解析に失敗しました: {err}"),
                        ));
                    }
                    cx.notify();
                    return;
                }
                if let Some(p) = editor.ai_assistant.as_mut() {
                    p.pending_old_lawtext = Some((file_name.clone(), text));
                }
                editor.submit_ai_assistant_text(
                    format!(
                        "現行法令「{file_name}」を読み込みました。現在開いている文書（改正後）と比較して、generate_lawtext_5set と lint_lawtext を実行し、新旧対照表・改め文（検算付き）・参照条文・要綱案・理由案を作成してください。"
                    ),
                    cx,
                );
            });
        })
        .detach();
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
        #[cfg(target_family = "wasm")]
        abort_browser_stream();
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
        #[cfg(target_family = "wasm")]
        abort_browser_stream();
        if let Some(p) = self.ai_assistant.as_mut() {
            p.streaming = false;
            p.run_id = p.run_id.wrapping_add(1);
            p.streaming_text.clear();
            p.messages.clear();
            p.history.clear();
            p.browser_pending = None;
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
        #[cfg(target_family = "wasm")]
        self.submit_browser_assistant(prompt, false, cx);
        #[cfg(not(target_family = "wasm"))]
        self.submit_desktop_assistant(prompt, cx);
    }

    #[cfg(not(target_family = "wasm"))]
    fn submit_desktop_assistant(&mut self, prompt: String, cx: &mut Context<Self>) {
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
                         For SD-JWT Verifiable Credentials (VC) tasks, use inspect_vc_bindings, apply_vc_template, infer_vc_schema, bind_vc_claim, issue_vc_preview, parse_vc_token, and prepare_oid4vp_presentation, and clearly explain mandatory claims (claim.*, e.g. vct, iss) vs selective-disclosure claims (sd.* with disclose.* toggles) and OID4VP Key Binding (aud/nonce). \
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
                         For Japanese legislative drafting (法制執務 / Lawtext / 5点セット) tasks, use export_active_lawtext, generate_lawtext_5set, verify_lawtext_aratame, and lint_lawtext to produce 新旧対照表, 改め文, 参照条文, and draft 要綱/理由, always verifying any polished 改め文 with verify_lawtext_aratame and checking reference/conjunction issues with lint_lawtext, and use open_markdown_document when the user wants to open any generated document in a new ibisOffice window. \
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

    #[cfg(target_family = "wasm")]
    fn load_ai_browser_catalog(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let catalog = load_browser_catalog().await;
            let _ = this.update(cx, |editor, cx| editor.apply_browser_catalog(catalog, cx));
        })
        .detach();
    }

    #[cfg(target_family = "wasm")]
    fn apply_browser_catalog(&mut self, catalog: Result<crate::ai_client::BrowserCatalog, String>, cx: &mut Context<Self>) {
        let Some(panel) = self.ai_assistant.as_mut() else { return };
        match catalog {
            Ok(catalog) => {
                set_browser_csrf(&catalog.csrf);
                panel.browser_status.clear();
                panel.browser_models = catalog.models;
                let current = AiConfig::load().model;
                let known = panel.browser_models.iter().any(|model| model.key == current);
                if !known {
                    let fallback = panel
                        .browser_models
                        .iter()
                        .find(|model| model.key == "deepseek-flash")
                        .or_else(|| panel.browser_models.first());
                    if let Some(model) = fallback {
                        crate::settings::set(crate::ai_client::SETTING_MODEL, &model.key);
                    }
                }
            }
            Err(err) => {
                set_browser_csrf("");
                let message = explain_ai_error(&err);
                panel.browser_status = if message.is_empty() { err } else { message };
            }
        }
        cx.notify();
    }

    #[cfg(target_family = "wasm")]
    fn select_browser_model(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        let allowed = self
            .ai_assistant
            .as_ref()
            .is_some_and(|panel| panel.browser_models.iter().any(|model| model.key == key));
        if !allowed {
            return;
        }
        crate::settings::set(crate::ai_client::SETTING_MODEL, &key);
        let prompt = self.ai_assistant.as_mut().and_then(|panel| panel.browser_pending.take()).map(|pending| pending.prompt);
        if let Some(panel) = self.ai_assistant.as_ref() {
            panel.model_input.update(cx, |input, cx| input.set_value(key, window, cx));
            if let Some(prompt) = prompt {
                panel.prompt_input.update(cx, |input, cx| input.set_value(prompt, window, cx));
            }
        }
        cx.notify();
    }

    #[cfg(target_family = "wasm")]
    fn browser_char_budget(&self) -> usize {
        let key = AiConfig::load().model;
        self.ai_assistant
            .as_ref()
            .and_then(|panel| panel.browser_models.iter().find(|model| model.key == key))
            .map(|model| model.max_input_chars)
            .unwrap_or(8_000)
    }

    #[cfg(target_family = "wasm")]
    fn push_ai_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(panel) = self.ai_assistant.as_mut() {
            panel.messages.push(UiMessage::new(UiMessageKind::Error, message));
            panel.scroll_handle.scroll_to_bottom();
        }
        cx.notify();
    }

    #[cfg(target_family = "wasm")]
    fn submit_browser_assistant(&mut self, prompt: String, rewrite_selection: bool, cx: &mut Context<Self>) {
        let Some(panel) = self.ai_assistant.as_ref() else { return };
        if panel.streaming {
            return;
        }
        if browser_csrf().is_empty() {
            let message = if panel.browser_status.is_empty() {
                "ログイン状態を確認しています。少し待ってからもう一度試してください。".to_string()
            } else {
                panel.browser_status.clone()
            };
            self.push_ai_error(message, cx);
            return;
        }
        let model_known = panel.browser_models.iter().any(|model| model.key == AiConfig::load().model);
        if !model_known {
            self.push_ai_error(explain_ai_error("invalid_model"), cx);
            return;
        }
        let budget = self.browser_char_budget();
        if utf16_units(&prompt) > budget {
            if let Some(panel) = self.ai_assistant.as_mut() {
                panel.browser_pending = Some(BrowserPendingSend {
                    prompt,
                    budget,
                    prefix_ok: false,
                    selection_fits: false,
                    selection_over: false,
                });
            }
            cx.notify();
            return;
        }
        let room = document_room(&prompt, budget);
        if rewrite_selection {
            let Some(target) = with_format!(self, ai_replacement_target(self)) else {
                self.push_ai_error("本文の1段落内で、置き換える文字を選択してください。".into(), cx);
                return;
            };
            if utf16_units(&target.before) > room {
                self.push_ai_error("選択範囲が長すぎます。範囲を狭めてください。".into(), cx);
                return;
            }
            self.begin_browser_turn(prompt, target.before.clone(), false, true, Some(target), budget, cx);
            return;
        }
        let document = self.browser_extract(BrowserScope::Document, room);
        if document.overflow {
            let selection = self.browser_extract(BrowserScope::Selection, room);
            let pending = BrowserPendingSend {
                prompt,
                budget,
                prefix_ok: room >= 32,
                selection_fits: !selection.overflow && !selection.text.is_empty(),
                selection_over: selection.overflow,
            };
            if let Some(panel) = self.ai_assistant.as_mut() {
                panel.browser_pending = Some(pending);
            }
            cx.notify();
            return;
        }
        self.begin_browser_turn(prompt, document.text, false, false, None, budget, cx);
    }

    #[cfg(target_family = "wasm")]
    fn confirm_browser_prefix(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.ai_assistant.as_ref().and_then(|panel| panel.browser_pending.clone()) else { return };
        if !pending.prefix_ok {
            return;
        }
        let room = document_room(&pending.prompt, pending.budget);
        let document = self.browser_extract(BrowserScope::Document, room);
        self.begin_browser_turn(pending.prompt, document.text, document.overflow, false, None, pending.budget, cx);
    }

    #[cfg(target_family = "wasm")]
    fn confirm_browser_selection(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.ai_assistant.as_ref().and_then(|panel| panel.browser_pending.clone()) else { return };
        let room = document_room(&pending.prompt, pending.budget);
        let selection = self.browser_extract(BrowserScope::Selection, room);
        if selection.overflow || selection.text.is_empty() {
            if let Some(panel) = self.ai_assistant.as_mut()
                && let Some(stored) = panel.browser_pending.as_mut()
            {
                stored.selection_fits = false;
                stored.selection_over = selection.overflow;
            }
            cx.notify();
            return;
        }
        self.begin_browser_turn(pending.prompt, selection.text, false, true, None, pending.budget, cx);
    }

    #[cfg(target_family = "wasm")]
    fn restore_browser_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.ai_assistant.as_mut().and_then(|panel| panel.browser_pending.take()).map(|pending| pending.prompt) else {
            return;
        };
        if let Some(panel) = self.ai_assistant.as_ref() {
            panel.prompt_input.update(cx, |input, cx| input.set_value(prompt, window, cx));
        }
        cx.notify();
    }

    #[cfg(target_family = "wasm")]
    fn begin_browser_turn(
        &mut self,
        prompt: String,
        document: String,
        truncated: bool,
        selection: bool,
        replacement: Option<AiReplacementTarget>,
        budget: usize,
        cx: &mut Context<Self>,
    ) {
        let latest = wrap_browser_user(&prompt, &document, truncated);
        let rewrite_selection = replacement.is_some();
        let prior = if rewrite_selection {
            Vec::new()
        } else {
            self.ai_assistant.as_ref().map(|panel| panel.history.clone()).unwrap_or_default()
        };
        let Ok(outbound) = fit_browser_messages(&prior, latest, budget) else {
            self.push_ai_error(explain_ai_error("too_long"), cx);
            return;
        };
        let note = if truncated {
            "\n\n（文書の先頭から、上限に収まる範囲を送りました）"
        } else if selection {
            "\n\n（選択範囲を送りました）"
        } else {
            ""
        };
        let shown = format!("{prompt}{note}");
        let cfg = AiConfig::load();
        let run_id = {
            let Some(panel) = self.ai_assistant.as_mut() else { return };
            panel.browser_pending = None;
            panel.run_id = panel.run_id.wrapping_add(1);
            panel.streaming = true;
            panel.streaming_text.clear();
            panel.messages.push(UiMessage::new(UiMessageKind::User, shown));
            if !rewrite_selection {
                panel.history.push(ChatMessage::user(prompt));
            }
            panel.scroll_handle.scroll_to_bottom();
            panel.run_id
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = stream_chat_completion(&cfg, &outbound, Vec::new(), |delta| {
                let delta_owned = delta.to_string();
                let _ = this.update(cx, |editor, cx| {
                    if let Some(panel) = editor.ai_assistant.as_mut()
                        && panel.streaming
                        && panel.run_id == run_id
                    {
                        panel.streaming_text.push_str(&delta_owned);
                        panel.scroll_handle.scroll_to_bottom();
                        cx.notify();
                    }
                });
            })
            .await;
            match outcome {
                Err(err) => {
                    let message = explain_ai_error(&err);
                    let _ = this.update(cx, |editor, cx| {
                        let Some(panel) = editor.ai_assistant.as_mut() else { return };
                        if panel.run_id != run_id {
                            return;
                        }
                        panel.streaming = false;
                        if !panel.streaming_text.is_empty() {
                            let text = std::mem::take(&mut panel.streaming_text);
                            panel.messages.push(UiMessage::new(UiMessageKind::Assistant, text));
                        }
                        if !message.is_empty() {
                            panel.messages.push(UiMessage::new(UiMessageKind::Error, message));
                        }
                        panel.scroll_handle.scroll_to_bottom();
                        cx.notify();
                    });
                }
                Ok(out) => {
                    let _ = this.update(cx, |editor, cx| {
                        let Some(panel) = editor.ai_assistant.as_mut() else { return };
                        if !panel.streaming || panel.run_id != run_id {
                            return;
                        }
                        panel.streaming = false;
                        panel.streaming_text.clear();
                        let empty = out.content.is_empty();
                        let complete = match out.finish_reason.as_deref() {
                            Some(reason) => reason == "stop",
                            None => out.received_done,
                        };
                        let text = if empty {
                            "応答に文章がありませんでした。".to_string()
                        } else {
                            out.content
                        };
                        if !rewrite_selection {
                            panel.history.push(ChatMessage::assistant(Some(text.clone()), Vec::new()));
                        }
                        let mut message = UiMessage::new(UiMessageKind::Assistant, text);
                        message.replacement = replacement.filter(|target| {
                            complete && !empty && !message.text.trim().is_empty() && target.before != message.text
                        });
                        panel.messages.push(message);
                        if rewrite_selection && !complete && !empty {
                            panel.messages.push(UiMessage::new(
                                UiMessageKind::Error,
                                "応答が途中で終了しました。選択範囲の書き換え案をもう一度生成してください。",
                            ));
                        }
                        panel.scroll_handle.scroll_to_bottom();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    #[cfg(target_family = "wasm")]
    fn set_ai_replacement_preview(&mut self, index: usize, visible: bool, cx: &mut Context<Self>) {
        if let Some(message) = self.ai_assistant.as_mut().and_then(|panel| panel.messages.get_mut(index)) {
            message.preview_replacement = visible;
            cx.notify();
        }
    }

    #[cfg(target_family = "wasm")]
    fn apply_browser_replacement(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(message) = self.ai_assistant.as_ref().and_then(|panel| panel.messages.get(index)) else { return };
        let Some(target) = message.replacement.clone().filter(|_| message.preview_replacement) else { return };
        let text = message.text.clone();
        let applied = with_format!(self, apply_ai_replacement(self, &target, &text, cx));
        if let Some(message) = self.ai_assistant.as_mut().and_then(|panel| panel.messages.get_mut(index)) {
            message.replacement = None;
            message.preview_replacement = false;
        }
        if applied {
            self.focus_handle.focus(window, cx);
            self.set_status("選択範囲を置き換えました。".into(), cx);
        } else {
            self.push_ai_error("文書または選択範囲が変わったか、編集できない状態です。範囲を選び直して書き換え案を作ってください。".into(), cx);
        }
        cx.notify();
    }

    #[cfg(target_family = "wasm")]
    fn browser_extract(&self, scope: BrowserScope, limit: usize) -> TextBudget {
        let mut out = TextBudget::new(limit);
        match (scope, self.ai_doc_kind()) {
            (BrowserScope::Selection, AiDocKind::Docx) => {
                if let Some(text) = with_format!(self, ai_selection_plain_text(self)) {
                    out.push_str(&text);
                }
            }
            (BrowserScope::Selection, AiDocKind::Pptx) => {
                for shape in self.selected_shapes() {
                    push_shape_body(shape, &mut out);
                }
            }
            (BrowserScope::Selection, AiDocKind::Xlsx) => {
                let range = with_format!(self, ai_sheet_selection(self, self.current_slide));
                if let Some(range) = range {
                    self.push_sheet_text(Some(range), &mut out);
                }
            }
            (BrowserScope::Document, AiDocKind::Docx) => {
                if let Some(flow) = self.presentation().flow.as_ref() {
                    push_flow_blocks(&flow.body, 0, &mut out);
                }
            }
            (BrowserScope::Document, AiDocKind::Pptx) => self.push_slide_text(&mut out),
            (BrowserScope::Document, AiDocKind::Xlsx) => self.push_sheet_text(None, &mut out),
        }
        out
    }

    #[cfg(target_family = "wasm")]
    fn push_slide_text(&self, out: &mut TextBudget) {
        let current = self.current_slide;
        let presentation = self.presentation();
        let mut ordered: Vec<(usize, &ibis_doc_model::model::Slide)> =
            presentation.slides.iter().enumerate().map(|(index, slide)| (index + 1, slide)).collect();
        ordered.sort_by_key(|(index, slide)| (slide.id != current, *index));
        for (index, slide) in ordered {
            if out.overflow {
                return;
            }
            out.push_str(&format!("スライド {index}\n"));
            for shape in &slide.shapes {
                push_shape_body(shape, out);
            }
            let notes = slide.notes.trim();
            if !notes.is_empty() {
                out.push_str("ノート\n");
                out.push_str(notes);
                out.push_char('\n');
            }
        }
    }

    #[cfg(target_family = "wasm")]
    fn push_sheet_text(&self, only: Option<ibis_doc_model::sheet::CellRange>, out: &mut TextBudget) {
        let current = self.current_slide;
        let presentation = self.presentation();
        let mut ordered: Vec<(usize, &ibis_doc_model::model::Slide)> =
            presentation.slides.iter().enumerate().map(|(index, slide)| (index + 1, slide)).collect();
        if only.is_some() {
            ordered.retain(|(_, slide)| slide.id == current);
        } else {
            ordered.sort_by_key(|(index, slide)| (slide.id != current, *index));
        }
        for (index, slide) in ordered {
            if out.overflow {
                return;
            }
            let Some(sheet) = sheet_of_slide(slide) else { continue };
            let name = slide.name.clone().unwrap_or_else(|| format!("Sheet {index}"));
            out.push_str(&name);
            out.push_char('\n');
            for cell in sheet.cells_in_rows(0, u32::MAX) {
                if out.overflow {
                    return;
                }
                if let Some(range) = only
                    && (cell.row < range.first_row
                        || cell.row > range.last_row
                        || cell.column < range.first_column
                        || cell.column > range.last_column)
                {
                    continue;
                }
                if cell.text.trim().is_empty() {
                    continue;
                }
                out.push_str(&ibis_doc_model::sheet::cell_name(cell.column, cell.row));
                out.push_char('\t');
                out.push_str(&cell.text);
                out.push_char('\n');
            }
        }
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
            AiDocKind::Docx => self.execute_docx_tool(call, cx),
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
            "find_cells" => {
                let parsed: Result<FindCellsArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ find_cells: {e}"),
                    ),
                    Ok(a) => {
                        let slide = match self.resolve_ai_target_slide(args) {
                            Ok(s) => s,
                            Err(e) => {
                                return (
                                    json!({ "error": e }).to_string(),
                                    None,
                                    format!("✗ find_cells: {e}"),
                                );
                            }
                        };
                        let Some(sheet) = self.presentation().slide(slide).and_then(sheet_of_slide)
                        else {
                            return (
                                json!({ "error": "target sheet not found" }).to_string(),
                                None,
                                "✗ find_cells: target sheet not found".to_string(),
                            );
                        };
                        let matches: Vec<_> = find_sheet_cells(
                            sheet,
                            &a.query,
                            a.search_formulas,
                            MAX_READ_CELLS,
                        )
                        .into_iter()
                        .map(|c| {
                            json!({
                                "address": c.cell,
                                "value": c.text,
                                "formula": c.formula,
                            })
                        })
                        .collect();
                        let count = matches.len();
                        (
                            json!({ "count": count, "matches": matches }).to_string(),
                            None,
                            format!("✓ find_cells ({count} matches)"),
                        )
                    }
                }
            }
            "audit_formulas" => {
                let filter_sheet = match parse_sheet_arg(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ audit_formulas: {e}"),
                        );
                    }
                };
                let mut errors = Vec::new();
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
                    for c in audit_sheet_errors(sheet, MAX_READ_CELLS) {
                        errors.push(json!({
                            "sheet": sheet_name,
                            "address": c.cell,
                            "value": c.text,
                            "formula": c.formula,
                        }));
                    }
                }
                let count = errors.len();
                (
                    json!({ "count": count, "errors": errors }).to_string(),
                    None,
                    format!("✓ audit_formulas ({count} errors)"),
                )
            }
            "set_axis_size" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ set_axis_size: {e}"),
                        );
                    }
                };
                match parse_set_axis_size_command(slide, args) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ set_axis_size: {e}"),
                    ),
                    Ok(cmd) => (
                        json!({ "ok": true }).to_string(),
                        Some(cmd),
                        "✓ set_axis_size".to_string(),
                    ),
                }
            }
            "set_cell_comment" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ set_cell_comment: {e}"),
                        );
                    }
                };
                match parse_set_cell_comment_command(slide, args) {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ set_cell_comment: {e}"),
                    ),
                    Ok(cmd) => (
                        json!({ "ok": true }).to_string(),
                        Some(cmd),
                        "✓ set_cell_comment".to_string(),
                    ),
                }
            }
            "inspect_vc_bindings" => {
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ inspect_vc_bindings: {e}"),
                        );
                    }
                };
                match extract_sheet_claims(self.presentation(), slide) {
                    None => (
                        json!({
                            "bound": false,
                            "claims": [],
                            "disclosures": [],
                            "hint": "No SD-JWT bindings found. Use apply_vc_template, infer_vc_schema, or bind_vc_claim to set up VC bindings."
                        })
                        .to_string(),
                        None,
                        "✓ inspect_vc_bindings (0 claims)".to_string(),
                    ),
                    Some(bindings) => {
                        let claims: Vec<_> = bindings
                            .claims
                            .iter()
                            .map(|c| {
                                let source = match c.source {
                                    BindingSource::Cell { row, column } => {
                                        format!("cell:{}{}", column_letters(column), row + 1)
                                    }
                                    BindingSource::Control { shape } => {
                                        format!("control:{}", shape.0)
                                    }
                                };
                                json!({
                                    "path": c.path_str,
                                    "selective": c.selective,
                                    "disclose": c.disclose,
                                    "value": claim_value_to_json(&c.value),
                                    "source": source,
                                })
                            })
                            .collect();
                        let count = claims.len();
                        let has_vct = bindings.claims.iter().any(|c| c.path_str == "vct");
                        let has_iss = bindings.claims.iter().any(|c| c.path_str == "iss");
                        (
                            json!({
                                "bound": true,
                                "claim_count": count,
                                "has_vct": has_vct,
                                "has_iss": has_iss,
                                "claims": claims,
                            })
                            .to_string(),
                            None,
                            format!("✓ inspect_vc_bindings ({count} claims)"),
                        )
                    }
                }
            }
            "apply_vc_template" => {
                let parsed: Result<ApplyVcTemplateArgs, _> = serde_json::from_str(args);
                let kind = match parsed.map_err(|e| format!("invalid arguments: {e}")).and_then(|a| {
                    parse_vc_template_kind(&a.template).map(|k| (a.template, k))
                }) {
                    Ok(v) => v,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ apply_vc_template: {e}"),
                        );
                    }
                };
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ apply_vc_template: {e}"),
                        );
                    }
                };
                let cmds = build_sd_jwt_template_commands(self.presentation(), slide, kind.1);
                (
                    json!({ "ok": true, "template": kind.0, "command_count": cmds.len() })
                        .to_string(),
                    Some(Command::Group(cmds)),
                    format!("✓ apply_vc_template ({})", kind.0),
                )
            }
            "infer_vc_schema" => {
                let parsed: Result<InferVcSchemaArgs, _> = if args.trim().is_empty() {
                    Ok(InferVcSchemaArgs {
                        vct: None,
                        apply: false,
                    })
                } else {
                    serde_json::from_str(args)
                };
                let arg = match parsed {
                    Ok(a) => a,
                    Err(e) => {
                        return (
                            json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                            None,
                            format!("✗ infer_vc_schema: {e}"),
                        );
                    }
                };
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ infer_vc_schema: {e}"),
                        );
                    }
                };
                match SdJwtSchema::infer_from_sheet(self.presentation(), slide, arg.vct.as_deref())
                {
                    None => (
                        json!({ "error": "could not infer VC schema from this worksheet (ensure key-value labels/values or tables exist)" }).to_string(),
                        None,
                        "✗ infer_vc_schema: no inferable layout".to_string(),
                    ),
                    Some(schema) => {
                        let schema_json = schema.to_json_pretty();
                        let claim_count = schema.claims.len();
                        let cmd = if arg.apply {
                            let cmds = schema.build_sheet_commands(self.presentation(), slide);
                            Some(Command::Group(cmds))
                        } else {
                            None
                        };
                        (
                            json!({
                                "ok": true,
                                "applied": arg.apply,
                                "claim_count": claim_count,
                                "schema_json": schema_json,
                            })
                            .to_string(),
                            cmd,
                            format!("✓ infer_vc_schema ({claim_count} claims, applied={})", arg.apply),
                        )
                    }
                }
            }
            "bind_vc_claim" => {
                let parsed: Result<BindVcClaimArgs, _> = serde_json::from_str(args);
                let arg = match parsed {
                    Ok(a) => a,
                    Err(e) => {
                        return (
                            json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                            None,
                            format!("✗ bind_vc_claim: {e}"),
                        );
                    }
                };
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ bind_vc_claim: {e}"),
                        );
                    }
                };
                let Some(range) = parse_cell_range(&arg.range) else {
                    return (
                        json!({ "error": format!("invalid cell range: {}", arg.range) }).to_string(),
                        None,
                        format!("✗ bind_vc_claim: invalid range {}", arg.range),
                    );
                };
                match bind_range_as_sd_claim_commands(
                    self.presentation(),
                    slide,
                    range,
                    &arg.claim_path,
                    arg.selective,
                ) {
                    None => (
                        json!({ "error": "invalid claim_path" }).to_string(),
                        None,
                        "✗ bind_vc_claim: invalid claim_path".to_string(),
                    ),
                    Some((full_name, cmds)) => (
                        json!({ "ok": true, "binding": full_name, "selective": arg.selective })
                            .to_string(),
                        Some(Command::Group(cmds)),
                        format!("✓ bind_vc_claim ({full_name})"),
                    ),
                }
            }
            "issue_vc_preview" => {
                let parsed: Result<IssueVcPreviewArgs, _> = if args.trim().is_empty() {
                    Ok(IssueVcPreviewArgs {
                        format: None,
                        disclose_paths: None,
                        vct: None,
                        iss: None,
                        name_space: None,
                    })
                } else {
                    serde_json::from_str(args)
                };
                let arg = match parsed {
                    Ok(a) => a,
                    Err(e) => {
                        return (
                            json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                            None,
                            format!("✗ issue_vc_preview: {e}"),
                        );
                    }
                };
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ issue_vc_preview: {e}"),
                        );
                    }
                };
                let Some(mut bindings) = extract_sheet_claims(self.presentation(), slide) else {
                    return (
                        json!({ "error": "no SD-JWT bindings found on this worksheet" }).to_string(),
                        None,
                        "✗ issue_vc_preview: no bindings".to_string(),
                    );
                };
                if let Some(ref paths) = arg.disclose_paths {
                    let wanted: std::collections::BTreeSet<&str> =
                        paths.iter().map(String::as_str).collect();
                    for claim in &mut bindings.claims {
                        if claim.selective {
                            claim.disclose = wanted.contains(claim.path_str.as_str());
                        }
                    }
                }
                let disclosed_paths: Vec<_> = bindings
                    .claims
                    .iter()
                    .filter(|c| !c.selective || c.disclose)
                    .map(|c| c.path_str.clone())
                    .collect();
                let hidden_paths: Vec<_> = bindings
                    .claims
                    .iter()
                    .filter(|c| c.selective && !c.disclose)
                    .map(|c| c.path_str.clone())
                    .collect();

                let format = arg
                    .format
                    .as_deref()
                    .unwrap_or("sd-jwt")
                    .trim()
                    .to_ascii_lowercase();
                if format == "mdoc" {
                    let mut mdoc_opts = MdocOptions::default();
                    if let Some(vct) = arg.vct.filter(|s| !s.trim().is_empty()) {
                        mdoc_opts.doc_type = vct;
                    }
                    if let Some(ns) = arg.name_space.filter(|s| !s.trim().is_empty()) {
                        mdoc_opts.name_space = ns;
                    }
                    let (issuer_key, _) = resolve_vc_issuer_key();
                    let mdoc = issue_mdoc(&bindings, &mdoc_opts, &issuer_key);
                    return (
                        json!({
                            "ok": true,
                            "format": "mdoc",
                            "doc_type": mdoc.doc_type,
                            "name_space": mdoc.name_space,
                            "total_items": mdoc.total_items,
                            "disclosed_items": mdoc.disclosed_items,
                            "disclosed_paths": disclosed_paths,
                            "hidden_paths": hidden_paths,
                            "mdoc_base64url_presentation": mdoc.base64url_presentation,
                            "mdoc_hex_presentation": mdoc.hex_presentation,
                            "mdoc_base64url_all_length": mdoc.base64url_all.len(),
                        })
                        .to_string(),
                        None,
                        format!("✓ issue_vc_preview (mdoc, {}/{} items)", mdoc.disclosed_items, mdoc.total_items),
                    );
                }

                let mut options = SdJwtOptions::default();
                if let Some(vct) = arg.vct.filter(|s| !s.trim().is_empty()) {
                    options.default_vct = Some(vct);
                }
                if let Some(iss) = arg.iss.filter(|s| !s.trim().is_empty()) {
                    options.default_iss = Some(iss);
                }
                let (issuer_key, _) = resolve_vc_issuer_key();
                let output = issue_sd_jwt(&bindings, &options, &issuer_key);
                let disc_count = output.disclosures.len();
                (
                    json!({
                        "ok": true,
                        "format": "sd-jwt",
                        "disclosures_total": disc_count,
                        "disclosed_paths": disclosed_paths,
                        "hidden_paths": hidden_paths,
                        "disclosed_claims": claim_value_to_json(&output.disclosed_claims),
                        "sd_jwt_presentation": output.sd_jwt_presentation,
                        "sd_jwt_all_length": output.sd_jwt_all.len(),
                    })
                    .to_string(),
                    None,
                    format!("✓ issue_vc_preview ({disc_count} disclosures)"),
                )
            }
            "parse_vc_token" => {
                let parsed: Result<ParseVcTokenArgs, _> = serde_json::from_str(args);
                let arg = match parsed {
                    Ok(a) => a,
                    Err(e) => {
                        return (
                            json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                            None,
                            format!("✗ parse_vc_token: {e}"),
                        );
                    }
                };
                let raw = arg.token.trim();
                let cred = if raw.contains('~') || raw.contains('.') {
                    parse_sd_jwt(raw, arg.issuer_jwk_json.as_deref())
                } else {
                    parse_mdoc_text(raw, arg.issuer_jwk_json.as_deref())
                };
                match cred {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ parse_vc_token: {e}"),
                    ),
                    Ok(parsed_cred) => {
                        let format = match parsed_cred.format {
                            ParsedCredentialFormat::SdJwt => "SD-JWT",
                            ParsedCredentialFormat::Mdoc => "mdoc",
                        };
                        let claims: Vec<_> = parsed_cred
                            .claims
                            .iter()
                            .map(|c| {
                                json!({
                                    "path": c.path,
                                    "value": claim_value_to_json(&c.value),
                                    "selective": c.selective,
                                    "disclose": c.disclose,
                                    "digest_verified": c.digest_verified,
                                })
                            })
                            .collect();
                        let cmd = if arg.import_to_sheet {
                            match self.resolve_ai_target_slide(args) {
                                Ok(slide) => {
                                    let cmds =
                                        parsed_cred.build_sheet_commands(self.presentation(), slide);
                                    Some(Command::Group(cmds))
                                }
                                Err(e) => {
                                    return (
                                        json!({ "error": e }).to_string(),
                                        None,
                                        format!("✗ parse_vc_token: {e}"),
                                    );
                                }
                            }
                        } else {
                            None
                        };
                        let count = claims.len();
                        (
                            json!({
                                "ok": true,
                                "format": format,
                                "vct_or_doctype": parsed_cred.vct_or_doctype,
                                "issuer": parsed_cred.issuer,
                                "signature_valid": parsed_cred.signature_valid,
                                "kb_sd_hash_valid": parsed_cred.kb_sd_hash_valid,
                                "digests_verified": parsed_cred.digests_verified,
                                "digests_total": parsed_cred.digests_total,
                                "imported": arg.import_to_sheet,
                                "claims": claims,
                            })
                            .to_string(),
                            cmd,
                            format!("✓ parse_vc_token ({format}, {count} claims)"),
                        )
                    }
                }
            }
            "prepare_oid4vp_presentation" => {
                let parsed: Result<PrepareOid4vpArgs, _> = serde_json::from_str(args);
                let arg = match parsed {
                    Ok(a) => a,
                    Err(e) => {
                        return (
                            json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                            None,
                            format!("✗ prepare_oid4vp_presentation: {e}"),
                        );
                    }
                };
                let Some(req) = Oid4vpRequest::parse(&arg.request) else {
                    return (
                        json!({ "error": "invalid OID4VP request (expected openid4vp:// URL or JSON with client_id/nonce)" }).to_string(),
                        None,
                        "✗ prepare_oid4vp_presentation: invalid request".to_string(),
                    );
                };
                let slide = match self.resolve_ai_target_slide(args) {
                    Ok(s) => s,
                    Err(e) => {
                        return (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ prepare_oid4vp_presentation: {e}"),
                        );
                    }
                };
                let Some(mut bindings) = extract_sheet_claims(self.presentation(), slide) else {
                    return (
                        json!({ "error": "no SD-JWT bindings found on this worksheet" }).to_string(),
                        None,
                        "✗ prepare_oid4vp_presentation: no bindings".to_string(),
                    );
                };

                let available_paths: std::collections::BTreeSet<String> =
                    bindings.claims.iter().map(|c| c.path_str.clone()).collect();
                let mut matched_claims = Vec::new();
                let mut missing_claims = Vec::new();
                for req_path in &req.requested_claims {
                    if available_paths.contains(req_path) {
                        matched_claims.push(req_path.clone());
                    } else {
                        missing_claims.push(req_path.clone());
                    }
                }

                let cmd = if arg.apply_to_sheet && !req.requested_claims.is_empty() {
                    let req_refs: Vec<&str> =
                        req.requested_claims.iter().map(String::as_str).collect();
                    let cmds = bindings.set_disclose_commands(self.presentation(), &req_refs, true);
                    if cmds.is_empty() {
                        None
                    } else {
                        Some(Command::Group(cmds))
                    }
                } else {
                    None
                };

                if !req.requested_claims.is_empty() {
                    let wanted: std::collections::BTreeSet<&str> =
                        req.requested_claims.iter().map(String::as_str).collect();
                    for claim in &mut bindings.claims {
                        if claim.selective {
                            claim.disclose = wanted.contains(claim.path_str.as_str());
                        }
                    }
                }

                let use_mdoc = match arg
                    .format
                    .as_deref()
                    .map(|s| s.trim().to_ascii_lowercase())
                    .as_deref()
                {
                    Some("mdoc" | "mso_mdoc") => true,
                    Some("sd-jwt" | "sd_jwt" | "vc+sd-jwt" | "dc+sd-jwt") => false,
                    Some(other) => {
                        return (
                            json!({ "error": format!("unsupported OID4VP format '{other}' (expected 'sd-jwt' or 'mdoc')") }).to_string(),
                            None,
                            format!("✗ prepare_oid4vp_presentation: unsupported format {other}"),
                        );
                    }
                    None => {
                        let lower = arg.request.to_ascii_lowercase();
                        lower.contains("mso_mdoc") || lower.contains("org.iso.18013")
                    }
                };

                let disclosed_paths: Vec<_> = bindings
                    .claims
                    .iter()
                    .filter(|c| !c.selective || c.disclose)
                    .map(|c| c.path_str.clone())
                    .collect();
                let hidden_paths: Vec<_> = bindings
                    .claims
                    .iter()
                    .filter(|c| c.selective && !c.disclose)
                    .map(|c| c.path_str.clone())
                    .collect();
                let req_count = req.requested_claims.len();
                let (issuer_key, _) = resolve_vc_issuer_key();

                if use_mdoc {
                    let mut mdoc_opts = MdocOptions::default();
                    if let Some(vct) = bindings
                        .claim("vct")
                        .and_then(|c| c.value.as_str())
                        .filter(|s| !s.trim().is_empty())
                    {
                        mdoc_opts.doc_type = vct.to_string();
                    }
                    if let Some(ns) = arg.name_space.filter(|s| !s.trim().is_empty()) {
                        mdoc_opts.name_space = ns;
                    }
                    let mdoc = issue_mdoc(&bindings, &mdoc_opts, &issuer_key);
                    return (
                        json!({
                            "ok": true,
                            "format": "mso_mdoc",
                            "doc_type": mdoc.doc_type,
                            "name_space": mdoc.name_space,
                            "client_id": req.client_id,
                            "nonce": req.nonce,
                            "response_uri": req.response_uri,
                            "requested_claims": req.requested_claims,
                            "matched_claims": matched_claims,
                            "missing_claims": missing_claims,
                            "disclosed_paths": disclosed_paths,
                            "hidden_paths": hidden_paths,
                            "applied_to_sheet": cmd.is_some(),
                            "disclosed_items": mdoc.disclosed_items,
                            "total_items": mdoc.total_items,
                            "vp_token": mdoc.base64url_presentation,
                            "device_response_base64url": mdoc.base64url_presentation,
                            "device_response_hex": mdoc.hex_presentation,
                        })
                        .to_string(),
                        cmd,
                        format!(
                            "✓ prepare_oid4vp_presentation (mso_mdoc, aud={}, {req_count} requested)",
                            req.client_id
                        ),
                    );
                }

                let (holder_key, _) = resolve_vc_holder_key();
                let mut options = SdJwtOptions::default();
                options.holder_public_jwk = Some(holder_key.to_public_jwk());
                let output = issue_sd_jwt(&bindings, &options, &issuer_key);
                let kb = output.attach_key_binding(&req.client_id, &req.nonce, 1_700_000_000, &holder_key);

                (
                    json!({
                        "ok": true,
                        "format": "vc+sd-jwt",
                        "client_id": req.client_id,
                        "nonce": req.nonce,
                        "response_uri": req.response_uri,
                        "requested_claims": req.requested_claims,
                        "matched_claims": matched_claims,
                        "missing_claims": missing_claims,
                        "disclosed_paths": disclosed_paths,
                        "hidden_paths": hidden_paths,
                        "applied_to_sheet": cmd.is_some(),
                        "sd_hash": kb.sd_hash,
                        "vp_token": kb.vp_token,
                    })
                    .to_string(),
                    cmd,
                    format!(
                        "✓ prepare_oid4vp_presentation (sd-jwt, aud={}, {req_count} requested)",
                        req.client_id
                    ),
                )
            }
            "get_vc_public_keys" => {
                let which = serde_json::from_str::<GetVcPublicKeysArgs>(args)
                    .ok()
                    .and_then(|a| a.which)
                    .unwrap_or_else(|| "all".to_string());
                match vc_public_keys_json(&which) {
                    Ok(payload) => (
                        payload.to_string(),
                        None,
                        format!("✓ get_vc_public_keys ({which})"),
                    ),
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ get_vc_public_keys: {e}"),
                    ),
                }
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
            "duplicate_slide" => {
                let parsed: Result<SlideIndexOptionalArg, _> = if args.trim().is_empty() {
                    Ok(SlideIndexOptionalArg { slide_index: None })
                } else {
                    serde_json::from_str(args)
                };
                match parsed
                    .map_err(|e| format!("invalid arguments: {e}"))
                    .and_then(|a| self.resolve_pptx_slide_id(a.slide_index))
                {
                    Err(e) => (
                        json!({ "error": e }).to_string(),
                        None,
                        format!("✗ duplicate_slide: {e}"),
                    ),
                    Ok(src_id) => {
                        let copy_id = self.document.ids().new_slide_id();
                        let Some(copy) = self.presentation().duplicate_slide(src_id, copy_id)
                        else {
                            return (
                                json!({ "error": "failed to duplicate slide" }).to_string(),
                                None,
                                "✗ duplicate_slide: failed".to_string(),
                            );
                        };
                        let index = self
                            .presentation()
                            .slide_index(src_id)
                            .map_or(self.presentation().slides.len(), |i| i + 1);
                        self.run(vec![Command::InsertSlide { index, slide: copy }], cx);
                        self.current_slide = copy_id;
                        self.selection = None;
                        cx.notify();
                        (
                            json!({ "ok": true, "slide_index": index + 1 }).to_string(),
                            None,
                            format!("✓ duplicate_slide (slide {})", index + 1),
                        )
                    }
                }
            }
            "remove_slide" => {
                let parsed: Result<SlideIndexRequiredArg, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ remove_slide: {e}"),
                    ),
                    Ok(a) => {
                        if self.presentation().slides.len() <= 1 {
                            return (
                                json!({ "error": "cannot remove the last remaining slide" })
                                    .to_string(),
                                None,
                                "✗ remove_slide: last slide".to_string(),
                            );
                        }
                        match self.resolve_pptx_slide_id(Some(a.slide_index)) {
                            Err(e) => (
                                json!({ "error": e }).to_string(),
                                None,
                                format!("✗ remove_slide: {e}"),
                            ),
                            Ok(slide_id) => (
                                json!({ "ok": true }).to_string(),
                                Some(Command::RemoveSlide { slide: slide_id }),
                                format!("✓ remove_slide (slide {})", a.slide_index),
                            ),
                        }
                    }
                }
            }
            "move_slide" => {
                let parsed: Result<MoveSlideArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ move_slide: {e}"),
                    ),
                    Ok(a) => {
                        let count = self.presentation().slides.len();
                        if a.to_index == 0 || a.to_index > count {
                            return (
                                json!({ "error": format!("to_index must be 1..={count}") })
                                    .to_string(),
                                None,
                                "✗ move_slide: invalid to_index".to_string(),
                            );
                        }
                        match self.resolve_pptx_slide_id(Some(a.slide_index)) {
                            Err(e) => (
                                json!({ "error": e }).to_string(),
                                None,
                                format!("✗ move_slide: {e}"),
                            ),
                            Ok(slide_id) => (
                                json!({ "ok": true }).to_string(),
                                Some(Command::MoveSlide {
                                    slide: slide_id,
                                    index: a.to_index - 1,
                                }),
                                format!("✓ move_slide ({} -> {})", a.slide_index, a.to_index),
                            ),
                        }
                    }
                }
            }
            "insert_text_box" => {
                let parsed: Result<InsertTextBoxArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ insert_text_box: {e}"),
                    ),
                    Ok(a) => match self.resolve_pptx_slide_id(a.slide_index) {
                        Err(e) => (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ insert_text_box: {e}"),
                        ),
                        Ok(slide_id) => {
                            let size = self.presentation().slide_size;
                            let x = ((a.x_pct.clamp(0.0, 100.0) / 100.0) * size.width as f32) as i64;
                            let y =
                                ((a.y_pct.clamp(0.0, 100.0) / 100.0) * size.height as f32) as i64;
                            let w =
                                ((a.w_pct.clamp(1.0, 100.0) / 100.0) * size.width as f32) as i64;
                            let h =
                                ((a.h_pct.clamp(1.0, 100.0) / 100.0) * size.height as f32) as i64;
                            let shape_id = self.document.ids().new_shape_id();
                            let index = self
                                .presentation()
                                .slide(slide_id)
                                .map_or(0, |s| s.shapes.len());
                            let mut auto = AutoShape::new(Geometry::Rect, Fill::None, None);
                            let mut style = TextStyle::default();
                            if let Some(pt) = a.font_size
                                && pt.is_finite()
                                && pt > 0.0
                            {
                                style.font_size = (pt * 100.0).round() as u32;
                            }
                            if let Some(b) = a.bold {
                                style.bold = b;
                            }
                            auto.text = Some(TextBody::plain(&a.text, style));
                            let shape = Shape::new(
                                shape_id,
                                EmuRect::new(x, y, w, h),
                                ShapeKind::Auto(auto),
                            );
                            (
                                json!({ "ok": true, "shape_id": shape_id.0 }).to_string(),
                                Some(Command::InsertShape {
                                    slide: slide_id,
                                    index,
                                    shape,
                                }),
                                format!("✓ insert_text_box (shape {})", shape_id.0),
                            )
                        }
                    },
                }
            }
            "set_shape_style" => {
                let parsed: Result<SetShapeStyleArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ set_shape_style: {e}"),
                    ),
                    Ok(a) => match self.resolve_pptx_slide_id(a.slide_index) {
                        Err(e) => (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ set_shape_style: {e}"),
                        ),
                        Ok(slide_id) => {
                            let shape_id = ShapeId(a.shape_id);
                            let Some(shape) = self
                                .presentation()
                                .slide(slide_id)
                                .and_then(|s| s.shape(shape_id))
                            else {
                                return (
                                    json!({ "error": format!("shape {} not found", a.shape_id) })
                                        .to_string(),
                                    None,
                                    format!("✗ set_shape_style: shape {} not found", a.shape_id),
                                );
                            };
                            let mut cmds = Vec::new();
                            if a.font_size.is_some()
                                || a.bold.is_some()
                                || a.italic.is_some()
                                || a.text_color.is_some()
                            {
                                let mut style = match &shape.kind {
                                    ShapeKind::Auto(auto) => auto
                                        .text
                                        .as_ref()
                                        .map(|tb| tb.style.clone())
                                        .unwrap_or_default(),
                                    _ => TextStyle::default(),
                                };
                                if let Some(pt) = a.font_size {
                                    if !pt.is_finite() || pt <= 0.0 || pt > 409.0 {
                                        return (
                                            json!({ "error": format!("invalid font_size '{pt}'") })
                                                .to_string(),
                                            None,
                                            "✗ set_shape_style: invalid font_size".to_string(),
                                        );
                                    }
                                    style.font_size = (pt * 100.0).round() as u32;
                                }
                                if let Some(b) = a.bold {
                                    style.bold = b;
                                }
                                if let Some(i) = a.italic {
                                    style.italic = i;
                                }
                                if let Some(hex) = a.text_color {
                                    match parse_hex_rgb(&hex) {
                                        Ok(rgb) => style.color = rgb,
                                        Err(e) => {
                                            return (
                                                json!({ "error": e }).to_string(),
                                                None,
                                                format!("✗ set_shape_style: {e}"),
                                            );
                                        }
                                    }
                                }
                                cmds.push(Command::SetTextStyle {
                                    slide: slide_id,
                                    shape: shape_id,
                                    style,
                                });
                            }
                            if let Some(fill_opt) = a.fill_color {
                                let fill = match fill_opt {
                                    None => Fill::None,
                                    Some(hex) => match parse_hex_rgb(&hex) {
                                        Ok(rgb) => Fill::Solid(Color::opaque(rgb)),
                                        Err(e) => {
                                            return (
                                                json!({ "error": e }).to_string(),
                                                None,
                                                format!("✗ set_shape_style: {e}"),
                                            );
                                        }
                                    },
                                };
                                cmds.push(Command::SetFill {
                                    slide: slide_id,
                                    shape: shape_id,
                                    fill,
                                });
                            }
                            if cmds.is_empty() {
                                return (
                                    json!({ "error": "no style properties specified" }).to_string(),
                                    None,
                                    "✗ set_shape_style: empty".to_string(),
                                );
                            }
                            let cmd = if cmds.len() == 1 {
                                cmds.pop().expect("single")
                            } else {
                                Command::Group(cmds)
                            };
                            (
                                json!({ "ok": true }).to_string(),
                                Some(cmd),
                                format!("✓ set_shape_style (shape {})", a.shape_id),
                            )
                        }
                    },
                }
            }
            other => (
                json!({ "error": format!("unknown tool: {other}") }).to_string(),
                None,
                format!("✗ {other}: unknown tool"),
            ),
        }
    }

    fn execute_docx_tool(
        &mut self,
        call: &ToolCall,
        cx: &mut Context<Self>,
    ) -> (String, Option<Command>, String) {
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
            "set_run_format" => {
                let parsed: Result<SetRunFormatArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ set_run_format: {e}"),
                    ),
                    Ok(a) => {
                        let Some(flow) = self.presentation().flow.as_ref() else {
                            return (
                                json!({ "error": "no active document" }).to_string(),
                                None,
                                "✗ set_run_format: no active document".to_string(),
                            );
                        };
                        let para = a.paragraph;
                        let Some(Block::Paragraph(p)) = flow.body.get(para as usize) else {
                            return (
                                json!({ "error": format!("paragraph {para} not found") })
                                    .to_string(),
                                None,
                                format!("✗ set_run_format: paragraph {para} not found"),
                            );
                        };
                        let para_len = p.len();
                        match build_set_run_format_command(a, para_len) {
                            Err(e) => (
                                json!({ "error": e }).to_string(),
                                None,
                                format!("✗ set_run_format: {e}"),
                            ),
                            Ok(cmd) => (
                                json!({ "ok": true }).to_string(),
                                Some(cmd),
                                format!("✓ set_run_format (p{para})"),
                            ),
                        }
                    }
                }
            }
            "set_paragraph_style" => {
                let parsed: Result<SetParagraphStyleArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ set_paragraph_style: {e}"),
                    ),
                    Ok(a) => (
                        json!({ "ok": true }).to_string(),
                        Some(Command::Flow(FlowCommand::SetParagraphFormat {
                            first: a.paragraph,
                            last: a.paragraph,
                            change: ParaChange::Style(a.style),
                        })),
                        format!("✓ set_paragraph_style (p{})", a.paragraph),
                    ),
                }
            }
            "insert_table" => {
                let parsed: Result<InsertTableArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ insert_table: {e}"),
                    ),
                    Ok(a) => {
                        let rows = a.rows.clamp(1, 20);
                        let cols = a.columns.clamp(1, 12);
                        (
                            json!({ "ok": true }).to_string(),
                            Some(Command::Flow(FlowCommand::InsertTable {
                                at: FlowPos::new(a.paragraph, 0),
                                rows,
                                columns: cols,
                                width: 5_486_400,
                            })),
                            format!("✓ insert_table ({rows}x{cols} at p{})", a.paragraph),
                        )
                    }
                }
            }
            "find_text_in_document" => {
                let parsed: Result<FindTextArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ find_text_in_document: {e}"),
                    ),
                    Ok(a) => {
                        let Some(flow) = self.presentation().flow.as_ref() else {
                            return (
                                json!({ "error": "no active document" }).to_string(),
                                None,
                                "✗ find_text_in_document: no active document".to_string(),
                            );
                        };
                        let q = a.query.to_ascii_lowercase();
                        let matches: Vec<_> = flow
                            .body
                            .iter()
                            .enumerate()
                            .filter_map(|(idx, block)| match block {
                                Block::Paragraph(p) => {
                                    let t = paragraph_plain_text(p);
                                    if t.to_ascii_lowercase().contains(&q) {
                                        Some(json!({
                                            "paragraph": idx,
                                            "text": t.chars().take(120).collect::<String>(),
                                        }))
                                    } else {
                                        None
                                    }
                                }
                                Block::Table(_) => None,
                            })
                            .take(50)
                            .collect();
                        let count = matches.len();
                        (
                            json!({ "count": count, "matches": matches }).to_string(),
                            None,
                            format!("✓ find_text_in_document ({count} matches)"),
                        )
                    }
                }
            }
            "export_active_lawtext" => {
                match ibis_pptx::lawtext::export_presentation_to_lawtext(self.presentation()) {
                    Ok(lt) => match ibis_pptx::lawtext::parse_lawtext(&lt) {
                        Ok(doc) => (
                            json!({
                                "law_title": doc.law_title,
                                "law_num": doc.law_num,
                                "lawtext": lt,
                            })
                            .to_string(),
                            None,
                            "✓ export_active_lawtext".to_string(),
                        ),
                        Err(e) => (
                            json!({ "error": e.to_string(), "lawtext": lt }).to_string(),
                            None,
                            format!("✗ export_active_lawtext: {e}"),
                        ),
                    },
                    Err(e) => (
                        json!({ "error": e.to_string() }).to_string(),
                        None,
                        format!("✗ export_active_lawtext: {e}"),
                    ),
                }
            }
            "generate_lawtext_5set" => {
                let parsed: Result<GenerateLawtext5SetArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ generate_lawtext_5set: {e}"),
                    ),
                    Ok(a) => {
                        let old_src = a
                            .old_lawtext
                            .as_deref()
                            .filter(|s| !s.trim().is_empty())
                            .or_else(|| {
                                self.ai_assistant
                                    .as_ref()
                                    .and_then(|p| p.pending_old_lawtext.as_ref())
                                    .map(|(_, s)| s.as_str())
                            });
                        let Some(old_str) = old_src else {
                            return (
                                json!({ "error": "old_lawtext が指定されていません（サイドバーから現行法令を選択するか、old_lawtext を渡してください）" }).to_string(),
                                None,
                                "✗ generate_lawtext_5set: missing old_lawtext".to_string(),
                            );
                        };
                        match (
                        parse_law_input_str(old_str),
                        match a.new_lawtext.as_deref() {
                            Some(s) => parse_law_input_str(s),
                            None => ibis_pptx::lawtext::export_presentation_to_lawtext(
                                self.presentation(),
                            )
                            .map_err(|e| e.to_string())
                            .and_then(|lt| parse_law_input_str(&lt)),
                        },
                    ) {
                        (Ok(old_doc), Ok(new_doc)) => {
                            let opts = ibis_pptx::lawtext::ShinkyuOptions {
                                new_on_left: !a.old_on_left.unwrap_or(false),
                                ..Default::default()
                            };
                            let pkg = ibis_pptx::lawtext::build_5set_package(
                                &old_doc,
                                &new_doc,
                                a.policy_memo.as_deref().unwrap_or(""),
                                &opts,
                            );
                            let exact = pkg.aratame_verification.is_exact_match;
                            (
                                pkg.to_json_value().to_string(),
                                None,
                                format!("✓ generate_lawtext_5set (aratame_exact={exact})"),
                            )
                        }
                        (Err(e), _) | (_, Err(e)) => (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ generate_lawtext_5set: {e}"),
                        ),
                    }
                    }
                }
            }
            "verify_lawtext_aratame" => {
                let parsed: Result<VerifyLawtextAratameArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ verify_lawtext_aratame: {e}"),
                    ),
                    Ok(a) => {
                        let old_src = a
                            .old_lawtext
                            .as_deref()
                            .filter(|s| !s.trim().is_empty())
                            .or_else(|| {
                                self.ai_assistant
                                    .as_ref()
                                    .and_then(|p| p.pending_old_lawtext.as_ref())
                                    .map(|(_, s)| s.as_str())
                            });
                        let Some(old_str) = old_src else {
                            return (
                                json!({ "error": "old_lawtext が指定されていません" }).to_string(),
                                None,
                                "✗ verify_lawtext_aratame: missing old_lawtext".to_string(),
                            );
                        };
                        match (
                        parse_law_input_str(old_str),
                        match a.expected_new_lawtext.as_deref() {
                            Some(s) => parse_law_input_str(s),
                            None => ibis_pptx::lawtext::export_presentation_to_lawtext(
                                self.presentation(),
                            )
                            .map_err(|e| e.to_string())
                            .and_then(|lt| parse_law_input_str(&lt)),
                        },
                    ) {
                        (Ok(old_doc), Ok(new_doc)) => {
                            let res = ibis_pptx::lawtext::verify_aratame(
                                &old_doc,
                                &a.aratame_text,
                                &new_doc,
                            );
                            let exact = res.is_exact_match;
                            (
                                res.to_json_value().to_string(),
                                None,
                                format!("✓ verify_lawtext_aratame (exact={exact})"),
                            )
                        }
                        (Err(e), _) | (_, Err(e)) => (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ verify_lawtext_aratame: {e}"),
                        ),
                    }
                    }
                }
            }
            "lint_lawtext" => {
                let parsed: Result<LintLawtextArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ lint_lawtext: {e}"),
                    ),
                    Ok(a) => {
                        let new_res = match a.new_lawtext.as_deref() {
                            Some(s) => parse_law_input_str(s),
                            None => ibis_pptx::lawtext::export_presentation_to_lawtext(
                                self.presentation(),
                            )
                            .map_err(|e| e.to_string())
                            .and_then(|lt| parse_law_input_str(&lt)),
                        };
                        let old_src = a
                            .old_lawtext
                            .as_deref()
                            .filter(|s| !s.trim().is_empty())
                            .or_else(|| {
                                self.ai_assistant
                                    .as_ref()
                                    .and_then(|p| p.pending_old_lawtext.as_ref())
                                    .map(|(_, s)| s.as_str())
                            });
                        let old_res = old_src.map(parse_law_input_str).transpose();
                        match (old_res, new_res) {
                            (Ok(old_opt), Ok(new_doc)) => {
                                let report = match old_opt.as_ref() {
                                    Some(old_doc) => {
                                        ibis_pptx::lawtext::lint_amendment(old_doc, &new_doc)
                                    }
                                    None => ibis_pptx::lawtext::lint_law(&new_doc),
                                };
                                let errs = report.error_count;
                                let warns = report.warning_count;
                                (
                                    json!({
                                        "report": report.to_json_value(),
                                        "markdown": report.to_markdown(),
                                    })
                                    .to_string(),
                                    None,
                                    format!("✓ lint_lawtext ({errs} errors, {warns} warnings)"),
                                )
                            }
                            (Err(e), _) | (_, Err(e)) => (
                                json!({ "error": e }).to_string(),
                                None,
                                format!("✗ lint_lawtext: {e}"),
                            ),
                        }
                    }
                }
            }
            "fetch_egov_law" => {
                let parsed: Result<FetchEgovLawArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ fetch_egov_law: {e}"),
                    ),
                    Ok(a) => match crate::ai_client::fetch_egov_law_xml(
                        a.law_title.as_deref(),
                        a.law_num.as_deref(),
                        a.law_id.as_deref(),
                    ) {
                        Ok((id, title, xml)) => match ibis_pptx::lawtext::parse_law_xml(&xml) {
                            Ok(doc) => {
                                let lt = ibis_pptx::lawtext::to_lawtext(&doc);
                                let set_baseline = a.set_as_baseline.unwrap_or(true);
                                if set_baseline {
                                    if let Some(p) = self.ai_assistant.as_mut() {
                                        p.pending_old_lawtext = Some((
                                            format!("{}.xml", doc.law_title),
                                            lt.clone(),
                                        ));
                                    }
                                }
                                (
                                    json!({
                                        "law_id": id,
                                        "law_title": if doc.law_title.is_empty() { title } else { doc.law_title },
                                        "law_num": doc.law_num,
                                        "set_as_baseline": set_baseline,
                                        "lawtext": lt,
                                    })
                                    .to_string(),
                                    None,
                                    format!("✓ fetch_egov_law ({id})"),
                                )
                            }
                            Err(e) => (
                                json!({ "error": e.to_string() }).to_string(),
                                None,
                                format!("✗ fetch_egov_law parse: {e}"),
                            ),
                        },
                        Err(e) => (
                            json!({ "error": e }).to_string(),
                            None,
                            format!("✗ fetch_egov_law: {e}"),
                        ),
                    },
                }
            }
            "open_markdown_document" => {
                let parsed: Result<OpenMarkdownDocumentArgs, _> = serde_json::from_str(args);
                match parsed {
                    Err(e) => (
                        json!({ "error": format!("invalid arguments: {e}") }).to_string(),
                        None,
                        format!("✗ open_markdown_document: {e}"),
                    ),
                    Ok(a) => match ibis_pptx::markdown::open_markdown(a.markdown.into_bytes()) {
                        Ok(opened) => {
                            let title = a.title;
                            let name: SharedString = title.clone().into();
                            #[cfg(not(target_family = "wasm"))]
                            {
                                crate::app::open_window_with(cx, move |editor, window, cx| {
                                    editor.show_new(opened, name, window, cx);
                                });
                                (
                                    json!({ "opened": true, "title": title }).to_string(),
                                    None,
                                    format!("✓ open_markdown_document ({title})"),
                                )
                            }
                            #[cfg(target_family = "wasm")]
                            {
                                let _ = (opened, name, cx);
                                (
                                    json!({
                                        "opened": false,
                                        "title": title,
                                        "note": "Opening a separate window is only supported on desktop builds."
                                    })
                                    .to_string(),
                                    None,
                                    format!("✓ open_markdown_document (wasm noop: {title})"),
                                )
                            }
                        }
                        Err(e) => (
                            json!({ "error": e.to_string() }).to_string(),
                            None,
                            format!("✗ open_markdown_document: {e}"),
                        ),
                    },
                }
            }
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
        #[cfg(not(target_family = "wasm"))]
        let base_url_input = panel.base_url_input.clone();
        #[cfg(not(target_family = "wasm"))]
        let model_input = panel.model_input.clone();
        #[cfg(not(target_family = "wasm"))]
        let api_key_input = panel.api_key_input.clone();
        #[cfg(target_family = "wasm")]
        let browser_models = panel.browser_models.clone();
        #[cfg(target_family = "wasm")]
        let browser_status = panel.browser_status.clone();
        #[cfg(target_family = "wasm")]
        let browser_pending = panel.browser_pending.clone();
        #[cfg(not(target_family = "wasm"))]
        let model_caption = format!("Model: {}", cfg.model);
        #[cfg(target_family = "wasm")]
        let model_caption = {
            let label = browser_models
                .iter()
                .find(|model| model.key == cfg.model)
                .map(|model| model.label.clone())
                .unwrap_or_else(|| cfg.model.clone());
            format!("モデル: {label}")
        };

        let header = h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(model_caption),
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

        #[cfg(not(target_family = "wasm"))]
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
        #[cfg(target_family = "wasm")]
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
                        .child("ブラウザでは、ログイン中の Docs から選んだモデルを使います。APIキーはこの画面には置きません。返答は提案です。文書は変わりません。"),
                )
        });
        #[cfg(target_family = "wasm")]
        let browser_box = {
            let selected = browser_models.iter().find(|model| model.key == cfg.model);
            let editor = cx.weak_entity();
            let items = browser_models
                .iter()
                .map(|model| {
                    let key = model.key.clone();
                    crate::chrome::DropdownItem::new(model.label.clone())
                        .checked(model.key == cfg.model)
                        .on_select(std::rc::Rc::new(move |editor, window, cx| {
                            editor.select_browser_model(key.clone(), window, cx);
                        }))
                })
                .collect();
            let label = selected.map(|model| model.label.clone()).unwrap_or_else(|| "モデル".to_string());
            let mut column = v_flex().gap_1().child(
                crate::chrome::Dropdown::new("ai-browser-model", label, items, &editor)
                    .searchable(false)
                    .fill_width(),
            );
            if let Some(model) = selected {
                if let (Some(input), Some(output)) = (&model.input_price, &model.output_price) {
                    column = column.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("100万トークンあたり入力 {input}、出力 {output}")),
                    );
                }
                if let Some(note) = &model.note {
                    column = column.child(div().text_xs().child(note.clone()));
                }
            }
            if !browser_status.is_empty() {
                column = column.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(browser_status.clone()),
                );
            }
            if browser_status.contains("ログイン") {
                column = column.child(
                    Button::new("ai-browser-login")
                        .outline()
                        .xsmall()
                        .label("ログイン画面へ")
                        .on_click(cx.listener(|_, _, _, _| open_browser_login())),
                );
            }
            Some(column.into_any_element())
        };
        #[cfg(not(target_family = "wasm"))]
        let browser_box: Option<gpui::AnyElement> = None;
        #[cfg(target_family = "wasm")]
        let pending_box = browser_pending.as_ref().map(|pending| {
            v_flex()
                .gap_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .child(div().text_xs().child(if !pending.prefix_ok && !pending.selection_fits {
                    if pending.selection_over {
                        "選択範囲も、開いている文書も、このモデルの上限を超えています。範囲を狭めるか、指示を短くしてから、入力に戻してください。"
                    } else {
                        "指示が長いため、文書を添えられません。指示を短くしてから、もう一度送ってください。"
                    }
                } else {
                    "開いている文書は、このモデルに一度に送れる文字数を超えています。送る範囲を選んでください。"
                }))
                .when(pending.selection_over && pending.prefix_ok, |column| {
                    column.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("選択範囲も上限を超えています。"),
                    )
                })
                .when(pending.prefix_ok, |column| {
                    column.child(
                        Button::new("ai-browser-prefix")
                            .outline()
                            .xsmall()
                            .label("収まる先頭から送る")
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_browser_prefix(cx))),
                    )
                })
                .when(pending.selection_fits, |column| {
                    column.child(
                        Button::new("ai-browser-selection")
                            .outline()
                            .xsmall()
                            .label("選択範囲だけ送る")
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_browser_selection(cx))),
                    )
                })
                .child(
                    Button::new("ai-browser-restore")
                        .ghost()
                        .xsmall()
                        .label("やめて入力に戻す")
                        .on_click(cx.listener(|this, _, window, cx| this.restore_browser_prompt(window, cx))),
                )
                .into_any_element()
        });
        #[cfg(not(target_family = "wasm"))]
        let pending_box: Option<gpui::AnyElement> = None;

        #[cfg(not(target_family = "wasm"))]
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
                        (
                            "ai-quick-law-lint",
                            "法制執務 Lint チェック",
                            "現在開いている文書を lint_lawtext でチェックし、参照切れ・接続詞階層・条番号・略称重複の指摘事項をわかりやすく報告してください。",
                        ),
                    ],
                ),
            };
        #[cfg(target_family = "wasm")]
        let (intro, quick_actions): (&str, Vec<(&'static str, &'static str, &'static str)>) = match kind {
            AiDocKind::Xlsx => (
                "表の要約や読み方の案を文章で受け取れます。返答は提案です。表は変わりません。コピーしてから貼り付けてください。",
                vec![
                    ("ai-quick-1", "選択範囲を要約", "選択範囲の内容を要約してください。選択がなければ、開いている表を要約してください。表は変えず、文章で答えてください。"),
                    ("ai-quick-2", "読みやすさの案", "表を読みやすくする案を文章で提案してください。表は変えず、文章で答えてください。"),
                    ("ai-quick-3", "気になる点", "表の内容で気になる点を文章で挙げてください。"),
                ],
            ),
            AiDocKind::Pptx => (
                "スライドの要約や言い換え案を文章で受け取れます。返答は提案です。スライドは変わりません。コピーしてから貼り付けてください。",
                vec![
                    ("ai-quick-1", "全体を要約", "プレゼンテーション全体の要点を要約してください。スライドは変えず、文章で答えてください。"),
                    ("ai-quick-2", "このスライドの案", "いま開いているスライドの文章を、より分かりやすい案として書いてください。"),
                    ("ai-quick-3", "ノートの案", "いま開いているスライドの発表用ノートの案を文章で書いてください。"),
                ],
            ),
            AiDocKind::Docx => (
                "文書の要約や校正案を受け取れます。選択範囲の書き換え案は、プレビューから文書に適用できます。",
                vec![
                    ("ai-quick-1", "文書を要約", "文書の要点を要約してください。文書は変えず、文章で答えてください。"),
                    ("ai-quick-2", "校正の案", "誤字や読みにくさの改善案を文章で提案してください。"),
                    ("ai-quick-3", "見出しの案", "見出しや章立ての案を文章で提案してください。"),
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
            #[cfg(not(target_family = "wasm"))]
            if kind == AiDocKind::Docx {
                quick_col = quick_col.child(
                    Button::new("ai-quick-pick-old-law")
                        .outline()
                        .xsmall()
                        .label("現行法令 (.xml / .lawtext) を選んで 5点セット生成...")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.pick_old_lawtext_for_assistant(window, cx);
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
            #[cfg(target_family = "wasm")]
            let replacement_ui = msg.replacement.as_ref().map(|target| {
                if msg.preview_replacement {
                    v_flex()
                        .gap_1()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .pt_1()
                        .child(div().text_xs().child("置換前"))
                        .child(div().text_xs().child(target.before.clone()))
                        .child(div().text_xs().child("置換後"))
                        .child(div().text_xs().child(msg.text.clone()))
                        .child(h_flex().gap_1().justify_end()
                            .child(Button::new(("ai-replacement-cancel", idx)).ghost().xsmall().label("キャンセル")
                                .on_click(cx.listener(move |this, _, _, cx| this.set_ai_replacement_preview(idx, false, cx))))
                            .child(Button::new(("ai-replacement-apply", idx)).primary().xsmall().label("選択範囲を置換")
                                .on_click(cx.listener(move |this, _, window, cx| this.apply_browser_replacement(idx, window, cx)))))
                        .into_any_element()
                } else {
                    Button::new(("ai-replacement-preview", idx)).outline().xsmall().label("置換をプレビュー")
                        .on_click(cx.listener(move |this, _, _, cx| this.set_ai_replacement_preview(idx, true, cx)))
                        .into_any_element()
                }
            });
            #[cfg(not(target_family = "wasm"))]
            let replacement_ui: Option<AnyElement> = None;
            msg_list = msg_list.child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .rounded_md()
                    .bg(bg)
                    .child(div().text_xs().text_color(fg).child(msg.text.clone()))
                    .children(replacement_ui)
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
            .children(browser_box)
            .children(settings_box)
            .children(pending_box)
            .child(scrollable_msgs);

        #[cfg(target_family = "wasm")]
        let rewrite_button = (kind == AiDocKind::Docx && !streaming).then(|| {
            Button::new("ai-rewrite-selection").outline().xsmall().label("選択範囲の書き換え案")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.submit_browser_assistant(
                        "選択した文章を、意味を変えずに読みやすく書き換えてください。書き換えた本文だけを、見出し・説明・Markdownの装飾を付けずに返してください。".into(),
                        true,
                        cx,
                    );
                }))
        });
        #[cfg(not(target_family = "wasm"))]
        let rewrite_button: Option<Button> = None;
        let footer = v_flex()
            .children(rewrite_button)
            .gap_2()
            .w_full()
            .child(Textarea::new(&prompt_input).small())
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
