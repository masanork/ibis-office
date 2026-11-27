// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
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

/// Setting keys for external agents (ACP).
pub(crate) const SETTING_EXTERNAL_AGENTS: &str = "ai.external_agents";
pub(crate) const SETTING_SELECTED_AGENT: &str = "ai.selected_agent";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ExternalAgentConfig {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

impl ExternalAgentConfig {
    pub(crate) fn command_line_display(&self) -> String {
        if self.args.is_empty() {
            self.command.clone()
        } else {
            format!("{} {}", self.command, self.args.join(" "))
        }
    }
}

pub(crate) fn external_agent_presets() -> Vec<ExternalAgentConfig> {
    let mut opencode_env = std::collections::BTreeMap::new();
    opencode_env.insert(
        "OPENCODE_PERMISSION".to_string(),
        r#"{"*":"deny","workbook_*":"allow"}"#.to_string(),
    );
    vec![
        ExternalAgentConfig {
            id: "opencode".into(),
            name: "OpenCode".into(),
            command: "opencode".into(),
            args: vec!["acp".into()],
            env: opencode_env,
        },
        ExternalAgentConfig {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            command: "claude-agent-acp".into(),
            args: Vec::new(),
            env: Default::default(),
        },
        ExternalAgentConfig {
            id: "gemini".into(),
            name: "Gemini CLI".into(),
            command: "gemini".into(),
            args: vec!["--experimental-acp".into()],
            env: Default::default(),
        },
        ExternalAgentConfig {
            id: "codex".into(),
            name: "Codex".into(),
            command: "codex-acp".into(),
            args: Vec::new(),
            env: Default::default(),
        },
    ]
}

pub(crate) fn load_external_agents() -> Vec<ExternalAgentConfig> {
    let Some(raw) = crate::settings::get(SETTING_EXTERNAL_AGENTS) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<ExternalAgentConfig>>(&raw)
        .unwrap_or_default()
        .into_iter()
        .filter(|agent| !agent.id.trim().is_empty() && !agent.command.trim().is_empty())
        .collect()
}

pub(crate) fn save_external_agents(agents: &[ExternalAgentConfig]) {
    if agents.is_empty() {
        crate::settings::remove(SETTING_EXTERNAL_AGENTS);
    } else if let Ok(json) = serde_json::to_string(agents) {
        crate::settings::set(SETTING_EXTERNAL_AGENTS, &json);
    }
}

pub(crate) fn load_selected_agent() -> Option<String> {
    crate::settings::get(SETTING_SELECTED_AGENT)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "builtin")
}

pub(crate) fn save_selected_agent(agent_id: Option<&str>) {
    match agent_id.map(str::trim).filter(|s| !s.is_empty() && *s != "builtin") {
        Some(id) => crate::settings::set(SETTING_SELECTED_AGENT, id),
        None => crate::settings::remove(SETTING_SELECTED_AGENT),
    }
}

pub(crate) fn env_acp_agent() -> Option<ExternalAgentConfig> {
    let executable = std::env::var_os("IBIS_ACP_COMMAND")?;
    let command = executable.to_string_lossy().into_owned();
    if command.trim().is_empty() {
        return None;
    }
    let args = std::env::var("IBIS_ACP_ARGS")
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default();
    Some(ExternalAgentConfig {
        id: "env-acp".into(),
        name: format!("環境変数 ({command})"),
        command,
        args,
        env: Default::default(),
    })
}

pub(crate) fn available_external_agents() -> Vec<ExternalAgentConfig> {
    let mut agents = load_external_agents();
    if let Some(env_agent) = env_acp_agent()
        && !agents.iter().any(|a| a.id == env_agent.id)
    {
        agents.insert(0, env_agent);
    }
    agents
}

pub(crate) fn find_external_agent(agent_id: Option<&str>) -> Option<ExternalAgentConfig> {
    let agents = available_external_agents();
    if let Some(id) = agent_id {
        if let Some(found) = agents.into_iter().find(|a| a.id == id) {
            return Some(found);
        }
    }
    env_acp_agent()
}

pub(crate) fn parse_args_line(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else if ch == '\\' && q == '"' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else {
                    current.push(ch);
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                } else if ch.is_whitespace() {
                    if !current.is_empty() {
                        args.push(std::mem::take(&mut current));
                    }
                } else if ch == '\\' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else {
                    current.push(ch);
                }
            }
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

pub(crate) fn command_exists(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    let path = Path::new(trimmed);
    if path.components().count() > 1 {
        return path.is_file();
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| {
        let candidate = dir.join(trimmed);
        candidate.is_file()
    })
}

/// A handle to the agent's saved session, scoped to the file and configured adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResumeRecord(PathBuf);

impl ResumeRecord {
    pub(crate) fn for_document(path: &Path, read_only: bool) -> Option<Self> {
        Self::for_document_with_agent(path, read_only, None)
    }

