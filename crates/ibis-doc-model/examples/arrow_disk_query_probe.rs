// SPDX-License-Identifier: AGPL-3.0-only

use ibis_doc_model::query::{SqliteQuerySession, read_arrow_ipc_dataset};
use serde_json::json;
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().ok_or("Expected memory or disk")?;
    let file = PathBuf::from(args.next().ok_or("Expected an Arrow IPC input file")?);
    let columns = ["code", "cohort", "amount", "tags"].map(String::from);
    let stage = || match mode.to_str() {
        Some("memory" | "memory-reuse") => {
            let table = read_arrow_ipc_dataset(&file, &columns, 100_000, 8 * 1024 * 1024)?;
            SqliteQuerySession::from_datasets(&[("facts".into(), table)])
        }
        Some("disk") => {
            SqliteQuerySession::from_arrow_disk(&[("facts".into(), file.clone(), columns.to_vec())])
        }
        _ => Err("Expected memory or disk".into()),
    };
    let queries = [
        "SELECT cohort, count(*) AS n, sum(amount) AS total FROM facts GROUP BY cohort ORDER BY cohort",
        "SELECT count(*) AS n, sum(amount) AS total FROM facts f WHERE code < '00024000' AND EXISTS (SELECT 1 FROM json_each(f.tags) j WHERE j.value = 'eligible')",
    ];
    let start = Instant::now();
    let mut session = Some(stage()?);
    let stage_ms = start.elapsed().as_secs_f64() * 1000.;
    let mut disk_bytes = session
        .as_ref()
        .unwrap()
        .staged_disk_bytes()
        .map_err(|error| error.to_string())?;
    let mut times = Vec::new();
    let mut outputs = Vec::new();
    for (index, sql) in queries.into_iter().chain(queries).enumerate() {
        let start = Instant::now();
        // The current memory route reloads sources for each independent report.
        if mode == "memory" && index != 0 {
            drop(session.take());
            session = Some(stage()?);
        }
        let session = session.as_ref().unwrap();
        let output = session.query(sql, &[], 1000, 1_048_576)?;
        if output.truncated {
            return Err("Unexpected truncated output".into());
        }
        times.push(start.elapsed().as_secs_f64() * 1000.);
        disk_bytes = disk_bytes.max(
            session
                .staged_disk_bytes()
                .map_err(|error| error.to_string())?,
        );
        outputs.push(output.rows);
    }
    println!(
        "{}",
        json!({"stage_ms":stage_ms,"queries_ms":times,"temp_disk_bytes":disk_bytes,"results":outputs})
    );
    Ok(())
}
