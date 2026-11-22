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
    html: bool,
    template: &Path,
    definition: &Path,
    record: &Path,
) -> Result<(Vec<u8>, Vec<form_template::Truncation>), Box<dyn Error>> {
    let definition = read_value(definition)?;
    let record = read_value(record)?;
    if html {
        let (document, diagnostics) = form_template::fill_html_with_diagnostics(
            &fs::read_to_string(template)?,
            &definition,
            &record,
        )?;
        Ok((document.into_bytes(), diagnostics))
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
    match args.as_slice() {
        [command, template] if command == "inspect" => {
            let fields = form_template::inspect(&fs::read(template)?)?;
            println!("{}", serde_json::to_string_pretty(&fields)?);
        }
        [command, template, definition, record] if command == "check" || command == "check-html" => {
            let (_, diagnostics) = generate(command == "check-html", Path::new(template), Path::new(definition), Path::new(record))?;
            let warnings: Vec<_> = diagnostics.iter().map(|item| serde_json::json!({
                "fieldId": item.field_id,
                "actualChars": item.actual_chars,
                "maxChars": item.max_chars
            })).collect();
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"truncations": warnings}))?);
        }
        [command, template, definition, record, output] if command == "fill" || command == "fill-html" => {
            let (document, diagnostics) = generate(command == "fill-html", Path::new(template), Path::new(definition), Path::new(record))?;
            for item in diagnostics {
                eprintln!("Truncated {} from {} to {} characters", item.field_id, item.actual_chars, item.max_chars);
            }
            fs::OpenOptions::new().write(true).create_new(true).open(output)?.write_all(&document)?;
        }
        _ => return Err("usage: docx_form inspect TEMPLATE | docx_form check|check-html TEMPLATE FORM RECORD | docx_form fill|fill-html TEMPLATE FORM RECORD OUTPUT".into()),
    }
    Ok(())
}
