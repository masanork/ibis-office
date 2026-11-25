// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Convert a PlantUML basic sequence diagram to Mermaid `sequenceDiagram`, verify it
//! through the existing Mermaid SVG renderer, and print any unsupported lines with
//! 1-based line numbers.
//!
//! Usage:
//!   cargo run --locked -p ibis-formats --example plantuml_sequence_convert -- <input.puml> [--svg <out.svg>]

use std::{env, fs, process};

use ibis_formats::mermaid::{MermaidOutput, render_plantuml_sequence};

fn main() {
    let mut args = env::args().skip(1);
    let Some(input_path) = args.next() else {
        eprintln!("Usage: plantuml_sequence_convert <input.puml> [--svg <out.svg>]");
        process::exit(2);
    };
    let mut svg_out: Option<String> = None;
    while let Some(arg) = args.next() {
        if arg == "--svg" {
            svg_out = args.next();
            if svg_out.is_none() {
                eprintln!("--svg requires an output path");
                process::exit(2);
            }
        } else {
            eprintln!("Unknown argument: {arg}");
            process::exit(2);
        }
    }

    let source = match fs::read_to_string(&input_path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Failed to read {input_path}: {err}");
            process::exit(1);
        }
    };

    let (conversion, rendered) = render_plantuml_sequence(&source);
    if let Some(mermaid) = &conversion.mermaid {
        print!("{mermaid}");
    } else {
        eprintln!("Not convertible to a Mermaid sequenceDiagram.");
    }

    if let Some(svg_path) = svg_out {
        match rendered {
            Some(MermaidOutput::Svg {
                bytes,
                width_px,
                height_px,
            }) => {
                if let Err(err) = fs::write(&svg_path, &bytes) {
                    eprintln!("Failed to write {svg_path}: {err}");
                    process::exit(1);
                }
                eprintln!("Rendered SVG ({width_px}x{height_px}px) to {svg_path}");
            }
            _ => {
                eprintln!("Existing Mermaid renderer produced no SVG.");
                process::exit(1);
            }
        }
    }

    if !conversion.unsupported.is_empty() {
        eprintln!(
            "Unsupported constructs ({}):",
            conversion.unsupported.len()
        );
        for item in &conversion.unsupported {
            eprintln!(
                "  line {} [{}]: {} -- {}",
                item.line, item.kind, item.source, item.reason
            );
        }
    }
}
