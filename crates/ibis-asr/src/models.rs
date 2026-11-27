// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Official Whisper-tiny ONNX model manifest and SHA-256 verification.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// License metadata for OpenAI Whisper (`onnx-community/whisper-tiny`).
pub const MODEL_LICENSE_NAME: &str = "MIT";
pub const MODEL_LICENSE_URL: &str = "https://huggingface.co/openai/whisper-tiny/blob/main/LICENSE";

/// One downloadable file for the Whisper ASR pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelKind {
    Encoder,
    Decoder,
    Tokenizer,
}

/// Metadata for a single downloadable ASR model/tokenizer file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSpec {
    pub kind: ModelKind,
    pub id: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub approx_size_bytes: u64,
    pub description_ja: &'static str,
    pub description_en: &'static str,
}

/// Whisper-tiny ONNX files (total ~148 MB).
pub const WHISPER_TINY_MODELS: [ModelSpec; 3] = [
    ModelSpec {
        kind: ModelKind::Encoder,
        id: "whisper-tiny-encoder",
        filename: "whisper_tiny_encoder.onnx",
        url: "https://huggingface.co/onnx-community/whisper-tiny/resolve/main/onnx/encoder_model.onnx",
        sha256: "6642befb640f950d4a8cbbd17834d59e7e75f575b81ccf213e06b050623ab1dd",
        approx_size_bytes: 33_500_000,
        description_ja: "Whisper-tiny 音声エンコーダ (ONNX)",
        description_en: "Whisper-tiny audio encoder (ONNX)",
    },
    ModelSpec {
        kind: ModelKind::Decoder,
        id: "whisper-tiny-decoder",
        filename: "whisper_tiny_decoder.onnx",
        url: "https://huggingface.co/onnx-community/whisper-tiny/resolve/main/onnx/decoder_model.onnx",
        sha256: "ab79e3f2a9a3d98f159f853a3172120a38af7eb5f7863d706aa7d39c228f009e",
        approx_size_bytes: 118_500_000,
        description_ja: "Whisper-tiny テキストデコーダ (ONNX)",
        description_en: "Whisper-tiny text decoder (ONNX)",
    },
    ModelSpec {
        kind: ModelKind::Tokenizer,
        id: "whisper-tiny-tokenizer",
        filename: "tokenizer.json",
        url: "https://huggingface.co/onnx-community/whisper-tiny/resolve/main/tokenizer.json",
        sha256: "27fc476bfe7f17299480be2273fc0608e4d5a99aba2ab5dec5374b4482d1a566",
        approx_size_bytes: 2_500_000,
        description_ja: "Whisper 多言語トークナイザ (JSON)",
        description_en: "Whisper multilingual tokenizer (JSON)",
    },
];

pub fn spec_for(kind: ModelKind) -> &'static ModelSpec {
    WHISPER_TINY_MODELS
        .iter()
        .find(|m| m.kind == kind)
        .expect("all ModelKind variants are in WHISPER_TINY_MODELS")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

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

pub fn missing_models(dir: &Path) -> Vec<&'static ModelSpec> {
    WHISPER_TINY_MODELS
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
    fn whisper_models_have_valid_urls_and_hashes() {
        for m in WHISPER_TINY_MODELS {
            assert!(m.url.starts_with("https://huggingface.co/"));
            assert_eq!(m.sha256.len(), 64);
            assert!(m.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }
}
