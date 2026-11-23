// SPDX-License-Identifier: AGPL-3.0-only
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

pub(crate) struct LiveAssistant {
    pub(crate) socket: PathBuf,
    pub(crate) read_only: bool,
}

const TOOLS: &[(&str, &str, &str, bool)] = &[
    (
        "assistant_status",
        "status",
        "Read the open GUI workbook, running turn and pending review.",
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
        "Send a prompt to the open GUI Assistant and return its run ID. Inspect any resulting proposal before applying.",
        false,
    ),
    (
        "assistant_apply",
        "apply",
        "Apply the pending undoable document edit for this run ID. Call only when the user explicitly asks to apply the changes. This does not save the document or apply source edits.",
        false,
    ),
];

impl LiveAssistant {
    pub(crate) fn schemas(&self) -> impl Iterator<Item = Value> + '_ {
        TOOLS.iter().filter(|tool| !self.read_only || tool.3).map(|(name, op, description, read)| {
            let (properties, required) = match *op {
                "prompt" => (json!({"text":{"type":"string","minLength":1}}), json!(["text"])),
                "result" => (json!({"run_id":{"type":"integer","minimum":0},"after":{"type":"integer","minimum":0}}), json!(["run_id"])),
                "apply" => (json!({"run_id":{"type":"integer","minimum":0}}), json!(["run_id"])),
                _ => (json!({}), json!([])),
            };
            json!({"name":name,"description":description,"inputSchema":{
                "type":"object","properties":properties,"required":required,"additionalProperties":false,
            },"annotations":{"readOnlyHint":read,"destructiveHint":false,"openWorldHint":!read}})
        })
    }

    pub(crate) fn call(&self, name: &str, args: Option<&Value>) -> Option<Result<Value, String>> {
        let (_, op, _, _) = TOOLS
            .iter()
            .find(|tool| tool.0 == name && (!self.read_only || tool.3))?;
        Some(self.request(op, args))
    }

    fn request(&self, op: &str, args: Option<&Value>) -> Result<Value, String> {
        let args = args
            .filter(|args| !args.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let keys: &[&str] = match op {
            "prompt" => &["text"],
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
        if matches!(op, "result" | "apply") && args["run_id"].as_u64().is_none() {
            return Err("run_id must be a nonnegative integer".into());
        }
        if args.get("after").is_some_and(|after| after.as_u64().is_none()) {
            return Err("after must be a nonnegative integer".into());
        }
        let mut request = args;
        request["op"] = json!(op);
        let mut bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        if bytes.len() >= 256 * 1024 {
            return Err("Assistant request exceeds 256 KiB".into());
        }
        bytes.push(b'\n');
        let exchange = || -> std::io::Result<Value> {
            let mut socket = UnixStream::connect(&self.socket)?;
            socket.set_write_timeout(Some(Duration::from_secs(5)))?;
            socket.set_read_timeout(Some(Duration::from_secs(5)))?;
            socket.write_all(&bytes)?;
            let mut line = Vec::new();
            BufReader::new(socket)
                .take(5 * 1024 * 1024)
                .read_until(b'\n', &mut line)?;
            Ok(serde_json::from_slice(&line)?)
        };
        let response = exchange()
            .map_err(|error| format!("cannot communicate with the GUI Assistant: {error}"))?;
        if let Some(error) = response["error"].as_str() {
            return Err(error.to_owned());
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_connection_excludes_write_tools() {
        let assistant = LiveAssistant {
            socket: PathBuf::new(),
            read_only: true,
        };
        assert_eq!(assistant.schemas().count(), 2);
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
            socket: PathBuf::new(),
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
