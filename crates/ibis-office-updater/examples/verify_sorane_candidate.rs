// SPDX-License-Identifier: AGPL-3.0-only
//! Check all files of an offline release candidate using an out-of-band key.
use ibis_office_updater::release_candidate::{decode_hex, verify_directory};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() {
    if let Err(err) = run() {
        eprintln!("verification failed: {err}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return Err("usage: verify_sorane_candidate DIRECTORY TRUSTED_PUBLIC_KEY_HEX REQUIRED_TARGET [REQUIRED_TARGET ...]".into());
    }
    let key = decode_hex::<32>(&args[1])?;
    let required: Vec<&str> = args[2..].iter().map(String::as_str).collect();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "invalid system clock")?
        .as_secs();
    let manifest = verify_directory(Path::new(&args[0]), &key, now, &required)?;
    println!(
        "Verified Sorane Docs {} ({} artifacts)",
        manifest.version,
        manifest.artifacts.len()
    );
    println!("This checks signatures and bytes, not provenance, OS signing or installation.");
    Ok(())
}
