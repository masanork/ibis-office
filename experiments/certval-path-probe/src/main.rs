// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    ibis_certval_path_probe::cli_main()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
