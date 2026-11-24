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
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::assistant_workspace::{SOCKET_NAME, Workspace};

const MAX_LINE: u64 = 256 * 1024;
const MAX_EVENTS: usize = 256;
const MAX_RUNS: usize = 32;
const MAX_QUEUED: usize = 16;
const RETRY_INTERVAL: Duration = Duration::from_millis(100);
const MAX_EVENT_BYTES: usize = 16 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(3);

pub fn next_run_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub type Request = (Value, Instant, mpsc::Sender<Value>);

pub struct SocketLifetime {
    path: PathBuf,
    closed: Arc<AtomicBool>,
    registration: Option<(PathBuf, tempfile::TempDir)>,
}

impl SocketLifetime {
    pub fn includes_document(&self, document: Option<&Path>) -> bool {
        self.registration.as_ref().is_none_or(|(root, _)| {
            document
                .and_then(|path| path.canonicalize().ok())
                .is_some_and(|path| path.starts_with(root))
        })
    }
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
            registration: None,
        };
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, lifetime })
    }

    pub fn bind_workspace(path: &Path) -> io::Result<Self> {
        let workspace = Workspace::open(path)?;
        let registration = workspace.register()?;
        let mut socket = Self::bind(&registration.path().join(SOCKET_NAME))?;
        socket.lifetime.registration = Some((workspace.root().to_owned(), registration));
        Ok(socket)
    }

    pub fn start(
        self,
        queue_operation: &'static str,
        mut dispatch: impl FnMut(Request, &mpsc::Sender<Value>) -> io::Result<()> + Send + 'static,
    ) -> (SocketLifetime, mpsc::Sender<Value>, thread::JoinHandle<()>) {
        let (output, events) = mpsc::channel();
        let output_for_dispatch = output.clone();
        let Self { listener, lifetime } = self;
        let closed = lifetime.closed.clone();
        let worker = thread::spawn(move || {
            let mut journal = Journal::default();
            let mut inbox = Inbox::default();
            let mut forward = |request| dispatch(request, &output_for_dispatch);
            while !closed.load(Ordering::Acquire) {
                journal.record(events.try_iter());
                inbox.advance(&mut forward, &mut journal);
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = respond(
                            stream,
                            &mut forward,
                            &mut journal,
                            &events,
                            &mut inbox,
                            queue_operation,
                        );
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        });
        (lifetime, output, worker)
    }
}

pub fn start_demo(workspace: &Path) -> io::Result<(SocketLifetime, thread::JoinHandle<()>)> {
    let socket = BoundSocket::bind_workspace(workspace)?;
    let (lifetime, _events, worker) = socket.start("prompt", move |(request, _, answer), events| {
        let response = match request["op"].as_str() {
            Some("status") => json!({
                "kind":"headless","label":"Assistant demo","path":null,"document_ready":false,"streaming":false,
                "operations":["assistant_status","assistant_prompt","assistant_result"],
            }),
            Some("prompt") if request["text"].as_str().is_some_and(|text| !text.trim().is_empty())
                && request.get("skill").is_none()
                && matches!(request["harness"].as_str(), None | Some("builtin")) =>
            {
                if request["queue_phase"] == "admit" {
                    return answer.send(json!({"accepted":true})).map_err(io::Error::other);
                }
                let Some(run_id) = request["run_id"].as_u64() else {
                    return answer.send(json!({"error":"run_id is required"})).map_err(io::Error::other);
                };
                for event in [
                    json!({"event":"started","run_id":run_id}),
                    json!({"event":"message","run_id":run_id,"text":"Headless Assistant demo response."}),
                    json!({"event":"completed","run_id":run_id,"review":false}),
                ] {
                    events.send(event).map_err(io::Error::other)?;
                }
                json!({"run_id":run_id})
            }
            _ => json!({"error":"unsupported Headless Assistant demo request"}),
        };
        answer.send(response).map_err(io::Error::other)
    });
    Ok((lifetime, worker))
}

pub fn run_demo(workspace: &Path) -> io::Result<()> {
    let (_lifetime, worker) = start_demo(workspace)?;
    worker
        .join()
        .map_err(|_| io::Error::other("Assistant endpoint stopped unexpectedly"))
}

#[derive(Default)]
struct Inbox {
    pending: VecDeque<Value>,
    retry_after: Option<Instant>,
}

fn forward(
    request: Value,
    dispatch: &mut impl FnMut(Request) -> io::Result<()>,
) -> io::Result<Value> {
    let (tx, rx) = mpsc::channel();
    dispatch((request, Instant::now() + IO_TIMEOUT, tx))?;
    rx.recv_timeout(IO_TIMEOUT).map_err(io::Error::other)
}

