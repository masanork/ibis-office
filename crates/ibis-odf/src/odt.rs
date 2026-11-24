// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! OpenDocument Text (`.odt`, ODF 1.2/1.3) import and export entry points.

use ibis_formats::{Error, Opened};

pub use ibis_formats::docx_convert::{export_docx_to_odt, export_odt, is_odt};

/// Convert OpenDocument Text (`.odt`) bytes into `.docx` package bytes.
pub fn import_odt_to_docx_bytes(data: &[u8]) -> Result<Vec<u8>, Error> {
    crate::odt_import::import(data)
}

/// Open OpenDocument Text (`.odt`) bytes as an editable Word document (`Opened`).
pub fn import_odt(data: &[u8]) -> Result<Opened, Error> {
    let docx_bytes = import_odt_to_docx_bytes(data)?;
    ibis_formats::open(docx_bytes)
}
