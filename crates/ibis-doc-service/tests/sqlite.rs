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
    let definition = call(
        &mut service,
        "xlsx_list_query_reports",
        json!({"document_id":id,"table":exported["table"]}),
    );
    assert_eq!(definition["definition"]["input_schema"]["sales \"2026\""]["code"], "TEXT");
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
    assert_eq!(edits.len(), 2);
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

#[test]
fn sqlite_queries_join_registered_csv_and_sqlite_datasets() {
    use ibis_doc_service::run_selected_datasets_tool;
    let dir = std::env::temp_dir().join(format!("ibis-datasets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let database = dir.join("source.sqlite3");
    let csv = dir.join("labels.csv");
    let original = include_bytes!("../../ibis-doc-model/src/query/fixtures/external.sqlite3");
    std::fs::write(&database, original).unwrap();
    std::fs::write(&csv, "code,label\n001,\n002,窓口\n").unwrap();
    let mut service = Service::new(&[dir.clone()], true, Limits::default());
    let sql = "SELECT a.code, b.label, a.remark FROM facts a JOIN labels b ON a.code=b.code ORDER BY a.code";
    let args = json!({"sql":sql,"datasets":[
        {"source":database,"table":"sales \"2026\"","alias":"facts"},
        {"source":csv,"alias":"labels"},
    ]});
    let schema_args = json!({"datasets": args["datasets"]});
    let schema = service.call("sqlite_describe_datasets", &schema_args).unwrap();
    assert_eq!(schema, json!({"datasets":[
        {"alias":"facts","format":"sqlite","table":"sales \"2026\"","columns":[
            {"name":"code","sql_type":"DYNAMIC"},
            {"name":"amount","sql_type":"DYNAMIC"},
            {"name":"remark","sql_type":"DYNAMIC"},
        ]},
        {"alias":"labels","format":"csv","columns":[
            {"name":"code","sql_type":"TEXT"},
            {"name":"label","sql_type":"TEXT"},
        ]},
    ]}));
    let selected = [database.clone(), csv.clone()];
    let numbered = json!({"datasets":[
        {"source":"1","table":"sales \"2026\"","alias":"facts"},
        {"source":"2","alias":"labels"},
    ]});
    assert_eq!(
        run_selected_datasets_tool(&selected, "sqlite_describe_datasets", numbered.clone()).unwrap(),
        schema
    );
    assert!(run_selected_datasets_tool(&selected, "sqlite_describe_datasets", schema_args.clone()).is_err());
    let mut invalid = numbered;
    invalid["datasets"][0]["source"] = json!("3");
    assert!(run_selected_datasets_tool(&selected, "sqlite_describe_datasets", invalid).is_err());
    assert!(Service::new(&[], true, Limits::default()).call("sqlite_describe_datasets", &schema_args).is_err());
    let mcp = service.call("sqlite_query", &args).unwrap();
    assert_eq!(
        mcp["rows"],
        json!([["001", "", null], ["002", "窓口", "ok"]])
    );
    let assistant_args = json!({"sql":sql,"datasets":[
        {"source":"1","table":"sales \"2026\"","alias":"facts"},
        {"source":"2","alias":"labels"},
    ]});
    assert_eq!(
        run_selected_datasets_tool(&selected, "sqlite_query", assistant_args).unwrap(),
        mcp
    );
    assert!(run_selected_datasets_tool(&selected, "sqlite_query", args.clone()).is_err());
    let mut invalid = args.clone();
    invalid["datasets"][1]["alias"] = json!("FACTS");
    assert!(service.call("sqlite_query", &invalid).is_err());
    assert!(
        Service::new(&[], true, Limits::default())
            .call("sqlite_query", &args)
            .is_err()
    );
    let workbook = dir.join("analysis.xlsx");
    std::fs::write(
        &workbook,
        ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap(),
    )
    .unwrap();
    let mut writer = Service::new(&[dir.clone()], false, Limits::default());
    let id = writer
        .call("app_open_document", &json!({"path":workbook}))
        .unwrap()["document_id"]
        .clone();
    let sheet = writer
        .call("xlsx_list_sheets", &json!({"document_id":id}))
        .unwrap()["sheets"][0]["name"]
        .clone();
    let mut output = args;
    output["document_id"] = id.clone();
    output["destination_sheet"] = sheet.clone();
    output["anchor"] = json!("A1");
    let table = writer.call("sqlite_query_to_table", &output).unwrap();
    assert_eq!(table["range"], "A1:C3");
    let pivot = writer.call("xlsx_create_pivot", &json!({
        "document_id": id, "sheet":sheet, "destination_sheet":sheet, "anchor":"E1",
        "table":table["table"], "rows":["label"],"values":[{"field":"code","aggregate":"count"}],
    })).unwrap();
    assert_eq!(pivot["pivot"], "PivotTable1");
    writer.call("xlsx_set_query_report_checks", &json!({
        "document_id":id,"table":table["table"],"checks":[{
            "name":"自治体マスタに未登録",
            "sql":"SELECT count(*) FROM facts a LEFT JOIN labels b ON a.code=b.code WHERE b.code IS NULL",
        }],
    })).unwrap();
    let saved = dir.join("report.xlsx");
    writer
        .call("app_save_document", &json!({"document_id":id,"path":saved}))
        .unwrap();
    let saved_bytes = std::fs::read(&saved).unwrap();
    let reopened = writer
        .call("app_open_document", &json!({"path":saved}))
        .unwrap()["document_id"]
        .clone();
    let reports = writer
        .call("xlsx_list_query_reports", &json!({"document_id":reopened}))
        .unwrap();
    assert_eq!(reports["reports"][0]["sources"][1]["file"], "labels.csv");
    assert_eq!(reports["reports"][0]["sources"][1]["alias"], "labels");
    assert!(!reports.to_string().contains(dir.to_str().unwrap()));

    std::fs::write(&csv, "code,label\n001,変更後\n").unwrap();
    let args = json!({"document_id":reopened,"table":table["table"],"sources":[database,csv]});
    assert!(
        writer
            .call(
                "sqlite_refresh_query_table",
                &json!({
                    "document_id":reopened,"table":table["table"],"sources":[csv,database],
                })
            )
            .is_err()
    );
    let refreshed = writer.call("sqlite_refresh_query_table", &args).unwrap();
    assert_eq!(refreshed["range"], "A1:C2");
    assert_eq!(refreshed["updated_pivots"], 1);
    assert_eq!(
        refreshed["checks"],
        json!([
            {"name":"自治体マスタに未登録","count":2,"max":null,"passed":null}
        ])
    );
    let read = |writer: &mut Service| {
        writer
            .call(
                "xlsx_get_cells",
                &json!({
                    "document_id":reopened,"sheet":sheet,"range":"A2:C3",
                }),
            )
            .unwrap()
    };
    assert_eq!(read(&mut writer)["rows"][0][1], "変更後");
    assert!(read(&mut writer)["rows"][1][0].is_null());
    let current = writer
        .call(
            "xlsx_list_pivots",
            &json!({"document_id":reopened,"sheet":sheet}),
        )
        .unwrap();
    assert_eq!(current["pivots"][0]["source"]["range"], "A1:C2");
    assert_eq!(
        writer
            .call("app_undo_document", &json!({"document_id":reopened}))
            .unwrap()["undone"],
        1
    );
    assert_eq!(read(&mut writer)["rows"][1][0], "002");
    let previous = writer
        .call(
            "xlsx_list_pivots",
            &json!({"document_id":reopened,"sheet":sheet}),
        )
        .unwrap();
    assert_eq!(previous["pivots"][0]["source"]["range"], "A1:C3");

    let mut draft = ibis_doc_model::Document::new(ibis_formats::read(&saved_bytes).unwrap());
    let slide = draft.presentation().slides[0].id;
    let source = draft.presentation().slide(slide).unwrap();
    let shape_index = source.shapes.len();
    let pivot = &ibis_doc_model::sheet_calc::sheet_of_slide(source)
        .unwrap()
        .pivots[0];
    let data = ibis_formats::pivot_data(draft.presentation(), slide, 0).unwrap();
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
            slide,
            shapes: vec![(shape_index, shape)],
        }])
        .unwrap();
    let draft_args = json!({"table":table["table"],"sources":["1","2"]});
    let definition =
        ibis_doc_service::query_report_definition(draft.presentation(), &draft_args).unwrap();
    let prepared =
        ibis_doc_service::prepare_selected_query_refresh(&selected, draft_args, definition)
            .unwrap();
    let (draft_result, commands) = prepared.stage(&mut draft, Default::default()).unwrap();
    assert_eq!(draft_result["updated_pivots"], 1);
    assert_eq!(draft_result["updated_charts"], 1);
    assert_eq!(commands.len(), 3);
    std::fs::write(&csv, "code,label\n").unwrap();
    let empty_args = json!({"table":table["table"],"sources":["1","2"]});
    let definition =
        ibis_doc_service::query_report_definition(draft.presentation(), &empty_args).unwrap();
    let prepared =
        ibis_doc_service::prepare_selected_query_refresh(&selected, empty_args, definition)
            .unwrap();
    let (empty, _) = prepared.stage(&mut draft, Default::default()).unwrap();
    assert_eq!(empty["rows"], 0);
    assert_eq!(empty["updated_charts"], 1);
    let chart = &draft.presentation().slide(slide).unwrap().shapes[shape_index].kind;
    let ibis_doc_model::ShapeKind::Chart(chart) = chart else {
        panic!("expected a pivot chart")
    };
    assert!(
        chart
            .groups
            .iter()
            .flat_map(|group| &group.series)
            .all(|series| {
                !series
                    .categories
                    .iter()
                    .any(|category| category == "変更後")
                    && series.values.iter().flatten().all(|value| value.0 == 0.0)
            })
    );

    std::fs::write(&csv, "code,label\n001,A\n002,B\n003,C\n").unwrap();
    writer
        .call(
            "xlsx_set_cells",
            &json!({"document_id":reopened,"sheet":sheet,"range":"A4","values":"Keep"}),
        )
        .unwrap();
    assert_eq!(
        writer
            .call("sqlite_refresh_query_table", &args)
            .unwrap_err()
            .code,
        "destination_occupied"
    );
    assert_eq!(read(&mut writer)["rows"][1][0], "002");

    assert_eq!(std::fs::read(&database).unwrap(), original);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sqlite_batch_refresh_previews_and_commits_zero_rows_atomically() {
    use ibis_doc_model::Document;
    use ibis_doc_service::{batch_query_report_definitions, prepare_selected_report_batch};
    use ibis_formats::Kind;

    let dir = std::env::temp_dir().join(format!("ibis-batch-refresh-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("monthly.csv");
    let next = dir.join("next.csv");
    std::fs::write(&csv, "code,category\n001,old\n002,old\n").unwrap();
    let file = dir.join("analysis.xlsx");
    std::fs::write(&file, ibis_formats::blank_file(Kind::Workbook).unwrap()).unwrap();
    let mut service = Service::new(&[dir.clone()], false, Limits::default());
    let call = |service: &mut Service, tool, args| service.call(tool, &args).unwrap();
    let id = call(&mut service, "app_open_document", json!({"path":file}))["document_id"].clone();
    let sheet =
        call(&mut service, "xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"]
            .clone();
    for anchor in ["A1", "G1"] {
        call(
            &mut service,
            "sqlite_query_to_table",
            json!({
                "sql":"SELECT code, category, CASE category WHEN 'old' THEN 12000 ELSE 6000 END AS cases FROM monthly ORDER BY code",
                "datasets":[{"source":csv,"alias":"monthly"}],
                "document_id":id,"destination_sheet":sheet,"anchor":anchor,
            }),
        );
    }
    call(
        &mut service,
        "xlsx_create_pivot",
        json!({
            "document_id":id,"sheet":sheet,"destination_sheet":sheet,"anchor":"A8",
            "table":"QueryResult1","rows":["category"],"values":[{"field":"code","aggregate":"count"}],
        }),
    );
    let check_args = json!({"document_id":id,"table":"QueryResult2","checks":[{
        "name":"重複コード","sql":"SELECT count(*) FROM (SELECT code FROM monthly GROUP BY code HAVING count(*) > ?)","params":[1],"max":0
    }]});
    call(&mut service, "xlsx_set_query_report_checks", check_args);
    let mut invalid = json!({"document_id":id,"table":"QueryResult2","checks":[{
        "name":"bad","sql":"SELECT 1; DELETE FROM monthly"
    }]});
    assert!(
        service
            .call("xlsx_set_query_report_checks", &invalid)
            .is_err()
    );
    invalid["checks"][0]["sql"] = json!("SELECT 1");
    invalid["checks"][0]["max"] = json!(-1);
    assert!(
        service
            .call("xlsx_set_query_report_checks", &invalid)
            .is_err()
    );
    let original = dir.join("original.xlsx");
    call(
        &mut service,
        "app_save_document",
        json!({"document_id":id,"path":original}),
    );

    let mut refresh = json!({"document_id":id, "bindings":{"monthly.csv":next}});
    std::fs::write(&next, "code,category\n001,duplicate\n001,duplicate\n").unwrap();
    refresh["preview"] = json!(true);
    let blocked = call(
        &mut service,
        "sqlite_refresh_query_reports",
        refresh.clone(),
    );
    assert_eq!(blocked["blocked"], true);
    assert_eq!(
        blocked["reports"][1]["checks"],
        json!([{"name":"重複コード","count":1,"max":0,"passed":false}])
    );
    let view = ibis_formats::read(&std::fs::read(&original).unwrap()).unwrap();
    let request = json!({"bindings":{"monthly.csv":"1"}});
    let definitions = batch_query_report_definitions(&view, &request).unwrap();
    let review =
        prepare_selected_report_batch(std::slice::from_ref(&next), request, definitions).unwrap();
    let (summary, commands) = review.review(&view).unwrap();
    assert_eq!(summary["blocked"], true);
    assert!(commands.is_empty());
    refresh["preview"] = json!(false);
    let failed = service
        .call("sqlite_refresh_query_reports", &refresh)
        .unwrap_err();
    assert_eq!(failed.code, "query_check_failed");
    assert!(failed.message.contains("QueryResult2"));
    assert_eq!(
        call(
            &mut service,
            "xlsx_get_cells",
            json!({"document_id":id,"sheet":sheet,"range":"B2"})
        )["rows"],
        json!([["old"]])
    );
    std::fs::write(&next, "code,category\n001,new\n002,new\n003,new\n").unwrap();
    call(
        &mut service,
        "xlsx_set_cells",
        json!({"document_id":id,"sheet":sheet,"range":"G4","values":"Keep"}),
    );
    let error = service
        .call("sqlite_refresh_query_reports", &refresh)
        .unwrap_err();
    assert_eq!(error.code, "destination_occupied");
    assert!(error.message.contains("QueryResult2"));
    let first = call(
        &mut service,
        "xlsx_get_cells",
        json!({"document_id":id,"sheet":sheet,"range":"B2"}),
    );
    assert_eq!(first["rows"], json!([["old"]]));
    call(
        &mut service,
        "xlsx_clear_cells",
        json!({"document_id":id,"sheet":sheet,"range":"G4","what":"contents"}),
    );

    std::fs::write(&next, "code,category\n001,new\n002,new\n").unwrap();
    refresh["metrics"] = json!([
        {"table":"QueryResult1","column":"cases"},
        {"table":"queryresult2","column":"cases"},
    ]);
    refresh["preview"] = json!(true);
    let compared = call(
        &mut service,
        "sqlite_refresh_query_reports",
        refresh.clone(),
    );
    for report in compared["reports"].as_array().unwrap() {
        assert_eq!(report["previous_rows"], report["rows"]);
        assert_eq!(
            report["metrics"],
            json!([
                {"column":"cases","previous":24000.0,"current":12000.0,"delta":-12000.0,"percent_change":-50.0}
            ])
        );
    }
    let mut invalid = refresh.clone();
    invalid["metrics"][1]["column"] = json!("category");
    let error = service
        .call("sqlite_refresh_query_reports", &invalid)
        .unwrap_err();
    assert_eq!(error.code, "invalid_argument");
    assert!(error.message.contains("finite numeric values"));

    std::fs::write(&next, "code,category\n").unwrap();
    refresh["preview"] = json!(true);
    let preview = call(
        &mut service,
        "sqlite_refresh_query_reports",
        refresh.clone(),
    );
    assert_eq!(preview["preview"], true);
    assert_eq!(preview["blocked"], false);
    assert_eq!(preview["reports"][1]["checks"][0]["count"], 0);
    assert_eq!(preview["reports"][1]["checks"][0]["passed"], true);
    assert_eq!(preview["reports"][0]["previous_rows"], 2);
    assert_eq!(preview["reports"][0]["rows"], 0);
    assert_eq!(preview["reports"][0]["updated_pivots"], 1);
    let still_old = call(
        &mut service,
        "xlsx_get_cells",
        json!({"document_id":id,"sheet":sheet,"range":"A2"}),
    );
    assert_eq!(still_old["rows"], json!([["001"]]));

    refresh["preview"] = json!(false);
    let updated = call(
        &mut service,
        "sqlite_refresh_query_reports",
        refresh.clone(),
    );
    assert_eq!(updated["reports"][0]["range"], "A1:C1");
    assert_eq!(updated["reports"][0]["metrics"][0]["delta"], -24000.0);
    assert_eq!(updated["reports"][1]["range"], "G1:I1");
    let cleared = call(
        &mut service,
        "xlsx_get_cells",
        json!({"document_id":id,"sheet":sheet,"range":"A2:I3"}),
    );
    assert!(cleared["rows"].as_array().unwrap().iter().all(|row| {
        row.as_array()
            .unwrap()
            .iter()
            .all(serde_json::Value::is_null)
    }));
    let undo = call(&mut service, "app_undo_document", json!({"document_id":id}));
    assert_eq!(undo["undone"], 1);
    let restored = call(
        &mut service,
        "xlsx_get_cells",
        json!({"document_id":id,"sheet":sheet,"range":"G2"}),
    );
    assert_eq!(restored["rows"], json!([["001"]]));

    call(
        &mut service,
        "sqlite_refresh_query_reports",
        refresh.clone(),
    );
    let empty = dir.join("empty.xlsx");
    call(
        &mut service,
        "app_save_document",
        json!({"document_id":id,"path":empty}),
    );
    let mut draft = Document::new(ibis_formats::read(&std::fs::read(&empty).unwrap()).unwrap());
    let empty_sheet =
        ibis_doc_model::sheet_calc::sheet_of_slide(&draft.presentation().slides[0]).unwrap();
    assert_eq!(empty_sheet.tables[0].range.last_row, 0);
    assert_eq!(
        ibis_formats::pivot_data(draft.presentation(), draft.presentation().slides[0].id, 0)
            .unwrap()
            .records
            .len(),
        0
    );

    std::fs::write(&next, "code,category\n003,new\n").unwrap();
    let args = json!({"bindings":{"monthly.csv":"1"},"metrics":[{"table":"QueryResult1","column":"cases"}]});
    let definitions = batch_query_report_definitions(draft.presentation(), &args).unwrap();
    let prepared =
        prepare_selected_report_batch(std::slice::from_ref(&next), args, definitions).unwrap();
    let (result, edits) = prepared.review(draft.presentation()).unwrap();
    std::fs::write(&next, "code,category\n004,changed after review\n").unwrap();
    draft.execute(edits.clone()).unwrap();
    let sheet =
        ibis_doc_model::sheet_calc::sheet_of_slide(&draft.presentation().slides[0]).unwrap();
    assert_eq!(
        sheet.cell(1, 0).unwrap().value,
        ibis_doc_model::CellValue::Text("003".into())
    );
    assert!(draft.undo());
    assert_eq!(result["reports"][0]["previous_rows"], 0);
    assert_eq!(result["reports"][0]["rows"], 1);
    assert_eq!(result["reports"][1]["rows"], 1);
    let metric = &result["reports"][0]["metrics"][0];
    assert_eq!(metric["previous"], 0.0);
    assert_eq!(metric["current"], 6000.0);
    assert!(metric["percent_change"].is_null());
    assert_eq!(edits.len(), 3);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parquet_query_report_preserves_projection_and_filter_after_reopen() {
    let dir =
        std::env::temp_dir().join(format!("ibis-parquet-query-columns-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.parquet");
    // Synthetic rows ("00", 0, [0]) and ("01", 1, [1]).
    std::fs::write(&source, include_bytes!("fixtures/query-projection.parquet")).unwrap();
    let workbook = dir.join("blank.xlsx");
    std::fs::write(
        &workbook,
        ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap(),
    )
    .unwrap();

    let mut service = Service::new(&[dir.clone()], false, Limits::default());
    let mut call = |tool, args| service.call(tool, &args).unwrap();
    let id = call("app_open_document", json!({"path":workbook}))["document_id"].clone();
    let sheet = call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].clone();
    let columns = json!(["amount", "code"]);
    let args = json!({
        "sql": "SELECT * FROM facts ORDER BY code",
        "datasets": [{"source":source,"alias":"facts","columns":columns}],
    });
    let query = call("sqlite_query", args.clone());
    assert_eq!(query["columns"], columns);
    assert_eq!(query["rows"], json!([[0, "00"], [1, "01"]]));
    let filter = json!({"amount":{"gte":1,"lt":2}, "code":"01"});
    let mut report = args;
    report["datasets"][0]["where"] = filter.clone();
    assert_eq!(
        call("sqlite_query", report.clone())["rows"],
        json!([[1, "01"]])
    );
    report["document_id"] = id.clone();
    report["destination_sheet"] = sheet;
    report["anchor"] = json!("A1");
    let name = call("sqlite_query_to_table", report)["table"].clone();
    let saved = dir.join("saved.xlsx");
    call("app_save_document", json!({"document_id":id,"path":saved}));
    call("app_close_document", json!({"document_id":id}));
    let id = call("app_open_document", json!({"path":saved}))["document_id"].clone();
    let reports = call("xlsx_list_query_reports", json!({"document_id":id}));
    assert_eq!(reports["reports"][0]["sources"][0]["columns"], columns);
    assert_eq!(reports["reports"][0]["sources"][0]["where"], filter);
    let preview = call(
        "sqlite_refresh_query_reports",
        json!({
            "document_id":id,"bindings":{"source.parquet":source},"preview":true,
        }),
    );
    assert_eq!(preview["reports"][0]["rows"], 1);
    assert_eq!(preview["dirty"], false);
    let refreshed = call(
        "sqlite_refresh_query_table",
        json!({
            "document_id":id,"table":name,"sources":[source],
        }),
    );
    assert_eq!(refreshed["rows"], 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parquet_text_lists_can_be_expanded_in_turso_without_duplicating_amounts() {
    let dir = std::env::temp_dir().join(format!("ibis-parquet-text-lists-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.parquet");
    std::fs::write(&source, include_bytes!("fixtures/query-text-lists.parquet")).unwrap();
    let mut service = Service::new(&[dir.clone()], true, Limits::default());
    let mut query = |sql| service.call("sqlite_query", &json!({
        "sql":sql,
        "datasets":[{"source":source,"alias":"facts","columns":["tags","code","amount"]}]
    })).unwrap()["rows"].clone();
    assert_eq!(query("SELECT code, tags FROM facts ORDER BY code"), json!([
        ["001", "[\"line\\\"1\",null,\"業務\",\"業務\"]"], ["002","[]"], ["003",null], ["004","[\"業務\"]"]
    ]));
    assert_eq!(
        query("SELECT f.code, j.key, j.value FROM facts f CROSS JOIN json_each(f.tags) j ORDER BY f.code, j.key"),
        json!([["001",0,"line\"1"],["001",1,null],["001",2,"業務"],["001",3,"業務"],["004",0,"業務"]])
    );
    assert_eq!(
        query("SELECT SUM(amount) FROM facts f WHERE EXISTS (SELECT 1 FROM json_each(f.tags) j WHERE j.value = '業務')"),
        json!([[500]])
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn arrow_ipc_and_parquet_join_and_refresh_from_a_saved_report() {
    let dir = std::env::temp_dir().join(format!("ibis-arrow-sql-datasets-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let facts = dir.join("facts.parquet");
    let lookup = dir.join("lookup.feather");
    let stream = dir.join("lookup.ipc");
    std::fs::write(&facts, include_bytes!("fixtures/query-text-lists.parquet")).unwrap();
    std::fs::write(&lookup, include_bytes!("fixtures/query-lookup.feather")).unwrap();
    std::fs::write(&stream, include_bytes!("fixtures/query-lookup.ipc")).unwrap();
    let workbook = dir.join("blank.xlsx");
    std::fs::write(&workbook, ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
    let mut service = Service::new(&[dir.clone()], false, Limits::default());
    let mut call = |tool, args| service.call(tool, &args).unwrap();
    let mut args = json!({
        "sql": "SELECT f.code, f.amount, l.labels, typeof(f.amount) AS amount_type FROM facts f JOIN labels l USING (code) ORDER BY f.code",
        "datasets": [
            {"source":facts,"alias":"facts","columns":["code","amount"]},
            {"source":lookup,"alias":"labels","columns":["labels","code"]},
        ],
    });
    let schema = call("sqlite_describe_datasets", json!({"datasets":[
        {"source":facts,"alias":"facts","columns":["amount","code"],"where":{"amount":{"gte":100}}},
        {"source":lookup,"alias":"labels","columns":["labels","code"]},
        {"source":stream,"alias":"stream","columns":["code"]},
    ]}));
    assert_eq!(schema, json!({"datasets":[
        {"alias":"facts","format":"parquet","columns":[
            {"name":"amount","sql_type":"INTEGER"},{"name":"code","sql_type":"TEXT"},
        ]},
        {"alias":"labels","format":"arrow_ipc","columns":[
            {"name":"labels","sql_type":"TEXT"},{"name":"code","sql_type":"TEXT"},
        ]},
        {"alias":"stream","format":"arrow_ipc","columns":[{"name":"code","sql_type":"TEXT"}]},
    ]}));
    assert_eq!(call("sqlite_query", args.clone())["rows"], json!([
        ["001",100,"[\"甲\",null,\"甲\"]","integer"],
        ["002",200,"[]","integer"],
        ["003",300,null,"integer"],
    ]));
    let mut nested = args.clone();
    nested["sql"] = json!("SELECT j.key, j.value FROM labels l CROSS JOIN json_each(l.labels) j WHERE l.code = '001' ORDER BY j.key");
    assert_eq!(call("sqlite_query", nested)["rows"], json!([[0,"甲"],[1,null],[2,"甲"]]));

    let id = call("app_open_document", json!({"path":workbook}))["document_id"].clone();
    let sheet = call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].clone();
    args["document_id"] = id.clone();
    args["destination_sheet"] = sheet;
    args["anchor"] = json!("A1");
    call("sqlite_query_to_table", args);
    let saved = dir.join("saved.xlsx");
    call("app_save_document", json!({"document_id":id,"path":saved}));
    call("app_close_document", json!({"document_id":id}));
    let id = call("app_open_document", json!({"path":saved}))["document_id"].clone();
    let reports = call("xlsx_list_query_reports", json!({"document_id":id}));
    assert_eq!(reports["reports"][0]["sources"][1]["kind"], "arrow_ipc");
    assert_eq!(reports["reports"][0]["sources"][1]["columns"], json!(["labels","code"]));
    let refreshed = call("sqlite_refresh_query_reports", json!({
        "document_id":id,"bindings":{"facts.parquet":facts,"lookup.feather":stream},
    }));
    assert_eq!(refreshed["reports"][0]["rows"], 3);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn disk_staged_arrow_reports_share_a_batch_after_reopening() {
    let dir = std::env::temp_dir().join(format!("ibis-arrow-disk-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let file = dir.join("input.feather");
    let stream = dir.join("input.ipc");
    std::fs::write(&file, include_bytes!("fixtures/query-lookup.feather")).unwrap();
    std::fs::write(&stream, include_bytes!("fixtures/query-lookup.ipc")).unwrap();
    let original = dir.join("blank.xlsx");
    std::fs::write(&original, ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
    let mut service = Service::new(&[dir.clone()], false, Limits::default());
    let mut call = |tool, args| service.call(tool, &args).unwrap();
    let id = call("app_open_document", json!({"path":original}))["document_id"].clone();
    let sheet = call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].clone();
    for (anchor, sql) in [
        ("A1", "SELECT count(*) AS n FROM facts"),
        ("D1", "SELECT count(*) AS n FROM facts f WHERE EXISTS (SELECT 1 FROM json_each(f.labels) j WHERE j.value = '甲')"),
    ] {
        call("sqlite_query_to_table", json!({
            "document_id":id,"destination_sheet":sheet,"anchor":anchor,"sql":sql,
            "datasets":[{"source":file,"alias":"facts","columns":["labels","code"]}],
        }));
    }
    let saved = dir.join("report.xlsx");
    call("app_save_document", json!({"document_id":id,"path":saved}));
    call("app_close_document", json!({"document_id":id}));
    let id = call("app_open_document", json!({"path":saved}))["document_id"].clone();
    for path in [&file, &stream] {
        let baseline = call("sqlite_refresh_query_reports", json!({
            "document_id":id,"bindings":{"input.feather":path},"preview":true,
            "metrics":[{"table":"QueryResult1","column":"n"},{"table":"QueryResult2","column":"n"}],
        }));
        let staged = call("sqlite_refresh_query_reports", json!({
            "document_id":id,"bindings":{"input.feather":path},"preview":true,"arrow_disk":true,
            "metrics":[{"table":"QueryResult1","column":"n"},{"table":"QueryResult2","column":"n"}],
        }));
        assert_eq!(baseline["reports"], staged["reports"]);
        assert_eq!(staged["reports"].as_array().unwrap().len(), 2);
    }
    let applied = call("sqlite_refresh_query_reports", json!({
        "document_id":id,"bindings":{"input.feather":stream},"arrow_disk":true,
    }));
    assert_eq!(applied["reports"].as_array().unwrap().len(), 2);
    assert_eq!(applied["dirty"], true);
    std::fs::write(&stream, include_bytes!("fixtures/query-large-lookup.ipc")).unwrap();
    let preview = call("sqlite_refresh_query_reports", json!({
        "document_id":id,"bindings":{"input.feather":stream},"preview":true,
        "metrics":[{"table":"QueryResult1","column":"n"}],
    }));
    assert_eq!(preview["reports"][0]["metrics"][0]["current"].as_f64(), Some(50_001.0));
    std::fs::remove_file(&stream).unwrap();
    let applied = call("sqlite_refresh_query_reports", json!({
        "document_id":id,"snapshot_id":preview["snapshot_id"],
    }));
    assert_eq!(applied["preview"], false);
    assert_eq!(applied["reports"], preview["reports"]);
    let sheet = call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].clone();
    assert_eq!(call("xlsx_get_cells", json!({
        "document_id":id,"sheet":sheet,"range":"A2",
    }))["rows"][0][0].as_f64(), Some(50_001.0));
    let stale = call("sqlite_refresh_query_reports", json!({
        "document_id":id,"bindings":{"input.feather":file},"preview":true,
    }));
    call("xlsx_set_cells", json!({"document_id":id,"sheet":sheet,"range":"B20","values":"changed"}));
    let error = service.call("sqlite_refresh_query_reports", &json!({
        "document_id":id,"snapshot_id":stale["snapshot_id"],
    })).unwrap_err();
    assert!(error.message.contains("Workbook changed"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn json_lines_and_avro_stream_into_shared_report_snapshots() {
    use std::io::Write;

    let dir = std::env::temp_dir().join(format!("ibis-json-avro-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let lines = dir.join("lookup.ndjson");
    let avro = dir.join("observations.avro");
    std::fs::write(&lines, "{\"code\":\"001\",\"labels\":[\"a\",null]}\n{\"code\":\"002\",\"late\":true}\n").unwrap();
    std::fs::write(&avro, include_bytes!("fixtures/query-observations.avro")).unwrap();
    let blank = dir.join("blank.xlsx");
    std::fs::write(&blank, ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
    let mut service = Service::new(&[dir.clone()], false, Limits::default());
    let mut call = |tool, args| service.call(tool, &args).unwrap();
    let sources = json!([
        {"source":lines,"alias":"labels","columns":["code","labels","late"]},
        {"source":avro,"alias":"facts","columns":["amount","code"]},
    ]);
    let schema = call("sqlite_describe_datasets", json!({"datasets": sources}));
    assert_eq!(schema, json!({"datasets":[
        {"alias":"labels","format":"json_lines","columns":[
            {"name":"code","sql_type":"TEXT"},{"name":"labels","sql_type":"TEXT"},
            {"name":"late","sql_type":"INTEGER"},
        ]},
        {"alias":"facts","format":"avro","columns":[
            {"name":"amount","sql_type":"INTEGER"},{"name":"code","sql_type":"TEXT"},
        ]},
    ]}));
    let preview = call("sqlite_query", json!({
        "datasets":sources,
        "sql":"SELECT f.code, f.amount, j.key, j.value, l.late FROM facts f JOIN labels l USING(code) LEFT JOIN json_each(l.labels) j ON true ORDER BY f.code, j.key",
    }));
    assert_eq!(preview["rows"], json!([["001",5,0,"a",null],["001",5,1,null,null],["002",null,null,null,1]]));
    let sparse = call("sqlite_query", json!({
        "datasets":[{"source":lines,"alias":"labels"}],
        "sql":"SELECT code, late, labels FROM labels ORDER BY code",
    }));
    assert_eq!(sparse["rows"], json!([["001",null,"[\"a\",null]"],["002",1,null]]));
    let id = call("app_open_document", json!({"path":blank}))["document_id"].clone();
    let sheet = call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].clone();
    for (anchor, sql) in [("A1","SELECT count(*) AS n FROM labels"), ("D1","SELECT sum(amount) AS n FROM facts")] {
        call("sqlite_query_to_table", json!({
            "document_id":id,"destination_sheet":sheet,"anchor":anchor,"sql":sql,"datasets":sources,
        }));
    }
    let saved = dir.join("saved.xlsx");
    call("app_save_document", json!({"document_id":id,"path":saved}));
    call("app_close_document", json!({"document_id":id}));
    let id = call("app_open_document", json!({"path":saved}))["document_id"].clone();

    std::fs::write(&avro, include_bytes!("fixtures/query-large-observations.avro")).unwrap();
    assert_eq!(call("sqlite_query", json!({
        "datasets":[sources[1].clone()],"sql":"SELECT count(*), sum(amount) FROM facts",
    }))["rows"], json!([[50_001,250_005]]));
    let mut writer = std::io::BufWriter::new(std::fs::File::create(&lines).unwrap());
    for _ in 0..50_001 {
        writer.write_all(b"{\"code\":\"001\",\"labels\":[]}\n").unwrap();
    }
    writer.flush().unwrap();
    drop(writer);
    let preview = call("sqlite_refresh_query_reports", json!({
        "document_id":id,"preview":true,"bindings":{"lookup.ndjson":lines,"observations.avro":avro},
        "metrics":[{"table":"QueryResult1","column":"n"},{"table":"QueryResult2","column":"n"}],
    }));
    assert_eq!(preview["reports"][0]["metrics"][0]["current"].as_f64(), Some(50_001.0));
    assert_eq!(preview["reports"][1]["metrics"][0]["current"].as_f64(), Some(250_005.0));
    std::fs::remove_file(lines).unwrap();
    std::fs::remove_file(avro).unwrap();
    let applied = call("sqlite_refresh_query_reports", json!({"document_id":id,"snapshot_id":preview["snapshot_id"]}));
    assert_eq!(applied["reports"], preview["reports"]);
    for (range, number) in [("A2",50_001.0),("D2",250_005.0)] {
        let cell = call("xlsx_get_cells", json!({"document_id":id,"sheet":sheet,"range":range}));
        assert_eq!(cell["rows"][0][0].as_f64(), Some(number));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn query_report_warns_about_changed_input_schema_and_applies_the_reviewed_baseline() {
    let dir = std::env::temp_dir().join(format!("ibis-report-schema-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let facts = dir.join("facts.jsonl");
    let next = dir.join("next.jsonl");
    let labels = dir.join("labels.csv");
    let workbook = dir.join("report.xlsx");
    std::fs::write(&facts, "{\"code\":\"001\",\"retired\":true}\n").unwrap();
    std::fs::write(&next, "{\"code\":1,\"extra\":\"north\"}\n").unwrap();
    std::fs::write(&labels, "code,label\n001,North\n").unwrap();
    std::fs::write(&workbook, ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();

    let mut service = Service::new(&[dir.clone()], false, Limits::default());
    let mut call = |name, args| service.call(name, &args).unwrap();
    let id = call("app_open_document", json!({"path":workbook}))["document_id"].clone();
    let sheet = call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].clone();
    let table = call("sqlite_query_to_table", json!({
        "document_id":id,"destination_sheet":sheet,"anchor":"A1",
        "sql":"SELECT count(*) AS matches FROM facts JOIN labels USING (code)",
        "datasets":[{"source":facts,"alias":"facts"},{"source":labels,"alias":"labels"}],
    }))["table"].clone();
    call("app_save_document", json!({"document_id":id,"path":workbook,"overwrite":true}));
    call("app_close_document", json!({"document_id":id}));
    let id = call("app_open_document", json!({"path":workbook}))["document_id"].clone();
    let report = json!({"document_id":id,"table":table});
    let original = call("xlsx_list_query_reports", report.clone());
    assert_eq!(original["definition"]["input_schema"]["facts"], json!({"code":"TEXT","retired":"INTEGER"}));

    let preview_args = json!({
        "document_id":id, "bindings":{"facts.jsonl":next,"labels.csv":labels}, "preview":true,
        "metrics":[{"table":table,"column":"matches"}],
    });
    let preview = call("sqlite_refresh_query_reports", preview_args.clone());
    assert_eq!(preview["blocked"], false);
    let changes = json!([
        {"source":"facts","column":"code","previous":"TEXT","current":"INTEGER"},
        {"source":"facts","column":"retired","previous":"INTEGER","current":null},
        {"source":"facts","column":"extra","previous":null,"current":"TEXT"},
    ]);
    assert_eq!(preview["reports"][0]["schema_change_count"], 3);
    assert_eq!(preview["reports"][0]["schema_changes"], changes);
    assert_eq!(preview["reports"][0]["metrics"][0]["previous"], 1.0);
    assert_eq!(preview["reports"][0]["metrics"][0]["current"], 0.0);
    assert_eq!(
        call("xlsx_list_query_reports", report.clone())["definition"],
        original["definition"],
    );

    // Both result rows and the new schema must come from the reviewed snapshot.
    std::fs::write(&next, "{\"code\":\"001\"}\n").unwrap();
    let applied = call("sqlite_refresh_query_reports", json!({
        "document_id":id,"snapshot_id":preview["snapshot_id"],
    }));
    assert_eq!(applied["reports"][0]["schema_changes"], changes);
    let updated = call("xlsx_list_query_reports", report.clone());
    assert_eq!(updated["definition"]["input_schema"]["facts"], json!({"code":"INTEGER","extra":"TEXT"}));

    std::fs::write(&next, "{\"code\":1,\"extra\":\"north\"}\n").unwrap();
    let unchanged = call("sqlite_refresh_query_reports", preview_args);
    assert!(unchanged["reports"][0]["schema_changes"].is_null());
    assert_eq!(unchanged["reports"][0]["metrics"][0]["previous"], 0.0);
    call("app_undo_document", json!({"document_id":id}));
    assert_eq!(
        call("xlsx_list_query_reports", report)["definition"],
        original["definition"],
    );
    let mut legacy = ibis_doc_model::Document::new(
        ibis_formats::read(&std::fs::read(&workbook).unwrap()).unwrap(),
    );
    let mut definition = original["definition"].clone();
    definition.as_object_mut().unwrap().remove("input_schema");
    let name = table.as_str().unwrap().to_owned();
    legacy.execute(vec![ibis_doc_model::Command::Sheet(
        ibis_doc_model::SheetCommand::SetQueryReport {
            table: name.clone(),
            definition: definition.clone(),
        },
    )]).unwrap();
    let (review, commands) = ibis_doc_service::prepare_selected_report_batch(
        &[next, labels],
        json!({"bindings":{"facts.jsonl":"1","labels.csv":"2"}}),
        vec![(name, definition)],
    ).unwrap().review(legacy.presentation()).unwrap();
    assert!(review["reports"][0]["schema_changes"].is_null());
    assert!(ibis_doc_service::query_report_definition(legacy.presentation(), &json!({"table":table}))
        .unwrap()["input_schema"].is_null());
    legacy.execute(commands).unwrap();
    assert_eq!(
        ibis_doc_service::query_report_definition(legacy.presentation(), &json!({"table":table}))
            .unwrap()["input_schema"]["facts"]["code"],
        "INTEGER",
    );
    drop(service);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn query_report_check_blocks_unmatched_keys_in_service_and_assistant() {
    use ibis_doc_service::{DraftToolContext, batch_query_report_definitions, prepare_selected_report_batch};

    let dir = std::env::temp_dir().join(format!("ibis-report-unmatched-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let facts = dir.join("facts.jsonl");
    let labels = dir.join("labels.csv");
    let workbook = dir.join("source.xlsx");
    let saved = dir.join("report.xlsx");
    std::fs::write(&facts, "{\"code\":\"001\"}\n").unwrap();
    std::fs::write(&labels, "code,label\n001,North\n").unwrap();
    std::fs::write(&workbook, ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
    let mut service = Service::new(&[dir.clone()], false, Limits::default());
    let call = |service: &mut Service, tool, args| service.call(tool, &args).unwrap();
    let id = call(&mut service, "app_open_document", json!({"path":workbook}))["document_id"].clone();
    let sheet = call(&mut service, "xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].clone();
    let table = call(&mut service, "sqlite_query_to_table", json!({
        "document_id":id, "destination_sheet":sheet, "anchor":"A1",
        "sql":"SELECT count(*) AS matches FROM facts JOIN labels USING (code)",
        "datasets":[{"source":facts,"alias":"facts"},{"source":labels,"alias":"labels"}],
    }))["table"].clone();
    let diagnostics_sql = "SELECT f.code AS unmatched_key, typeof(f.code) AS key_type, count(*) AS occurrences
FROM facts AS f
WHERE NOT EXISTS (SELECT 1 FROM labels AS l WHERE l.code = f.code)
GROUP BY f.code, typeof(f.code)
ORDER BY occurrences DESC, key_type, unmatched_key
LIMIT 20";
    call(&mut service, "xlsx_set_query_report_checks", json!({
        "document_id":id, "table":table, "checks":[{
            "name":"マスタ未登録",
            "sql":"SELECT count(*) FROM facts f WHERE NOT EXISTS (SELECT 1 FROM labels l WHERE l.code = f.code)",
            "max":0, "diagnostic":{"sql":diagnostics_sql},
        }],
    }));
    call(&mut service, "app_save_document", json!({"document_id":id,"path":saved}));

    let mut draft = ibis_doc_model::Document::new(
        ibis_formats::read(&std::fs::read(&saved).unwrap()).unwrap(),
    );
    let unchanged = draft.presentation().clone();
    let selected = [facts.clone(), labels.clone()];
    let path_bindings = json!({"facts.jsonl":facts,"labels.csv":labels});
    let assistant_batch = |draft: &ibis_doc_model::Document, preview| {
        let request = json!({"bindings":{"facts.jsonl":"1","labels.csv":"2"},"preview":preview});
        let definitions = batch_query_report_definitions(draft.presentation(), &request).unwrap();
        prepare_selected_report_batch(&selected, request, definitions).unwrap()
    };

    for (facts_json, labels_csv, schema_changed, expected_key) in [
        ("{\"code\":1}\n", "code,label\n001,North\n", true, json!([1,"integer",1])),
        ("{\"code\":\"001\"}\n", "code,label\n002,South\n", false, json!(["001","text",1])),
    ] {
        std::fs::write(&facts, facts_json).unwrap();
        std::fs::write(&labels, labels_csv).unwrap();
        let preview = call(&mut service, "sqlite_refresh_query_reports", json!({
            "document_id":id,"bindings":path_bindings,"preview":true,
        }));
        assert_eq!(preview["blocked"], true);
        assert!(preview["snapshot_id"].is_null());
        assert_eq!(preview["reports"][0]["schema_changes"].is_array(), schema_changed);
        assert_eq!(preview["reports"][0]["checks"], json!([
            {"name":"マスタ未登録","count":1,"max":0,"passed":false,"diagnostic":{
                "columns":["unmatched_key","key_type","occurrences"],
                "rows":[expected_key],"truncated":false,
            }},
        ]));
        let mut trial_definition = ibis_doc_service::query_report_definition(
            draft.presentation(), &json!({"table":table}),
        ).unwrap();
        // A check trial does not execute the report SELECT or plan a workbook edit.
        trial_definition["sql"] = json!("SELECT * FROM missing_report_table");
        assert_eq!(
            json!(ibis_doc_service::trial_selected_report_checks(&selected, &trial_definition).unwrap()),
            preview["reports"][0]["checks"],
        );
        let diagnostics = call(&mut service, "sqlite_query", json!({
            "sql":diagnostics_sql,"limit":20,
            "datasets":[{"source":facts,"alias":"facts"},{"source":labels,"alias":"labels"}],
        }));
        assert_eq!(diagnostics["columns"], json!(["unmatched_key","key_type","occurrences"]));
        assert_eq!(diagnostics["rows"], json!([expected_key]));
        let assistant_diagnostics = ibis_doc_service::run_selected_datasets_tool(
            &selected, "sqlite_query", json!({
                "sql":diagnostics_sql,"limit":20,
                "datasets":[{"source":"1","alias":"facts"},{"source":"2","alias":"labels"}],
            }),
        ).unwrap();
        assert_eq!(assistant_diagnostics, diagnostics);
        let (assistant_preview, commands) = assistant_batch(&draft, true)
            .stage(&mut draft, DraftToolContext::default()).unwrap();
        assert_eq!(assistant_preview["reports"], preview["reports"]);
        assert!(commands.is_empty());
        assert_eq!(service.call("sqlite_refresh_query_reports", &json!({
            "document_id":id,"bindings":path_bindings,
        })).unwrap_err().code, "query_check_failed");
        assert_eq!(assistant_batch(&draft, false)
            .stage(&mut draft, DraftToolContext::default()).err().unwrap().code, "query_check_failed");
        assert_eq!(draft.presentation(), &unchanged);
        assert_eq!(call(&mut service, "xlsx_get_cells", json!({
            "document_id":id,"sheet":sheet,"range":"A2",
        }))["rows"][0][0].as_f64(), Some(1.0));
    }

    // The repaired inputs join successfully and contain two matches.
    std::fs::write(&facts, "{\"code\":\"001\"}\n{\"code\":\"002\"}\n").unwrap();
    std::fs::write(&labels, "code,label\n001,North\n002,South\n").unwrap();
    let preview = call(&mut service, "sqlite_refresh_query_reports", json!({
        "document_id":id,"bindings":path_bindings,"preview":true,
    }));
    assert_eq!(preview["blocked"], false);
    assert_eq!(preview["reports"][0]["checks"][0]["count"], 0);
    assert!(preview["reports"][0]["checks"][0]["diagnostic"].is_null());
    let (assistant, commands) = assistant_batch(&draft, false)
        .stage(&mut draft, DraftToolContext::default()).unwrap();
    assert!(!commands.is_empty());
    let applied = call(&mut service, "sqlite_refresh_query_reports", json!({
        "document_id":id,"snapshot_id":preview["snapshot_id"],
    }));
    assert_eq!(assistant["reports"], applied["reports"]);
    assert_eq!(call(&mut service, "xlsx_get_cells", json!({
        "document_id":id,"sheet":sheet,"range":"A2",
    }))["rows"][0][0].as_f64(), Some(2.0));
    drop(service);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn selected_large_sources_export_counts_beyond_the_sheet_limit() {
    use ibis_doc_service::{DraftToolContext, WorkbookToolContext, prepare_selected_datasets_export};

    let dir = tempfile::tempdir().unwrap();
    for (name, header, row) in [
        ("over-sheet.csv", "code\n", "001\n"),
        ("over-sheet.jsonl", "", "{\"code\":\"001\"}\n"),
        ("over-sheet.syslog", "", "<34>Oct 11 auth: accepted\n"),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, format!("{header}{}", row.repeat(1_048_577))).unwrap();
        let opened = ibis_formats::open(ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
        let mut draft = ibis_doc_model::Document::new(opened.presentation);
        let sheet = &draft.presentation().workbook.as_ref().unwrap().sheets[0];
        let prepared = prepare_selected_datasets_export(&[path], json!({
            "destination_sheet":sheet.name, "anchor":"A1",
            "sql":"SELECT count(*) AS records FROM input",
            "datasets":[{"source":"1","alias":"input"}],
        })).unwrap_or_else(|error| panic!("{name}: {}", error.message));
        let (_, commands) = prepared.stage(&mut draft, DraftToolContext {
            source: Some(std::sync::Arc::new(opened.source)),
            workbook: WorkbookToolContext::default(),
        }).unwrap();
        assert!(!commands.is_empty(), "{name}");
        let sheet = draft.presentation().slides.iter().flat_map(|slide| &slide.shapes).find_map(|shape| {
            if let ibis_doc_model::ShapeKind::Sheet(sheet) = &shape.kind { Some(sheet) } else { None }
        }).unwrap();
        assert!(sheet.cells.iter().any(|cell| cell.value == ibis_doc_model::CellValue::Number(1_048_577.0)), "{name}");
    }
}

#[test]
fn selected_json_lines_recover_into_a_refreshable_preview() {
    use ibis_doc_service::{
        DraftToolContext, WorkbookToolContext, prepare_selected_datasets_export,
        run_selected_datasets_tool,
    };

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.ndjson");
    let mut bytes = b"{\"event\":\"start\",\"id\":9007199254740993}\n{broken\n".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', 9 * 1024 * 1024));
    bytes.extend_from_slice(b"\n\xff\n{\"event\":\"done\",\"late_field\":true}\n");
    std::fs::write(&path, &bytes).unwrap();

    let args = json!({
        "sql":"SELECT * FROM input ORDER BY _jsonl_line LIMIT 25",
        "datasets":[{"source":"1","alias":"input","recover":true}],
    });
    let files = [path];
    let result = run_selected_datasets_tool(&files, "sqlite_query", args.clone()).unwrap();
    assert_eq!(result["columns"], json!(["_jsonl_line", "_jsonl_error", "event", "id", "late_field"]));
    let rows = result["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0], json!([1,null,"start","9007199254740993",null]));
    assert_eq!(rows[1][0], 2);
    assert!(rows[1][1].as_str().unwrap().starts_with("Expected one JSON object"));
    assert_eq!(rows[2], json!([3,"JSON Lines record exceeds 1 MiB",null,null,null]));
    assert!(rows[3][1].as_str().unwrap().starts_with("Expected one JSON object"));
    assert_eq!(rows[4], json!([5,null,"done",null,1]));

    let mut strict = args.clone();
    strict["datasets"][0]["recover"] = json!(false);
    assert!(run_selected_datasets_tool(&files, "sqlite_query", strict).is_err());

    let opened = ibis_formats::open(ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
    let mut draft = ibis_doc_model::Document::new(opened.presentation);
    let name = draft.presentation().workbook.as_ref().unwrap().sheets[0].name.clone();
    let mut export = args;
    export["anchor"] = json!("A1");
    export["destination_sheet"] = json!(name);
    let prepared = prepare_selected_datasets_export(&files, export).unwrap();
    let (_, commands) = prepared.stage(&mut draft, DraftToolContext {
        source: Some(std::sync::Arc::new(opened.source)),
        workbook: WorkbookToolContext::default(),
    }).unwrap();
    assert!(!commands.is_empty());
    let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&draft.presentation().slides[0]).unwrap();
    assert_eq!(sheet.cell(5, 2).unwrap().text, "done");
    let definition = ibis_doc_service::query_report_definition(
        draft.presentation(),
        &json!({"table":sheet.tables[0].name}),
    ).unwrap();
    assert_eq!(definition["sources"][0]["recover"], true);
    assert_eq!(std::fs::read(&files[0]).unwrap(), bytes);
}

#[test]
fn json_lines_catalog_paths_build_a_reopenable_table() {
    use ibis_doc_service::{DraftToolContext, WorkbookToolContext, prepare_selected_datasets_export};
    use ibis_doc_model::{Command, Document, SheetCommand, sheet_calc::sheet_of_slide};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    std::fs::write(&path, concat!(
        "{\"actor\":{\"id\":9007199254740993},\"a/b\":{\"~key\":\"001\"},\"items\":[1]}\n",
        "{broken\n",
        "{\"actor\":{\"id\":null},\"late\":true}\n",
    )).unwrap();

    let catalog = ibis_doc_model::query::json_lines_fields(&path).unwrap();
    assert_eq!((catalog.valid_records, catalog.invalid_records), (2, 1));
    let field = |name| catalog.fields.iter().find(|field| field.path == name).unwrap();
    assert_eq!(field("/actor/id").occurrences, 2);
    assert_eq!(field("/actor/id").types, ["number", "null"]);
    assert_eq!(field("/actor/id").example.as_deref(), Some("9007199254740993"));
    assert_eq!(field("/a~1b/~0key").example.as_deref(), Some("\"001\""));
    assert_eq!(field("/items").types, ["array"]);
    assert!(!catalog.fields.iter().any(|field| field.path.starts_with("/items/")));
    assert_eq!(field("/late").occurrences, 1);

    let opened = ibis_formats::open(ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap()).unwrap();
    let mut draft = Document::new(opened.presentation);
    let slide = draft.ids().new_slide_id();
    let shape = draft.ids().new_shape_id();
    draft.execute(vec![Command::Sheet(SheetCommand::AddSheet {
        index: 1, name: "JSONL 1".into(), sheet: slide, shape,
    })]).unwrap();
    let paths = json!(["/late", "/actor/id", "/a~1b/~0key", "/items"]);
    let prepared = prepare_selected_datasets_export(&[path], json!({
        "destination_sheet":"JSONL 1", "anchor":"A1",
        "sql":"SELECT * FROM input ORDER BY _jsonl_line LIMIT 1000",
        "datasets":[{"source":"1","alias":"input","paths":paths}],
    })).unwrap();
    let source = std::sync::Arc::new(opened.source);
    prepared.stage(&mut draft, DraftToolContext {
        source: Some(source.clone()), workbook: WorkbookToolContext::default(),
    }).unwrap();

    let bytes = source.save(draft.presentation(), ibis_formats::SaveOptions::default()).unwrap();
    let reopened = ibis_formats::open(bytes).unwrap();
    assert_eq!(reopened.presentation.workbook.as_ref().unwrap().sheets[1].name, "JSONL 1");
    let sheet = sheet_of_slide(&reopened.presentation.slides[1]).unwrap();
    for (column, path) in (2..).zip(["/late", "/actor/id", "/a~1b/~0key", "/items"]) {
        assert_eq!(sheet.cell(0, column).unwrap().text, path);
    }
    assert_eq!(sheet.cell(1, 3).unwrap().text, "9007199254740993");
    assert_eq!(sheet.cell(1, 4).unwrap().text, "001");
    assert_eq!(sheet.cell(1, 5).unwrap().text, "[1]");
    assert!(sheet.cell(2, 1).unwrap().text.starts_with("Expected one JSON object"));
    assert!(sheet.cell(3, 2).is_some());
    let definition = ibis_doc_service::query_report_definition(
        &reopened.presentation, &json!({"table":sheet.tables[0].name}),
    ).unwrap();
    assert_eq!(definition["sources"][0]["paths"], paths);
}
