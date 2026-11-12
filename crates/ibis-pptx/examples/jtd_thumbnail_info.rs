// SPDX-License-Identifier: AGPL-3.0-only
//! Inspect candidate metadata; no image or page is reconstructed.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: jtd_thumbnail_info INPUT")?;
    if args.next().is_some() {
        return Err("usage: jtd_thumbnail_info INPUT".into());
    }
    let data = std::fs::read(path)?;
    match ibis_pptx::jtd::inspect_thumbnail(&data)? {
        None => println!("thumbnail_present=false"),
        Some(t) => println!(
            "thumbnail_present=true\twidth={}\theight={}\tbit_depth={}\tdeclared_dib_bytes={}\tstored_dib_bytes={}\tstored_payload_bytes={}\tpixels_decoded=false\tpage_proven=false",
            t.width,
            t.height,
            t.bit_depth,
            t.declared_dib_bytes,
            t.stored_dib_bytes,
            t.stored_payload_bytes
        ),
    }
    Ok(())
}
