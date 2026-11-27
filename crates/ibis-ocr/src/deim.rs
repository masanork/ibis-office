// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! DEIMv2 layout and text-line detector over `tract-onnx`.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use image::imageops::FilterType;
use image::RgbImage;
use tract_onnx::prelude::*;

use crate::{Error, Result};

/// Layout classes from `ndlocr-lite/src/config/ndl.yaml`.
pub const NDL_CLASSES: [&str; 17] = [
    "text_block",
    "line_main",
    "line_caption",
    "line_ad",
    "line_note",
    "line_note_tochu",
    "block_fig",
    "block_ad",
    "block_pillar",
    "block_folio",
    "block_rubi",
    "block_chart",
    "block_eqn",
    "block_cfm",
    "block_eng",
    "block_table",
    "line_title",
];

/// Returns `true` if `class_index` is a text line that should be passed to PARSeq.
pub fn is_text_line_class(class_index: usize) -> bool {
    matches!(class_index, 1 | 2 | 3 | 4 | 5 | 16)
}

/// A single detected layout or text-line bounding box.
#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    pub class_index: usize,
    pub class_name: &'static str,
    pub confidence: f32,
    pub box_xyxy: [u32; 4],
    pub pred_char_count: f32,
}

type RunnablePlan = std::sync::Arc<TypedRunnableModel>;

/// Pure-Rust DEIMv2 detector (`deim-s-1024x1024-261124.onnx`, input 800x800).
pub struct DeimDetector {
    plan: RunnablePlan,
    input_width: usize,
    input_height: usize,
    conf_threshold: f32,
    overlap_threshold: f32,
}

impl DeimDetector {
    /// Load from an ONNX file on disk.
    pub fn from_path(path: &Path) -> Result<Self> {
        let model = tract_onnx::onnx()
            .model_for_path(path)
            .map_err(|e| Error::new(format!("DEIM model_for_path failed: {e}")))?;
        Self::from_inference_model(model)
    }

