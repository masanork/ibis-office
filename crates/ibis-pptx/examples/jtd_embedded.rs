// SPDX-License-Identifier: AGPL-3.0-only
//! Report bounded, unplaced metafile candidates without printing document data.
use ibis_pptx::jtd;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: jtd_embedded INPUT.jtd")?;
    let data = std::fs::read(path)?;
    let result = jtd::inspect_embedded_metafiles(&data)?;
    let bytes: usize = result.metafiles.iter().map(|entry| entry.wmf.len()).sum();
    println!(
        "{{\"metafiles\":{},\"bytes\":{},\"rejected\":{},\"placement_proven\":false}}",
        result.metafiles.len(),
        bytes,
        result.rejected
    );
    Ok(())
}