impl Inbox {
    fn enqueue(
        &mut self,
        mut request: Value,
        dispatch: &mut impl FnMut(Request) -> io::Result<()>,
        journal: &mut Journal,
    ) -> io::Result<Value> {
        if self.pending.len() >= MAX_QUEUED {
            return Ok(json!({"error":"Assistant inbox is full"}));
        }
        request["queue_phase"] = json!("admit");
        let admission = forward(request.clone(), dispatch)?;
        if admission["accepted"] != true {
            return Ok(admission);
        }
        let run_id = next_run_id();
        request["run_id"] = json!(run_id);
        request["queue_phase"] = json!("start");
        request["queue_target"] = admission["target"].clone();
        self.pending.push_back(request);
        journal.record(std::iter::once(json!({"event":"queued","run_id":run_id})));
        Ok(json!({"run_id":run_id,"state":"queued"}))
    }

    fn advance(
        &mut self,
        dispatch: &mut impl FnMut(Request) -> io::Result<()>,
        journal: &mut Journal,
    ) {
        if self
            .retry_after
            .is_some_and(|deadline| deadline > Instant::now())
        {
            return;
        }
        if let Some(request) = self.pending.front() {
            let run_id = request["run_id"].clone();
            match forward(request.clone(), dispatch) {
                Ok(response) if response["retry"] == true => {
                    self.retry_after = Some(Instant::now() + RETRY_INTERVAL);
                    return;
                }
                Ok(response) if response["run_id"] == run_id => {}
                response => {
                    let message = response
                        .as_ref()
                        .ok()
                        .and_then(|value| value["error"].as_str())
                        .unwrap_or("Assistant could not start the queued request");
                    journal.record(std::iter::once(
                        json!({"event":"error","run_id":run_id,"message":message}),
                    ));
                }
            }
            self.retry_after = None;
            self.pending.pop_front();
        }
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
            event["sequence"] = json!(self.cursor);
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
                    Some("queued") => Some("queued"),
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
                        let oldest = self
                            .runs
                            .iter()
                            .filter(|(_, value)| {
                                !matches!(
                                    value["state"].as_str(),
                                    Some("queued" | "running" | "awaiting_review")
                                )
                            })
                            .min_by_key(|(_, value)| value["event"]["sequence"].as_u64())
                            .map(|(&id, _)| id);
                        if let Some(id) = oldest {
                            self.runs.remove(&id);
                        }
                    }
                }
            }
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
    dispatch: &mut impl FnMut(Request) -> io::Result<()>,
    journal: &mut Journal,
    events: &mpsc::Receiver<Value>,
    inbox: &mut Inbox,
    queue_operation: &str,
) -> io::Result<()> {
    // The listener is nonblocking. Accepted sockets may inherit that mode.
    stream.set_nonblocking(false)?;
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
    } else if request["op"] == queue_operation {
        inbox.enqueue(request, dispatch, journal)?
    } else {
        let status = request["op"] == "status";
        let mut response = forward(request, dispatch)?;
        if status && response.is_object() && response.get("error").is_none() {
            response["queued_count"] = json!(inbox.pending.len());
        }
        response
    };
    let mut bytes = serde_json::to_vec(&response)?;
    bytes.push(b'\n');
    stream.write_all(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonblocking_listener_can_return_a_large_analysis_response() {
        let (server, mut client) = UnixStream::pair().unwrap();
        server.set_nonblocking(true).unwrap();
        client.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        let evidence = "集計根拠\n".repeat(12_000);
        let expected = json!({"evidence":evidence});
        let response = expected.clone();
        let worker = thread::spawn(move || {
            let (_output, events) = mpsc::channel();
            let mut dispatch = |(_, _, answer): Request| {
                answer.send(response.clone()).map_err(io::Error::other)
            };
            respond(
                server, &mut dispatch, &mut Journal::default(), &events,
                &mut Inbox::default(), "prompt",
            ).unwrap();
        });
        client.write_all(b"{\"op\":\"analysis_preview_diff\"}\n").unwrap();
        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), expected);
        worker.join().unwrap();
    }

    #[test]
    fn pending_runs_survive_rollover_and_full_inbox_does_not_admit_more() {
        let mut inbox = Inbox::default();
        let mut journal = Journal::default();
        let mut admit = |(_, _, answer): Request| {
            answer
                .send(json!({"accepted":true}))
                .map_err(io::Error::other)
        };
        let mut ids = Vec::new();
        for _ in 0..MAX_QUEUED {
            let value = inbox
                .enqueue(
                    json!({"op":"prompt","text":"test"}),
                    &mut admit,
                    &mut journal,
                )
                .unwrap();
            ids.push(value["run_id"].as_u64().unwrap());
        }
        assert_eq!(
            inbox
                .enqueue(json!({"op":"prompt"}), &mut admit, &mut journal)
                .unwrap()["error"],
            "Assistant inbox is full"
        );
        journal.record(
            (0..MAX_RUNS as u64).map(|i| json!({"event":"completed","run_id":i+1_000_000})),
        );
        assert!(
            ids.iter()
                .all(|id| journal.result(&json!({"run_id":id}))["summary"]["state"] == "queued")
        );
    }

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
