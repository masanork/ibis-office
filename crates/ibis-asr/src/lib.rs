// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust speech-to-text (Whisper ONNX over `tract-onnx`) with `symphonia`
//! audio decoding and 80-bin Log-Mel spectrogram computation.
//!
//! Model weights are not bundled; they are downloaded on demand by the user
//! and verified by SHA-256 before loading.

pub mod mel;
pub mod models;
pub mod tokenizer;

#[cfg(feature = "engine")]
pub mod audio;
#[cfg(feature = "engine")]
pub mod whisper;

/// Errors returned by the ASR engine, audio decoder, and model loader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// A single timestamped transcription segment.
#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptSegment {
    pub start_secs: f32,
    pub end_secs: f32,
    pub text: String,
}

/// Complete transcription result for an audio stream or file.
#[derive(Clone, Debug, PartialEq)]
pub struct Transcript {
    pub language: Option<String>,
    pub segments: Vec<TranscriptSegment>,
    pub text: String,
}

impl Transcript {
    /// Format as Markdown with `[MM:SS - MM:SS]` timestamps per segment.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        for seg in &self.segments {
            let trimmed = seg.text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            let s_min = (seg.start_secs / 60.0).floor() as u32;
            let s_sec = (seg.start_secs % 60.0).floor() as u32;
            let e_min = (seg.end_secs / 60.0).floor() as u32;
            let e_sec = (seg.end_secs % 60.0).floor() as u32;
            out.push_str(&format!(
                "- **[{s_min:02}:{s_sec:02} – {e_min:02}:{e_sec:02}]** {trimmed}"
            ));
        }
        out
    }
}
