// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! JSON Lines transport for the Assistant in a single GUI window.

use std::io::{self, BufRead, Read, Write};
use std::sync::mpsc;

use futures::{StreamExt, channel::mpsc as async_channel};
use gpui_kit::{Context, Window};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::editor::SlideEditor;

const MAX_LINE_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
struct Request {
    id: Value,
    #[serde(flatten)]
    command: Command,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Command {
    Open,
    SetInput { text: String },
    Send,
    Prompt { text: String },
    Stop { run_id: u64 },
    Apply { run_id: u64 },
    Discard { run_id: u64 },
}

pub fn start(
    editor: &mut SlideEditor,
    window: &Window,
    loading: bool,
    cx: &mut Context<SlideEditor>,
) {
    let (output, events) = mpsc::channel();
    let (requests, mut inbox) = async_channel::unbounded::<(Vec<u8>, mpsc::Sender<()>)>();
    editor.assistant_output = Some(output.clone());
    std::thread::spawn(move || {
        for event in events {
            let mut stdout = io::stdout().lock();
            if writeln!(stdout, "{event}")
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
        }
    });
    let input_errors = output.clone();
    let _ = output.send(json!({"event":"ready", "version":1, "loading":loading}));
    std::thread::spawn(move || {
        if let Err(error) = read_requests(requests) {
            let _ =
                input_errors.send(json!({"event":"protocol_error", "message":error.to_string()}));
        }
    });

    cx.spawn_in(window, async move |editor, cx| {
        while let Some((line, received)) = inbox.next().await {
            if editor
                .update_in(cx, |editor, window, cx| {
                    let response = match serde_json::from_slice::<Request>(&line) {
                        Ok(request) => match dispatch(editor, request.command, window, cx) {
                            Ok(None) => json!({"id":request.id, "ok":true}),
                            Ok(Some(run_id)) => {
                                json!({"id":request.id, "ok":true, "run_id":run_id})
                            }
                            Err(message) => json!({"id":request.id, "ok":false, "error":message}),
                        },
                        Err(error) => {
                            let id = serde_json::from_slice::<Value>(&line)
                                .ok()
                                .and_then(|value| value.get("id").cloned());
                            json!({"id":id, "ok":false, "error":error.to_string()})
                        }
                    };
                    editor.emit_assistant_event(response);
                })
                .is_err()
            {
                break;
            }
            let _ = received.send(());
        }
    })
    .detach();
}

fn read_requests(
    requests: async_channel::UnboundedSender<(Vec<u8>, mpsc::Sender<()>)>,
) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    loop {
        let mut line = Vec::new();
        let read = stdin
            .by_ref()
            .take((MAX_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(());
        }
        if read > MAX_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "assistant request exceeds 256 KiB",
            ));
        }
        let (received, acknowledged) = mpsc::channel();
        if requests.unbounded_send((line, received)).is_err() || acknowledged.recv().is_err() {
            return Ok(());
        }
    }
}

fn dispatch(
    editor: &mut SlideEditor,
    command: Command,
    window: &mut Window,
    cx: &mut Context<SlideEditor>,
) -> Result<Option<u64>, &'static str> {
    match command {
        Command::Open => editor.open_assistant_stdio(window, cx).map(|()| None),
        Command::SetInput { text } => editor
            .set_assistant_stdio_input(text, window, cx)
            .map(|()| None),
        Command::Send => editor.send_assistant_stdio(window, cx).map(Some),
        Command::Prompt { text } => editor
            .submit_assistant_stdio_prompt(text, window, cx)
            .map(Some),
        Command::Stop { run_id } => {
            editor.stop_assistant_stdio(run_id, cx)?;
            Ok(Some(run_id))
        }
        Command::Apply { run_id } | Command::Discard { run_id } => {
            editor.review_assistant_stdio(run_id, matches!(command, Command::Apply { .. }), cx)?;
            Ok(Some(run_id))
        }
    }
}
