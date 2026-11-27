// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! High-level NDLOCR-Lite pipeline orchestrating DEIMv2, PARSeq, and reading-order sorting.

use std::path::{Path, PathBuf};

use image::{DynamicImage, GenericImageView};

use crate::deim::{is_text_line_class, DeimDetector};
use crate::models::{spec_for, verify_sha256_file, ModelKind};
use crate::parseq::ParseqRecognizer;
use crate::reading_order::sort_reading_order;
use crate::{OcrLine, OcrPage, Result};

/// High-level NDLOCR-Lite engine.
///
/// `rec_100` (41 MB, slow graph optimization) is loaded lazily only when a
/// detected line exceeds 50 predicted characters.
pub struct NdlOcrEngine {
    model_dir: Option<PathBuf>,
    det: DeimDetector,
    rec_30: ParseqRecognizer,
    rec_50: Option<ParseqRecognizer>,
    rec_100: Option<ParseqRecognizer>,
}

impl NdlOcrEngine {
    /// Load core models from `dir` after verifying their SHA-256 checksums.
    ///
    /// Loads `Det` and `Rec30` immediately; `Rec50` and `Rec100` are loaded on
    /// demand when longer lines are encountered.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let det_spec = spec_for(ModelKind::Det);
        let rec30_spec = spec_for(ModelKind::Rec30);

        let det_path = dir.join(det_spec.filename);
        let rec30_path = dir.join(rec30_spec.filename);

        verify_sha256_file(&det_path, det_spec.sha256)?;
        verify_sha256_file(&rec30_path, rec30_spec.sha256)?;

        let det = DeimDetector::from_path(&det_path)?;
        let rec_30 = ParseqRecognizer::from_path(&rec30_path, 256)?;

        Ok(Self {
            model_dir: Some(dir.to_path_buf()),
            det,
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
            det,
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

        let mut lines = Vec::with_capacity(order.len());
        for (rank, &idx) in order.iter().enumerate() {
            let det = &line_dets[idx];
            let [x1, y1, x2, y2] = det.box_xyxy;
            let lw = x2.saturating_sub(x1);
            let lh = y2.saturating_sub(y1);
            let is_vertical = (lh as f32) > (lw as f32 * 0.8);

            let crop = rgb.view(x1, y1, lw, lh).to_image();
            let text = self.recognize_line(&crop, det.pred_char_count)?;

            lines.push(OcrLine {
                box_xyxy: det.box_xyxy,
                class_index: det.class_index,
                class_name: det.class_name,
                confidence: det.confidence,
                pred_char_count: det.pred_char_count,
                is_vertical,
                text,
                order: rank,
            });
        }

        Ok(OcrPage {
            width,
            height,
            lines,
        })
    }

    fn recognize_line(&mut self, crop: &image::RgbImage, pred_char_count: f32) -> Result<String> {
        if pred_char_count > 50.0 {
            if let Some(rec) = self.ensure_rec_100()? {
                return rec.read(crop);
            }
        }
        if pred_char_count > 30.0 {
            if let Some(rec) = self.ensure_rec_50()? {
                return rec.read(crop);
            }
        }
        self.rec_30.read(crop)
    }

    fn ensure_rec_50(&mut self) -> Result<Option<&ParseqRecognizer>> {
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
        Ok(self.rec_50.as_ref())
    }

    fn ensure_rec_100(&mut self) -> Result<Option<&ParseqRecognizer>> {
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
        Ok(self.rec_100.as_ref())
    }
}
