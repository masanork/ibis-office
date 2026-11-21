// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() {
    // Use production format drivers with the shell's headless scenario driver.
    // The format-specific scenario collections are not dependencies of this runner.
    ibis_office_app::register_hooks();
    const CASE: &str = "shell::image_provenance_open_save";
    assert!(
        ibis_office_shell::scenario_examples::all()
            .iter()
            .any(|(name, _)| *name == CASE)
    );
    let args = vec![CASE.to_string()];
    std::process::exit(ibis_office_shell::scenario::run(
        &args,
        ibis_office_shell::scenario_examples::all,
        Vec::new,
    ));
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {
    eprintln!("The native editor regression runner requires macOS or Linux.");
    std::process::exit(1);
}
