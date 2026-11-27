// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Official NDLOCR-Lite v2 ONNX model manifest and SHA-256 verification.
//!
//! Model weights are published by the National Diet Library, Japan under
//! CC BY 4.0, with the DEIMv2 layout backbone subject to the DINOv3 License.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// Official base URL for NDLOCR-Lite v2 ONNX models.
pub const MODEL_BASE_URL: &str = "https://lab.ndl.go.jp/dataset/ndlocr-lite/onnx/v2/";

/// License summary shown to the user before downloading.
pub const MODEL_LICENSE_NAME: &str = "CC BY 4.0 + DINOv3 License";
pub const MODEL_LICENSE_URL: &str = "https://github.com/ndl-lab/ndlocr-lite/blob/master/LICENCE";
pub const DINOV3_LICENSE_URL: &str = "https://github.com/facebookresearch/dinov3/blob/main/LICENSE.md";

/// One ONNX model file required by NDLOCR-Lite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelKind {
    /// Layout & line detection (DEIMv2, 800x800).
    Det,
    /// Line text recognition up to 30 characters (PARSeq 24x256).
    Rec30,
    /// Line text recognition up to 50 characters (PARSeq 24x384).
    Rec50,
    /// Line text recognition up to 100 characters (PARSeq 24x768).
    Rec100,
}

/// Metadata for a single downloadable ONNX model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSpec {
    pub kind: ModelKind,
    pub id: &'static str,
    pub filename: &'static str,
    pub sha256: &'static str,
    pub approx_size_bytes: u64,
    pub description_ja: &'static str,
    pub description_en: &'static str,
}

/// Core 4 ONNX models used by NDLOCR-Lite v2 (total ~153 MB).
pub const CORE_MODELS: [ModelSpec; 4] = [
    ModelSpec {
        kind: ModelKind::Det,
        id: "ndlocr-det-v2",
        filename: "deim-s-1024x1024-261124.onnx",
        sha256: "f9a41c8453dd687f370bee199fd6d47ecf583cc14f1d839a0b416db39b356e8c",
        approx_size_bytes: 39_000_000,
        description_ja: "レイアウト・行領域検出モデル (DEIMv2)",
        description_en: "Layout and text line detection model (DEIMv2)",
    },
    ModelSpec {
        kind: ModelKind::Rec30,
        id: "ndlocr-rec30-v2",
        filename: "parseq-ndl-24x256-30-tiny-499epoch-261124.onnx",
        sha256: "063ec5b3201bce79d56c3ad73b5726cf76ec6a82f42e11a89466ae683755296c",
        approx_size_bytes: 36_000_000,
        description_ja: "文字列認識モデル・短行用 (PARSeq ≤30文字)",
        description_en: "Text recognition model for short lines (PARSeq ≤30 chars)",
    },
    ModelSpec {
        kind: ModelKind::Rec50,
        id: "ndlocr-rec50-v2",
        filename: "parseq-ndl-24x384-50-tiny-278epoch-261124.onnx",
        sha256: "d3dd9e7733aeff5c9f50687c7da6e8d01915a0bf07cfbefe1d9ac58241c61471",
        approx_size_bytes: 37_000_000,
        description_ja: "文字列認識モデル・標準行用 (PARSeq ≤50文字)",
        description_en: "Text recognition model for medium lines (PARSeq ≤50 chars)",
    },
    ModelSpec {
        kind: ModelKind::Rec100,
        id: "ndlocr-rec100-v2",
        filename: "parseq-ndl-24x768-100-tiny-153epoch-261124.onnx",
        sha256: "06462b0dbd5b0b8508545c8c3d485cf20dbf4ffa652fe145e69c9e7457080602",
        approx_size_bytes: 41_000_000,
        description_ja: "文字列認識モデル・長行用 (PARSeq ≤100文字)",
        description_en: "Text recognition model for long lines (PARSeq ≤100 chars)",
    },
];

impl ModelSpec {
    /// Full HTTPS download URL on `lab.ndl.go.jp`.
    pub fn url(&self) -> String {
        format!("{MODEL_BASE_URL}{}", self.filename)
    }
}

/// Look up the [`ModelSpec`] for a [`ModelKind`].
pub fn spec_for(kind: ModelKind) -> &'static ModelSpec {
    CORE_MODELS
        .iter()
        .find(|m| m.kind == kind)
        .expect("all ModelKind variants are in CORE_MODELS")
}

/// Compute lowercase hex SHA-256 of an in-memory buffer.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_lower(&digest)
}

/// Verify that `bytes` matches `expected_hex` (case-insensitive).
pub fn verify_sha256_bytes(bytes: &[u8], expected_hex: &str) -> Result<()> {
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(Error::new(format!(
            "SHA-256 mismatch: expected {expected_hex}, got {actual}"
        )))
    }
}

/// Verify that a file on disk matches `expected_hex`.
pub fn verify_sha256_file(path: &Path, expected_hex: &str) -> Result<()> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| Error::new(format!("failed to open {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| Error::new(format!("failed to read {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex_lower(&hasher.finalize());
    if actual.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(Error::new(format!(
            "SHA-256 mismatch for {}: expected {expected_hex}, got {actual}",
            path.display()
        )))
    }
}

/// Check whether all core models exist in `dir` (and optionally pass SHA-256).
pub fn missing_models(dir: &Path) -> Vec<&'static ModelSpec> {
    CORE_MODELS
        .iter()
        .filter(|spec| !dir.join(spec.filename).is_file())
        .collect()
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_models_have_valid_urls_and_hashes() {
        for m in CORE_MODELS {
            assert!(m.url().starts_with("https://lab.ndl.go.jp/"));
            assert_eq!(m.sha256.len(), 64);
            assert!(m.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn sha256_verification_works() {
        let empty_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_sha256_bytes(b"", empty_hash).is_ok());
        assert!(verify_sha256_bytes(b"abc", empty_hash).is_err());
    }
}
