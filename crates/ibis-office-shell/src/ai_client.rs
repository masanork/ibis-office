// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! OpenAI-compatible Chat Completions client with SSE streaming and tool calling.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SETTING_BASE_URL: &str = "ai.base_url";
pub const SETTING_MODEL: &str = "ai.model";
pub const SETTING_API_KEY: &str = "ai.api_key";

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_MODEL: &str = "gpt-6.1-terra";

/// Local configuration for the OpenAI-compatible endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

impl AiConfig {
    /// Load configuration from `settings.txt` with environment variable fallbacks.
    pub fn load() -> Self {
        let base_url = crate::settings::get(SETTING_BASE_URL)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let model = crate::settings::get(SETTING_MODEL)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let mut api_key = crate::settings::get(SETTING_API_KEY).unwrap_or_default();
        if api_key.trim().is_empty() {
            #[cfg(not(target_family = "wasm"))]
            {
                api_key = std::env::var("IBIS_AI_API_KEY")
                    .or_else(|_| std::env::var("OPENAI_API_KEY"))
                    .unwrap_or_default();
            }
        }
        Self {
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            model: model.trim().to_string(),
            api_key: api_key.trim().to_string(),
        }
    }

    /// Persist configuration into `settings.txt`.
    pub fn save(&self) {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            crate::settings::remove(SETTING_BASE_URL);
        } else {
            crate::settings::set(SETTING_BASE_URL, base);
        }
        let model = self.model.trim();
        if model.is_empty() {
            crate::settings::remove(SETTING_MODEL);
        } else {
            crate::settings::set(SETTING_MODEL, model);
        }
        let key = self.api_key.trim();
        if key.is_empty() {
            crate::settings::remove(SETTING_API_KEY);
        } else {
            crate::settings::set(SETTING_API_KEY, key);
        }
    }

    pub fn completions_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolCallFunction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".to_string(),
            content,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: ToolFunctionDef,
}

#[derive(Clone, Debug, Serialize)]
pub struct ToolFunctionDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Clone, Debug, Default)]
pub struct CompletionOutcome {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Incremental parser for OpenAI-compatible SSE `data: ...` lines.
#[derive(Default)]
pub struct SseStreamParser {
    content: String,
    tool_calls: BTreeMap<usize, PartialToolCall>,
    finish_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SseEvent {
    TextDelta(String),
    Done,
}

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Deserialize)]
struct StreamToolCallDelta {
    index: Option<usize>,
    id: Option<String>,
    function: Option<StreamToolFunctionDelta>,
}

#[derive(Deserialize)]
struct StreamToolFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: Option<ErrorDetail>,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: Option<String>,
}

impl SseStreamParser {
    /// Feed one line from an SSE stream. Returns `Some(SseEvent)` when text arrives or `[DONE]` is seen.
    pub fn feed_line(&mut self, raw_line: &str) -> Result<Option<SseEvent>, String> {
        let line = raw_line.trim_end_matches(&['\r', '\n'][..]).trim();
        if line.is_empty() || line.starts_with(':') {
            return Ok(None);
        }
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(None);
        };
        let data = data.trim();
        if data == "[DONE]" {
            return Ok(Some(SseEvent::Done));
        }
        if let Ok(err) = serde_json::from_str::<ErrorEnvelope>(data) {
            if let Some(detail) = err.error {
                return Err(detail.message.unwrap_or_else(|| "API error".to_string()));
            }
        }
        let chunk: StreamChunk = serde_json::from_str(data)
            .map_err(|e| format!("failed to parse SSE chunk: {e}"))?;
        let mut text_out = String::new();
        for choice in chunk.choices {
            if let Some(reason) = choice.finish_reason {
                self.finish_reason = Some(reason);
            }
            if let Some(text) = choice.delta.content {
                if !text.is_empty() {
                    self.content.push_str(&text);
                    text_out.push_str(&text);
                }
            }
            if let Some(deltas) = choice.delta.tool_calls {
                for delta in deltas {
                    let idx = delta.index.unwrap_or(0);
                    let entry = self.tool_calls.entry(idx).or_default();
                    if let Some(id) = delta.id {
                        entry.id.push_str(&id);
                    }
                    if let Some(func) = delta.function {
                        if let Some(name) = func.name {
                            entry.name.push_str(&name);
                        }
                        if let Some(args) = func.arguments {
                            entry.arguments.push_str(&args);
                        }
                    }
                }
            }
        }
        if text_out.is_empty() {
            Ok(None)
        } else {
            Ok(Some(SseEvent::TextDelta(text_out)))
        }
    }

    pub fn into_outcome(self) -> CompletionOutcome {
        let tool_calls = self
            .tool_calls
            .into_values()
            .filter(|tc| !tc.name.is_empty())
            .enumerate()
            .map(|(i, tc)| ToolCall {
                id: if tc.id.is_empty() {
                    format!("call_{i}")
                } else {
                    tc.id
                },
                kind: "function".to_string(),
                function: ToolCallFunction {
                    name: tc.name,
                    arguments: if tc.arguments.is_empty() {
                        "{}".to_string()
                    } else {
                        tc.arguments
                    },
                },
            })
            .collect();
        CompletionOutcome {
            content: self.content,
            tool_calls,
            finish_reason: self.finish_reason,
        }
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
    stream: bool,
}