    pub(crate) fn for_document_with_agent(
        path: &Path,
        read_only: bool,
        agent: Option<&ExternalAgentConfig>,
    ) -> Option<Self> {
        use sha2::{Digest, Sha256};
        let path = path.canonicalize().ok()?;
        let metadata = path.metadata().ok()?;
        if !metadata.is_file() {
            return None;
        }
        let mut scope = Sha256::new();
        let mut add = |value: &[u8]| {
            scope.update((value.len() as u64).to_le_bytes());
            scope.update(value);
        };
        add(path.as_os_str().as_bytes());
        for value in [
            metadata.dev(),
            metadata.ino(),
            metadata.size(),
            metadata.mtime() as u64,
            metadata.mtime_nsec() as u64,
            metadata.ctime() as u64,
            metadata.ctime_nsec() as u64,
        ] {
            add(&value.to_le_bytes());
        }
        add(&[read_only as u8]);
        if let Some(agent) = agent {
            add(agent.command.as_bytes());
            for arg in &agent.args {
                add(arg.as_bytes());
            }
        } else {
            for name in ["IBIS_ACP_COMMAND", "IBIS_ACP_ARGS", "IBIS_OPENCODE"] {
                add(std::env::var_os(name)
                    .as_deref()
                    .unwrap_or_default()
                    .as_bytes());
            }
        }
        let root = ibis_office_base::app_data::base()?
            .join("ibisOffice")
            .join("acp-sessions");
        root.is_absolute()
            .then(|| Self(root.join(format!("{:x}", scope.finalize()))))
    }

    fn open(&self) -> std::io::Result<(PathBuf, File)> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.0)?;
        let cwd = self.0.join("work");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&cwd)?;
        for path in [&self.0, &cwd] {
            let info = fs::symlink_metadata(path)?;
            if !info.is_dir() || info.permissions().mode() & 0o077 != 0 {
                return Err(std::io::Error::other(
                    "ACP session directory is not private",
                ));
            }
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(self.0.join("lock"))?;
        lock.try_lock()?;
        Ok((cwd, lock))
    }

    fn load(&self) -> Option<String> {
        let mut stored = String::new();
        File::open(self.0.join("session"))
            .ok()?
            .take(4097)
            .read_to_string(&mut stored)
            .ok()?;
        (valid_session_id(&stored)).then_some(stored)
    }

    pub(crate) fn save(&self, id: &str) -> std::io::Result<()> {
        if !valid_session_id(id) {
            return Err(std::io::Error::other("Invalid ACP session ID"));
        }
        let pending = self.0.join("session.new");
        let _ = fs::remove_file(&pending);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&pending)?;
        file.write_all(id.as_bytes())?;
        fs::rename(pending, self.0.join("session"))
    }

    pub(crate) fn clear(&self) {
        let _ = fs::remove_file(self.0.join("session"));
    }
}

fn valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 4096 && !id.chars().any(char::is_control)
}

type Work = Box<dyn FnOnce(&mut Option<Session>) + Send>;

pub(crate) struct Conversation {
    send: mpsc::Sender<Work>,
    resume: Option<ResumeRecord>,
    agent: Option<ExternalAgentConfig>,
}

impl Conversation {
    pub(crate) fn start(
        resume: Option<ResumeRecord>,
        agent: Option<ExternalAgentConfig>,
    ) -> Result<Self, String> {
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
        Ok(Self {
            send,
            resume,
            agent,
        })
    }

    pub(crate) fn restrict_resume(&mut self, current: Option<&ResumeRecord>) {
        if self.resume.as_ref() != current {
            self.clear_resume();
            self.resume = None;
        }
    }

    pub(crate) fn can_save(&self, record: &ResumeRecord) -> bool {
        self.resume.as_ref() == Some(record)
    }

    pub(crate) fn clear_resume(&self) {
        if let Some(record) = &self.resume {
            record.clear();
        }
    }

