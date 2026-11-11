// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! On Windows, gives ibisOfficeUpdater.exe its version (the 詳細 tab of the
//! file's properties), as ibisOffice.exe's (ibis-office-app/windows/resource.rs).

fn main() {
    #[cfg(windows)]
    {
        let target = |name: &str| std::env::var(name).unwrap_or_default();
        if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
            let mut res = Vec::new();
            // An empty resource first marks the file as 32-bit.
            push_resource(&mut res, 0, 0, &[]);
            push_version_resource(&mut res, &windows_version(), "Sorane Docs Updater", "ibisOfficeUpdater.exe");
            let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("version.res");
            std::fs::write(&out, res).unwrap();
            println!("cargo:rustc-link-arg-bin=ibisOfficeUpdater={}", out.display());
        }
    }
}

#[cfg(windows)]
include!("../ibis-office-app/windows/resource.rs");
