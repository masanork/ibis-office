// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::DirBuilderExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use serde_json::{Value, json};

const MAX_MESSAGE: usize = 1024 * 1024;

#[derive(Default)]
pub(crate) struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

impl Drop for Cancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn line(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    let count = reader
        .take(MAX_MESSAGE as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if count == 0 {
        return Ok(None);
    }
    if count > MAX_MESSAGE {
        return Err("protocol message exceeds 1 MiB".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn send(writer: &mut impl Write, value: Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, &value).map_err(|e| e.to_string())?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|e| e.to_string())
}

type Work = Box<dyn FnOnce(&mut Option<Session>) + Send>;

pub(crate) struct Conversation(mpsc::Sender<Work>);

impl Conversation {
    pub(crate) fn start() -> Result<Self, String> {
        let (send, receive) = mpsc::channel::<Work>();
        // Draft operations can exceed the GPUI worker stack.
        std::thread::Builder::new()
            .name("assistant-acp".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let mut session: Option<Session> = None;
                loop {
                    if let Some(active) = session.as_mut()
                        && !active.serve_idle()
                    {
                        session = None;
                    }
                    match receive.recv_timeout(Duration::from_millis(20)) {
                        Ok(work) => {
                            if let Some(active) = session.as_mut()
                                && !active.serve_idle()
                            {
                                session = None;
                            }
                            work(&mut session);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map_err(|error| format!("Could not start the ACP worker: {error}"))?;
        Ok(Self(send))
    }

    pub(crate) fn submit<T: Send + 'static>(
        &self,
        tools: Value,
        work: impl FnOnce(&mut Session) -> Result<T, String> + Send + 'static,
    ) -> Result<futures::channel::oneshot::Receiver<Result<T, String>>, String> {
        let (finished, outcome) = futures::channel::oneshot::channel();
        self.0
            .send(Box::new(move |session| {
                let result = (|| {
                    if session.is_none() {
                        *session = Some(Session::start(tools.clone())?);
                    }
                    let active = session.as_mut().expect("ACP session exists");
                    active.tools = tools;
                    work(active)
                })();
                if result.is_err() {
                    *session = None;
                }
                let _ = finished.send(result);
            }))
            .map_err(|_| "The ACP worker stopped unexpectedly.".to_owned())?;
        Ok(outcome)
    }
}

pub(crate) struct Session {
    child: Child,
    input: ChildStdin,
    output: Option<mpsc::Receiver<Result<Value, String>>>,
    dir: Directory,
    socket: PathBuf,
    listener: UnixListener,
    id: Option<String>,
    next_request: u64,
    tools: Value,
}

impl Drop for Session {
    fn drop(&mut self) {
        drop(self.output.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        // An agent subprocess could still have inherited stdout.
        // Detach the reader rather than waiting for all inherited descriptors.
    }
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}

impl Session {
    fn start(tools: Value) -> Result<Self, String> {
        let mut random = [0_u8; 12];
        getrandom::fill(&mut random).map_err(|e| e.to_string())?;
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = std::env::temp_dir().join(format!("ibis-acp-{suffix}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|e| e.to_string())?;
        let dir = Directory(path);
        let socket = dir.0.join("mcp");
        let listener = UnixListener::bind(&socket).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let mut command = if let Some(executable) = std::env::var_os("IBIS_ACP_COMMAND") {
            let args = std::env::var("IBIS_ACP_ARGS").unwrap_or_else(|_| "[]".into());
            let args: Vec<String> = serde_json::from_str(&args)
                .map_err(|error| format!("Invalid IBIS_ACP_ARGS: {error}"))?;
            let mut command = Command::new(executable);
            command.args(args);
            command
        } else {
            let mut command = Command::new(
                std::env::var_os("IBIS_OPENCODE").unwrap_or_else(|| "opencode".into()),
            );
            command.arg("acp").env(
                "OPENCODE_PERMISSION",
                r#"{"*":"deny","workbook_*":"allow"}"#,
            );
            command
        };
        let mut child = command
            .current_dir(&dir.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not start ACP agent: {e}"))?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::sync_channel(32);
        std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                match line(&mut output) {
                    Ok(Some(message)) => {
                        if tx.send(Ok(message)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });
        let mut session = Self {
            child,
            input,
            output: Some(rx),
            dir,
            socket,
            listener,
            id: None,
            next_request: 3,
            tools,
        };
        send(
            &mut session.input,
            request(
                1,
                "initialize",
                json!({"protocolVersion":1,"clientCapabilities":{}}),
            ),
        )?;
        Ok(session)
    }

    fn serve_tool(&self, run: impl FnOnce(Value) -> Result<Value, String>) {
        let Ok((mut connection, _)) = self.listener.accept() else {
            return;
        };
        let timeout = Some(Duration::from_millis(500));
        let _ = connection.set_read_timeout(timeout);
        let _ = connection.set_write_timeout(timeout);
        if let Ok(Some(call)) = line(&mut BufReader::new(&connection)) {
            let result = if call["list"] == true {
                Ok(self.tools.clone())
            } else {
                run(call)
            };
            let response = match result {
                Ok(value) => json!({"value":value}),
                Err(error) => json!({"error":error}),
            };
            let _ = send(&mut connection, response);
        }
    }

    fn serve_idle(&mut self) -> bool {
        self.serve_tool(|_| Err("No active document turn".into()));
        let output = self.output.as_ref().expect("ACP output is open");
        for _ in 0..32 {
            match output.try_recv() {
                Ok(Ok(message)) => {
                    if let (Some(id), Some(_)) = (message.get("id"), message["method"].as_str()) {
                        let reply = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"No active document turn"}});
                        if send(&mut self.input, reply).is_err() {
                            return false;
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return true,
                _ => return false,
            }
        }
        true
    }

    fn send_prompt(&mut self, request_id: u64, prompt: &str) -> Result<(), String> {
        send(
            &mut self.input,
            request(
                request_id,
                "session/prompt",
                json!({"sessionId":self.id,"prompt":[{"type":"text","text":prompt}]}),
            ),
        )
    }

    pub(crate) fn prompt(
        &mut self,
        prompt: &str,
        cancelled: Arc<AtomicBool>,
        mut on_text: impl FnMut(&str),
        mut tool: impl FnMut(&str, Value) -> Result<Value, String>,
    ) -> Result<String, String> {
        let request_id = self.next_request;
        self.next_request += 1;
        if self.id.is_some() && !cancelled.load(Ordering::Relaxed) {
            self.send_prompt(request_id, prompt)?;
        }
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut text = String::new();
        let mut stop = None;
        loop {
            if cancelled.load(Ordering::Relaxed) && stop.is_none() {
                if let Some(id) = &self.id {
                    let _ = send(
                        &mut self.input,
                        json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":id}}),
                    );
                    stop = Some(Instant::now() + Duration::from_millis(600));
                } else {
                    return Err("ACP run cancelled".into());
                }
            }
            if stop.is_some_and(|stop| Instant::now() >= stop) {
                return Err("ACP run cancelled".into());
            }
            if Instant::now() >= deadline {
                return Err("ACP run timed out".into());
            }
            self.serve_tool(|call| {
                if stop.is_some() || self.id.is_none() {
                    return Err("No active document turn".into());
                }
                tool(
                    call["name"].as_str().unwrap_or_default(),
                    call.get("arguments").cloned().unwrap_or_else(|| json!({})),
                )
            });
            let message = match self
                .output
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_millis(20))
            {
                Ok(result) => result?,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err("ACP agent disconnected".into()),
            };
            if let Some(method) = message["method"].as_str() {
                if method == "session/update" {
                    if self.id.as_deref() != message["params"]["sessionId"].as_str()
                        || self.id.is_none()
                    {
                        continue;
                    }
                    let update = &message["params"]["update"];
                    if update["sessionUpdate"] == "agent_message_chunk"
                        && let Some(chunk) = update["content"]["text"].as_str()
                    {
                        text.push_str(chunk);
                        if text.len() > MAX_MESSAGE {
                            return Err("ACP response exceeds 1 MiB".into());
                        }
                        if stop.is_none() {
                            on_text(chunk);
                        }
                    }
                } else if let Some(id) = message.get("id") {
                    let reply = if method == "session/request_permission" {
                        json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}})
                    } else {
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unsupported client operation"}})
                    };
                    send(&mut self.input, reply)?;
                }
                continue;
            }
            if let Some(error) = message["error"]["message"].as_str() {
                return Err(error.to_owned());
            }
            match message["id"].as_u64() {
                Some(1) if self.id.is_none() && message["result"]["protocolVersion"] == 1 => send(
                    &mut self.input,
                    request(
                        2,
                        "session/new",
                        json!({"cwd":self.dir.0,"mcpServers":[{"name":"workbook","command":std::env::current_exe().map_err(|error| error.to_string())?,"args":["--assistant-acp-workbook",self.socket],"env":[]}]}),
                    ),
                )?,
                Some(1) => return Err("ACP agent did not negotiate ACP v1".into()),
                Some(2) if self.id.is_none() => {
                    let id = message["result"]["sessionId"]
                        .as_str()
                        .ok_or("ACP agent returned no session ID")?;
                    self.id = Some(id.to_owned());
                    self.send_prompt(request_id, prompt)?;
                }
                Some(id)
                    if id == request_id
                        && stop.is_none()
                        && message["result"]["stopReason"] == "end_turn" =>
                {
                    return Ok(text);
                }
                Some(id) if id == request_id => {
                    return Err(format!(
                        "ACP agent ended without a completed turn: {}",
                        message["result"]["stopReason"]
                    ));
                }
                _ => {}
            }
        }
    }
}

fn request_tool(socket: &Path, call: Value) -> Result<Value, String> {
    let mut connection = UnixStream::connect(socket).map_err(|e| e.to_string())?;
    let timeout = Some(Duration::from_secs(3));
    connection
        .set_read_timeout(timeout)
        .and_then(|_| connection.set_write_timeout(timeout))
        .map_err(|e| e.to_string())?;
    send(&mut connection, call)?;
    line(&mut BufReader::new(connection))?.ok_or_else(|| "Document turn ended".into())
}

pub(crate) fn document_schema(kind: ibis_formats::Kind, read_only: bool) -> Vec<Value> {
    ibis_doc_service::document_tools(kind)
        .filter(|tool| !read_only || tool.access == ibis_doc_service::Access::Read)
        .map(|tool| {
            let mut schema = (tool.schema)();
            if let Some(properties) = schema["properties"].as_object_mut() {
                properties.remove("document_id");
            }
            if let Some(required) = schema["required"].as_array_mut() {
                required.retain(|key| key != "document_id");
            }
            json!({"name":tool.name,"description":tool.description,"inputSchema":schema,
            "annotations":{"readOnlyHint":tool.access == ibis_doc_service::Access::Read}})
        })
        .collect()
}

/// Stdio MCP endpoint for one private ACP document turn.
pub fn workbook_mcp(socket: &Path) -> Result<(), String> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Some(request) = line(&mut input)? {
        let Some(id) = request.get("id") else {
            continue;
        };
        let result = match request["method"].as_str() {
            Some("initialize") => json!({
                "protocolVersion": request["params"]["protocolVersion"].as_str().unwrap_or("2024-11-05"),
                "capabilities":{"tools":{}},"serverInfo":{"name":"ibisOffice document draft","version":"0.1"}
            }),
            Some("ping") => json!({}),
            Some(method @ ("tools/list" | "tools/call")) => {
                let call = if method == "tools/list" {
                    json!({"list":true})
                } else {
                    request["params"].clone()
                };
                let answer = request_tool(socket, call);
                if method == "tools/list" {
                    match answer {
                        Ok(value) if value.get("error").is_none() => {
                            send(
                                &mut output,
                                json!({"jsonrpc":"2.0","id":id,"result":{"tools":value["value"]}}),
                            )?;
                        }
                        Ok(value) => send(
                            &mut output,
                            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":value["error"]}}),
                        )?,
                        Err(error) => send(
                            &mut output,
                            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":error}}),
                        )?,
                    }
                    continue;
                }
                match answer {
                    Ok(value) if value.get("error").is_none() => {
                        json!({"content":[{"type":"text","text":value["value"].to_string()}]})
                    }
                    Ok(value) => {
                        json!({"isError":true,"content":[{"type":"text","text":value["error"].as_str()}]})
                    }
                    Err(error) => json!({"isError":true,"content":[{"type":"text","text":error}]}),
                }
            }
            _ => {
                send(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unknown MCP method"}}),
                )?;
                continue;
            }
        };
        send(
            &mut output,
            json!({"jsonrpc":"2.0","id":id,"result":result}),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::document_schema;
    use ibis_formats::Kind;

    #[test]
    fn catalog_schemas_are_scoped_to_the_active_draft() {
        for kind in [Kind::Document, Kind::Workbook, Kind::Presentation] {
            let tools = document_schema(kind, false);
            assert!(
                tools
                    .iter()
                    .any(|tool| tool["annotations"]["readOnlyHint"] == false)
            );
            assert!(tools.iter().all(|tool| {
                tool["inputSchema"]["properties"]
                    .get("document_id")
                    .is_none()
                    && !tool["name"].as_str().unwrap().starts_with("app_")
            }));
            assert!(
                document_schema(kind, true)
                    .iter()
                    .all(|tool| tool["annotations"]["readOnlyHint"] == true)
            );
        }
    }
}
