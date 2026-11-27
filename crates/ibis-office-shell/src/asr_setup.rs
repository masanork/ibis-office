// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Optional Whisper-tiny speech-to-text model download and cache management
//! (modeled on [`super::ocr_setup`]).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use ibis_asr::models::{
    MODEL_LICENSE_URL, ModelSpec, WHISPER_TINY_MODELS, missing_models as missing_in_dir,
    verify_sha256_file,
};

use super::font_setup::DownloadError;

const WHISPER_RAW_LICENSE_URL: &str =
    "https://huggingface.co/openai/whisper-tiny/raw/main/LICENSE";

/// Directory where Whisper-tiny ONNX models are stored.
#[cfg(not(target_family = "wasm"))]
pub fn models_dir() -> Option<PathBuf> {
    ibis_office_base::app_data::asr_models_dir()
}

/// Returns the models from [`WHISPER_TINY_MODELS`] not yet present in [`models_dir`].
#[cfg(not(target_family = "wasm"))]
pub fn missing_models() -> Vec<&'static ModelSpec> {
    let Some(dir) = models_dir() else {
        return WHISPER_TINY_MODELS.iter().collect();
    };
    missing_in_dir(&dir)
}

/// Returns `true` when all Whisper-tiny models and tokenizer are present in [`models_dir`].
#[cfg(not(target_family = "wasm"))]
pub fn are_models_installed() -> bool {
    missing_models().is_empty()
}

/// Approximate total size in bytes of all [`WHISPER_TINY_MODELS`].
pub fn total_model_bytes() -> u64 {
    WHISPER_TINY_MODELS.iter().map(|m| m.approx_size_bytes).sum()
}

/// License reference URL shown to the user before downloading.
pub fn license_url() -> &'static str {
    MODEL_LICENSE_URL
}

/// Download all missing Whisper-tiny models into [`models_dir`].
#[cfg(not(target_family = "wasm"))]
pub fn download_missing(
    current_index: &AtomicUsize,
    progress_bytes: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    let dir = models_dir()
        .ok_or_else(|| DownloadError::Failed("no application data directory for ASR models".into()))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| DownloadError::Failed(format!("{}: {e}", dir.display())))?;

    download_license_if_missing(&dir, "LICENCE-WHISPER.txt", WHISPER_RAW_LICENSE_URL, cancel)?;

    let to_download = missing_in_dir(&dir);
    for (idx, spec) in to_download.iter().enumerate() {
        current_index.store(idx + 1, Ordering::SeqCst);
        progress_bytes.store(0, Ordering::SeqCst);

        let dest = dir.join(spec.filename);
        let part = dir.join(format!("{}.part", spec.filename));
        let url = model_url(spec);
        let result = super::font_setup::fetch_to_file(&url, &part, 1800, Some(progress_bytes), cancel);
        if let Err(e) = result {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        if let Err(e) = verify_sha256_file(&part, spec.sha256) {
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Failed(e.to_string()));
        }
        if let Err(e) = std::fs::rename(&part, &dest) {
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Failed(format!("{}: {e}", dest.display())));
        }
    }

    Ok(())
}

#[cfg(not(target_family = "wasm"))]
fn model_url(spec: &ModelSpec) -> String {
    if let Ok(base) = std::env::var("IBIS_ASR_MODEL_BASE_URL") {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            return format!("{}/{}", trimmed.trim_end_matches('/'), spec.filename);
        }
    }
    spec.url.to_string()
}

#[cfg(not(target_family = "wasm"))]
fn download_license_if_missing(
    dir: &std::path::Path,
    filename: &str,
    url: &str,
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    let dest = dir.join(filename);
    if dest.is_file() {
        return Ok(());
    }
    let part = dir.join(format!("{filename}.part"));
    let res = super::font_setup::fetch_to_file(url, &part, 60, None, cancel);
    match res {
        Ok(()) => {
            let _ = std::fs::rename(&part, &dest);
            Ok(())
        }
        Err(DownloadError::Cancelled) => {
            let _ = std::fs::remove_file(&part);
            Err(DownloadError::Cancelled)
        }
        Err(_) => {
            let _ = std::fs::remove_file(&part);
            Ok(())
        }
    }
}

