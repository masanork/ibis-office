// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust Whisper BPE tokenizer decoder reading HuggingFace `tokenizer.json`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;

use crate::{Error, Result};

#[derive(Deserialize)]
struct TokenizerJson {
    model: VocabModel,
    #[serde(default)]
    added_tokens: Vec<AddedToken>,
}

#[derive(Deserialize)]
struct VocabModel {
    vocab: HashMap<String, u32>,
}

#[derive(Deserialize)]
struct AddedToken {
    id: u32,
    content: String,
    #[serde(default)]
    special: bool,
}

/// Decodes Whisper token IDs to UTF-8 text without external C++ tokenizer libraries.
pub struct WhisperTokenizer {
    id_to_token: HashMap<u32, String>,
    token_to_id: HashMap<String, u32>,
    special_ids: HashSet<u32>,
    byte_decoder: HashMap<char, u8>,
}

impl WhisperTokenizer {
    pub fn from_file(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| Error::new(format!("failed to read {}: {e}", path.display())))?;
        Self::from_json_str(&data)
    }

    pub fn from_json_str(json: &str) -> Result<Self> {
        let parsed: TokenizerJson = serde_json::from_str(json)
            .map_err(|e| Error::new(format!("invalid tokenizer.json: {e}")))?;
        let mut id_to_token = HashMap::with_capacity(parsed.model.vocab.len() + parsed.added_tokens.len());
        let mut token_to_id = HashMap::with_capacity(parsed.model.vocab.len() + parsed.added_tokens.len());
        let mut special_ids = HashSet::new();

        for (tok, id) in parsed.model.vocab {
            id_to_token.insert(id, tok.clone());
            token_to_id.insert(tok, id);
        }
        for added in parsed.added_tokens {
            if added.special {
                special_ids.insert(added.id);
            }
            id_to_token.insert(added.id, added.content.clone());
            token_to_id.insert(added.content, added.id);
        }

        Ok(Self {
            id_to_token,
            token_to_id,
            special_ids,
            byte_decoder: build_byte_decoder(),
        })
    }

    pub fn token_id(&self, token: &str) -> Option<u32> {
        self.token_to_id.get(token).copied()
    }

    pub fn eot_id(&self) -> u32 {
        self.token_id("<|endoftext|>").unwrap_or(50257)
    }

    pub fn sot_id(&self) -> u32 {
        self.token_id("<|startoftranscript|>").unwrap_or(50258)
    }

    pub fn transcribe_id(&self) -> u32 {
        self.token_id("<|transcribe|>").unwrap_or(50359)
    }

    pub fn no_timestamps_id(&self) -> u32 {
        self.token_id("<|notimestamps|>").unwrap_or(50363)
    }

    /// Decode a sequence of token IDs into a UTF-8 string, skipping special tokens.
    pub fn decode(&self, ids: &[u32]) -> String {
        let mut bytes = Vec::with_capacity(ids.len() * 3);
        for &id in ids {
            if self.special_ids.contains(&id) {
                continue;
            }
            let Some(tok) = self.id_to_token.get(&id) else {
                continue;
            };
            if tok.starts_with("<|") && tok.ends_with("|>") {
                continue;
            }
            for ch in tok.chars() {
                if let Some(&b) = self.byte_decoder.get(&ch) {
                    bytes.push(b);
                } else {
                    let mut buf = [0u8; 4];
                    bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Inverse of OpenAI GPT-2 / Whisper `bytes_to_unicode()`.
fn build_byte_decoder() -> HashMap<char, u8> {
    let mut bs: Vec<u8> = (b'!'..=b'~')
        .chain(0xA1u8..=0xACu8)
        .chain(0xAEu8..=0xFFu8)
        .collect();
    let mut cs: Vec<u32> = bs.iter().map(|&b| u32::from(b)).collect();
    let mut n = 0u32;
    for b in 0u8..=255u8 {
        if !bs.contains(&b) {
            bs.push(b);
            cs.push(256 + n);
            n += 1;
        }
    }
    let mut map = HashMap::with_capacity(256);
    for (b, c) in bs.into_iter().zip(cs.into_iter()) {
        if let Some(ch) = char::from_u32(c) {
            map.insert(ch, b);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_gpt2_byte_tokens() {
        let json = r#"{
            "model": { "vocab": { "hello": 1, "Ġworld": 2 } },
            "added_tokens": [ { "id": 50257, "content": "<|endoftext|>", "special": true } ]
        }"#;
        let tok = WhisperTokenizer::from_json_str(json).unwrap();
        assert_eq!(tok.decode(&[1, 2, 50257]), "hello world");
    }
}
