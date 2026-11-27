// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Optional NDLOCR-Lite model download and cache management (modeled on [`super::font_setup`]).
//!
//! Model weights (~153 MB total) are not bundled with the app. Users who want
//! local Japanese OCR can download them on demand from the National Diet
//! Library's official distribution (`https://lab.ndl.go.jp/dataset/ndlocr-lite/onnx/v2/`).
//! Each downloaded file is verified against its expected SHA-256 checksum
//! before replacing any cached file, and the CC BY 4.0 + DINOv3 license files
//! are saved alongside the weights.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use ibis_ocr::models::{
    CORE_MODELS, DINOV3_LICENSE_URL, MODEL_LICENSE_URL, ModelSpec, missing_models as missing_in_dir,
    verify_sha256_file,
};

use super::font_setup::DownloadError;

const NDL_RAW_LICENSE_URL: &str =
    "https://raw.githubusercontent.com/ndl-lab/ndlocr-lite/master/LICENCE";
const DINOV3_RAW_LICENSE_URL: &str =
    "https://raw.githubusercontent.com/facebookresearch/dinov3/main/LICENSE.md";

/// Directory where NDLOCR-Lite v2 ONNX models are stored.
#[cfg(not(target_family = "wasm"))]
pub fn models_dir() -> Option<PathBuf> {
    ibis_office_base::app_data::ocr_models_dir()
}

/// Returns the models from [`CORE_MODELS`] that are not yet present in [`models_dir`].
#[cfg(not(target_family = "wasm"))]
pub fn missing_models() -> Vec<&'static ModelSpec> {
    let Some(dir) = models_dir() else {
        return CORE_MODELS.iter().collect();
    };
    missing_in_dir(&dir)
}

/// Returns `true` when all core NDLOCR-Lite models are present in [`models_dir`].
#[cfg(not(target_family = "wasm"))]
pub fn are_models_installed() -> bool {
    missing_models().is_empty()
}

/// Approximate total size in bytes of all [`CORE_MODELS`].
pub fn total_model_bytes() -> u64 {
    CORE_MODELS.iter().map(|m| m.approx_size_bytes).sum()
}

/// License reference URLs shown to the user before downloading.
pub fn license_urls() -> (&'static str, &'static str) {
    (MODEL_LICENSE_URL, DINOV3_LICENSE_URL)
}

/// Download all missing core NDLOCR-Lite models into [`models_dir`].
///
/// * `current_index`: 1-based index of the model currently being downloaded.
/// * `progress_bytes`: bytes received so far for the current model file.
/// * `cancel`: set to `true` to abort the download cleanly (partial files are removed).
#[cfg(not(target_family = "wasm"))]
pub fn download_missing(
    current_index: &AtomicUsize,
    progress_bytes: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    let dir = models_dir()
        .ok_or_else(|| DownloadError::Failed("no application data directory for OCR models".into()))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| DownloadError::Failed(format!("{}: {e}", dir.display())))?;

    // Save license files beside the weights (best-effort if mirror is unreachable, but fail on cancel).
    download_license_if_missing(&dir, "LICENCE-NDLOCR.txt", NDL_RAW_LICENSE_URL, cancel)?;
    download_license_if_missing(&dir, "LICENCE-DINOV3.md", DINOV3_RAW_LICENSE_URL, cancel)?;

    let to_download = missing_in_dir(&dir);
    for (idx, spec) in to_download.iter().enumerate() {
        current_index.store(idx + 1, Ordering::SeqCst);
        progress_bytes.store(0, Ordering::SeqCst);

        let dest = dir.join(spec.filename);
        let part = dir.join(format!("{}.part", spec.filename));
        let url = model_url(spec);
        let result = super::font_setup::fetch_to_file(&url, &part, 1800, Some(progress_bytes), cancel);
        if let Err(e) = result {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        if let Err(e) = verify_sha256_file(&part, spec.sha256) {
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Failed(e.to_string()));
        }
        if let Err(e) = std::fs::rename(&part, &dest) {
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Failed(format!("{}: {e}", dest.display())));
        }
    }

    Ok(())
}

#[cfg(not(target_family = "wasm"))]
fn model_url(spec: &ModelSpec) -> String {
    if let Ok(base) = std::env::var("IBIS_OCR_MODEL_BASE_URL") {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            return format!("{}/{}", trimmed.trim_end_matches('/'), spec.filename);
        }
    }
    spec.url()
}

#[cfg(not(target_family = "wasm"))]
fn download_license_if_missing(
    dir: &std::path::Path,
    filename: &str,
    url: &str,
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    let dest = dir.join(filename);
    if dest.is_file() {
        return Ok(());
    }
    let part = dir.join(format!("{filename}.part"));
    let res = super::font_setup::fetch_to_file(url, &part, 60, None, cancel);
    match res {
        Ok(()) => {
            let _ = std::fs::rename(&part, &dest);
            Ok(())
        }
        Err(DownloadError::Cancelled) => {
            let _ = std::fs::remove_file(&part);
            Err(DownloadError::Cancelled)
        }
        Err(_) => {
            let _ = std::fs::remove_file(&part);
            Ok(())
        }
    }
}

/// Remove all downloaded NDLOCR-Lite models and licenses to free disk space.
#[cfg(not(target_family = "wasm"))]
pub fn remove_models() -> Result<(), String> {
    let Some(dir) = models_dir() else {
        return Ok(());
    };
    if !dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_bytes_is_around_153mb() {
        let total = total_model_bytes();
        assert!(total > 140_000_000 && total < 170_000_000);
    }

    #[test]
    fn missing_models_reports_all_when_dir_empty() {
        let tmp = std::env::temp_dir().join("ibis-ocr-setup-test-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert_eq!(missing_in_dir(&tmp).len(), 4);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
