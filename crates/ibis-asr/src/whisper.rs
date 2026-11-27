// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust Whisper ONNX inference engine over `tract-onnx`.

use std::path::Path;

use tract_onnx::prelude::*;

use crate::audio::decode_audio_to_16k_mono;
use crate::mel::{log_mel_spectrogram, CHUNK_SAMPLES, N_FRAMES, N_MELS, SAMPLE_RATE};
use crate::models::{spec_for, verify_sha256_file, ModelKind};
use crate::tokenizer::WhisperTokenizer;
use crate::{Error, Result, Transcript, TranscriptSegment};

type RunnablePlan = std::sync::Arc<TypedRunnableModel>;

const MAX_DECODE_TOKENS: usize = 224;
const SILENCE_RMS_THRESHOLD: f32 = 0.002;

/// Pure-Rust Whisper ASR engine (`whisper-tiny` ONNX + `tokenizer.json`).
pub struct WhisperEngine {
    encoder: RunnablePlan,
    decoder_model: InferenceModel,
    tokenizer: WhisperTokenizer,
}

impl WhisperEngine {
    /// Load encoder, decoder, and tokenizer from `dir` after verifying SHA-256.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let enc_spec = spec_for(ModelKind::Encoder);
        let dec_spec = spec_for(ModelKind::Decoder);
        let tok_spec = spec_for(ModelKind::Tokenizer);

        let enc_path = dir.join(enc_spec.filename);
        let dec_path = dir.join(dec_spec.filename);
        let tok_path = dir.join(tok_spec.filename);

        verify_sha256_file(&enc_path, enc_spec.sha256)?;
        verify_sha256_file(&dec_path, dec_spec.sha256)?;
        verify_sha256_file(&tok_path, tok_spec.sha256)?;

        let encoder = tract_onnx::onnx()
            .model_for_path(&enc_path)
            .map_err(|e| Error::new(format!("Whisper encoder parse failed: {e}")))?
            .with_input_fact(
                0,
                InferenceFact::dt_shape(f32::datum_type(), tvec!(1, N_MELS, N_FRAMES)),
            )
            .map_err(|e| Error::new(format!("Whisper encoder input fact failed: {e}")))?
            .into_optimized()
            .map_err(|e| Error::new(format!("Whisper encoder optimize failed: {e}")))?
            .into_runnable()
            .map_err(|e| Error::new(format!("Whisper encoder runnable failed: {e}")))?;

        let decoder_model = tract_onnx::onnx()
            .model_for_path(&dec_path)
            .map_err(|e| Error::new(format!("Whisper decoder parse failed: {e}")))?;

        let tokenizer = WhisperTokenizer::from_file(&tok_path)?;