fn default_reasoning_effort(model: &str, has_tools: bool) -> Option<&'static str> {
    if !has_tools {
        return None;
    }
    let lower = model.trim().to_ascii_lowercase();
    if lower.starts_with("gpt-5") || lower.starts_with("gpt-6") {
        Some("none")
    } else {
        None
    }
}

/// Send a streaming Chat Completions request and invoke `on_delta` for each text chunk.
///
/// Automatically retries once if the endpoint returns HTTP 400 complaining about
/// `reasoning_effort` (switching between `Some("none")` and `None`).
#[cfg(not(target_family = "wasm"))]
pub async fn stream_chat_completion(
    config: &AiConfig,
    messages: &[ChatMessage],
    tools: Vec<ToolDefinition>,
    mut on_delta: impl FnMut(&str),
) -> Result<CompletionOutcome, String> {
    let initial = default_reasoning_effort(&config.model, !tools.is_empty());
    match stream_chat_completion_once(config, messages, tools.clone(), initial, &mut on_delta)
        .await
    {
        Ok(outcome) => Ok(outcome),
        Err(err)
            if err.starts_with("HTTP 400:")
                && err.to_ascii_lowercase().contains("reasoning_effort") =>
        {
            let fallback = if initial.is_some() { None } else { Some("none") };
            stream_chat_completion_once(config, messages, tools, fallback, &mut on_delta).await
        }
        Err(err) => Err(err),
    }
}

