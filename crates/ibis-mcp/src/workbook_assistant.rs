// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    io,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use ibis_doc_service::{Limits, Service, ToolError};
use serde_json::{Value, json};

use crate::assistant_endpoint::{BoundSocket, SocketLifetime};

pub fn start(
    workspace: &Path,
    summarize: impl Fn(&Value) -> Result<String, String> + Send + Sync + 'static,
) -> io::Result<(SocketLifetime, thread::JoinHandle<()>)> {
    let root = workspace.canonicalize()?;
    let socket = BoundSocket::bind_workspace(&root)?;
    let busy = Arc::new(AtomicBool::new(false));
    let summarize = Arc::new(summarize);
    let mut next_run = 0u64;

    let (lifetime, _, worker) = socket.start(move |(request, deadline, answer), events| {
        let response = match request["op"].as_str() {
            Some("status") => json!({
                "kind":"headless", "label":"Workbook Assistant",
                "path":null, "document_ready":false, "streaming":busy.load(Ordering::Acquire),
                "operations":["assistant_status","assistant_inspect_workbook","assistant_result"],
            }),
            Some("inspect_workbook") if std::time::Instant::now() >= deadline => {
                json!({"error":"Assistant request expired"})
            }
            Some("inspect_workbook") => match workbook_path(&request["path"], &root) {
                Err(error) => json!({"error":error}),
                Ok(_) if busy.load(Ordering::Acquire) => json!({"error":"an Assistant run is already running"}),
                Ok(path) => {
                    let run = next_run;
                    next_run += 1;
                    busy.store(true, Ordering::Release);
                    emit(events, json!({"event":"started","run_id":run}));
                    let (root, busy, summarize, events) =
                        (root.clone(), busy.clone(), summarize.clone(), events.clone());
                    thread::spawn(move || {
                        let result = inspect(&root, &path, run, &events).map_err(|error| error.to_json())
                            .and_then(|evidence| {
                                emit(&events, json!({"event":"progress","run_id":run,"stage":"summarizing"}));
                                summarize(&evidence).map_err(|error| json!({"code":"summary_failed","message":error}))
                            });
                        match result {
                            Ok(text) => {
                                emit(&events, json!({"event":"message","run_id":run,"text":bounded(&text, 12_000)}));
                                emit(&events, json!({"event":"completed","run_id":run,"review":false}));
                            }
                            Err(error) => emit(&events, json!({"event":"error","run_id":run,"code":error["code"],"message":bounded(error["message"].as_str().unwrap_or_default(), 2048)})),
                        }
                        busy.store(false, Ordering::Release);
                    });
                    json!({"run_id":run})
                }
            },
            _ => json!({"error":"unsupported Workbook Assistant request"}),
        };
        answer.send(response).map_err(io::Error::other)
    });
    Ok((lifetime, worker))
}

fn workbook_path(value: &Value, root: &Path) -> Result<PathBuf, &'static str> {
    let name = value.as_str().ok_or("path is required")?;
    let path = Path::new(name);
    if name.contains('\\')
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || !path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
    {
        return Err("path must identify an XLSX relative to the shared workspace");
    }
    Ok(root.join(path))
}

fn inspect(
    root: &Path,
    path: &Path,
    run: u64,
    events: &mpsc::Sender<Value>,
) -> Result<Value, ToolError> {
    emit(
        events,
        json!({"event":"progress","run_id":run,"stage":"opening"}),
    );
    let mut service = Service::new(&[root.to_owned()], true, Limits::default());
    let opened = service.call("app_open_document", &json!({"path":path}))?;
    if opened["kind"] != "xlsx" {
        return Err(ToolError::invalid("the file is not an XLSX workbook"));
    }
    let document_id = &opened["document_id"];
    emit(
        events,
        json!({"event":"progress","run_id":run,"stage":"inspecting"}),
    );
    let listing = service.call("xlsx_list_sheets", &json!({"document_id":document_id}))?;
    let sheets = listing["sheets"]
        .as_array()
        .ok_or_else(|| ToolError::new("internal", "missing worksheet listing"))?;
    let mut inspected = Vec::new();
    for sheet in sheets.iter().take(24) {
        let name = &sheet["name"];
        let checks = service.call(
            "xlsx_check_machine_readability",
            &json!({"document_id":document_id,"sheet":name,"limit":100}),
        )?;
        let candidates = checks["findings"]
            .as_array()
            .ok_or_else(|| ToolError::new("internal", "missing worksheet findings"))?;
        let mut formulas = Vec::new();
        for finding in candidates
            .iter()
            .filter(|item| item["kind"] == "formula")
            .take(4)
        {
            let cell = &finding["location"];
            let result = service.call("xlsx_get_cells", &json!({"document_id":document_id,"sheet":name,"range":cell,"kind":"formula","limit":1}))?;
            if let Some(formula) = result["rows"][0][0].as_str() {
                formulas.push(json!({"cell":cell,"formula":bounded(formula, 384),"truncated":formula.len()>384}));
            }
        }
        let sheet_evidence =
            json!({"sheet":name,"used_range":sheet["used_range"],"formula_examples":formulas});
        emit(
            events,
            json!({"event":"evidence","run_id":run,"data":sheet_evidence}),
        );
        inspected.push(sheet_evidence);
    }
    Ok(
        json!({"sheet_count":sheets.len(),"sheets":inspected,
        "sampling":"At most 24 sheets, first 100 structural findings per sheet, at most 4 formula examples per sheet."}),
    )
}

fn emit(events: &mpsc::Sender<Value>, event: Value) {
    let _ = events.send(event);
}

fn bounded(text: &str, limit: usize) -> &str {
    &text[..text.floor_char_boundary(limit.min(text.len()))]
}
