// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::io::Write;
use std::process::ExitCode;

use ibis_lawtext::{LawEditFormat, apply_law_source_edits, parse_law_xml, to_lawtext, to_markdown};
use serde_json::json;

const USAGE: &str = "Usage: ibis-lawtext-apply ORIGINAL.xml (EDITED | --prepare) [--format lawtext|markdown]\n    Write the patched XML to stdout, or exit 2 with diagnostics and no XML.";

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let original = args.next().ok_or(USAGE)?;
    if original == "--help" {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    let edited = std::path::PathBuf::from(args.next().ok_or(USAGE)?);
    let prepare = edited.as_os_str() == "--prepare";
    let format = match args.next() {
        None if edited.extension().is_some_and(|ext| ext == "md" || ext == "markdown") => LawEditFormat::Markdown,
        None => LawEditFormat::Lawtext,
        Some(flag) if flag == "--format" => match args.next().as_deref().and_then(|value| value.to_str()) {
            Some("markdown") => LawEditFormat::Markdown,
            Some("lawtext") => LawEditFormat::Lawtext,
            _ => return Err(USAGE.into()),
        },
        _ => return Err(USAGE.into()),
    };
    if args.next().is_some() {
        return Err(USAGE.into());
    }
    let original = std::fs::read_to_string(original)?;
    if prepare {
        let law = parse_law_xml(&original)?;
        let text = match format {
            LawEditFormat::Lawtext => to_lawtext(&law),
            LawEditFormat::Markdown => to_markdown(&law),
        };
        std::io::stdout().lock().write_all(text.as_bytes())?;
        return Ok(ExitCode::SUCCESS);
    }
    let result = apply_law_source_edits(&original, &std::fs::read_to_string(edited)?, format)?;
    if let Some(xml) = result.xml {
        std::io::stdout().lock().write_all(xml.as_bytes())?;
        return Ok(ExitCode::SUCCESS);
    }
    let diagnostics: Vec<_> = result.diagnostics.iter()
        .map(|diagnostic| json!({"path": diagnostic.path, "reason": diagnostic.reason}))
        .collect();
    eprintln!("{}", serde_json::to_string_pretty(&diagnostics)?);
    Ok(ExitCode::from(2))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