        Ok(Self {
            encoder,
            decoder_model,
            tokenizer,
        })
    }

    /// Decode and transcribe an audio file in memory (WAV, MP3, AAC/M4A, FLAC, OGG).
    pub fn transcribe_audio_bytes(
        &self,
        bytes: &[u8],
        ext_hint: Option<&str>,
        language: Option<&str>,
    ) -> Result<Transcript> {
        let samples = decode_audio_to_16k_mono(bytes, ext_hint)?;
        self.transcribe_16k_mono(&samples, language)
    }

    /// Transcribe 16 kHz mono `f32` PCM samples.
    pub fn transcribe_16k_mono(
        &self,
        samples: &[f32],
        language: Option<&str>,
    ) -> Result<Transcript> {
        if samples.is_empty() {
            return Ok(Transcript {
                language: language.map(str::to_string),
                segments: Vec::new(),
                text: String::new(),
            });
        }

        let mut segments = Vec::new();
        let mut full_text = String::new();

        for (chunk_idx, chunk) in samples.chunks(CHUNK_SAMPLES).enumerate() {
            let start_secs = (chunk_idx * CHUNK_SAMPLES) as f32 / SAMPLE_RATE as f32;
            let end_secs = start_secs + (chunk.len() as f32 / SAMPLE_RATE as f32);

            if rms_energy(chunk) < SILENCE_RMS_THRESHOLD {
                continue;
            }

            let text = self.transcribe_chunk(chunk, language)?;
            let trimmed = text.trim().to_string();
            if !trimmed.is_empty() {
                if !full_text.is_empty() {
                    full_text.push(' ');
                }
                full_text.push_str(&trimmed);
                segments.push(TranscriptSegment {
                    start_secs,
                    end_secs,
                    text: trimmed,
                });
            }
        }

        Ok(Transcript {
            language: language.map(str::to_string),
            segments,
            text: full_text,
        })
    }

    fn transcribe_chunk(&self, chunk: &[f32], language: Option<&str>) -> Result<String> {
        let mel = log_mel_spectrogram(chunk);
        let mel_arr = tract_ndarray::Array3::from_shape_vec((1, N_MELS, N_FRAMES), mel)
            .map_err(|e| Error::new(format!("mel shape error: {e}")))?;

        let enc_out = self
            .encoder
            .run(tvec!(mel_arr.into_tensor().into()))
            .map_err(|e| Error::new(format!("Whisper encoder run failed: {e}")))?;
        let enc_hidden = enc_out[0]
            .to_plain_array_view::<f32>()
            .map_err(|e| Error::new(format!("Whisper encoder view failed: {e}")))?;
        let enc_shape = enc_hidden.shape().to_vec();
        let enc_data = enc_hidden.iter().copied().collect::<Vec<f32>>();

        let mut tokens: Vec<i64> = vec![self.tokenizer.sot_id() as i64];
        if let Some(lang) = language {
            let lang_tok = format!("<|{lang}|>");
            if let Some(id) = self.tokenizer.token_id(&lang_tok) {
                tokens.push(id as i64);
            }
        } else if let Some(ja_id) = self.tokenizer.token_id("<|ja|>") {
            tokens.push(ja_id as i64);
        }
        tokens.push(self.tokenizer.transcribe_id() as i64);
        tokens.push(self.tokenizer.no_timestamps_id() as i64);

        let eot = self.tokenizer.eot_id() as i64;
        let mut generated: Vec<u32> = Vec::new();

        for _ in 0..MAX_DECODE_TOKENS {
            let seq_len = tokens.len();
            let dec_plan = self
                .decoder_model
                .clone()
                .with_input_fact(
                    0,
                    InferenceFact::dt_shape(i64::datum_type(), tvec!(1, seq_len)),
                )
                .map_err(|e| Error::new(format!("Whisper decoder input[0] failed: {e}")))?
                .with_input_fact(
                    1,
                    InferenceFact::dt_shape(
                        f32::datum_type(),
                        tvec!(enc_shape[0], enc_shape[1], enc_shape[2]),
                    ),
                )
                .map_err(|e| Error::new(format!("Whisper decoder input[1] failed: {e}")))?
                .into_optimized()
                .map_err(|e| Error::new(format!("Whisper decoder optimize failed: {e}")))?
                .into_runnable()
                .map_err(|e| Error::new(format!("Whisper decoder runnable failed: {e}")))?;

            let ids_arr = tract_ndarray::Array2::from_shape_vec((1, seq_len), tokens.clone())
                .map_err(|e| Error::new(format!("decoder ids shape error: {e}")))?;
            let enc_arr = tract_ndarray::Array3::from_shape_vec(
                (enc_shape[0], enc_shape[1], enc_shape[2]),
                enc_data.clone(),
            )
            .map_err(|e| Error::new(format!("decoder enc shape error: {e}")))?;

            let dec_out = dec_plan
                .run(tvec!(ids_arr.into_tensor().into(), enc_arr.into_tensor().into()))
                .map_err(|e| Error::new(format!("Whisper decoder run failed: {e}")))?;
            let logits = dec_out[0]
                .to_plain_array_view::<f32>()
                .map_err(|e| Error::new(format!("Whisper decoder view failed: {e}")))?;

            let last_step = logits.shape()[1] - 1;
            let vocab = logits.shape()[2];
            let mut best_id = 0usize;
            let mut best_val = f32::NEG_INFINITY;
            for v in 0..vocab {
                let val = logits[[0, last_step, v]];
                if val > best_val {
                    best_val = val;
                    best_id = v;
                }
            }

            if best_id as i64 == eot {
                break;
            }
            tokens.push(best_id as i64);
            generated.push(best_id as u32);
        }

        Ok(self.tokenizer.decode(&generated))
    }
}

fn rms_energy(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}
