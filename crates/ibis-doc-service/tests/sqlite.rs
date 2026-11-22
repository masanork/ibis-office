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
    let filters = json!([
        {"column":"code","operator":"eq","value":"002"},
        {"column":"amount","operator":"ge","value":12.5},
        {"column":"remark","operator":"is_not_null"},
    ]);
    let grouped = service
        .call(
            "sqlite_count_rows",
            &json!({
                "path":path,"table":table,"group_by":"remark","filters":filters,
            }),
        )
        .unwrap();
    let assistant = run_selected_sqlite_tool(
        &path,
        "sqlite_count_rows",
        json!({
            "table":table,"group_by":"remark","filters":filters,
        }),
    )
    .unwrap();
    assert_eq!(grouped, assistant);
    assert_eq!(grouped["groups"], json!([{"value":"ok","count":1}]));
    assert_eq!(grouped["filters"], filters);
    let only_null = service
        .call(
            "sqlite_count_rows",
            &json!({
                "path":path,"table":table,"filters":[{"column":"remark","operator":"is_null"}],
            }),
        )
        .unwrap();
    assert_eq!(only_null["count"], 1);
    let literal = service.call("sqlite_count_rows", &json!({
        "path":path,"table":table,"filters":[{"column":"remark","operator":"eq","value":"ok' OR 1=1"}],
    })).unwrap();
    assert_eq!(literal["count"], 0);
    for (filter, code) in [
        (
            json!({"column":"remark","operator":"is_null","value":1}),
            "invalid_argument",
        ),
        (
            json!({"column":"remark","operator":"eq"}),
            "invalid_argument",
        ),
        (
            json!({"column":"remark","operator":"eq","value":u64::MAX}),
            "invalid_argument",
        ),
        (
            json!({"column":"absent","operator":"is_null"}),
            "sqlite_error",
        ),
    ] {
        let error = service
            .call(
                "sqlite_count_rows",
                &json!({
                    "path":path,"table":table,"filters":[filter],
                }),
            )
            .unwrap_err();
        assert_eq!(error.code, code);
    }

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

