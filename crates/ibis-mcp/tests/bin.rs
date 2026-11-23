// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! GitHub issue #159: the program itself, as a client starts it: standard input and output only, the log on standard error, and it ends when its input is closed.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn mcp_the_program_talks_over_standard_input_and_output() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/mcp-tests/bin");
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ibisOffice-mcp")).arg("--allow-dir").arg(&dir).arg("--read-only").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for (id, method) in [(1, "server/discover"), (2, "tools/list")] {
        writeln!(stdin, "{}", json!({ "jsonrpc": "2.0", "id": id, "method": method })).unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let lines: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).expect("stdout holds the messages only")).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["result"]["serverInfo"]["name"], "ibisOffice");
    assert!(lines[1]["result"]["tools"].as_array().unwrap().iter().all(|t| t["annotations"]["readOnlyHint"] == true), "read-only was asked");
    // A wrong argument stops the program with a message on standard error.
    let bad = Command::new(env!("CARGO_BIN_EXE_ibisOffice-mcp")).arg("--allow-dir").arg("/no/such/folder").output().unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(bad.stdout.is_empty() && !bad.stderr.is_empty());
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn headless_assistant_results_survive_mcp_client_exit() {
    let folder = tempfile::tempdir().unwrap();
    let (endpoint, worker) = ibis_mcp::assistant_endpoint::start_demo(folder.path()).unwrap();
    let call = |name: &str, arguments: Value| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ibisOffice-mcp"))
            .arg("--assistant-workspace")
            .arg(folder.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(
            child.stdin.take().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}})
        ).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone()
    };
    let sessions = call("assistant_list_sessions", json!({}));
    let session = &sessions["structuredContent"]["sessions"][0];
    assert_eq!(session["kind"], "headless");
    assert_eq!(
        session["operations"],
        json!(["assistant_status", "assistant_prompt", "assistant_result"])
    );
    let id = &session["session_id"];
    let outcome = call(
        "assistant_prompt",
        json!({"session_id":id,"text":"Please respond"}),
    );
    assert_eq!(outcome["isError"], false);
    let run = &outcome["structuredContent"]["run_id"];
    let outcome = call("assistant_result", json!({"session_id":id,"run_id":run}));
    assert_eq!(outcome["isError"], false);
    let result = &outcome["structuredContent"];
    assert_eq!(result["summary"]["state"], "completed");
    assert_eq!(
        result["events"][1]["text"],
        "Headless Assistant demo response."
    );
    let denied = call("assistant_apply", json!({"session_id":id,"run_id":run}));
    assert_eq!(denied["isError"], true);
    drop(endpoint);
    worker.join().unwrap();
    assert_eq!(
        call("assistant_list_sessions", json!({}))["structuredContent"]["sessions"],
        json!([])
    );
}
