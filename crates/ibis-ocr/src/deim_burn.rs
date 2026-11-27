// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! GPU-accelerated DEIMv2 layout and text-line detector using `burn` (`wgpu` backend).

use std::path::Path;

use burn::prelude::*;
use burn::tensor::{Bytes, TensorData};
use image::RgbImage;

use crate::deim::{postprocess_deim, preprocess_deim_chw, Detection};
use crate::{Error, Result};

/// BurnPack filename corresponding to `deim-s-1024x1024-261124.onnx`.
pub const DEIM_BPK_FILENAME: &str = "deim-s-1024x1024-261124.bpk";

/// GPU-accelerated DEIMv2 detector backed by `burn` + `wgpu`.
pub struct BurnDeimDetector {
    model: crate::deim_burn_model::Model,
    device: Device,
    input_width: usize,
    input_height: usize,
    conf_threshold: f32,
    overlap_threshold: f32,
}

impl BurnDeimDetector {
    /// Load from a `.bpk` (BurnPack) weights file on disk.
    pub fn from_bpk_file(path: &Path) -> Result<Self> {
        let device = Device::default();
        let model = crate::deim_burn_model::Model::from_file(path, &device);
        Ok(Self {
            model,
            device,
            input_width: 800,
            input_height: 800,
            conf_threshold: 0.25,
            overlap_threshold: 0.9,
        })
    }

    /// Load from in-memory `.bpk` bytes (usable on WASM/WebGPU).
    pub fn from_bpk_bytes(bytes: &[u8]) -> Result<Self> {
        let device = Device::default();
        let model = crate::deim_burn_model::Model::from_bytes(
            Bytes::from_bytes_vec(bytes.to_vec()),
            &device,
        );
        Ok(Self {
            model,
            device,
            input_width: 800,
            input_height: 800,
            conf_threshold: 0.25,
            overlap_threshold: 0.9,
        })
    }

    pub fn with_conf_threshold(mut self, threshold: f32) -> Self {
        self.conf_threshold = threshold;
        self
    }

    /// Run GPU layout and text-line detection on an RGB image.
    pub fn detect(&self, img: &RgbImage) -> Result<Vec<Detection>> {
        let (orig_w, orig_h) = img.dimensions();
        if orig_w == 0 || orig_h == 0 {
            return Ok(Vec::new());
        }
        let (chw, max_wh) = preprocess_deim_chw(img, self.input_width, self.input_height);

        let images = Tensor::<4>::from_data(
            TensorData::new(chw, [1, 3, self.input_height, self.input_width]),
            &self.device,
        );
        let orig_target_sizes = Tensor::<2, Int>::from_data(
            TensorData::new(
                vec![self.input_height as i64, self.input_width as i64],
                [1, 2],
            ),
            &self.device,
        );

        let (class_ids_t, bboxes_t, scores_t, char_counts_t) =
            self.model.forward(images, orig_target_sizes);

        let class_ids = class_ids_t
            .into_data()
            .convert::<i64>()
            .try_to_vec::<i64>()
            .map_err(|e| Error::new(format!("Burn DEIM class_ids read failed: {e:?}")))?;
        let bboxes = bboxes_t
            .into_data()
            .convert::<f32>()
            .try_to_vec::<f32>()
            .map_err(|e| Error::new(format!("Burn DEIM bboxes read failed: {e:?}")))?;
        let scores = scores_t
            .into_data()
            .convert::<f32>()
            .try_to_vec::<f32>()
            .map_err(|e| Error::new(format!("Burn DEIM scores read failed: {e:?}")))?;
        let char_counts = char_counts_t
            .into_data()
            .convert::<f32>()
            .try_to_vec::<f32>()
            .map_err(|e| Error::new(format!("Burn DEIM char_counts read failed: {e:?}")))?;

        Ok(postprocess_deim(
            orig_w,
            orig_h,
            max_wh,
            self.input_width,
            self.input_height,
            self.conf_threshold,
            self.overlap_threshold,
            &class_ids,
            &bboxes,
            &scores,
            Some(&char_counts),
        ))
    }
}
