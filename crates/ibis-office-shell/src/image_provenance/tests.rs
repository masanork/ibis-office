// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::*;
use c2pa::{Builder, SigningAlg};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};

const ROOT: &str = include_str!("../../tests/fixtures/c2pa/root.pem");
const SIGNER: &[u8] = include_bytes!("../../tests/fixtures/c2pa/signer.pem");
const KEY: &[u8] = include_bytes!("../../tests/fixtures/c2pa/signer-key.pem");
const COMMENT: &[u8] = b"synthetic C2PA hash binding test";

fn trusted_settings() -> Settings {
    let mut settings = offline_settings();
    settings.trust.anchors = Some(vec![TrustAnchor {
        trust_anchors: ROOT.into(),
        trust_uri: Some("urn:ibis-office:synthetic-test-only".into()),
        ..TrustAnchor::default()
    }]);
    settings
}

fn image_bytes(format: ImageFormat) -> Vec<u8> {
    let image = if format == ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(RgbImage::from_fn(16, 16, |x, y| {
            Rgb([(x * 16) as u8, (y * 16) as u8, 127])
        }))
    } else {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(16, 16, |x, y| {
            Rgba([(x * 16) as u8, (y * 16) as u8, 127, (x * 16) as u8])
        }))
    };
    let mut dest = Cursor::new(Vec::new());
    image.write_to(&mut dest, format).unwrap();
    let mut data = dest.into_inner();
    if format == ImageFormat::Jpeg {
        // A legal JPEG COM segment remains decodable after its text is changed.
        let mut comment = vec![0xff, 0xfe];
        comment.extend_from_slice(&((COMMENT.len() + 2) as u16).to_be_bytes());
        comment.extend_from_slice(COMMENT);
        data.splice(2..2, comment);
    }
    data
}

fn signed(original: &[u8], mime: &str, history: bool, remote: bool) -> Vec<u8> {
    let signer = c2pa::create_signer::from_keys(SIGNER, KEY, SigningAlg::Es256, None).unwrap();
    let context = C2paContext::new()
        .with_settings(trusted_settings())
        .unwrap();
    // Legacy v1 claims may omit actions; v2 requires a creation/opening action.
    let claim_version = if history { 2 } else { 1 };
    let mut builder = Builder::from_context(context)
        .with_definition(
            serde_json::json!({"title": "Synthetic image", "claim_version": claim_version}),
        )
        .unwrap();
    if history {
        builder
            .add_action(serde_json::json!({"action": "c2pa.created", "digitalSourceType": "http://cv.iptc.org/newscodes/digitalsourcetype/algorithmicMedia"}))
            .unwrap();
        builder.add_ingredient(serde_json::from_value::<c2pa::Ingredient>(serde_json::json!({
            "title": "Synthetic source", "format": "image/png",
            "relationship": "componentOf", "instance_id": "urn:uuid:00000000-0000-4000-8000-000000000001"
        })).unwrap());
    }
    if remote {
        builder.set_remote_url("https://127.0.0.1:9/must-not-fetch.c2pa");
        builder.set_no_embed(true);
    }
    let mut dest = Cursor::new(Vec::new());
    builder
        .sign(signer.as_ref(), mime, &mut Cursor::new(original), &mut dest)
        .unwrap();
    dest.into_inner()
}

#[test]
fn signed_jpeg_png_tiff_match_sdk_trust_state_and_preserve_pixels() {
    for (extension, format) in [
        ("jpg", ImageFormat::Jpeg),
        ("png", ImageFormat::Png),
        ("tiff", ImageFormat::Tiff),
    ] {
        let original = image_bytes(format);
        let bytes = signed(&original, mime_type(extension).unwrap(), true, false);
        let before = bytes.clone();
        let reader =
            read_original(mime_type(extension).unwrap(), &bytes, trusted_settings()).unwrap();
        assert_eq!(
            reader.validation_state(),
            ValidationState::Trusted,
            "{extension}: {}",
            reader.json()
        );
        let result = report(Ok(reader));
        assert_eq!(result.status, "view.provenance-trusted", "{result:?}");
        assert_eq!(result.signer.as_deref(), Some("Synthetic image signer"));
        assert_eq!(result.actions, ["c2pa.created"]);
        assert_eq!(result.ingredients, ["Synthetic source"]);
        let untrusted = inspect(extension, &bytes).unwrap();
        assert_eq!(
            untrusted.status, "view.provenance-untrusted",
            "{untrusted:?}"
        );
        assert_eq!(untrusted.signer, result.signer);
        assert_eq!(
            image::load_from_memory_with_format(&bytes, format)
                .unwrap()
                .into_rgba8(),
            image::load_from_memory_with_format(&original, format)
                .unwrap()
                .into_rgba8()
        );
        assert_eq!(bytes, before, "verification must not modify the source");
    }
}

