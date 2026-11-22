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

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [command, template] if command == "inspect" => {
            let fields = form_template::inspect(&fs::read(template)?)?;
            println!("{}", serde_json::to_string_pretty(&fields)?);
        }
        [command, template, definition, record, output] if command == "fill" || command == "fill-html" => {
            let definition = read_value(Path::new(definition))?;
            let record = read_value(Path::new(record))?;
            let result = if command == "fill" {
                form_template::fill(&fs::read(template)?, &definition, &record)?
            } else {
                form_template::fill_html(&fs::read_to_string(template)?, &definition, &record)?.into_bytes()
            };
            fs::OpenOptions::new().write(true).create_new(true).open(output)?.write_all(&result)?;
        }
        _ => return Err("usage: docx_form inspect TEMPLATE | docx_form fill|fill-html TEMPLATE FORM RECORD OUTPUT (FORM and RECORD: .json, .yaml, .yml)".into()),
    }
    Ok(())
}
