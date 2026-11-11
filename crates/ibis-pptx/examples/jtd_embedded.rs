// SPDX-License-Identifier: AGPL-3.0-only
//! Inspect or explicitly recover unplaced embedded metafiles.
use ibis_pptx::jtd::{self, EmbeddedMetafileInspection};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: jtd_embedded INPUT [NEW_OUTPUT_DIRECTORY]")?;
    let output = args.next();
    if args.next().is_some() {
        return Err("usage: jtd_embedded INPUT [NEW_OUTPUT_DIRECTORY]".into());
    }
    let data = fs::read(path)?;
    let result = jtd::inspect_embedded_metafiles(&data)?;
    let bytes: usize = result.metafiles.iter().map(|entry| entry.wmf.len()).sum();
    let rendered = if let Some(output) = output {
        Some(recover(&result, Path::new(&output))?)
    } else {
        None
    };
    println!(
        "{{\"metafiles\":{},\"bytes\":{},\"rejected\":{},\"svg_written\":{},\"placement_proven\":false}}",
        result.metafiles.len(),
        bytes,
        result.rejected,
        rendered.map_or("null".to_string(), |n| n.to_string())
    );
    Ok(())
}

/// Only create a fresh directory; never replace or add files in an existing one.
fn recover(
    result: &EmbeddedMetafileInspection,
    output: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    if result.metafiles.is_empty() {
        return Err("no supported embedded metafiles to recover".into());
    }
    fs::create_dir(output)?;
    write_new(&output.join("README.txt"), b"Recovered embedded images, not document pages.\nPage, position, size, order and visual fidelity are not established.\nSVG rendering is approximate; keep the original WMF and source document.\nThe directory may be incomplete if recovery was interrupted.\n")?;
    let mut rendered = 0;
    for (ordinal, entry) in result.metafiles.iter().enumerate() {
        // The ordinal makes duplicate candidate indices unambiguous. Never
        // use an embedded filename or other untrusted text as a path.
        let stem = format!(
            "image-{:03}-embedding-{}",
            ordinal + 1,
            entry.embedding_index
        );
        write_new(&output.join(format!("{stem}.wmf")), &entry.wmf)?;
        if let Some(svg) = ibis_emf::wmf_to_svg(&entry.wmf) {
            write_new(&output.join(format!("{stem}.svg")), svg.as_bytes())?;
            rendered += 1;
        }
    }
    Ok(rendered)
}

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibis_pptx::jtd::EmbeddedMetafile;

    #[test]
    fn recovers_wmf_and_svg_without_overwriting_a_directory() {
        let root = std::env::temp_dir().join(format!(
            "ibis-jtd-recovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let wmf = include_bytes!("../../ibis-emf/tests/fixtures/shapes.wmf").to_vec();
        let result = EmbeddedMetafileInspection {
            metafiles: vec![
                EmbeddedMetafile {
                    embedding_index: 3,
                    wmf: wmf.clone(),
                },
                EmbeddedMetafile {
                    embedding_index: 3,
                    wmf: wmf.clone(),
                },
            ],
            rejected: 0,
        };
        assert_eq!(recover(&result, &root).unwrap(), 2);
        assert_eq!(
            fs::read(root.join("image-001-embedding-3.wmf")).unwrap(),
            wmf
        );
        assert!(
            fs::read_to_string(root.join("image-001-embedding-3.svg"))
                .unwrap()
                .contains("<svg")
        );
        assert!(recover(&result, &root).is_err());
        assert_eq!(
            fs::read(root.join("image-002-embedding-3.wmf")).unwrap(),
            wmf
        );
        assert_eq!(
            fs::read(root.join("image-001-embedding-3.wmf")).unwrap(),
            wmf
        );
        fs::remove_dir_all(&root).unwrap();
        assert!(recover(&EmbeddedMetafileInspection::default(), &root).is_err());
        assert!(!root.exists());
    }
}