#[test]
fn sqlite_query_exports_complete_results_and_builds_a_pivot() {
    use ibis_doc_model::Document;
    use ibis_doc_service::{
        DraftToolContext, WorkbookToolContext, prepare_selected_sqlite_export, run_draft_tool,
    };
    use ibis_formats::Kind;

    let dir = std::env::temp_dir().join(format!("ibis-sqlite-pivot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let database = dir.join("source.sqlite3");
    let bytes = include_bytes!("../../ibis-doc-model/src/query/fixtures/external.sqlite3");
    std::fs::write(&database, bytes).unwrap();
    let workbook = dir.join("analysis.xlsx");
    let blank = ibis_formats::blank_file(Kind::Workbook).unwrap();
    std::fs::write(&workbook, &blank).unwrap();
    let mut service = Service::new(&[dir.clone()], false, Limits::default());
    let call = |service: &mut Service, name, args| service.call(name, &args).unwrap();
    let id =
        call(&mut service, "app_open_document", json!({"path":workbook}))["document_id"].clone();
    let sheets = call(&mut service, "xlsx_list_sheets", json!({"document_id":id}));
    let sheet = sheets["sheets"][0]["name"].clone();
    let sql = r#"WITH filtered AS (
        SELECT code, remark FROM "sales ""2026""" WHERE code >= ?
    ) SELECT a.code, a.remark AS category, substr(a.code, 3, 1) AS bucket FROM filtered AS a
      JOIN filtered AS b ON b.code = a.code ORDER BY a.code"#;
    let params = json!(["001"]);
    let preview = call(
        &mut service,
        "sqlite_query",
        json!({
            "path":database,"sql":sql,"params":params,"limit":1,
        }),
    );
    assert_eq!(preview["columns"], json!(["code", "category", "bucket"]));
    assert_eq!(preview["rows"], json!([["001", null, "1"]]));
    assert_eq!(preview["truncated"], true);
    assert_eq!(preview["sql"], sql);
    assert_eq!(preview["params"], params);

    for sql in [
        "ATTACH DATABASE 'other.db' AS other",
        "SELECT 1; DELETE FROM empty",
        "WITH c AS (SELECT 1) DELETE FROM empty",
        "PRAGMA query_only=OFF",
        "SELECT load_extension('missing')",
    ] {
        assert!(
            service
                .call("sqlite_query", &json!({"path":database,"sql":sql}))
                .is_err(),
            "{sql}"
        );
    }
    assert!(
        service
            .call("sqlite_query", &json!({"path":database,"sql":"SELECT ?"}))
            .is_err()
    );
    assert!(service.call("sqlite_query_to_table", &json!({
        "path":database,"sql":"WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10001) SELECT x FROM n",
        "document_id":id,"destination_sheet":sheet,"anchor":"A1"
    })).unwrap_err().code == "result_too_large");

    let exported = call(
        &mut service,
        "sqlite_query_to_table",
        json!({
            "path":database,"sql":sql,"params":params,"document_id":id,"destination_sheet":sheet,"anchor":"A1",
        }),
    );
    assert_eq!(exported["range"], "A1:C4");
    assert_eq!(exported["rows"], 3);
    assert!(service.call("sqlite_query_to_table", &json!({
        "path":database,"sql":sql,"params":params,"document_id":id,"destination_sheet":sheet,"anchor":"A1",
    })).is_err());
    let pivot_args = json!({
        "sheet":sheet,"table":exported["table"],"destination_sheet":sheet,"anchor":"E3",
        "rows":["category"],"columns":["bucket"],"values":[{"field":"code","aggregate":"count"}],
    });
    let mut args = pivot_args.clone();
    args["document_id"] = id.clone();
    let pivot = call(&mut service, "xlsx_create_pivot", args.clone());
    assert_eq!(pivot["pivot"], "PivotTable1");
    assert!(service.call("xlsx_create_pivot", &args).is_err());
    let list_args = json!({"document_id":id,"sheet":sheet});
    let listed = call(&mut service, "xlsx_list_pivots", list_args.clone());
    assert_eq!(
        listed["pivots"][0]["source"],
        json!({"sheet":sheet,"name":"QueryResult1","range":"A1:C4"})
    );
    assert_eq!(listed["pivots"][0]["rows"], json!(["category"]));
    assert_eq!(listed["pivots"][0]["columns"], json!(["bucket"]));
    assert_eq!(listed["pivots"][0]["values"][0]["aggregate"], "count");
    let bottom = pivot["range"]
        .as_str()
        .unwrap()
        .split(':')
        .next_back()
        .unwrap();
    let first_digit = bottom.find(|c: char| c.is_ascii_digit()).unwrap();
    let row = bottom[first_digit..].parse::<u32>().unwrap();
    let blocker = format!("E{}", row + 1);
    call(
        &mut service,
        "xlsx_set_cells",
        json!({"document_id":id,"sheet":sheet,"range":blocker,"values":"Keep me"}),
    );
    call(
        &mut service,
        "xlsx_set_cells",
        json!({"document_id":id,"sheet":sheet,"range":"B3","values":"new category"}),
    );
    let refresh_args = json!({"document_id":id,"sheet":sheet,"pivot":"PivotTable1"});
    let error = service
        .call("xlsx_refresh_pivot", &refresh_args)
        .unwrap_err();
    assert_eq!(error.code, "destination_occupied");
    assert_eq!(
        call(&mut service, "xlsx_list_pivots", list_args.clone()),
        listed
    );
    let blocked = call(
        &mut service,
        "xlsx_get_cells",
        json!({"document_id":id,"sheet":sheet,"range":blocker}),
    );
    assert_eq!(blocked["rows"], json!([["Keep me"]]));
    call(
        &mut service,
        "xlsx_clear_cells",
        json!({"document_id":id,"sheet":sheet,"range":blocker,"what":"contents"}),
    );
    let refreshed = call(&mut service, "xlsx_refresh_pivot", refresh_args.clone());
    assert_ne!(refreshed["range"], pivot["range"]);
    assert_eq!(refreshed["updated_charts"], 0);
    assert_eq!(
        call(&mut service, "app_undo_document", json!({"document_id":id}))["undone"],
        1
    );
    assert_eq!(call(&mut service, "xlsx_list_pivots", list_args), listed);
    call(&mut service, "xlsx_refresh_pivot", refresh_args);
    let saved = dir.join("saved.xlsx");
    call(
        &mut service,
        "app_save_document",
        json!({"document_id":id,"path":saved}),
    );
    let presentation = ibis_formats::read(&std::fs::read(saved).unwrap()).unwrap();
    let saved_sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&presentation.slides[0]).unwrap();
    assert_eq!(saved_sheet.tables[0].name, "QueryResult1");
    assert_eq!(
        saved_sheet.pivots[0].source.name.as_deref(),
        Some("QueryResult1")
    );
    assert_eq!(saved_sheet.pivots[0].data_fields[0].function, "count");
    assert_eq!(saved_sheet.pivots[0].row_fields, [1]);
    assert_eq!(saved_sheet.pivots[0].col_fields, [2]);
    let pivot_area = saved_sheet.pivots[0].location;
    assert!(
        saved_sheet
            .cells_in_rows(pivot_area.first_row, pivot_area.last_row)
            .iter()
            .any(|cell| {
                (pivot_area.first_column..=pivot_area.last_column).contains(&cell.column)
                    && cell.value == ibis_doc_model::sheet::CellValue::Number(3.0)
            })
    );

    let mut draft = Document::new(ibis_formats::read(&blank).unwrap());
    let active = draft.presentation().slides[0].id;
    let context = || DraftToolContext {
        workbook: WorkbookToolContext {
            active_sheet: Some(active),
            ..Default::default()
        },
        ..Default::default()
    };
    let export = prepare_selected_sqlite_export(
        &database,
        json!({
            "sql":sql,"params":params,"destination_sheet":sheet,"anchor":"A1",
        }),
    )
    .unwrap();
    let (_, edits) = export.stage(&mut draft, context()).unwrap();
    assert_eq!(edits.len(), 1);
    let (built, edits) = run_draft_tool(
        Kind::Workbook,
        &mut draft,
        context(),
        "xlsx_create_pivot",
        pivot_args,
    )
    .unwrap();
    assert_eq!(
        built,
        serde_json::Value::Object(
            pivot
                .as_object()
                .unwrap()
                .iter()
                .filter(|(key, _)| *key != "document_id")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        )
    );
    assert_eq!(edits.len(), 1);
    let (listed, edits) = run_draft_tool(
        Kind::Workbook,
        &mut draft,
        context(),
        "xlsx_list_pivots",
        json!({}),
    )
    .unwrap();
    assert_eq!(listed["count"], 1);
    assert!(edits.is_empty());
    let active_slide = draft.presentation().slide(active).unwrap();
    let shape_index = active_slide.shapes.len();
    let pivot = &ibis_doc_model::sheet_calc::sheet_of_slide(active_slide)
        .unwrap()
        .pivots[0];
    let data = ibis_formats::pivot_data(draft.presentation(), active, 0).unwrap();
    let chart = ibis_doc_model::chart_from_pivot(pivot, &pivot.build(&data));
    let shape = ibis_doc_model::Shape {
        id: draft.ids().new_shape_id(),
        frame: ibis_doc_model::EmuRect::new(0, 0, 4_500_000, 3_000_000),
        rotation: 0,
        flip_h: false,
        flip_v: false,
        kind: ibis_doc_model::ShapeKind::Chart(Box::new(chart)),
        floating: None,
    };
    draft
        .execute(vec![ibis_doc_model::Command::InsertShape {
            slide: active,
            shapes: vec![(shape_index, shape)],
        }])
        .unwrap();
    run_draft_tool(
        Kind::Workbook,
        &mut draft,
        context(),
        "xlsx_set_cells",
        json!({"range":"B3","values":"new category"}),
    )
    .unwrap();
    let (updated, edits) = run_draft_tool(
        Kind::Workbook,
        &mut draft,
        context(),
        "xlsx_refresh_pivot",
        json!({"pivot":"PivotTable1"}),
    )
    .unwrap();
    assert_eq!(updated["range"], refreshed["range"]);
    assert_eq!(updated["updated_charts"], 1);
    assert_eq!(edits.len(), 2);
    assert_eq!(std::fs::read(&database).unwrap(), bytes);
    std::fs::remove_dir_all(dir).unwrap();
}