#[test]
fn changed_original_is_a_hash_mismatch_even_with_a_trusted_signer() {
    let mut bytes = signed(&image_bytes(ImageFormat::Jpeg), "image/jpeg", true, false);
    let start = bytes
        .windows(COMMENT.len())
        .position(|window| window == COMMENT)
        .unwrap();
    bytes[start] ^= 1;
    assert!(image::load_from_memory(&bytes).is_ok());
    let reader = read_original("image/jpeg", &bytes, trusted_settings()).unwrap();
    assert_eq!(reader.validation_state(), ValidationState::Invalid);
    let result = report(Ok(reader));
    assert_eq!(result.status, "view.provenance-mismatch", "{result:?}");
    assert!(result.signer.is_none());
    assert!(result.actions.is_empty());
}

#[test]
fn external_manifest_is_not_fetched_or_reported_as_verified() {
    let bytes = signed(&image_bytes(ImageFormat::Jpeg), "image/jpeg", false, true);
    let result = inspect("jpeg", &bytes).unwrap();
    assert_eq!(result.status, "view.provenance-external", "{result:?}");
    assert!(result.signer.is_none());
    assert!(image::load_from_memory(&bytes).is_ok());
}

#[test]
fn broken_manifest_is_not_reported_as_verified() {
    let mut bytes = signed(&image_bytes(ImageFormat::Jpeg), "image/jpeg", false, false);
    let start = bytes
        .windows(4)
        .position(|window| window == b"c2pa")
        .unwrap();
    bytes[start..start + 4].copy_from_slice(b"xxxx");
    let result = inspect("jpg", &bytes).unwrap();
    assert!(
        !matches!(
            result.status,
            "view.provenance-trusted" | "view.provenance-untrusted"
        ),
        "{result:?}"
    );
    assert!(image::load_from_memory(&bytes).is_ok());
}

#[test]
fn missing_history_is_empty_and_unsigned_images_remain_openable() {
    let bytes = signed(&image_bytes(ImageFormat::Png), "image/png", false, false);
    let result = report(read_original("image/png", &bytes, trusted_settings()));
    assert_eq!(result.status, "view.provenance-trusted", "{result:?}");
    assert!(result.actions.is_empty());
    assert!(result.ingredients.is_empty());
    for (ext, format) in [
        ("JPEG", ImageFormat::Jpeg),
        ("PNG", ImageFormat::Png),
        ("TIF", ImageFormat::Tiff),
    ] {
        assert_eq!(
            inspect(ext, &image_bytes(format)).unwrap().status,
            "view.provenance-absent"
        );
    }
}

#[test]
fn unsupported_formats_have_no_report() {
    assert!(inspect("j2k", &[]).is_none());
    assert!(inspect("exr", &[]).is_none());
}

#[test]
fn captions_bound_untrusted_metadata_and_strip_controls() {
    assert_eq!(caption("a\n\r\t\u{202e}b\u{2066}c"), "abc");
    assert_eq!(caption(&"字".repeat(49)), format!("{}…", "字".repeat(48)));
}

#[test]
fn offline_policy_keeps_verification_on_and_network_fetches_off() {
    let settings = offline_settings();
    assert!(settings.verify.verify_after_reading);
    assert!(!settings.verify.ocsp_fetch);
    assert!(!settings.verify.remote_manifest_fetch);
    assert!(
        settings
            .core
            .allowed_network_hosts
            .as_ref()
            .is_some_and(Vec::is_empty)
    );
    assert!(!MANIFEST_TRUST.contains(ROOT));
}

#[test]
fn source_file_loading_and_docx_export_do_not_transfer_credentials() {
    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).unwrap();
    let directory = TempDir(std::env::temp_dir().join(format!(
        "ibis-c2pa-{}-{}",
        std::process::id(),
        u64::from_ne_bytes(nonce)
    )));
    std::fs::create_dir(&directory.0).unwrap();
    for (ext, format) in [
        ("jpg", ImageFormat::Jpeg),
        ("png", ImageFormat::Png),
        ("tif", ImageFormat::Tiff),
    ] {
        let bytes = signed(&image_bytes(format), mime_type(ext).unwrap(), true, false);
        let path = directory.0.join(format!("frame.0001.{ext}"));
        std::fs::write(&path, &bytes).unwrap();
        let loaded = crate::read_file::read_file(&path).unwrap();
        assert_eq!(
            loaded.image_provenance.unwrap().status,
            "view.provenance-untrusted"
        );
        let expected = image::load_from_memory_with_format(&bytes, format)
            .unwrap()
            .into_rgba8();
        let docx = loaded
            .source
            .save(&loaded.presentation, Default::default())
            .unwrap();
        let docx_path = directory.0.join("preview.docx");
        std::fs::write(&docx_path, docx).unwrap();
        let reopened = crate::read_file::read_file(&docx_path).unwrap();
        assert!(reopened.image_provenance.is_none());
        let media = reopened.presentation.media.last().unwrap();
        assert_eq!(
            image::load_from_memory(&media.data).unwrap().into_rgba8(),
            expected
        );
        assert_eq!(
            inspect("png", &media.data).unwrap().status,
            "view.provenance-absent"
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let next_path = directory.0.join(format!("frame.0002.{ext}"));
        std::fs::write(&next_path, image_bytes(format)).unwrap();
        let next = crate::read_file::read_file(&next_path).unwrap();
        assert_eq!(
            next.image_provenance.unwrap().status,
            "view.provenance-absent"
        );
    }
}

