// SPDX-License-Identifier: AGPL-3.0-only
//! Inspect an embedded snapshot; optional BMP output is not a reconstructed page.
use std::io::Write;
use std::path::Path;

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: jtd_thumbnail_info INPUT [NEW_OUTPUT.bmp]")?;
    let output = args.next();
    if args.next().is_some() {
        return Err("usage: jtd_thumbnail_info INPUT [NEW_OUTPUT.bmp]".into());
    }
    let data = std::fs::read(path)?;
    match ibis_pptx::jtd::inspect_thumbnail(&data)? {
        None => {
            if output.is_some() {
                return Err("no observed thumbnail candidate".into());
            }
            println!("thumbnail_present=false");
        }
        Some(t) => {
            if let Some(output) = &output {
                let bmp = ibis_pptx::jtd::decode_thumbnail_bmp(&data)?
                    .ok_or("no observed thumbnail candidate")?;
                write_new(Path::new(output), &bmp)?;
            }
            println!(
                "thumbnail_present=true\twidth={}\theight={}\tbit_depth={}\tdeclared_dib_bytes={}\tstored_dib_bytes={}\tstored_payload_bytes={}\tpixels_decoded={}\tpage_proven=false",
                t.width,
                t.height,
                t.bit_depth,
                t.declared_dib_bytes,
                t.stored_dib_bytes,
                t.stored_payload_bytes,
                output.is_some()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_to_replace_existing_output() {
        let path = std::env::temp_dir().join(format!(
            "ibis-thumbnail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write_new(&path, b"first").unwrap();
        assert!(write_new(&path, b"second").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        std::fs::remove_file(path).unwrap();
    }
}
