// SPDX-License-Identifier: AGPL-3.0-only
use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-env-changed=IBIS_VARIANT_BITMAP");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("variant-clusters.zst");
    match env::var_os("IBIS_VARIANT_BITMAP") {
        Some(path) => {
            let path = PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            fs::copy(&path, out).unwrap_or_else(|e| panic!("cannot copy {}: {e}", path.display()));
        }
        None => fs::write(out, []).unwrap(),
    }
}
