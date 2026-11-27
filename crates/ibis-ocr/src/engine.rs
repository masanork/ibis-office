// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! High-level NDLOCR-Lite pipeline orchestrating DEIMv2 (CPU or `burn-wgpu` GPU),
//! parallel PARSeq text recognition, and reading-order sorting.

use std::path::{Path, PathBuf};

use image::{DynamicImage, GenericImageView};
use rayon::prelude::*;

use crate::deim::{is_text_line_class, DeimDetector, Detection};
use crate::models::{spec_for, verify_sha256_file, ModelKind};
use crate::parseq::ParseqRecognizer;
use crate::reading_order::sort_reading_order;
use crate::{OcrLine, OcrPage, Result};

enum DetBackend {
    Tract(DeimDetector),
    #[cfg(feature = "gpu-wgpu")]
    Burn(crate::deim_burn::BurnDeimDetector),
}

impl DetBackend {
    fn detect(&self, img: &image::RgbImage) -> Result<Vec<Detection>> {
        match self {
            Self::Tract(det) => det.detect(img),
            #[cfg(feature = "gpu-wgpu")]
            Self::Burn(det) => det.detect(img),
        }
    }
}

/// High-level NDLOCR-Lite engine.
///
/// * When compiled with `gpu-wgpu` and `deim-s-1024x1024-261124.bpk` is present
///   in `model_dir`, DEIMv2 runs on the GPU via `burn-wgpu`. Otherwise it falls
///   back to `tract-onnx` CPU inference.
/// * `rec_50` and `rec_100` are loaded lazily only when longer lines appear.
/// * Cropped text lines are recognized in parallel using `rayon`.
pub struct NdlOcrEngine {
    model_dir: Option<PathBuf>,
    det: DetBackend,
    rec_30: ParseqRecognizer,
    rec_50: Option<ParseqRecognizer>,
    rec_100: Option<ParseqRecognizer>,
}

impl NdlOcrEngine {
    /// Load core models from `dir` after verifying their SHA-256 checksums.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let det_spec = spec_for(ModelKind::Det);
        let rec30_spec = spec_for(ModelKind::Rec30);

        let rec30_path = dir.join(rec30_spec.filename);
        verify_sha256_file(&rec30_path, rec30_spec.sha256)?;
        let rec_30 = ParseqRecognizer::from_path(&rec30_path, 256)?;

        #[cfg(feature = "gpu-wgpu")]
        {
            let bpk_path = dir.join(crate::deim_burn::DEIM_BPK_FILENAME);
            if bpk_path.is_file() {
                if let Ok(burn_det) = crate::deim_burn::BurnDeimDetector::from_bpk_file(&bpk_path) {
                    return Ok(Self {
                        model_dir: Some(dir.to_path_buf()),
                        det: DetBackend::Burn(burn_det),
                        rec_30,
                        rec_50: None,
                        rec_100: None,
                    });
                }
            }
        }

        let det_path = dir.join(det_spec.filename);
        verify_sha256_file(&det_path, det_spec.sha256)?;
        let det = DeimDetector::from_path(&det_path)?;

        Ok(Self {
            model_dir: Some(dir.to_path_buf()),
            det: DetBackend::Tract(det),
            rec_30,
            rec_50: None,
            rec_100: None,
        })
    }

    /// Construct from pre-loaded in-memory models (e.g. from IndexedDB on WASM).
    pub fn from_models(
        det: DeimDetector,
        rec_30: ParseqRecognizer,
        rec_50: Option<ParseqRecognizer>,
        rec_100: Option<ParseqRecognizer>,
    ) -> Self {
        Self {
            model_dir: None,
            det: DetBackend::Tract(det),
            rec_30,
            rec_50,
            rec_100,
        }
    }

    /// Construct using a `burn-wgpu` GPU DEIMv2 detector and `tract` PARSeq recognizers.
    #[cfg(feature = "gpu-wgpu")]
    pub fn from_burn_models(
        det: crate::deim_burn::BurnDeimDetector,
        rec_30: ParseqRecognizer,
        rec_50: Option<ParseqRecognizer>,
        rec_100: Option<ParseqRecognizer>,
    ) -> Self {
        Self {
            model_dir: None,
            det: DetBackend::Burn(det),
            rec_30,
            rec_50,
            rec_100,
        }
    }

    /// Recognize all text lines on `image` and return them sorted in reading order.
    pub fn recognize_image(&mut self, image: &DynamicImage) -> Result<OcrPage> {
        let rgb = image.to_rgb8();
        let (width, height) = rgb.dimensions();

        let detections = self.det.detect(&rgb)?;
        let line_dets: Vec<_> = detections
            .into_iter()
            .filter(|d| is_text_line_class(d.class_index))
            .collect();

        let boxes: Vec<[u32; 4]> = line_dets.iter().map(|d| d.box_xyxy).collect();
        let order = sort_reading_order(&boxes);

        // Pre-load optional larger PARSeq models once before parallel recognition.
        let needs_50 = line_dets.iter().any(|d| d.pred_char_count > 30.0);
        let needs_100 = line_dets.iter().any(|d| d.pred_char_count > 50.0);
        if needs_50 {
            self.ensure_rec_50()?;
        }
        if needs_100 {
            self.ensure_rec_100()?;
        }

        let rec_30 = &self.rec_30;
        let rec_50 = self.rec_50.as_ref();
        let rec_100 = self.rec_100.as_ref();

        let lines: Result<Vec<OcrLine>> = order
            .par_iter()
            .enumerate()
            .map(|(rank, &idx)| {
                let det = &line_dets[idx];
                let [x1, y1, x2, y2] = det.box_xyxy;
                let lw = x2.saturating_sub(x1);
                let lh = y2.saturating_sub(y1);
                let is_vertical = (lh as f32) > (lw as f32 * 0.8);

                let crop = rgb.view(x1, y1, lw, lh).to_image();
                let recognizer = if det.pred_char_count > 50.0 {
                    rec_100.or(rec_50).unwrap_or(rec_30)
                } else if det.pred_char_count > 30.0 {
                    rec_50.unwrap_or(rec_30)
                } else {
                    rec_30
                };
                let text = recognizer.read(&crop)?;

                Ok(OcrLine {
                    box_xyxy: det.box_xyxy,
                    class_index: det.class_index,
                    class_name: det.class_name,
                    confidence: det.confidence,
                    pred_char_count: det.pred_char_count,
                    is_vertical,
                    text,
                    order: rank,
                })
            })
            .collect();

        Ok(OcrPage {
            width,
            height,
            lines: lines?,
        })
    }

    fn ensure_rec_50(&mut self) -> Result<()> {
        if self.rec_50.is_none() {
            if let Some(dir) = &self.model_dir {
                let spec = spec_for(ModelKind::Rec50);
                let path = dir.join(spec.filename);
                if path.is_file() {
                    verify_sha256_file(&path, spec.sha256)?;
                    self.rec_50 = Some(ParseqRecognizer::from_path(&path, 384)?);
                }
            }
        }
        Ok(())
    }

    fn ensure_rec_100(&mut self) -> Result<()> {
        if self.rec_100.is_none() {
            if let Some(dir) = &self.model_dir {
                let spec = spec_for(ModelKind::Rec100);
                let path = dir.join(spec.filename);
                if path.is_file() {
                    verify_sha256_file(&path, spec.sha256)?;
                    self.rec_100 = Some(ParseqRecognizer::from_path(&path, 768)?);
                }
            }
        }
        Ok(())
    }
}
