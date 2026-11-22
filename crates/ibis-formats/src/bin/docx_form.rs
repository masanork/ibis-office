// SPDX-License-Identifier: AGPL-3.0-only
use ibis_formats::form_template;
use std::{error::Error, fs, io::Write};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [command, template] if command == "inspect" => {
            let fields = form_template::inspect(&fs::read(template)?)?;
            println!("{}", serde_json::to_string_pretty(&fields)?);
        }
        [command, template, definition, record, output] if command == "fill" => {
            let definition = serde_json::from_slice(&fs::read(definition)?)?;
            let record = serde_json::from_slice(&fs::read(record)?)?;
            let result = form_template::fill(&fs::read(template)?, &definition, &record)?;
            fs::OpenOptions::new().write(true).create_new(true).open(output)?.write_all(&result)?;
        }
        _ => return Err("usage: docx_form inspect TEMPLATE | docx_form fill TEMPLATE FORM.json RECORD.json OUTPUT.docx".into()),
    }
    Ok(())
}
