// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust 80-channel Log-Mel spectrogram matching OpenAI Whisper.

use std::f32::consts::PI;

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

pub const SAMPLE_RATE: usize = 16_000;
pub const N_FFT: usize = 400;
pub const HOP_LENGTH: usize = 160;
pub const N_MELS: usize = 80;
pub const CHUNK_SAMPLES: usize = 30 * SAMPLE_RATE; // 480,000
pub const N_FRAMES: usize = 3000;

/// Compute `[N_MELS, N_FRAMES]` (`[80, 3000]`) row-major Log-Mel spectrogram
/// for a 16 kHz mono chunk (padded or truncated to 30 seconds).
pub fn log_mel_spectrogram(samples: &[f32]) -> Vec<f32> {
    // Pad or truncate to 30 seconds, plus reflect/zero pad N_FFT/2 on both sides.
    let pad = N_FFT / 2;
    let mut padded = vec![0.0f32; CHUNK_SAMPLES + 2 * pad];
    let copy_len = samples.len().min(CHUNK_SAMPLES);
    padded[pad..pad + copy_len].copy_from_slice(&samples[..copy_len]);

    let hann: Vec<f32> = (0..N_FFT)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / N_FFT as f32).cos()))
        .collect();

    let filters = mel_filterbank(SAMPLE_RATE as f32, N_FFT, N_MELS);
    let n_freqs = N_FFT / 2 + 1;

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);
    let mut buf = vec![Complex::new(0.0f32, 0.0f32); N_FFT];
    let mut power = vec![0.0f32; n_freqs];

    let mut mel = vec![0.0f32; N_MELS * N_FRAMES];
    let mut max_val = f32::NEG_INFINITY;

    for frame in 0..N_FRAMES {
        let start = frame * HOP_LENGTH;
        for i in 0..N_FFT {
            let s = padded.get(start + i).copied().unwrap_or(0.0);
            buf[i] = Complex::new(s * hann[i], 0.0);
        }
        fft.process(&mut buf);

        for (k, p) in power.iter_mut().enumerate() {
            *p = buf[k].norm_sqr();
        }

        for m in 0..N_MELS {
            let row = &filters[m * n_freqs..(m + 1) * n_freqs];
            let mut sum = 0.0f32;
            for k in 0..n_freqs {
                sum += row[k] * power[k];
            }
            let log_spec = sum.max(1e-10).log10();
            if log_spec > max_val {
                max_val = log_spec;
            }
            mel[m * N_FRAMES + frame] = log_spec;
        }
    }

    let clamp_min = max_val - 8.0;
    for v in &mut mel {
        *v = ((*v).max(clamp_min) + 4.0) / 4.0;
    }
    mel
}

/// Slaney-style Mel filterbank matching librosa / OpenAI Whisper.
fn mel_filterbank(sr: f32, n_fft: usize, n_mels: usize) -> Vec<f32> {
    let n_freqs = n_fft / 2 + 1;
    let f_min = 0.0f32;
    let f_max = sr / 2.0;

    let mel_min = hz_to_mel_slaney(f_min);
    let mel_max = hz_to_mel_slaney(f_max);

    let mut mel_pts = Vec::with_capacity(n_mels + 2);
    for i in 0..(n_mels + 2) {
        let m = mel_min + (mel_max - mel_min) * (i as f32 / (n_mels + 1) as f32);
        mel_pts.push(mel_to_hz_slaney(m));
    }

    let fft_freqs: Vec<f32> = (0..n_freqs)
        .map(|k| k as f32 * (sr / n_fft as f32))
        .collect();

    let mut weights = vec![0.0f32; n_mels * n_freqs];
    for m in 0..n_mels {
        let f_left = mel_pts[m];
        let f_center = mel_pts[m + 1];
        let f_right = mel_pts[m + 2];
        let enorm = 2.0 / (f_right - f_left).max(1e-6);
        for (k, &f) in fft_freqs.iter().enumerate() {
            let lower = (f - f_left) / (f_center - f_left).max(1e-6);
            let upper = (f_right - f) / (f_right - f_center).max(1e-6);
            let w = lower.min(upper).max(0.0) * enorm;
            weights[m * n_freqs + k] = w;
        }
    }
    weights
}

fn hz_to_mel_slaney(freq: f32) -> f32 {
    const F_SP: f32 = 200.0 / 3.0;
    const MIN_LOG_HZ: f32 = 1000.0;
    const MIN_LOG_MEL: f32 = MIN_LOG_HZ / F_SP;
    let logstep = (6.4f32).ln() / 27.0;
    if freq >= MIN_LOG_HZ {
        MIN_LOG_MEL + (freq / MIN_LOG_HZ).ln() / logstep
    } else {
        freq / F_SP
    }
}

fn mel_to_hz_slaney(mel: f32) -> f32 {
    const F_SP: f32 = 200.0 / 3.0;
    const MIN_LOG_HZ: f32 = 1000.0;
    const MIN_LOG_MEL: f32 = MIN_LOG_HZ / F_SP;
    let logstep = (6.4f32).ln() / 27.0;
    if mel >= MIN_LOG_MEL {
        MIN_LOG_HZ * (logstep * (mel - MIN_LOG_MEL)).exp()
    } else {
        F_SP * mel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mel_shape_and_finite() {
        let samples = vec![0.0f32; SAMPLE_RATE];
        let mel = log_mel_spectrogram(&samples);
        assert_eq!(mel.len(), N_MELS * N_FRAMES);
        assert!(mel.iter().all(|v| v.is_finite()));
    }
}
