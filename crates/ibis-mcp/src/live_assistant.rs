// SPDX-License-Identifier: AGPL-3.0-only
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::assistant_workspace::Workspace;

pub(crate) enum Endpoint {
    Socket(PathBuf),
    Workspace(Workspace),
}

pub(crate) struct LiveAssistant {
    pub(crate) endpoint: Endpoint,
    pub(crate) read_only: bool,
}

const TOOLS: &[(&str, &str, &str, bool)] = &[
    (
        "assistant_status",
        "status",
        "Read the Assistant endpoint state, including its supported operations and any open document.",
        true,
    ),
    (
        "assistant_result",
        "result",
        "Read events for a turn, including whether its proposal was applied or discarded.",
        true,
    ),
    (
        "assistant_prompt",
        "prompt",
        "Send a prompt to the selected Assistant endpoint and return its run ID. Pass a skill name only when the user selects it. Inspect any resulting proposal before applying.",
        false,
    ),
    (
        "assistant_apply",
        "apply",
        "Apply the pending undoable document edit for this run ID. Call only when the user explicitly asks to apply the changes. This does not save the document or apply source edits.",
        false,
    ),
    (
        "assistant_reset",
        "reset",
        "Start a fresh GUI Assistant conversation after the user requests it. Requires an idle Assistant with no pending review. Workspace settings and recent run results are retained.",
        false,
    ),
];

impl LiveAssistant {
    pub(crate) fn schemas(&self) -> impl Iterator<Item = Value> + '_ {
        let workspace = matches!(self.endpoint, Endpoint::Workspace(_));
        let discovery = workspace.then(|| json!({
            "name":"assistant_list_sessions",
            "description":"List explicitly shared Assistant endpoints in the configured workspace. Use session_id to select an endpoint and check its operations when present.",
            "inputSchema":{"type":"object","properties":{},"additionalProperties":false},
            "annotations":{"readOnlyHint":true}
        }));
        discovery.into_iter().chain(TOOLS.iter().filter(|tool| !self.read_only || tool.3).map(move |(name, op, description, read)| {
            let (mut properties, mut required) = match *op {
                "prompt" => (json!({"text":{"type":"string","minLength":1},"skill":{"type":"string","minLength":1,"maxLength":64}}), json!(["text"])),
                "result" => (json!({"run_id":{"type":"integer","minimum":0},"after":{"type":"integer","minimum":0}}), json!(["run_id"])),
                "apply" => (json!({"run_id":{"type":"integer","minimum":0}}), json!(["run_id"])),
                _ => (json!({}), json!([])),
            };
            if workspace {
                properties["session_id"] = json!({"type":"string","description":"Session ID from assistant_list_sessions."});
                required.as_array_mut().unwrap().push(json!("session_id"));
            }
            json!({"name":name,"description":description,"inputSchema":{
                "type":"object","properties":properties,"required":required,"additionalProperties":false,
            },"annotations":{"readOnlyHint":read,"destructiveHint":false,"openWorldHint":!read}})
        }))
    }

    pub(crate) fn call(&self, name: &str, args: Option<&Value>) -> Option<Result<Value, String>> {
        if name == "assistant_list_sessions" {
            let Endpoint::Workspace(workspace) = &self.endpoint else {
                return None;
            };
            return Some(list_sessions(workspace, args));
        }
        let (_, op, _, _) = TOOLS
            .iter()
            .find(|tool| tool.0 == name && (!self.read_only || tool.3))?;
        Some(self.request(op, args))
    }

    fn request(&self, op: &str, args: Option<&Value>) -> Result<Value, String> {
        let mut args = args
            .filter(|args| !args.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let socket = match &self.endpoint {
            Endpoint::Socket(socket) => socket.clone(),
            Endpoint::Workspace(workspace) => {
                let id = args
                    .as_object_mut()
                    .and_then(|args| args.remove("session_id"));
                let id = id
                    .as_ref()
                    .and_then(Value::as_str)
                    .ok_or("session_id is required")?;
                workspace
                    .socket(id)
                    .ok_or("Assistant session is unavailable in this workspace")?
            }
        };
        let keys: &[&str] = match op {
            "prompt" => &["text", "skill"],
            "result" => &["run_id", "after"],
            "apply" => &["run_id"],
            _ => &[],
        };
        if args
            .as_object()
            .is_none_or(|object| object.keys().any(|key| !keys.contains(&key.as_str())))
        {
            return Err("unexpected Assistant arguments".into());
        }
        if op == "prompt"
            && !args["text"]
                .as_str()
                .is_some_and(|text| !text.trim().is_empty())
        {
            return Err("text is required".into());
        }
        if args.get("skill").is_some_and(|value| {
            value
                .as_str()
                .is_none_or(|name| name.trim().is_empty() || name.len() > 64)
        }) {
            return Err("skill must be a nonempty name of at most 64 characters".into());
        }
        if matches!(op, "result" | "apply") && args["run_id"].as_u64().is_none() {
            return Err("run_id must be a nonnegative integer".into());
        }
        if args
            .get("after")
            .is_some_and(|after| after.as_u64().is_none())
        {
            return Err("after must be a nonnegative integer".into());
        }
        args["op"] = json!(op);
        exchange(&socket, &args, Duration::from_secs(5))
    }
}