// Run explicitly on an otherwise idle machine with --release --ignored --nocapture.
// This separates SDK verification from image decoding / preview construction.
#[test]
#[ignore = "manual performance measurement"]
fn measure_image_provenance_cost() {
    use std::time::Instant;
    for (ext, format) in [
        ("jpg", ImageFormat::Jpeg),
        ("png", ImageFormat::Png),
        ("tif", ImageFormat::Tiff),
    ] {
        let mut source = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(RgbImage::from_fn(1920, 1080, |x, y| {
            Rgb([(x % 251) as u8, (y % 251) as u8, ((x + y) % 251) as u8])
        }))
        .write_to(&mut source, format)
        .unwrap();
        let bytes = signed(source.get_ref(), mime_type(ext).unwrap(), true, false);
        let start = Instant::now();
        let first = inspect(ext, &bytes).unwrap();
        assert_eq!(first.status, "view.provenance-untrusted");
        let cold = start.elapsed();
        let mut verify = Vec::new();
        let mut preview = Vec::new();
        for _ in 0..11 {
            let start = Instant::now();
            std::hint::black_box(inspect(ext, &bytes).unwrap());
            verify.push(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            std::hint::black_box(
                ibis_formats::production_image::open(Some(ext), &bytes)
                    .unwrap()
                    .unwrap(),
            );
            preview.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        verify.sort_by(f64::total_cmp);
        preview.sort_by(f64::total_cmp);
        println!(
            "C2PA {ext}: bytes={} first_verify_ms={:.3} median_verify_ms={:.3} median_preview_ms={:.3}",
            bytes.len(),
            cold.as_secs_f64() * 1000.0,
            verify[5],
            preview[5]
        );
    }
}

#[test]
fn corrupted_signing_certificate_is_invalid_even_when_root_is_trusted() {
    let mut bytes = signed(&image_bytes(ImageFormat::Jpeg), "image/jpeg", true, false);
    let name = b"Synthetic image signer";
    let start = bytes
        .windows(name.len())
        .position(|window| window == name)
        .unwrap();
    bytes[start] ^= 1;
    let result = report(read_original("image/jpeg", &bytes, trusted_settings()));
    assert!(
        matches!(
            result.status,
            "view.provenance-invalid" | "view.provenance-error"
        ),
        "{result:?}"
    );
    assert!(result.signer.is_none());
    assert!(result.actions.is_empty());
    assert!(result.ingredients.is_empty());
    assert!(image::load_from_memory(&bytes).is_ok());
}

/// Exercise the asynchronous native loader and live save state, rather than
/// Scenario::open's shortcut (which does not install image provenance).
#[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
pub(crate) fn image_provenance_open_save() {
    use crate::scenario::{Scenario, repo_root};
    let directory = repo_root().join("target/scenario/c2pa-open-save-inputs");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("frame.0001.png");
    let next = directory.join("frame.0002.png");
    let original = signed(&image_bytes(ImageFormat::Png), "image/png", true, false);
    std::fs::write(&source, &original).unwrap();
    std::fs::write(&next, image_bytes(ImageFormat::Png)).unwrap();
    let mut scenario = Scenario::start_view();
    let open = |scenario: &mut Scenario, path: &std::path::Path, expected| {
        let name = format!("{}.docx", path.file_stem().unwrap().to_string_lossy());
        scenario.update(|e, w, cx| e.open_path(path.to_path_buf(), None, w, cx));
        for _ in 0..500 {
            scenario.settle();
            if scenario.with(|e, _| {
                e.file_name.as_deref() == Some(name.as_str())
                    && e.image_provenance
                        .as_ref()
                        .is_some_and(|r| r.status == expected)
            }) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            scenario.with(|e, _| e.file_name.clone()).as_deref(),
            Some(name.as_str())
        );
        assert!(
            scenario.with(|e, _| e.path.is_none()),
            "images open as unsaved DOCX previews"
        );
        assert_eq!(
            scenario.with(|e, _| e.image_provenance.as_ref().map(|r| r.status)),
            Some(expected)
        );
    };
    open(&mut scenario, &source, "view.provenance-untrusted");
    open(&mut scenario, &next, "view.provenance-absent");
    open(&mut scenario, &source, "view.provenance-untrusted");
    // Refusing to overwrite the source must keep its verification report.
    assert!(
        scenario
            .update(|e, w, _| e.save_to(source.clone(), w))
            .is_err()
    );
    assert!(scenario.with(|e, _| e.image_provenance.is_some()));
    let docx = scenario.save_as("preview.docx");
    assert!(
        scenario.with(|e, _| e.image_provenance.is_none()),
        "saved DOCX must not retain the original image's status"
    );
    assert!(
        crate::read_file::read_file(&docx)
            .unwrap()
            .image_provenance
            .is_none()
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
}
