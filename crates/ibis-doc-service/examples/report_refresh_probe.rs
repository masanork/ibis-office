// SPDX-License-Identifier: AGPL-3.0-only

//! Measure the report refresh service with shared or separate previews.
//! Generate matching Arrow IPC, JSON Lines and Avro inputs with `generate <directory> <rows>`.
//! Measure with `shared|separate <seed> <input> [reports]`. Use different directories for
//! the seed and each input size. Generate fixtures in a separate process from the measurement.
//! Build with `--features report-profiling` for the JSON Lines schema pass, remaining
//! staging work, and SQL execution timings.

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn main() {}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[path = "../tests/support/streaming_fixtures.rs"]
mod streaming_fixtures;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod desktop {
    use ibis_doc_service::{Limits, Service};
    use serde_json::{Value, json};
    use std::{path::PathBuf, time::Instant};

    pub fn run() {
        let mut args = std::env::args_os().skip(1);
        let mode = args.next().expect("Expected generate, shared or separate");
        if mode == "generate" {
            let dir = PathBuf::from(args.next().expect("Expected a fixture directory"));
            let rows = args
                .next()
                .expect("Expected a row count")
                .into_string()
                .unwrap()
                .parse()
                .unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            super::streaming_fixtures::write_sources(&dir, rows);
            return;
        }
        assert!(mode == "shared" || mode == "separate");
        let seed = PathBuf::from(args.next().expect("Expected seed dataset"))
            .canonicalize()
            .unwrap();
        let input = PathBuf::from(args.next().expect("Expected measurement dataset"))
            .canonicalize()
            .unwrap();
        let report_count = args.next().map_or(2, |value| {
            value
                .into_string()
                .unwrap()
                .parse::<usize>()
                .expect("Expected a report count")
        });
        assert!((2..=12).contains(&report_count), "Expected 2 to 12 reports");
        let temp = tempfile::tempdir().unwrap();
        let workbook = temp.path().join("probe.xlsx");
        std::fs::write(
            &workbook,
            ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap(),
        )
        .unwrap();
        let mut service = Service::new(
            &[
                temp.path().into(),
                seed.parent().unwrap().into(),
                input.parent().unwrap().into(),
            ],
            false,
            Limits::default(),
        );
        let mut call = |tool, args: Value| {
            service
                .call(tool, &args)
                .unwrap_or_else(|error| panic!("{tool}: {}", error.message))
        };
        let id = call("app_open_document", json!({"path":workbook}))["document_id"].clone();
        let sheet =
            call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].clone();
        let mut tables = Vec::new();
        let columns = (b'A'..=b'Y')
            .step_by(2)
            .take(report_count)
            .map(char::from)
            .collect::<Vec<_>>();
        for (index, column) in columns.iter().enumerate() {
            let anchor = format!("{column}1");
            let sql = if index == 0 {
                "SELECT count(*) AS n FROM facts".to_owned()
            } else {
                format!("SELECT count(*) AS n FROM facts WHERE code = '001' AND amount < {index}")
            };
            let result = call(
                "sqlite_query_to_table",
                json!({
                    "document_id":id,"destination_sheet":sheet,"anchor":anchor,"sql":sql,
                    "datasets":[{"source":seed,"alias":"facts","columns":["code","amount"]}],
                }),
            );
            tables.push(
                result["table"]
                    .as_str()
                    .expect("Expected a table name")
                    .to_owned(),
            );
        }
        let groups = if mode == "shared" {
            vec![tables]
        } else {
            tables.into_iter().map(|table| vec![table]).collect()
        };
        let mut timings = Vec::new();
        let mut results = Vec::new();
        for _ in 0..2 {
            #[cfg(feature = "report-profiling")]
            ibis_doc_model::query::report_profile::take();
            let mut preview_ms = 0.0;
            let mut apply_ms = 0.0;
            for tables in &groups {
                let metrics = tables
                    .iter()
                    .map(|table| json!({"table":table,"column":"n"}))
                    .collect::<Vec<_>>();
                let start = Instant::now();
                let preview = call(
                    "sqlite_refresh_query_reports",
                    json!({
                        "document_id":id,"preview":true,"tables":tables,"metrics":metrics,
                        "bindings":{seed.file_name().unwrap().to_str().unwrap():input},
                    }),
                );
                preview_ms += start.elapsed().as_secs_f64() * 1000.0;
                assert!(preview["snapshot_id"].is_string());
                let start = Instant::now();
                let apply = call(
                    "sqlite_refresh_query_reports",
                    json!({
                        "document_id":id,"snapshot_id":preview["snapshot_id"],
                    }),
                );
                apply_ms += start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(preview["reports"], apply["reports"]);
            }
            let timing = json!({"preview_ms":preview_ms,"apply_ms":apply_ms});
            #[cfg(feature = "report-profiling")]
            let timing = {
                let mut timing = timing;
                let profile = ibis_doc_model::query::report_profile::take();
                timing["schema_ms"] = json!(profile.schema.as_secs_f64() * 1000.0);
                let stage = profile.stage.saturating_sub(profile.schema);
                timing["stage_ms"] = json!(stage.as_secs_f64() * 1000.0);
                timing["stage_write_ms"] = json!(profile.write.as_secs_f64() * 1000.0);
                timing["stage_read_ms"] =
                    json!(stage.saturating_sub(profile.write).as_secs_f64() * 1000.0);
                timing["sql_ms"] = json!(profile.sql.as_secs_f64() * 1000.0);
                timing
            };
            timings.push(timing);
            let cells = columns
                .iter()
                .map(|column| {
                    let range = format!("{column}2");
                    call(
                        "xlsx_get_cells",
                        json!({"document_id":id,"sheet":sheet,"range":range}),
                    )["rows"][0][0]
                        .clone()
                })
                .collect::<Vec<_>>();
            results.push(cells);
        }
        assert_eq!(results[0], results[1]);
        println!("{}", json!({"timings":timings,"results":results[0]}));
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn main() {
    desktop::run();
}