#[cfg(not(target_family = "wasm"))]
async fn stream_chat_completion_once(
    config: &AiConfig,
    messages: &[ChatMessage],
    tools: Vec<ToolDefinition>,
    reasoning_effort: Option<&'static str>,
    on_delta: &mut impl FnMut(&str),
) -> Result<CompletionOutcome, String> {
    use futures::StreamExt as _;
    use std::io::{BufRead as _, BufReader as StdBufReader, Write as _};
    use std::process::{Command, Stdio};

    let url = config.completions_url();
    let payload = serde_json::to_vec(&ChatCompletionRequest {
        model: &config.model,
        messages,
        tools,
        reasoning_effort,
        stream: true,
    })
    .map_err(|e| format!("failed to serialize request: {e}"))?;

    let api_key = config.api_key.clone();
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<Result<Option<String>, String>>();

    std::thread::spawn(move || {
        const STATUS_MARKER: &str = "__IBIS_HTTP_STATUS__:";
        let mut cmd = Command::new("curl");
        cmd.arg("--disable")
            .arg("-sS")
            .arg("-N")
            .args(["--proto", "=https,http"])
            .args(["--connect-timeout", "15"])
            .args(["--max-time", "180"])
            .args(["-X", "POST"])
            .args(["-H", "Content-Type: application/json"])
            .args(["-H", "Accept: text/event-stream"]);
        if !api_key.is_empty() {
            cmd.args(["-H", &format!("Authorization: Bearer {api_key}")]);
        }
        cmd.args(["--data-binary", "@-"])
            .args(["-w", &format!("\n{STATUS_MARKER}%{{http_code}}\n")])
            .arg(&url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.unbounded_send(Err(format!("failed to launch curl: {e}")));
                return;
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(&payload) {
                let _ = tx.unbounded_send(Err(format!("failed to write request body: {e}")));
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }

        let Some(stdout) = child.stdout.take() else {
            let _ = tx.unbounded_send(Err("missing curl stdout".to_string()));
            let _ = child.wait();
            return;
        };

        let mut reader = StdBufReader::new(stdout);
        let mut line = String::new();
        let mut http_status: Option<u16> = None;
        let mut raw_body = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if let Some(code_str) = line.trim().strip_prefix(STATUS_MARKER) {
                        http_status = code_str.trim().parse::<u16>().ok();
                        continue;
                    }
                    if raw_body.len() < 4096 {
                        raw_body.push_str(&line);
                    }
                    if tx.unbounded_send(Ok(Some(line.clone()))).is_err() {
                        let _ = child.kill();
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.unbounded_send(Err(format!("stream read error: {e}")));
                    break;
                }
            }
        }

        let output = child.wait_with_output();
        if let Ok(out) = output {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let msg = if stderr.is_empty() {
                    format!("curl exited with {}", out.status)
                } else {
                    stderr
                };
                let _ = tx.unbounded_send(Err(msg));
                return;
            }
        }
        if let Some(code) = http_status {
            if code >= 400 {
                let _ = tx.unbounded_send(Err(format!("HTTP {code}: {}", raw_body.trim())));
                return;
            }
        }
        let _ = tx.unbounded_send(Ok(None));
    });

    let mut parser = SseStreamParser::default();
    while let Some(item) = rx.next().await {
        match item? {
            None => break,
            Some(line) => match parser.feed_line(&line)? {
                Some(SseEvent::TextDelta(delta)) => on_delta(&delta),
                Some(SseEvent::Done) => break,
                None => {}
            },
        }
    }
    Ok(parser.into_outcome())
}

#[cfg(target_family = "wasm")]
pub async fn stream_chat_completion(
    _config: &AiConfig,
    _messages: &[ChatMessage],
    _tools: Vec<ToolDefinition>,
    _on_delta: impl FnMut(&str),
) -> Result<CompletionOutcome, String> {
    Err("Assistant is not supported in browser builds yet".to_string())
}

#[cfg(not(target_family = "wasm"))]
fn percent_encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