    pub(crate) fn submit<T: Send + 'static>(
        &self,
        tools: Value,
        work: impl FnOnce(&mut Session) -> Result<T, String> + Send + 'static,
    ) -> Result<futures::channel::oneshot::Receiver<Result<T, String>>, String> {
        let (finished, outcome) = futures::channel::oneshot::channel();
        let resume = self.resume.clone();
        let agent = self.agent.clone();
        self.send
            .send(Box::new(move |session| {
                let result = (|| {
                    if session.is_none() {
                        *session = Some(Session::start(tools.clone(), resume, agent.as_ref())?);
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
    _dir: Directory,
    cwd: PathBuf,
    resume: Option<(ResumeRecord, File)>,
    resumable: bool,
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
    fn start(
        tools: Value,
        record: Option<ResumeRecord>,
        agent: Option<&ExternalAgentConfig>,
    ) -> Result<Self, String> {
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
        let (cwd, resume) = record
            .and_then(|record| {
                let (cwd, lock) = record.open().ok()?;
                Some((cwd, Some((record, lock))))
            })
            .unwrap_or_else(|| (dir.0.clone(), None));
        let listener = UnixListener::bind(&socket).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let mut command = if let Some(agent) = agent {
            let mut command = Command::new(&agent.command);
            command.args(&agent.args);
            command.envs(&agent.env);
            command
        } else if let Some(executable) = std::env::var_os("IBIS_ACP_COMMAND") {
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
        let agent_label = agent
            .map(|a| a.command.as_str())
            .unwrap_or("ACP agent");
        let mut child = command
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not start {agent_label}: {e}"))?;
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
            _dir: dir,
            cwd,
            resume,
            resumable: false,
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

    fn session_params(&self) -> Result<Value, String> {
        Ok(json!({"cwd":self.cwd,"mcpServers":[{"name":"workbook",
            "command":std::env::current_exe().map_err(|error| error.to_string())?,
            "args":["--assistant-acp-workbook",self.socket],"env":[]}]}))
    }

    pub(crate) fn checkpoint(&self) -> Option<(ResumeRecord, String)> {
        if !self.resumable {
            return None;
        }
        Some((self.resume.as_ref()?.0.clone(), self.id.clone()?))
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
        let mut loading = None;
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
                if message["id"] == 2 && loading.take().is_some() {
                    if let Some((record, _)) = &self.resume {
                        record.clear();
                    }
                    on_text(
                        "The saved ACP session could not be restored. Starting a new conversation.\n",
                    );
                    let params = self.session_params()?;
                    send(&mut self.input, request(2, "session/new", params))?;
                    continue;
                }
                return Err(error.to_owned());
            }
            match message["id"].as_u64() {
                Some(1) if self.id.is_none() && message["result"]["protocolVersion"] == 1 => {
                    let capabilities = &message["result"]["agentCapabilities"];
                    let mode = if capabilities
                        .pointer("/sessionCapabilities/resume")
                        .is_some_and(Value::is_object)
                    {
                        Some("session/resume")
                    } else if capabilities["loadSession"] == true {
                        Some("session/load")
                    } else {
                        None
                    };
                    self.resumable = mode.is_some();
                    let mut params = self.session_params()?;
                    let method = match (
                        mode,
                        self.resume.as_ref().and_then(|(record, _)| record.load()),
                    ) {
                        (Some(method), Some(id)) => {
                            params["sessionId"] = json!(&id);
                            loading = Some(id);
                            method
                        }
                        _ => "session/new",
                    };
                    send(&mut self.input, request(2, method, params))?;
                }
                Some(1) => return Err("ACP agent did not negotiate ACP v1".into()),
                Some(2) if self.id.is_none() => {
                    self.id = Some(match loading.take() {
                        Some(id) => id,
                        None => message["result"]["sessionId"]
                            .as_str()
                            .filter(|id| valid_session_id(id))
                            .ok_or("ACP agent returned no session ID")?
                            .to_owned(),
                    });
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
    fn resume_records_are_scoped_to_the_saved_revision_and_permissions() {
        use super::ResumeRecord;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = super::Directory(std::env::temp_dir().join(format!("ibis-acp-test-{nonce}")));
        std::fs::create_dir(&dir.0).unwrap();
        let file = dir.0.join("document.xlsx");
        std::fs::write(&file, b"first version").unwrap();
        let writable = ResumeRecord::for_document(&file, false).unwrap();
        assert_eq!(
            ResumeRecord::for_document(&file, false),
            Some(writable.clone())
        );
        assert_ne!(
            ResumeRecord::for_document(&file, true),
            Some(writable.clone())
        );
        std::fs::write(&file, b"replacement version").unwrap();
        assert_ne!(ResumeRecord::for_document(&file, false), Some(writable));

        let record = ResumeRecord(dir.0.join("test-record"));
        let (_, lock) = record.open().unwrap();
        assert!(record.open().is_err());
        record.save("sample-session").unwrap();
        assert_eq!(record.load().as_deref(), Some("sample-session"));
        assert!(record.save("bad\nvalue").is_err());
        record.clear();
        assert!(record.load().is_none());
        drop(lock);
    }

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

    #[test]
    fn parse_args_line_handles_spaces_and_quotes() {
        assert_eq!(super::parse_args_line("acp"), vec!["acp"]);
        assert_eq!(
            super::parse_args_line(r#"--flag "hello world" 'single quoted'"#),
            vec!["--flag", "hello world", "single quoted"]
        );
        assert!(super::parse_args_line("   ").is_empty());
    }

    #[test]
    fn resume_records_differ_across_external_agents() {
        use super::{ExternalAgentConfig, ResumeRecord};
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = super::Directory(std::env::temp_dir().join(format!("ibis-acp-agent-test-{nonce}")));
        std::fs::create_dir(&dir.0).unwrap();
        let file = dir.0.join("document.xlsx");
        std::fs::write(&file, b"content").unwrap();
        let agent_a = ExternalAgentConfig {
            id: "a".into(),
            name: "Agent A".into(),
            command: "opencode".into(),
            args: vec!["acp".into()],
            env: Default::default(),
        };
        let agent_b = ExternalAgentConfig {
            id: "b".into(),
            name: "Agent B".into(),
            command: "claude-agent-acp".into(),
            args: vec![],
            env: Default::default(),
        };
        let rec_a = ResumeRecord::for_document_with_agent(&file, false, Some(&agent_a)).unwrap();
        let rec_b = ResumeRecord::for_document_with_agent(&file, false, Some(&agent_b)).unwrap();
        assert_ne!(rec_a, rec_b);
    }
}
