// SPDX-License-Identifier: AGPL-3.0-only
//! Synthetic program used only by the isolated updater process test.
fn main() {
    let mode = option_env!("SORANE_FIXTURE_MODE").unwrap_or("invalid");
    if let Some(path) = option_env!("SORANE_FIXTURE_MARKER") {
        std::fs::write(path, mode).unwrap();
    }
    match mode {
        "parent" => std::thread::sleep(std::time::Duration::from_millis(800)),
        "new-running" => std::thread::sleep(std::time::Duration::from_secs(7)),
        "new-failing" => std::process::exit(1),
        "old" => (),
        _ => std::process::exit(2),
    }
}
