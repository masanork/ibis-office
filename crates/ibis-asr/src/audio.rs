// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust audio container decoding (`symphonia`) and 16 kHz mono resampling.

use std::io::Cursor;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::mel::SAMPLE_RATE;
use crate::{Error, Result};

/// Decode an audio file in memory (WAV, MP3, AAC/M4A, FLAC, OGG) into 16 kHz mono `f32` PCM.
pub fn decode_audio_to_16k_mono(bytes: &[u8], ext_hint: Option<&str>) -> Result<Vec<f32>> {
    let cursor = Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = ext_hint {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| Error::new(format!("audio probe failed: {e}")))?;

    let track = format
        .default_track(symphonia::core::formats::TrackType::Audio)
        .ok_or_else(|| Error::new("no audio track found"))?;
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| Error::new("track has no audio codec parameters"))?;
    let src_rate = audio_params
        .sample_rate
        .ok_or_else(|| Error::new("audio track has unknown sample rate"))?;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .map_err(|e| Error::new(format!("audio decoder init failed: {e}")))?;

    let mut mono = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(Error::new(format!("audio packet read failed: {e}"))),
        };

        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(Error::new(format!("audio decode failed: {e}"))),
        };

        let channels = decoded.spec().channels().count().max(1);
        let frames = decoded.frames();
        if frames == 0 {
            continue;
        }

        interleaved.clear();
        decoded.copy_to_vec_interleaved(&mut interleaved);

        for frame in interleaved.chunks_exact(channels) {
            let sum: f32 = frame.iter().sum();
            mono.push(sum / channels as f32);
        }
    }

    Ok(resample_linear(&mono, src_rate, SAMPLE_RATE as u32))
}

/// Convert raw signed 16-bit PCM mono samples to 16 kHz `f32` in `[-1.0, 1.0]`.
pub fn pcm_s16le_to_16k_mono(samples: &[i16], src_rate: u32) -> Vec<f32> {
    let floats: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();
    resample_linear(&floats, src_rate, SAMPLE_RATE as u32)
}

/// Simple linear resampler for speech recognition input.
pub fn resample_linear(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if input.is_empty() || src_rate == 0 || dst_rate == 0 || src_rate == dst_rate {
        return input.to_vec();
    }
    let ratio = f64::from(src_rate) / f64::from(dst_rate);
    let out_len = ((input.len() as f64) / ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let s0 = input.get(idx).copied().unwrap_or(0.0);
        let s1 = input.get(idx + 1).copied().unwrap_or(s0);
        out.push(s0 + (s1 - s0) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_8k_to_16k_doubles_length() {
        let src = vec![0.0f32, 1.0, 0.0, -1.0];
        let dst = resample_linear(&src, 8000, 16000);
        assert_eq!(dst.len(), 8);
    }
}
