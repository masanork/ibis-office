// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use std::{
    fs::File,
    io::{BufWriter, Write},
};

#[path = "../tests/support/streaming_fixtures.rs"]
mod streaming_fixtures;
use streaming_fixtures::write_sources;

#[test]
fn streaming_formats_match_at_the_decoded_cell_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::new(&[dir.path().to_owned()], false, crate::Limits::default());
    for row_count in [50_000, 50_001] {
        for path in write_sources(dir.path(), row_count) {
            let spec = json!({"source":path,"alias":"facts","columns":["code","amount"]});
            let args = json!({"datasets":[spec]});
            let session = batch_session(&service, &args, false).unwrap();
            assert_eq!(
                session.staged_disk_bytes().unwrap() > 0,
                row_count > 50_000,
                "{path:?}"
            );

            // At the boundary compare both backends on identical source bytes.
            let disk = (row_count == 50_000).then(|| {
                disk_dataset_session(&service, args["datasets"].as_array().unwrap(), false).unwrap()
            });
            for (sql, expected) in [
                (
                    "SELECT count(*), sum(amount) FROM facts",
                    json!([[row_count, (0..row_count).map(|i| i % 7).sum::<usize>()]]),
                ),
                (
                    "SELECT code, count(*) FROM facts WHERE amount = 0 GROUP BY code",
                    json!([["001", row_count.div_ceil(7)]]),
                ),
            ] {
                let rows = session.query(sql, &[], 10, 1024).unwrap();
                assert!(!rows.truncated);
                assert_eq!(json!(rows.rows), expected, "{path:?}");
                if let Some(disk) = &disk {
                    let staged = disk.query(sql, &[], 10, 1024).unwrap();
                    assert!(!staged.truncated);
                    assert_eq!(rows.rows, staged.rows, "{path:?}");
                }
            }
        }
    }
}

thread_local! {
    pub(super) static SESSION_PREPARATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[test]
fn report_preview_shares_one_session_and_apply_opens_none() {
    let seed_dir = tempfile::tempdir().unwrap();
    let input_dir = tempfile::tempdir().unwrap();
    let seeds = write_sources(seed_dir.path(), 1);
    let inputs = write_sources(input_dir.path(), 50_001);
    let workbook = seed_dir.path().join("reports.xlsx");
    std::fs::write(
        &workbook,
        ibis_formats::blank_file(ibis_formats::Kind::Workbook).unwrap(),
    )
    .unwrap();
    for (seed, input) in seeds.iter().zip(inputs) {
        let mut service = Service::new(
            &[seed_dir.path().into(), input_dir.path().into()],
            false,
            crate::Limits::default(),
        );
        let mut call = |name, args| service.call(name, &args).unwrap();
        let id = call("app_open_document", json!({"path":workbook}))["document_id"].clone();
        let sheet =
            call("xlsx_list_sheets", json!({"document_id":id}))["sheets"][0]["name"].clone();
        for (anchor, sql) in [
            ("A1", "SELECT count(*) AS n FROM facts"),
            ("D1", "SELECT count(*) AS n FROM facts WHERE amount = 0"),
        ] {
            call(
                "sqlite_query_to_table",
                json!({
                    "document_id":id,"destination_sheet":sheet,"anchor":anchor,"sql":sql,
                    "datasets":[{"source":seed,"alias":"facts","columns":["code","amount"]}],
                }),
            );
        }
        for (source, expected) in [(seed, [1.0, 1.0]), (&input, [50_001.0, 7_143.0])] {
            SESSION_PREPARATIONS.with(|opens| opens.set(0));
            let preview = call(
                "sqlite_refresh_query_reports",
                json!({
                    "document_id":id,"preview":true,
                    "bindings":{seed.file_name().unwrap().to_str().unwrap():source},
                }),
            );
            assert_eq!(
                SESSION_PREPARATIONS.with(|opens| opens.get()),
                1,
                "{source:?}"
            );
            if source == &input {
                std::fs::remove_file(source).unwrap();
            }
            let applied = call(
                "sqlite_refresh_query_reports",
                json!({
                    "document_id":id,"snapshot_id":preview["snapshot_id"],
                }),
            );
            assert_eq!(
                SESSION_PREPARATIONS.with(|opens| opens.get()),
                1,
                "{source:?}"
            );
            assert_eq!(applied["reports"], preview["reports"]);
            for (range, expected) in ["A2", "D2"].into_iter().zip(expected) {
                let cell = call(
                    "xlsx_get_cells",
                    json!({"document_id":id,"sheet":sheet,"range":range}),
                );
                assert_eq!(cell["rows"][0][0].as_f64(), Some(expected));
            }
        }
    }
}

#[test]
fn file_size_boundary_uses_disk_without_exceeding_the_decoded_budget() {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::new(&[dir.path().to_owned()], false, crate::Limits::default());
    let path = dir.path().join("padded.jsonl");
    let args = json!({"datasets":[{"source":path,"alias":"facts","columns":["amount"]}]});
    let mut line = b"{\"amount\":5}".to_vec();
    line.resize(256 * 1024 - 1, b' ');
    line.push(b'\n');
    for extra in [0, 1] {
        let mut writer = BufWriter::new(File::create(&path).unwrap());
        for _ in 0..31 {
            writer.write_all(&line).unwrap();
        }
        writer.write_all(&line[..line.len() - 1]).unwrap();
        writer.write_all(&b" "[..extra]).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
        drop(writer);
        let session = batch_session(&service, &args, false).unwrap();
        assert_eq!(session.staged_disk_bytes().unwrap() > 0, extra == 1);
        let output = session
            .query("SELECT count(*), sum(amount) FROM facts", &[], 10, 1024)
            .unwrap();
        assert_eq!(json!(output.rows), json!([[32, 160]]));
    }
}

#[test]
fn csv_stages_only_above_the_budget_and_matches_legacy_decoding() {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::new(&[dir.path().into()], false, crate::Limits::default());
    let path = dir.path().join("facts.csv");
    let spec = json!({"source":path,"alias":"facts"});
    let args = json!({"datasets":[spec]});
    let text = "code,note\r\n001,\"あ\r\n\"\"x\"\"\"\r\n002,\r\n";
    let expected = json!([["001", "あ\r\n\"x\""], ["002", ""]]);
    let encodings = [
        text.as_bytes().to_vec(),
        [b"\xff\xfe".as_slice(), &text.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>()].concat(),
        text.split('あ').map(str::as_bytes).collect::<Vec<_>>().join(&[0x82, 0xa0][..]),
    ];
    for bytes in encodings {
        std::fs::write(&path, bytes).unwrap();
        let memory = batch_session(&service, &args, false).unwrap();
        let disk = disk_dataset_session(&service, args["datasets"].as_array().unwrap(), false).unwrap();
        assert_eq!(memory.staged_disk_bytes().unwrap(), 0);
        for session in [memory, disk] {
            let output = session.query("SELECT code,note FROM facts ORDER BY code", &[], 10, 1024).unwrap();
            assert_eq!(json!(output.rows), expected);
        }
    }

    for row_count in [50_000, 50_001] {
        std::fs::write(&path, format!("code,note\n{}", "001,a\n".repeat(row_count))).unwrap();
        let session = batch_session(&service, &args, false).unwrap();
        assert_eq!(session.staged_disk_bytes().unwrap() > 0, row_count > 50_000);
        let output = session.query("SELECT count(*), min(code) FROM facts", &[], 10, 1024).unwrap();
        assert_eq!(json!(output.rows), json!([[row_count, "001"]]));
    }
}
