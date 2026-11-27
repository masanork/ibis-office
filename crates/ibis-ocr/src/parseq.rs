// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! PARSeq text-line recognizer over `tract-onnx`.

use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

use image::imageops::FilterType;
use image::RgbImage;
use tract_onnx::prelude::*;

use crate::{Error, Result};

const CHARSET_RAW: &str = include_str!("charset.txt");

fn ndl_charset() -> &'static [char] {
    static CHARSET: OnceLock<Vec<char>> = OnceLock::new();
    CHARSET.get_or_init(|| CHARSET_RAW.chars().collect())
}

type RunnablePlan = std::sync::Arc<TypedRunnableModel>;

/// Pure-Rust PARSeq recognizer (`parseq-ndl-24x{256,384,768}-*.onnx`).
pub struct ParseqRecognizer {
    plan: RunnablePlan,
    input_height: usize,
    input_width: usize,
}

impl ParseqRecognizer {
    /// Load from an ONNX file on disk with the expected input width (`256`, `384`, or `768`).
    pub fn from_path(path: &Path, input_width: usize) -> Result<Self> {
        let model = tract_onnx::onnx()
            .model_for_path(path)
            .map_err(|e| Error::new(format!("PARSeq model_for_path failed: {e}")))?;
        Self::from_inference_model(model, input_width)
    }

    /// Load from an in-memory ONNX buffer (usable on WASM/IndexedDB).
    pub fn from_bytes(bytes: &[u8], input_width: usize) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let model = tract_onnx::onnx()
            .model_for_read(&mut cursor)
            .map_err(|e| Error::new(format!("PARSeq model_for_read failed: {e}")))?;
        Self::from_inference_model(model, input_width)
    }

    fn from_inference_model(model: InferenceModel, input_width: usize) -> Result<Self> {
        let input_height = 24usize;
        let plan = model
            .with_input_fact(
                0,
                InferenceFact::dt_shape(f32::datum_type(), tvec!(1, 3, input_height, input_width)),
            )
            .map_err(|e| Error::new(format!("PARSeq input fact failed: {e}")))?
            .into_optimized()
            .map_err(|e| Error::new(format!("PARSeq into_optimized failed: {e}")))?
            .into_runnable()
            .map_err(|e| Error::new(format!("PARSeq into_runnable failed: {e}")))?;

        Ok(Self {
            plan,
            input_height,
            input_width,
        })
    }

    /// Recognize a cropped text line image.
    ///
    /// Vertical lines (`height > width * 0.8`) are automatically rotated 90°
    /// counter-clockwise before recognition, matching `ndlocr-lite/src/parseq.py`.
    pub fn read(&self, crop: &RgbImage) -> Result<String> {
        let (w, h) = crop.dimensions();
        if w == 0 || h == 0 {
            return Ok(String::new());
        }

        let oriented: std::borrow::Cow<'_, RgbImage> = if (h as f32) > (w as f32 * 0.8) {
            std::borrow::Cow::Owned(image::imageops::rotate270(crop))
        } else {
            std::borrow::Cow::Borrowed(crop)
        };

        let resized = image::imageops::resize(
            oriented.as_ref(),
            self.input_width as u32,
            self.input_height as u32,
            FilterType::Triangle,
        );

        let mut input =
            tract_ndarray::Array4::<f32>::zeros((1, 3, self.input_height, self.input_width));
        for (x, y, px) in resized.enumerate_pixels() {
            let xu = x as usize;
            let yu = y as usize;
            // BGR order, normalized to [-1, 1] as in parseq.py.
            input[[0, 0, yu, xu]] = px[2] as f32 / 127.5 - 1.0;
            input[[0, 1, yu, xu]] = px[1] as f32 / 127.5 - 1.0;
            input[[0, 2, yu, xu]] = px[0] as f32 / 127.5 - 1.0;
        }

        let outputs = self
            .plan
            .run(tvec!(input.into_tensor().into()))
            .map_err(|e| Error::new(format!("PARSeq inference failed: {e}")))?;

        let logits = outputs[0]
            .to_plain_array_view::<f32>()
            .map_err(|e| Error::new(format!("PARSeq output view failed: {e}")))?;

        let charset = ndl_charset();
        let steps = logits.shape()[1];
        let vocab = logits.shape()[2];

        let mut text = String::new();
        for t in 0..steps {
            let mut best_idx = 0usize;
            let mut best_val = f32::NEG_INFINITY;
            for c in 0..vocab {
                let v = logits[[0, t, c]];
                if v > best_val {
                    best_val = v;
                    best_idx = c;
                }
            }
            if best_idx == 0 {
                break;
            }
            if let Some(&ch) = charset.get(best_idx - 1) {
                text.push(ch);
            }
        }

        Ok(remove_ascii_spaces_around_japanese_punctuation(&text))
    }
}

fn remove_ascii_spaces_around_japanese_punctuation(text: &str) -> String {
    let mut out = text.to_string();
    for punct in ["、", "。"] {
        out = out.replace(&format!(" {punct}"), punct);
        out = out.replace(&format!("{punct} "), punct);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charset_has_expected_size() {
        assert_eq!(ndl_charset().len(), 7141);
    }

    #[test]
    fn punctuation_space_cleanup_works() {
        assert_eq!(
            remove_ascii_spaces_around_japanese_punctuation("春はあけぼの 。 夏は 、 夜"),
            "春はあけぼの。夏は、夜"
        );
    }
}
