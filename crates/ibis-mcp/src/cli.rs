// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The arguments of the server (design 4.2): `[--allow-dir <folder>]… [--read-only] [--max-documents N] [--max-file-size MB] [--assistant-socket <path>]`.

use std::path::PathBuf;

use ibis_doc_service::{Limits, Service};

const HELP: &str = "ibisOffice --mcp [--allow-dir <folder>]... [--read-only] [--max-documents N] [--max-file-size MB] [--assistant-socket <path>]\n\n\
Runs a Model Context Protocol server on standard input and output: docx, xlsx and pptx files under the allowed folders can be read, edited and saved by an AI.\n\
  --allow-dir <folder>   a folder whose files may be opened and saved (repeatable; with none, no file is reachable)\n\
  --assistant-socket <path>  connect to a running GUI Assistant (Linux/macOS)\n\
  --read-only            offer only the tools that read\n\
  --max-documents N      documents open at once (default 8)\n\
  --max-file-size MB     the largest file that is opened (default 200)\n";

/// What the arguments say, or why they are wrong.
pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<(Vec<PathBuf>, bool, Limits, Option<PathBuf>)>, String> {
    let mut dirs = Vec::new();
    let mut read_only = false;
    let mut assistant_socket = None;
    let mut limits = Limits::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let mut value = || iter.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--allow-dir" => dirs.push(PathBuf::from(value()?)),
            "--read-only" => read_only = true,
            "--assistant-socket" => {
                if !cfg!(any(target_os = "macos", target_os = "linux")) {
                    return Err("--assistant-socket is available on macOS and Linux".into());
                }
                let path = PathBuf::from(value()?);
                if !path.is_absolute() {
                    return Err("--assistant-socket needs an absolute path".into());
                }
                assistant_socket = Some(path);
            }
            "--max-documents" => limits.max_documents = value()?.parse().ok().filter(|n| *n >= 1).ok_or("--max-documents needs a number, 1 or more")?,
            "--max-file-size" => limits.max_file_size = value()?.parse::<u64>().ok().filter(|n| *n >= 1).ok_or("--max-file-size needs a number of megabytes, 1 or more")? * 1024 * 1024,
            "--help" | "-h" => return Ok(None),
            other => return Err(format!("{other} is not an argument (see --help)")),
        }
    }
    Ok(Some((dirs, read_only, limits, assistant_socket)))
}

/// The `ibisOffice` program that draws the exports: this program itself when it is `ibisOffice --mcp`, the `ibisOffice` next to it when it is `ibisOffice-mcp`.
fn exporter() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let stem = me.file_stem()?.to_string_lossy().to_ascii_lowercase();
    if stem == "ibisoffice" {
        return Some(me);
    }
    let sibling = me.with_file_name(format!("ibisOffice{}", std::env::consts::EXE_SUFFIX));
    sibling.is_file().then_some(sibling)
}

/// Run the server on the standard input and output; the exit code.
pub fn run_from_args(args: impl IntoIterator<Item = String>) -> i32 {
    match parse(args) {
        Err(message) => {
            eprintln!("ibisOffice --mcp: {message}");
            2
        }
        Ok(None) => {
            print!("{HELP}");
            0
        }
        Ok(Some((dirs, read_only, limits, assistant_socket))) => {
            for dir in &dirs {
                if !dir.is_dir() {
                    eprintln!("ibisOffice --mcp: {} is not a folder", dir.display());
                    return 2;
                }
            }
            let mut service = Service::new(&dirs, read_only, limits);
            service.set_exporter(exporter());
            service.set_version(crate::VERSION);
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let server = crate::Server::new(service);
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            let server = if let Some(socket) = assistant_socket {
                server.with_live_assistant(socket, read_only)
            } else { server };
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            let _ = assistant_socket;
            match crate::stdio::serve_server(server, stdin.lock(), stdout.lock()) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("ibisOffice --mcp: {e}");
                    1
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_arguments_are_read_or_refused() {
        let (dirs, read_only, limits, socket) = parse(args(&["--allow-dir", "/a", "--allow-dir", "/b", "--read-only", "--max-documents", "3", "--max-file-size", "10"])).unwrap().unwrap();
        assert_eq!((dirs.len(), read_only, limits.max_documents, limits.max_file_size), (2, true, 3, 10 * 1024 * 1024));
        assert!(socket.is_none());
        let (dirs, read_only, limits, socket) = parse(args(&[])).unwrap().unwrap();
        assert_eq!((dirs.len(), read_only, limits.max_file_size), (0, false, 200 * 1024 * 1024));
        assert!(socket.is_none());
        assert!(parse(args(&["--help"])).unwrap().is_none());
        for bad in [&["--nope"][..], &["--allow-dir"], &["--max-documents", "0"], &["--max-file-size", "x"]] {
            assert!(parse(args(bad)).is_err(), "{bad:?}");
        }
    }
}
