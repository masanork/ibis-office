// SPDX-License-Identifier: AGPL-3.0-only
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use futures::{StreamExt, channel::mpsc as async_channel};
use gpui_kit::{Context, Window};
use serde_json::{Value, json};

use crate::editor::SlideEditor;

const MAX_LINE: u64 = 256 * 1024;
const MAX_EVENTS: usize = 256;
const MAX_RUNS: usize = 32;
const MAX_EVENT_BYTES: usize = 16 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(3);

type Request = (Value, Instant, mpsc::Sender<Value>);

pub struct SocketLifetime {
    path: PathBuf,
    closed: Arc<AtomicBool>,
}

impl Drop for SocketLifetime {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        let _ = fs::remove_file(&self.path);
    }
}

pub struct BoundSocket {
    listener: UnixListener,
    lifetime: SocketLifetime,
}

impl BoundSocket {
    pub fn bind(path: &Path) -> io::Result<Self> {
        let parent = path
            .parent()
            .filter(|_| path.is_absolute())
            .ok_or_else(|| io::Error::other("provide an absolute socket path"))?;
        if fs::metadata(parent)?.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::other(
                "the socket directory must be private (mode 0700)",
            ));
        }
        // Never replace an existing connection endpoint.
        let listener = UnixListener::bind(path)?;
        let lifetime = SocketLifetime {
            path: path.to_owned(),
            closed: Arc::new(AtomicBool::new(false)),
        };
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, lifetime })
    }

    pub fn start(self, editor: &mut SlideEditor, window: &Window, cx: &mut Context<SlideEditor>) {
        let (output, events) = mpsc::channel();
        let (requests, mut inbox) = async_channel::unbounded::<Request>();
        editor.assistant_output = Some(output);
        let Self { listener, lifetime } = self;
        let closed = lifetime.closed.clone();
        editor.assistant_socket = Some(lifetime);

        thread::spawn(move || {
            let mut journal = Journal::default();
            while !closed.load(Ordering::Acquire) {
                journal.record(events.try_iter());
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = respond(stream, &requests, &mut journal, &events);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        });
        cx.spawn_in(window, async move |editor, cx| {
            while let Some((request, deadline, answer)) = inbox.next().await {
                let response = editor.update_in(cx, |editor, window, cx| {
                    if Instant::now() >= deadline {
                        return json!({"error":"Assistant request expired"});
                    }
                    if request["op"] == "status" {
                        return editor.assistant_connection_status();
                    }
                    if request["op"] == "apply" {
                        let Some(run_id) = request["run_id"].as_u64() else {
                            return json!({"error":"run_id is required"});
                        };
                        return match editor.apply_assistant_socket_review(run_id, cx) {
                            Ok(()) => json!({"run_id":run_id,"state":"applied"}),
                            Err(message) => json!({"error":message}),
                        };
                    }
                    if request["op"] == "prompt"
                        && let Some(text) = request["text"].as_str()
                    {
                        return match editor.submit_assistant_socket_prompt(
                            text.to_owned(),
                            window,
                            cx,
                        ) {
                            Ok(run_id) => json!({"run_id":run_id}),
                            Err(message) => json!({"error":message}),
                        };
                    }
                    json!({"error":"unsupported Assistant request"})
                });
                let Ok(response) = response else { break };
                let _ = answer.send(response);
            }
        })
        .detach();
    }
}

#[derive(Default)]
struct Journal {
    cursor: u64,
    events: VecDeque<Value>,
    runs: BTreeMap<u64, Value>,
}

impl Journal {
    fn record(&mut self, events: impl Iterator<Item = Value>) {
        for mut event in events {
            self.cursor += 1;
            if event.to_string().len() > MAX_EVENT_BYTES {
                event = json!({
                    "event":event["event"], "run_id":event["run_id"], "truncated":true,
                });
            }
            if event["event"] == "reset" {
                for summary in self.runs.values_mut() {
                    if matches!(
                        summary["state"].as_str(),
                        Some("running" | "awaiting_review")
                    ) {
                        *summary = json!({"state":"reset","event":event});
                    }
                }
            }
            if let Some(run) = event["run_id"].as_u64() {
                let state = match event["event"].as_str() {
                    Some("started") => Some("running"),
                    Some("review") => Some("awaiting_review"),
                    Some("completed") if event["review"] != true => Some("completed"),
                    Some("error") => Some("error"),
                    Some("stopped") => Some("stopped"),
                    Some("review_resolved") => event["outcome"].as_str(),
                    _ => None,
                };
                if let Some(state) = state {
                    self.runs.insert(run, json!({"state":state,"event":event}));
                    if self.runs.len() > MAX_RUNS {
                        self.runs.pop_first();
                    }
                }
            }
            event["sequence"] = json!(self.cursor);
            if self.events.len() == MAX_EVENTS {
                self.events.pop_front();
            }
            self.events.push_back(event);
        }
    }

    fn result(&self, request: &Value) -> Value {
        let Some(run_id) = request["run_id"].as_u64() else {
            return json!({"error":"run_id is required"});
        };
        let after = request["after"].as_u64().unwrap_or(0);
        let first = self
            .events
            .front()
            .and_then(|event| event["sequence"].as_u64())
            .unwrap_or(1);
        let events: Vec<_> = self
            .events
            .iter()
            .filter(|event| {
                event["run_id"].as_u64() == Some(run_id)
                    && event["sequence"]
                        .as_u64()
                        .is_some_and(|sequence| sequence > after)
            })
            .collect();
        json!({"run_id":run_id,"summary":self.runs.get(&run_id),"events":events,"cursor":self.cursor,"history_truncated":after < first - 1})
    }
}

fn respond(
    mut stream: UnixStream,
    requests: &async_channel::UnboundedSender<Request>,
    journal: &mut Journal,
    events: &mpsc::Receiver<Value>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut bytes = Vec::new();
    BufReader::new(&stream)
        .take(MAX_LINE + 1)
        .read_until(b'\n', &mut bytes)?;
    if bytes.len() as u64 > MAX_LINE {
        return Err(io::Error::other("Assistant request is too long"));
    }
    let request: Value = serde_json::from_slice(&bytes)?;
    let response = if request["op"] == "result" {
        journal.record(events.try_iter());
        journal.result(&request)
    } else {
        let (tx, rx) = mpsc::channel();
        requests
            .unbounded_send((request, Instant::now() + IO_TIMEOUT, tx))
            .map_err(io::Error::other)?;
        rx.recv_timeout(IO_TIMEOUT).map_err(io::Error::other)?
    };
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_review_outcome_survives_event_stream_rollover() {
        let mut journal = Journal::default();
        journal.record(
            [
                json!({"event":"started","run_id":0}),
                json!({"event":"review","run_id":0,"preview":"A1"}),
                json!({"event":"review_resolved","run_id":0,"outcome":"applied"}),
            ]
            .into_iter(),
        );
        journal.record((0..MAX_EVENTS).map(|_| json!({"event":"delta","run_id":1})));
        let result = journal.result(&json!({"run_id":0}));
        assert_eq!(result["summary"]["state"], "applied");
        assert_eq!(result["events"], json!([]));
        assert_eq!(result["history_truncated"], true);

        journal.record(
            [
                json!({"event":"review","run_id":1}),
                json!({"event":"reset"}),
            ]
            .into_iter(),
        );
        assert_eq!(
            journal.result(&json!({"run_id":1}))["summary"]["state"],
            "reset"
        );
        assert_eq!(
            journal.result(&json!({"run_id":0}))["summary"]["state"],
            "applied"
        );
    }
}
