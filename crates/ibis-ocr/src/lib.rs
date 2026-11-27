// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust Japanese and multilingual OCR engine implementing NDL OCR-Lite
//! (DEIMv2 layout/line detection + PARSeq text recognition) over `tract-onnx`.
//!
//! Model weights are not bundled; they are downloaded on demand by the user
//! from the National Diet Library (NDL) official distribution and verified by
//! SHA-256 before loading.

pub mod models;
pub mod reading_order;

#[cfg(feature = "tract")]
pub mod deim;
#[cfg(feature = "tract")]
pub mod engine;
#[cfg(feature = "tract")]
pub mod parseq;

/// Errors returned by the OCR engine and model loader.
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

/// A detected and recognized line of text on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct OcrLine {
    /// Bounding box `[x1, y1, x2, y2]` in source image pixels.
    pub box_xyxy: [u32; 4],
    /// Layout class index (e.g. `1` = `line_main`, `16` = `line_title`).
    pub class_index: usize,
    /// Layout class name from `ndl.yaml`.
    pub class_name: &'static str,
    /// Detection confidence score.
    pub confidence: f32,
    /// Predicted character count from DEIMv2.
    pub pred_char_count: f32,
    /// Whether the line is vertical (`height > width * 0.8`).
    pub is_vertical: bool,
    /// Recognized text string.
    pub text: String,
    /// Reading order rank (0-based).
    pub order: usize,
}

/// Recognized OCR result for a single page or image.
#[derive(Clone, Debug, PartialEq)]
pub struct OcrPage {
    pub width: u32,
    pub height: u32,
    pub lines: Vec<OcrLine>,
}

impl OcrPage {
    /// Join recognized lines in reading order as plain text.
    pub fn to_plain_text(&self) -> String {
        self.lines
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Format recognized lines as Markdown, preserving title headings when detected.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            let trimmed = line.text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            if line.class_name == "line_title" {
                out.push_str("## ");
            }
            out.push_str(trimmed);
        }
        out
    }
}