/// Fetch a Japanese statute XML from e-Gov 法令 API v2 (`laws.e-gov.go.jp/api/2`).
/// Returns `(law_id, law_title, law_xml)`.
pub fn fetch_egov_law_xml(
    law_title: Option<&str>,
    law_num: Option<&str>,
    law_id: Option<&str>,
) -> Result<(String, String, String), String> {
    #[cfg(target_family = "wasm")]
    {
        let _ = (law_title, law_num, law_id);
        return Err("e-Gov 法令 API からの取得はデスクトップ版でのみ利用可能です。".to_string());
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let mut resolved_id = law_id.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        let mut resolved_title = law_title.unwrap_or("").to_string();

        if resolved_id.is_none() {
            let mut params = Vec::new();
            if let Some(t) = law_title.map(str::trim).filter(|s| !s.is_empty()) {
                params.push(format!("law_title={}", percent_encode_component(t)));
            }
            if let Some(n) = law_num.map(str::trim).filter(|s| !s.is_empty()) {
                params.push(format!("law_num={}", percent_encode_component(n)));
            }
            if params.is_empty() {
                return Err("law_title, law_num, または law_id のいずれかを指定してください。".to_string());
            }
            params.push("limit=5".to_string());
            let search_url = format!("https://laws.e-gov.go.jp/api/2/laws?{}", params.join("&"));
            let out = std::process::Command::new("curl")
                .args(["-sSL", "--fail", "--max-time", "15", &search_url])
                .output()
                .map_err(|e| format!("e-Gov API 呼び出しに失敗しました: {e}"))?;
            if !out.status.success() {
                return Err(format!("e-Gov 法令検索に失敗しました (status={})", out.status));
            }
            let body: Value = serde_json::from_slice(&out.stdout)
                .map_err(|e| format!("e-Gov API 応答の JSON 解析に失敗しました: {e}"))?;
            let first = body
                .get("laws")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .ok_or_else(|| "該当する法令が見つかりませんでした。".to_string())?;
            let id = first
                .pointer("/law_info/law_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "law_id が取得できませんでした。".to_string())?;
            resolved_id = Some(id.to_string());
            if let Some(t) = first
                .pointer("/current_revision_info/law_title")
                .and_then(|v| v.as_str())
            {
                resolved_title = t.to_string();
            }
        }

        let id = resolved_id.unwrap();
        let data_url = format!(
            "https://laws.e-gov.go.jp/api/2/law_data/{}?response_format=xml",
            percent_encode_component(&id)
        );
        let out = std::process::Command::new("curl")
            .args(["-sSL", "--fail", "--max-time", "20", &data_url])
            .output()
            .map_err(|e| format!("e-Gov 法令データ取得に失敗しました: {e}"))?;
        if !out.status.success() {
            return Err(format!("e-Gov 法令データ取得に失敗しました (status={})", out.status));
        }
        let raw = String::from_utf8(out.stdout)
            .map_err(|_| "e-Gov API 応答が有効な UTF-8 ではありません".to_string())?;
        let xml = if raw.trim_start().starts_with('{') {
            let v: Value = serde_json::from_str(&raw)
                .map_err(|e| format!("e-Gov 法令データ JSON 解析に失敗しました: {e}"))?;
            v.get("law_full_text")
                .and_then(|x| x.as_str())
                .map(str::to_string)
                .ok_or_else(|| "law_full_text が応答に含まれていません。".to_string())?
        } else {
            raw
        };
        Ok((id, resolved_title, xml))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_streaming_text_and_tool_calls() {
        let mut parser = SseStreamParser::default();
        assert_eq!(
            parser
                .feed_line(r#"data: {"choices":[{"delta":{"content":"Hello "},"finish_reason":null}]}"#)
                .unwrap(),
            Some(SseEvent::TextDelta("Hello ".to_string()))
        );
        assert_eq!(
            parser
                .feed_line(r#"data: {"choices":[{"delta":{"content":"world"},"finish_reason":null}]}"#)
                .unwrap(),
            Some(SseEvent::TextDelta("world".to_string()))
        );
        assert_eq!(
            parser
                .feed_line(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"set_cells","arguments":"{\"edits\":"}}]},"finish_reason":null}]}"#)
                .unwrap(),
            None
        );
        assert_eq!(
            parser
                .feed_line(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"[]}"}}]},"finish_reason":"tool_calls"}]}"#)
                .unwrap(),
            None
        );
        assert_eq!(parser.feed_line("data: [DONE]").unwrap(), Some(SseEvent::Done));

        let outcome = parser.into_outcome();
        assert_eq!(outcome.content, "Hello world");
        assert_eq!(outcome.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(outcome.tool_calls.len(), 1);
        assert_eq!(outcome.tool_calls[0].id, "call_1");
        assert_eq!(outcome.tool_calls[0].function.name, "set_cells");
        assert_eq!(outcome.tool_calls[0].function.arguments, r#"{"edits":[]}"#);
    }

    #[test]
    fn surfaces_sse_error_envelope() {
        let mut parser = SseStreamParser::default();
        let err = parser
            .feed_line(r#"data: {"error":{"message":"invalid api key"}}"#)
            .unwrap_err();
        assert_eq!(err, "invalid api key");
    }
}
