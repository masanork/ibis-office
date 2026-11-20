// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::io::Cursor;

use c2pa::settings::{TrustAnchor, TrustListKind};
use c2pa::{Context as C2paContext, Error, Reader, Settings, ValidationState};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{Context, IntoElement};

use crate::editor::SlideEditor;
use crate::i18n::{format, text};

const MAX_ITEMS: usize = 8;
const MANIFEST_TRUST: &str = include_str!("../data/c2pa/C2PA-TRUST-LIST.pem");
const TSA_TRUST: &str = include_str!("../data/c2pa/C2PA-TSA-TRUST-LIST.pem");

#[derive(Default)]
pub(crate) struct Report {
    status: &'static str,
    signer: Option<String>,
    issuer: Option<String>,
    actions: Vec<String>,
    ingredients: Vec<String>,
}

pub(crate) fn inspect(extension: &str, original: &[u8]) -> Option<Report> {
    let mime = match extension.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "tif" | "tiff" => "image/tiff",
        _ => return None,
    };
    let mut settings = Settings::new();
    settings.verify.ocsp_fetch = false;
    settings.verify.remote_manifest_fetch = false;
    settings.trust.anchors = Some(vec![
        TrustAnchor {
            trust_anchors: MANIFEST_TRUST.into(),
            trust_uri: Some("https://c2pa.org/trustlist".into()),
            ..TrustAnchor::default()
        },
        TrustAnchor {
            trust_anchors: TSA_TRUST.into(),
            trust_uri: Some("https://github.com/c2pa-org/conformance-public/blob/92de7d8c2510160fa816d2ae90957e52a9ea62d4/trust-list/C2PA-TSA-TRUST-LIST.pem".into()),
            trust_kind: TrustListKind::TSA,
            ..TrustAnchor::default()
        },
    ]);
    Some(inspect_with_settings(mime, original, settings))
}

fn inspect_with_settings(mime: &str, original: &[u8], settings: Settings) -> Report {
    let result = C2paContext::new()
        .with_settings(settings)
        .and_then(|context| Reader::from_context(context).with_stream(mime, Cursor::new(original)));
    let reader = match result {
        Ok(reader) => reader,
        Err(error) => {
            let status = match error {
                Error::JumbfNotFound => "view.provenance-absent",
                Error::RemoteManifestUrl(_) | Error::RemoteManifestFetch(_) => {
                    "view.provenance-external"
                }
                _ => "view.provenance-error",
            };
            return Report {
                status,
                ..Report::default()
            };
        }
    };
    let status = match reader.validation_state() {
        ValidationState::Trusted => "view.provenance-trusted",
        ValidationState::Valid => "view.provenance-untrusted",
        ValidationState::Invalid => {
            let hash_mismatch = reader
                .validation_results()
                .and_then(|results| results.active_manifest())
                .is_some_and(|active| {
                    active.failure().iter().any(|status| {
                        matches!(
                            status.code(),
                            "assertion.dataHash.mismatch" | "assertion.boxesHash.mismatch"
                        )
                    })
                });
            return Report {
                status: if hash_mismatch {
                    "view.provenance-mismatch"
                } else {
                    "view.provenance-invalid"
                },
                ..Report::default()
            };
        }
    };
    let Some(manifest) = reader.active_manifest() else {
        return Report {
            status: "view.provenance-error",
            ..Report::default()
        };
    };
    let signature = manifest.signature_info();
    let mut actions = Vec::new();
    for assertion in manifest
        .assertions()
        .iter()
        .filter(|item| matches!(item.label(), "c2pa.actions" | "c2pa.actions.v2"))
    {
        if let Ok(record) = assertion.to_assertion::<c2pa::assertions::Actions>() {
            actions.extend(
                record
                    .actions()
                    .iter()
                    .take(MAX_ITEMS - actions.len())
                    .map(|action| caption(action.action())),
            );
            if actions.len() == MAX_ITEMS {
                break;
            }
        }
    }
    Report {
        status,
        signer: signature
            .and_then(|info| info.common_name.as_deref())
            .map(caption),
        issuer: signature
            .and_then(|info| info.issuer.as_deref())
            .map(caption),
        actions,
        ingredients: manifest
            .ingredients()
            .iter()
            .take(MAX_ITEMS)
            .map(|ingredient| {
                ingredient
                    .title()
                    .map(caption)
                    .unwrap_or_else(|| text("view.provenance-untitled").to_string())
            })
            .collect(),
    }
}

fn caption(value: &str) -> String {
    let mut chars = value.chars().filter(|ch| {
        !ch.is_control() && !matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    });
    let mut caption: String = chars.by_ref().take(48).collect();
    if chars.next().is_some() {
        caption.push('…');
    }
    caption
}

impl SlideEditor {
    pub(crate) fn render_image_provenance(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        self.image_provenance.as_ref()?;
        Some(
            Button::new("image-provenance")
                .ghost()
                .xsmall()
                .label(text("view.provenance"))
                .on_click(cx.listener(|this, _, _, cx| {
                    let Some(report) = this.image_provenance.as_ref() else {
                        return;
                    };
                    let mut lines = vec![text(report.status).to_string()];
                    if let Some(signer) = &report.signer {
                        lines.push(format("view.provenance-signer", &[("value", signer)]));
                    }
                    if let Some(issuer) = &report.issuer {
                        lines.push(format("view.provenance-issuer", &[("value", issuer)]));
                    }
                    if matches!(
                        report.status,
                        "view.provenance-trusted" | "view.provenance-untrusted"
                    ) {
                        let none = text("view.provenance-not-recorded");
                        for (key, items) in [
                            ("view.provenance-actions", &report.actions),
                            ("view.provenance-ingredients", &report.ingredients),
                        ] {
                            let value = if items.is_empty() {
                                none.to_string()
                            } else {
                                items.join(", ")
                            };
                            lines.push(format(key, &[("value", &value)]));
                        }
                    }
                    lines.push(text("view.provenance-offline").to_string());
                    this.show_alert(text("view.provenance"), lines.join("\n\n").into(), cx);
                })),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_png_opens_without_credentials() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("logo/png/ico/ibisOffice_icon_win_16px.png");
        let loaded = crate::read_file::read_file(&path).unwrap();
        assert_eq!(
            loaded.image_provenance.unwrap().status,
            "view.provenance-absent"
        );
        assert!(!loaded.presentation.media.is_empty());
    }

    #[test]
    fn unsupported_formats_have_no_report() {
        assert!(inspect("j2k", &[]).is_none());
        assert!(inspect("exr", &[]).is_none());
    }
}
