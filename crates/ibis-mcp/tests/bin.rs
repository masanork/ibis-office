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
fn assistant_call(workspace: &std::path::Path, name: &str, arguments: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ibisOffice-mcp"))
        .arg("--assistant-workspace")
        .arg(workspace)
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
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn headless_assistant_results_survive_mcp_client_exit() {
    let folder = tempfile::tempdir().unwrap();
    let (endpoint, worker) = ibis_mcp::assistant_endpoint::start_demo(folder.path()).unwrap();
    let call = |name, arguments| assistant_call(folder.path(), name, arguments);
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
    assert!(result["events"].as_array().unwrap().iter().any(|event|
        event["event"] == "message" && event["text"] == "Headless Assistant demo response."));
    let denied = call("assistant_apply", json!({"session_id":id,"run_id":run}));
    assert_eq!(denied["isError"], true);
    drop(endpoint);
    worker.join().unwrap();
    assert_eq!(
        call("assistant_list_sessions", json!({}))["structuredContent"]["sessions"],
        json!([])
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn workbook_assistant_reports_queue_progress_and_failures_across_client_processes() {
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("report.xlsx");
    let source = folder.path().join("source.xlsx");
    std::fs::write(&source, include_bytes!("../../ibis-odf/tests/fixtures/ods_features.xlsx")).unwrap();
    let mut fixture = ibis_doc_service::Service::new(&[folder.path().to_owned()], false, Default::default());
    let document = fixture.call("app_open_document", &json!({"path":source})).unwrap();
    let id = &document["document_id"];
    fixture.call("xlsx_set_cells", &json!({"document_id":id,"sheet":"Data","range":"F1","values":"=SUM(A1:A2)"})).unwrap();
    fixture.call("app_save_document", &json!({"document_id":id,"path":path})).unwrap();
    let (evidence_tx, evidence_rx) = mpsc::channel();
    let (finish, wait_to_finish) = mpsc::channel();
    let wait_to_finish = Mutex::new(wait_to_finish);
    let (endpoint, worker) = ibis_mcp::workbook_assistant::start(folder.path(), move |evidence| {
        evidence_tx.send(evidence.clone()).unwrap();
        wait_to_finish.lock().unwrap().recv_timeout(Duration::from_secs(10)).unwrap();
        Ok("Workbook summary".into())
    }).unwrap();
    let call = |name, arguments| assistant_call(folder.path(), name, arguments);
    let listed = call("assistant_list_sessions", json!({}));
    let session = &listed["structuredContent"]["sessions"][0];
    let id = &session["session_id"];
    assert_eq!(session["label"], "Workbook Assistant");
    assert!(session["operations"].as_array().unwrap().contains(&json!("assistant_inspect_workbook")));
    let send = |path| call("assistant_inspect_workbook", json!({"session_id":id,"path":path}));
    assert_eq!(send("../report.xlsx")["isError"], true);
    let accepted = send("report.xlsx");
    let run = &accepted["structuredContent"]["run_id"];
    assert!(run.is_u64(), "{accepted}");
    let evidence = evidence_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(evidence["sheets"][0]["sheet"], "Data");
    assert_eq!(evidence["sheets"][0]["formula_examples"][0]["formula"], "=SUM(A1:A2)");
    let result = |run: &Value| call("assistant_result", json!({"session_id":id,"run_id":run}))["structuredContent"].clone();
    let running = result(run);
    assert_eq!(running["summary"]["state"], "running");
    assert!(running["events"].as_array().unwrap().iter().any(|event| event["stage"] == "summarizing"));
    let second = send("report.xlsx");
    assert_eq!(second["structuredContent"]["state"], "queued");
    let queued = &second["structuredContent"]["run_id"];
    assert_ne!(queued, run);
    let missing = send("missing.xlsx");
    let missing = &missing["structuredContent"]["run_id"];
    assert_eq!(result(queued)["summary"]["state"], "queued");
    assert_eq!(result(missing)["summary"]["state"], "queued");
    finish.send(()).unwrap();
    let wait = |run: &Value, state: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let response = result(run);
            if response["summary"]["state"] == state {
                return response;
            }
            assert!(Instant::now() < deadline, "{response}");
            std::thread::sleep(Duration::from_millis(25));
        }
    };
    let completed = wait(run, "completed");
    assert!(completed["events"].as_array().unwrap().iter().any(|event| event["text"] == "Workbook summary"));
    evidence_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(result(queued)["summary"]["state"], "running");
    assert_eq!(result(missing)["summary"]["state"], "queued");
    finish.send(()).unwrap();
    let second_done = wait(queued, "completed");
    assert_eq!(second_done["run_id"], *queued);
    let failed = wait(missing, "error");
    assert_eq!(failed["summary"]["event"]["code"], "file_not_found");
    drop(endpoint);
    worker.join().unwrap();
}
