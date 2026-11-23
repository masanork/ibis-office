// SPDX-License-Identifier: AGPL-3.0-only

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        fs::File,
        io::{Read, Write},
        path::Path,
    };

    let mut args = std::env::args_os().skip(1);
    let usage = "Usage: sbom_build_search <package-url> <output.xlsx> <sbom.json>...";
    let purl = args
        .next()
        .and_then(|arg| arg.into_string().ok())
        .ok_or(usage)?;
    let output = args.next().ok_or(usage)?;
    let output = Path::new(&output);
    if !output
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
    {
        return Err(usage.into());
    }
    let mut remaining = 32 * 1024 * 1024;
    let mut sources = Vec::new();
    for path in args {
        let path = Path::new(&path);
        let mut bytes = Vec::new();
        File::open(path)?
            .take(remaining + 1)
            .read_to_end(&mut bytes)?;
        remaining = remaining
            .checked_sub(bytes.len() as u64)
            .ok_or("The combined SBOM inputs exceed 32 MiB")?;
        sources.push((path.to_string_lossy(), bytes));
    }
    let inputs = sources
        .iter()
        .map(|(path, bytes)| (path.as_ref(), bytes.as_slice()))
        .collect::<Vec<_>>();
    let bytes = ibis_formats::sbom_review::search_sbom_builds_xlsx(&inputs, &purl)?;
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&bytes)?;
    file.persist_noclobber(output)?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn main() {
    eprintln!("This example requires macOS, Windows, or Linux.");
    std::process::exit(1);
}
