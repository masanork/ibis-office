// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
#![cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]

use ibis_doc_service::{Limits, Service, run_selected_sqlite_tool};
use serde_json::json;

#[test]
fn sqlite_analysis_is_shared_and_read_only() {
    let dir = std::env::temp_dir().join(format!("ibis-sqlite-tools-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("source.sqlite3");
    let bytes = include_bytes!("../../ibis-doc-model/src/query/fixtures/external.sqlite3");
    std::fs::write(&path, bytes).unwrap();
    let mut service = Service::new(&[dir.clone()], true, Limits::default());
    let table = "sales \"2026\"";

    assert!(
        service
            .tools()
            .iter()
            .any(|tool| tool.name == "sqlite_count_rows")
    );
    let tables = service
        .call("sqlite_list_tables", &json!({"path":path}))
        .unwrap();
    assert_eq!(tables["tables"], json!(["binary_data", "empty", table]));
    let columns = service
        .call("sqlite_describe_table", &json!({"path":path,"table":table}))
        .unwrap();
    assert_eq!(columns["columns"][0]["name"], "code");
    assert_eq!(columns["columns"][0]["declared_type"], "TEXT");
    assert_eq!(columns["columns"][2]["name"], "remark");

    let args = json!({"table":table,"group_by":"remark","limit":1});
    let selected = run_selected_sqlite_tool(&path, "sqlite_count_rows", args.clone()).unwrap();
    let server = service
        .call(
            "sqlite_count_rows",
            &json!({"path":path,"table":table,"group_by":"remark","limit":1}),
        )
        .unwrap();
    assert_eq!(server, selected);
    assert_eq!(server["groups"], json!([{"value":"ok","count":2}]));
    assert_eq!(server["truncated"], true);
    let full = service
        .call(
            "sqlite_count_rows",
            &json!({"path":path,"table":table,"group_by":"remark"}),
        )
        .unwrap();
    assert_eq!(
        full["groups"],
        json!([{"value":"ok","count":2},{"value":null,"count":1}])
    );
    assert_eq!(full["truncated"], false);
    let total = service
        .call("sqlite_count_rows", &json!({"path":path,"table":table}))
        .unwrap();
    assert_eq!(total["count"], 3);
    assert!(
        service
            .call(
                "sqlite_count_rows",
                &json!({"path":path,"table":"empty","group_by":"missing"})
            )
            .is_err()
    );
    assert!(
        service
            .call(
                "sqlite_count_rows",
                &json!({"path":path,"table":"sales_view"})
            )
            .is_err()
    );
    assert!(run_selected_sqlite_tool(&path, "sqlite_list_tables", json!({"path":path})).is_err());
    assert_eq!(
        Service::new(&[], true, Limits::default())
            .call("sqlite_list_tables", &json!({"path":path}))
            .unwrap_err()
            .code,
        "not_allowed_path"
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::remove_dir_all(dir).unwrap();
}