    /// Load from an in-memory ONNX buffer (usable on WASM/IndexedDB).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let model = tract_onnx::onnx()
            .model_for_read(&mut cursor)
            .map_err(|e| Error::new(format!("DEIM model_for_read failed: {e}")))?;
        Self::from_inference_model(model)
    }

    fn from_inference_model(model: InferenceModel) -> Result<Self> {
        let input_width = 800usize;
        let input_height = 800usize;
        let plan = model
            .with_input_fact(
                0,
                InferenceFact::dt_shape(f32::datum_type(), tvec!(1, 3, input_height, input_width)),
            )
            .map_err(|e| Error::new(format!("DEIM input[0] fact failed: {e}")))?
            .with_input_fact(
                1,
                InferenceFact::dt_shape(i64::datum_type(), tvec!(1, 2)),
            )
            .map_err(|e| Error::new(format!("DEIM input[1] fact failed: {e}")))?
            .into_optimized()
            .map_err(|e| Error::new(format!("DEIM into_optimized failed: {e}")))?
            .into_runnable()
            .map_err(|e| Error::new(format!("DEIM into_runnable failed: {e}")))?;

        Ok(Self {
            plan,
            input_width,
            input_height,
            conf_threshold: 0.25,
            overlap_threshold: 0.9,
        })
    }

    pub fn with_conf_threshold(mut self, threshold: f32) -> Self {
        self.conf_threshold = threshold;
        self
    }

    /// Run layout and text-line detection on an RGB image.
    pub fn detect(&self, img: &RgbImage) -> Result<Vec<Detection>> {
        let (orig_w, orig_h) = img.dimensions();
        if orig_w == 0 || orig_h == 0 {
            return Ok(Vec::new());
        }
        let max_wh = orig_w.max(orig_h);

        let mut padded = RgbImage::new(max_wh, max_wh);
        image::imageops::overlay(&mut padded, img, 0, 0);
        let resized = image::imageops::resize(
            &padded,
            self.input_width as u32,
            self.input_height as u32,
            FilterType::Triangle,
        );

        let mean = [0.485f32, 0.456, 0.406];
        let inv_std = [1.0 / 0.229f32, 1.0 / 0.224, 1.0 / 0.225];
        let mut input = tract_ndarray::Array4::<f32>::zeros((1, 3, self.input_height, self.input_width));
        for (x, y, px) in resized.enumerate_pixels() {
            let xu = x as usize;
            let yu = y as usize;
            input[[0, 0, yu, xu]] = (px[0] as f32 / 255.0 - mean[0]) * inv_std[0];
            input[[0, 1, yu, xu]] = (px[1] as f32 / 255.0 - mean[1]) * inv_std[1];
            input[[0, 2, yu, xu]] = (px[2] as f32 / 255.0 - mean[2]) * inv_std[2];
        }

        let size_tensor =
            tract_ndarray::arr2(&[[self.input_height as i64, self.input_width as i64]]).into_tensor();
        let outputs = self
            .plan
            .run(tvec!(input.into_tensor().into(), size_tensor.into()))
            .map_err(|e| Error::new(format!("DEIM inference failed: {e}")))?;

        if outputs.len() < 3 {
            return Err(Error::new(format!(
                "DEIM expected at least 3 outputs, got {}",
                outputs.len()
            )));
        }

        let class_ids_cow = outputs[0]
            .cast_to::<i64>()
            .map_err(|e| Error::new(format!("DEIM class_ids cast failed: {e}")))?;
        let class_ids_plain = class_ids_cow
            .try_as_plain_ram()
            .map_err(|e| Error::new(format!("DEIM class_ids plain failed: {e}")))?;
        let class_ids = class_ids_plain
            .as_slice::<i64>()
            .map_err(|e| Error::new(format!("DEIM class_ids slice failed: {e}")))?;

        let bboxes_plain = outputs[1]
            .try_as_plain_ram()
            .map_err(|e| Error::new(format!("DEIM bboxes plain failed: {e}")))?;
        let bboxes = bboxes_plain
            .as_slice::<f32>()
            .map_err(|e| Error::new(format!("DEIM bboxes slice failed: {e}")))?;

        let scores_plain = outputs[2]
            .try_as_plain_ram()
            .map_err(|e| Error::new(format!("DEIM scores plain failed: {e}")))?;
        let scores = scores_plain
            .as_slice::<f32>()
            .map_err(|e| Error::new(format!("DEIM scores slice failed: {e}")))?;

        let char_counts_cow = if outputs.len() >= 4 {
            Some(
                outputs[3]
                    .cast_to::<f32>()
                    .map_err(|e| Error::new(format!("DEIM char_counts cast failed: {e}")))?,
            )
        } else {
            None
        };
        let char_counts_plain = match char_counts_cow.as_ref() {
            Some(c) => Some(
                c.try_as_plain_ram()
                    .map_err(|e| Error::new(format!("DEIM char_counts plain failed: {e}")))?,
            ),
            None => None,
        };
        let char_counts = match char_counts_plain.as_ref() {
            Some(p) => Some(
                p.as_slice::<f32>()
                    .map_err(|e| Error::new(format!("DEIM char_counts slice failed: {e}")))?,
            ),
            None => None,
        };

        let scale_x = max_wh as f32 / self.input_width as f32;
        let scale_y = max_wh as f32 / self.input_height as f32;

        let mut detections = Vec::new();
        for (i, &score) in scores.iter().enumerate() {
            if score <= self.conf_threshold {
                continue;
            }
            let raw_label = class_ids.get(i).copied().unwrap_or(0);
            if raw_label <= 0 {
                continue;
            }
            let class_index = (raw_label - 1) as usize;
            let Some(&class_name) = NDL_CLASSES.get(class_index) else {
                continue;
            };
            let base = i * 4;
            if base + 3 >= bboxes.len() {
                break;
            }
            let x1 = (bboxes[base] * scale_x).clamp(0.0, orig_w as f32) as u32;
            let y1 = (bboxes[base + 1] * scale_y).clamp(0.0, orig_h as f32) as u32;
            let x2 = (bboxes[base + 2] * scale_x).clamp(0.0, orig_w as f32) as u32;
            let y2 = (bboxes[base + 3] * scale_y).clamp(0.0, orig_h as f32) as u32;
            if x2 <= x1 || y2 <= y1 {
                continue;
            }
            let pred_char_count = char_counts
                .and_then(|c| c.get(i).copied())
                .unwrap_or(100.0);

            detections.push(Detection {
                class_index,
                class_name,
                confidence: score,
                box_xyxy: [x1, y1, x2, y2],
                pred_char_count,
            });
        }

        Ok(suppress_overlapping_regions(detections, self.overlap_threshold))
    }
}

/// Prefer larger same-label boxes when `intersection / smaller_area >= overlap_threshold`.
fn suppress_overlapping_regions(detections: Vec<Detection>, overlap_threshold: f32) -> Vec<Detection> {
    if overlap_threshold <= 0.0 || detections.len() <= 1 {
        return detections;
    }

    let areas: Vec<u64> = detections
        .iter()
        .map(|d| {
            let w = u64::from(d.box_xyxy[2].saturating_sub(d.box_xyxy[0]));
            let h = u64::from(d.box_xyxy[3].saturating_sub(d.box_xyxy[1]));
            w * h
        })
        .collect();

    let mut order: Vec<usize> = (0..detections.len()).collect();
    order.sort_by(|&a, &b| areas[b].cmp(&areas[a]));

    let mut kept_by_class: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut suppressed = vec![false; detections.len()];

    for idx in order {
        let area = areas[idx];
        if area == 0 {
            continue;
        }
        let [x1, y1, x2, y2] = detections[idx].box_xyxy;
        let kept = kept_by_class.entry(detections[idx].class_index).or_default();
        let mut drop_it = false;
        for &other in kept.iter() {
            if areas[other] <= area {
                continue;
            }
            let [ox1, oy1, ox2, oy2] = detections[other].box_xyxy;
            let iw = ox2.min(x2).saturating_sub(ox1.max(x1)) as u64;
            let ih = oy2.min(y2).saturating_sub(oy1.max(y1)) as u64;
            let inter = iw * ih;
            if (inter as f64 / area as f64) >= f64::from(overlap_threshold) {
                drop_it = true;
                break;
            }
        }
        if drop_it {
            suppressed[idx] = true;
        } else {
            kept.push(idx);
        }
    }

    detections
        .into_iter()
        .enumerate()
        .filter_map(|(i, d)| (!suppressed[i]).then_some(d))
        .collect()
}
