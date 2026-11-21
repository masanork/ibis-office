// SPDX-License-Identifier: AGPL-3.0-only

use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};

fn is_xlsx(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input: Vec<_> = std::env::args_os().skip(1).collect();
    let [workbook, source, mapping, operation @ ..] = input.as_slice() else {
        return Err("usage: prefill_preview <workbook.xlsx> <source> <mapping.json> [--apply <preview.json> <new.xlsx> <target>...]".into());
    };
    let workbook = PathBuf::from(workbook).canonicalize()?;
    let source = PathBuf::from(source);
    let format = source
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or("source needs an extension")?
        .to_ascii_lowercase();
    let settings: Value = serde_json::from_slice(&std::fs::read(mapping)?)?;
    let source_dir = workbook.parent().ok_or("workbook has no folder")?;

    let mut bytes = Vec::new();
    std::fs::File::open(source)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err("source exceeds 1 MiB".into());
    }
    let mut params = json!({
        "sheet":settings["sheet"], "format":format,
        "source":String::from_utf8(bytes)?, "mappings":settings["mappings"],
    });
    let report = match operation {
        [] => {
            let mut service =
                ibis_doc_service::Service::new(&[source_dir.to_owned()], true, Default::default());
            params["document_id"] =
                service.call("app_open_document", &json!({"path":workbook}))?["document_id"]
                    .clone();
            service.call("xlsx_preview_prefill", &params)?
        }
        [flag, review, output, targets @ ..] if flag == "--apply" && !targets.is_empty() => {
            let output = Path::new(output);
            if !is_xlsx(&workbook) || !is_xlsx(output) {
                return Err("apply requires an XLSX source and new XLSX destination".into());
            }
            let folder = output
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .canonicalize()?;
            let output = folder.join(output.file_name().ok_or("output needs a filename")?);
            if output.try_exists()? || output.is_symlink() {
                return Err("output must be a new XLSX file".into());
            }
            let review: Value = serde_json::from_slice(&std::fs::read(review)?)?;
            params["preview_id"] = review["preview_id"].clone();
            params["targets"] = json!(
                targets
                    .iter()
                    .map(|target| target.to_str().ok_or("target must be UTF-8"))
                    .collect::<Result<Vec<_>, _>>()?
            );
            let staging = tempfile::Builder::new()
                .prefix(".ibis-prefill-")
                .tempdir_in(folder)?;
            let mut service = ibis_doc_service::Service::new(
                &[source_dir.to_owned(), staging.path().to_owned()],
                false,
                Default::default(),
            );
            params["document_id"] =
                service.call("app_open_document", &json!({"path":workbook}))?["document_id"]
                    .clone();
            let result = service.call("xlsx_apply_prefill", &params)?;
            let staged = staging.path().join("filled.xlsx");
            let saved = service.call(
                "app_save_document",
                &json!({
                    "document_id":params["document_id"], "path":staged,
                }),
            )?;
            // Publish without replacing a file created after the existence check.
            std::fs::hard_link(staged, &output)?;
            json!({"path":output, "applied_targets":result["applied_targets"], "warnings":saved["warnings"]})
        }
        _ => return Err("apply requires --apply <preview.json> <new.xlsx> <target>...".into()),
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
