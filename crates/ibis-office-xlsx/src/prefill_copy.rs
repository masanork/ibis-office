// SPDX-License-Identifier: AGPL-3.0-only
//! Recheck and save a reviewed workbook copy without updating the editor.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use ibis_office_base::i18n::text;
use serde_json::Value;

pub(crate) struct ReviewInput {
    pub(crate) source: PathBuf,
    pub(crate) mapping: PathBuf,
    pub(crate) source_text: String,
    pub(crate) mapping_text: String,
    pub(crate) format: String,
    pub(crate) report: Value,
}

pub(crate) fn is_xlsx(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"))
}

pub(crate) fn save_reviewed_copy(
    input: &ReviewInput,
    presentation: ibis_doc_model::Presentation,
    package: &ibis_formats::Source,
    slide: ibis_doc_model::SlideId,
    workbook: &Path,
    output: &Path,
    targets: &BTreeSet<String>,
) -> Result<(), String> {
    if !is_xlsx(workbook) || !is_xlsx(output) || output.exists() || output.is_symlink() {
        return Err(text("xlsx.prefill.new-file").to_string());
    }
    let stale = || text("xlsx.prefill.changed").to_string();
    if read_text(&input.source)? != input.source_text
        || read_text(&input.mapping)? != input.mapping_text
    {
        return Err(stale());
    }
    let settings: Value = serde_json::from_str(&input.mapping_text).map_err(|_| stale())?;
    let signed = package.has_package_signatures().unwrap_or(true);
    let current = ibis_doc_service::review_prefill(
        &presentation,
        slide,
        signed,
        &input.format,
        &input.source_text,
        &settings["mappings"],
    )
    .map_err(|_| stale())?;
    if current != input.report {
        return Err(stale());
    }
    let original = package
        .save(package.baseline(), ibis_formats::SaveOptions::default())
        .map_err(|error| error.to_string())?;
    let file = std::fs::File::open(workbook).map_err(|error| error.to_string())?;
    let mut disk = Vec::new();
    file.take(original.len() as u64 + 1)
        .read_to_end(&mut disk)
        .map_err(|error| error.to_string())?;
    if disk != original {
        return Err(stale());
    }

    let mut document = ibis_doc_model::Document::new(presentation);
    let command =
        ibis_doc_service::reviewed_prefill_command(&current, slide, &serde_json::json!(targets))
            .map_err(|error| error.message)?;
    document
        .execute(vec![command])
        .map_err(|error| error.to_string())?;
    let bytes = package
        .save(
            document.presentation(),
            ibis_formats::SaveOptions::default(),
        )
        .map_err(|error| error.to_string())?;
    let directory = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut staged =
        tempfile::NamedTempFile::new_in(directory).map_err(|error| error.to_string())?;
    staged
        .write_all(&bytes)
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    staged
        .persist_noclobber(output)
        .map_err(|error| error.error.to_string())?;
    Ok(())
}

pub(crate) fn read_text(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(1_048_577).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err(text("xlsx.prefill.too-large").to_string());
    }
    String::from_utf8(bytes).map_err(|_| text("xlsx.prefill.utf8").to_string())
}
