// SPDX-License-Identifier: AGPL-3.0-only
use base64::{Engine, engine::general_purpose::STANDARD};
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
        [command, layout, definition, record, mapping, output]
            if command == "review-html-claims" =>
        {
            let preview = form_template::preview_html_claims(
                &fs::read_to_string(layout)?,
                &read_value(Path::new(definition))?,
                &read_value(Path::new(record))?,
                &read_value(Path::new(mapping))?,
            )?;
            let page = review_html(&preview)?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output)?
                .write_all(page.as_bytes())?;
        }
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
        _ => return Err("usage: docx_form export-md|compile-md INPUT OUTPUT | docx_form inspect|inspect-md TEMPLATE | docx_form check|check-html|check-md TEMPLATE FORM RECORD | docx_form fill|fill-html|fill-md TEMPLATE FORM RECORD OUTPUT | docx_form preview-html-claims LAYOUT FORM RECORD MAPPING | docx_form review-html-claims LAYOUT FORM RECORD MAPPING OUTPUT | docx_form inspect-pdf|check-pdf|fill-pdf …".into()),
    }
    Ok(())
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn review_html(preview: &serde_json::Value) -> Result<String, Box<dyn Error>> {
    let html = preview["html"].as_str().ok_or("Missing print preview")?;
    let claims = format!("{}\n", serde_json::to_string_pretty(&preview["claims"])?);
    let claim_download = STANDARD.encode(claims.as_bytes());
    let claims = escape_html(&claims);
    let print_preview = escape_html(html);
    let mut warnings = String::new();
    if let Some(truncations) = preview["truncations"].as_array().filter(|items| !items.is_empty()) {
        warnings.push_str("<aside><h2>紙面で切り詰められた欄</h2><ul>");
        for item in truncations {
            let field = escape_html(item["fieldId"].as_str().unwrap_or_default());
            warnings.push_str(&format!(
                "<li><code>{field}</code>（{}文字、上限{}文字）</li>",
                item["actualChars"], item["maxChars"],
            ));
        }
        warnings.push_str("</ul></aside>");
    }
    Ok(format!(r#"<!doctype html>
<html lang="ja">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; frame-src 'self'">
<title>帳票とクレームの確認</title>
<style>
  body {{ margin: 0; background: #f3f5f4; color: #172821; font: 1rem/1.5 system-ui, sans-serif; }}
  main {{ max-width: 85rem; margin: auto; padding: clamp(1rem, 3vw, 2.5rem); }}
  h1 {{ margin: 0 0 1.5rem; font-size: clamp(1.5rem, 3vw, 2.25rem); }}
  .panels {{ display: grid; grid-template-columns: 3fr 2fr; gap: 1rem; align-items: start; }}
  section, aside {{ min-width: 0; background: white; border: 1px solid #ccd7d1; border-radius: .6rem; padding: 1rem; }}
  aside {{ margin-bottom: 1rem; }}
  h2 {{ margin: 0 0 .8rem; font-size: 1.15rem; }}
  iframe {{ display: block; width: 100%; min-height: 30rem; border: 0; }}
  pre {{ overflow: auto; background: #edf2ef; border-radius: .4rem; padding: 1rem; }}
  a {{ display: inline-block; background: #154d39; color: white; padding: .55rem .9rem; border-radius: .35rem; }}
  a:focus-visible {{ outline: 3px solid #236849; outline-offset: 3px; }}
  @media (max-width: 50rem) {{ .panels {{ grid-template-columns: 1fr; }} }}
</style>
<main>
  <h1>帳票とクレームの確認</h1>
  {warnings}
  <div class="panels">
    <section>
      <h2>紙面プレビュー</h2>
      <iframe title="紙面プレビュー" sandbox srcdoc="{print_preview}"></iframe>
    </section>
    <section>
      <h2>クレーム候補</h2>
      <pre tabindex="0">{claims}</pre>
      <a download="claims.json" href="data:application/json;base64,{claim_download}">クレームをJSONで保存</a>
    </section>
  </div>
</main>
</html>
"#))
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
