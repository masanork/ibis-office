// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

#[cfg(target_arch = "wasm32")]
mod web {
    use ibis_formats::lawtext::{ShinkyuOptions, build_5set_package, parse_law_input, to_markdown};
    use serde_json::json;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn compare_laws(old: &str, new: &str, memo: &str) -> Result<String, JsError> {
        let old = parse_law_input(old)?;
        let new = parse_law_input(new)?;
        let package = build_5set_package(&old, &new, memo, &ShinkyuOptions::default());
        Ok(json!({
            "old": to_markdown(&old),
            "new": to_markdown(&new),
            "package": package.to_json_value(),
        })
        .to_string())
    }

    #[wasm_bindgen]
    pub fn export_comparison_docx(comparison: &str) -> Result<Vec<u8>, JsError> {
        Ok(ibis_formats::markdown::import_markdown_to_docx_bytes(
            comparison.as_bytes(),
        )?)
    }

    #[wasm_bindgen]
    pub fn export_docx(old: &str, new: &str, memo: &str) -> Result<Vec<u8>, JsError> {
        let old = parse_law_input(old)?;
        let new = parse_law_input(new)?;
        Ok(build_5set_package(&old, &new, memo, &ShinkyuOptions::default()).to_print_docx()?)
    }
}