/// Remove all downloaded Whisper-tiny models and licenses to free disk space.
#[cfg(not(target_family = "wasm"))]
pub fn remove_models() -> Result<(), String> {
    #[cfg(feature = "asr")]
    invalidate_cached_engine();
    let Some(dir) = models_dir() else {
        return Ok(());
    };
    if !dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))
}

#[cfg(all(not(target_family = "wasm"), feature = "asr"))]
static CACHED_ENGINE: std::sync::OnceLock<
    std::sync::Mutex<Option<(PathBuf, std::sync::Arc<ibis_asr::whisper::WhisperEngine>)>>,
> = std::sync::OnceLock::new();

#[cfg(all(not(target_family = "wasm"), feature = "asr"))]
fn engine_cache()
-> &'static std::sync::Mutex<Option<(PathBuf, std::sync::Arc<ibis_asr::whisper::WhisperEngine>)>> {
    CACHED_ENGINE.get_or_init(|| std::sync::Mutex::new(None))
}

/// Drops any cached [`ibis_asr::whisper::WhisperEngine`] instance (e.g. after removing models).
#[cfg(all(not(target_family = "wasm"), feature = "asr"))]
pub fn invalidate_cached_engine() {
    if let Ok(mut guard) = engine_cache().lock() {
        *guard = None;
    }
}

/// Loads (or returns the cached) Whisper-tiny engine from [`models_dir`].
#[cfg(all(not(target_family = "wasm"), feature = "asr"))]
pub fn load_engine() -> Result<std::sync::Arc<ibis_asr::whisper::WhisperEngine>, String> {
    let dir = models_dir().ok_or_else(|| "no application data directory for ASR models".to_string())?;
    if !missing_in_dir(&dir).is_empty() {
        return Err("Whisper-tiny models are not installed. Open Help > Speech Recognition Setup to download them.".to_string());
    }
    {
        let guard = engine_cache().lock().map_err(|_| "ASR engine cache poisoned".to_string())?;
        if let Some((cached_dir, engine)) = guard.as_ref()
            && cached_dir == &dir
        {
            return Ok(std::sync::Arc::clone(engine));
        }
    }
    let engine = std::sync::Arc::new(
        ibis_asr::whisper::WhisperEngine::from_dir(&dir).map_err(|e| e.to_string())?,
    );
    if let Ok(mut guard) = engine_cache().lock() {
        *guard = Some((dir, std::sync::Arc::clone(&engine)));
    }
    Ok(engine)
}

/// Transcribes encoded audio bytes (WAV, MP3, AAC/M4A, Ogg/Vorbis, FLAC) using the cached Whisper-tiny engine.
#[cfg(all(not(target_family = "wasm"), feature = "asr"))]
pub fn transcribe_audio_bytes(
    bytes: &[u8],
    ext_hint: Option<&str>,
    language: Option<&str>,
) -> Result<ibis_asr::Transcript, String> {
    let engine = load_engine()?;
    engine
        .transcribe_audio_bytes(bytes, ext_hint, language)
        .map_err(|e| e.to_string())
}

/// Reads an audio file from disk and transcribes it using the cached Whisper-tiny engine.
#[cfg(all(not(target_family = "wasm"), feature = "asr"))]
pub fn transcribe_audio_file(
    path: &std::path::Path,
    language: Option<&str>,
) -> Result<ibis_asr::Transcript, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ext = path.extension().and_then(|s| s.to_str());
    transcribe_audio_bytes(&bytes, ext, language)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_bytes_is_around_154mb() {
        let total = total_model_bytes();
        assert!(total > 140_000_000 && total < 170_000_000);
    }

    #[test]
    fn missing_models_reports_all_when_dir_empty() {
        let tmp = std::env::temp_dir().join("ibis-asr-setup-test-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert_eq!(missing_in_dir(&tmp).len(), 3);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
