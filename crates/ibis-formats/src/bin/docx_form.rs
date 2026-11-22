// SPDX-License-Identifier: AGPL-3.0-only
use ibis_formats::form_template;
use std::{error::Error, fs, io::Write, path::Path};

fn read_value(path: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
    let yaml = matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    );
    Ok(form_template::parse_input(
        &fs::read_to_string(path)?,
        yaml,
    )?)
}

fn generate(
    command: &std::ffi::OsStr,
    template: &Path,
    definition: &Path,
    record: &Path,
) -> Result<(Vec<u8>, Vec<form_template::Truncation>), Box<dyn Error>> {
    let definition = read_value(definition)?;
    let record = read_value(record)?;
    if command == "check-html" || command == "fill-html" {
        let (document, diagnostics) = form_template::fill_html_with_diagnostics(
            &fs::read_to_string(template)?,
            &definition,
            &record,
        )?;
        Ok((document.into_bytes(), diagnostics))
    } else if command == "check-md" || command == "fill-md" {
        Ok(form_template::fill_markdown_with_diagnostics(
            &fs::read(template)?,
            &definition,
            &record,
        )?)
    } else {
        Ok(form_template::fill_with_diagnostics(
            &fs::read(template)?,
            &definition,
            &record,
        )?)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    #[cfg(not(target_family = "wasm"))]
    if let Some(result) = pdf_command(&args) {
        return result;
    }
    match args.as_slice() {
        [command, layout, definition, record, mapping] if command == "preview-html-claims" => {
            let preview = form_template::preview_html_claims(
                &fs::read_to_string(layout)?,
                &read_value(Path::new(definition))?,
                &read_value(Path::new(record))?,
                &read_value(Path::new(mapping))?,
            )?;
            println!("{}", serde_json::to_string_pretty(&preview)?);
        }
        [command, input, output] if command == "export-md" || command == "compile-md" => {
            let source = fs::read(input)?;
            let result = if command == "export-md" {
                ibis_formats::markdown::export_docx_to_markdown(&source)?.into_bytes()
            } else {
                ibis_formats::markdown::import_markdown_to_docx_bytes(&source)?
            };
            fs::OpenOptions::new().write(true).create_new(true).open(output)?.write_all(&result)?;
        }
        [command, template] if command == "inspect" || command == "inspect-md" => {
            let bytes = fs::read(template)?;
            let fields = if command == "inspect-md" {
                form_template::inspect_markdown(&bytes)?
            } else {
                form_template::inspect(&bytes)?
            };
            println!("{}", serde_json::to_string_pretty(&fields)?);
        }
        [command, template, definition, record] if command == "check" || command == "check-html" || command == "check-md" => {
            let (_, diagnostics) = generate(command, Path::new(template), Path::new(definition), Path::new(record))?;
            print_diagnostics(&diagnostics)?;
        }
        [command, template, definition, record, output] if command == "fill" || command == "fill-html" || command == "fill-md" => {
            let (document, diagnostics) = generate(command, Path::new(template), Path::new(definition), Path::new(record))?;
            for item in diagnostics {
                eprintln!("Truncated {} from {} to {} characters", item.field_id, item.actual_chars, item.max_chars);
            }
            fs::OpenOptions::new().write(true).create_new(true).open(output)?.write_all(&document)?;
        }
        _ => return Err("usage: docx_form export-md|compile-md INPUT OUTPUT | docx_form inspect|inspect-md TEMPLATE | docx_form check|check-html|check-md TEMPLATE FORM RECORD | docx_form fill|fill-html|fill-md TEMPLATE FORM RECORD OUTPUT | docx_form preview-html-claims LAYOUT FORM RECORD MAPPING | docx_form inspect-pdf|check-pdf|fill-pdf …".into()),
    }
    Ok(())
}

fn print_diagnostics(diagnostics: &[form_template::Truncation]) -> Result<(), Box<dyn Error>> {
    let warnings: Vec<_> = diagnostics
        .iter()
        .map(|item| {
            serde_json::json!({
                "fieldId": item.field_id,
                "actualChars": item.actual_chars,
                "maxChars": item.max_chars
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({"truncations": warnings}))?
    );
    Ok(())
}

#[cfg(not(target_family = "wasm"))]
fn pdf_command(args: &[std::ffi::OsString]) -> Option<Result<(), Box<dyn Error>>> {
    let command = args.first()?.to_str()?;
    if !matches!(command, "inspect-pdf" | "check-pdf" | "fill-pdf") {
        return None;
    }
    Some((|| {
        match (command, &args[1..]) {
            ("inspect-pdf", [template]) => {
                let fields = form_template::pdf::inspect(fs::read(template)?)?;
                println!("{}", serde_json::to_string_pretty(&fields)?);
            }
            ("check-pdf", [template, definition, record, font, validator]) |
            ("fill-pdf", [template, definition, record, font, validator, _]) => {
                let (document, diagnostics) = form_template::pdf::fill_validated(
                    Path::new(template),
                    &read_value(Path::new(definition))?,
                    &read_value(Path::new(record))?,
                    fs::read(font)?,
                    Path::new(validator),
                )?;
                if let Some(output) = args.get(6) {
                    let mut file = tempfile::NamedTempFile::new_in(
                        Path::new(output).parent().filter(|path| !path.as_os_str().is_empty())
                            .unwrap_or_else(|| Path::new(".")),
                    )?;
                    file.write_all(&document)?;
                    file.persist_noclobber(output)?;
                    for item in diagnostics {
                        eprintln!("Truncated {} from {} to {} characters", item.field_id, item.actual_chars, item.max_chars);
                    }
                } else {
                    print_diagnostics(&diagnostics)?;
                }
            }
            _ => return Err("usage: docx_form inspect-pdf TEMPLATE | docx_form check-pdf TEMPLATE FORM RECORD FONT VERAPDF | docx_form fill-pdf TEMPLATE FORM RECORD FONT VERAPDF OUTPUT".into()),
        }
        Ok(())
    })())
}