fn exchange(path: &Path, request: &Value, timeout: Duration) -> Result<Value, String> {
    let mut bytes = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    if bytes.len() >= 256 * 1024 {
        return Err("Assistant request exceeds 256 KiB".into());
    }
    bytes.push(b'\n');
    let send = || -> std::io::Result<Value> {
        let mut socket = UnixStream::connect(path)?;
        socket.set_write_timeout(Some(timeout))?;
        socket.set_read_timeout(Some(timeout))?;
        socket.write_all(&bytes)?;
        let mut line = Vec::new();
        BufReader::new(socket)
            .take(5 * 1024 * 1024)
            .read_until(b'\n', &mut line)?;
        Ok(serde_json::from_slice(&line)?)
    };
    let response =
        send().map_err(|error| format!("cannot communicate with the Assistant endpoint: {error}"))?;
    if let Some(error) = response["error"].as_str() {
        return Err(error.to_owned());
    }
    Ok(response)
}

fn list_sessions(workspace: &Workspace, args: Option<&Value>) -> Result<Value, String> {
    if args.is_some_and(|args| {
        !args.is_null() && !args.as_object().is_some_and(|args| args.is_empty())
    }) {
        return Err("unexpected Assistant discovery arguments".into());
    }

    let mut sessions = Vec::new();
    for (id, socket) in workspace.sessions().map_err(|error| error.to_string())? {
        let Ok(status) = exchange(&socket, &json!({"op":"status"}), Duration::from_millis(250))
        else {
            continue;
        };
        sessions.push(json!({
            "session_id":id,
            "kind":status["kind"].as_str().unwrap_or("gui"),
            "label":status["label"],
            "operations":status["operations"],
            "path":status["path"],
            "document_ready":status["document_ready"],
            "streaming":status["streaming"],
            "awaiting_review":!status["review"].is_null(),
        }));
    }
    sessions.sort_unstable_by(|a, b| {
        a["path"]
            .as_str()
            .cmp(&b["path"].as_str())
            .then_with(|| a["session_id"].as_str().cmp(&b["session_id"].as_str()))
    });
    Ok(json!({"sessions":sessions}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_connection_excludes_write_tools() {
        let assistant = LiveAssistant {
            endpoint: Endpoint::Socket(PathBuf::new()),
            read_only: true,
        };
        assert_eq!(assistant.schemas().count(), 2);
        assert!(assistant.call("assistant_reset", None).is_none());
        assert!(
            assistant
                .call("assistant_prompt", Some(&json!({"text":"modify"})))
                .is_none()
        );
        assert!(
            assistant
                .call("assistant_apply", Some(&json!({"run_id":0})))
                .is_none()
        );
        assert!(
            assistant
                .call("assistant_result", Some(&json!({"run_id":0,"after":-1})))
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn apply_requires_a_valid_run_id() {
        let assistant = LiveAssistant {
            endpoint: Endpoint::Socket(PathBuf::new()),
            read_only: false,
        };
        for args in [json!({}), json!({"run_id":-1}), json!({"run_id":"1"})] {
            assert_eq!(
                assistant
                    .call("assistant_apply", Some(&args))
                    .unwrap()
                    .unwrap_err(),
                "run_id must be a nonnegative integer"
            );
        }
    }
}
