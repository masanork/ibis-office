// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Display bounded observations about the package that was opened.

use gpui_kit::*;
use ibis_pptx::ibis_ooxml::signatures::{ContentTypeCheck, ManifestDigest, ObjectDigest, SignatureInspection, SignatureValueCheck};
use crate::editor::SlideEditor;

impl SlideEditor {
    pub(crate) fn inspect_document_signatures(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else { return };
        self.set_status(crate::i18n::text("file.signature-inspect.checking").to_string(), cx);
        cx.spawn_in(window, async move |this, cx| {
            let worker_source = source.clone();
            let result = cx.background_executor().spawn(async move { worker_source.inspect_package_signatures() }).await;
            this.update(cx, |this, cx| {
                // A result for a previous document must not appear in a new one.
                if !this.has_document || !this.source.as_ref().is_some_and(|current| std::sync::Arc::ptr_eq(current, &source)) { return; }
                let message = match result {
                    Ok(reports) => format_report(&reports),
                    Err(_) => crate::i18n::text("file.signature-inspect.failed").to_string(),
                };
                this.show_alert(crate::i18n::text("file.signature-inspect.title"), message.into(), cx);
            }).ok();
        }).detach();
    }
}

fn format_report(reports: &[SignatureInspection]) -> String {
    let mut message = crate::i18n::text("file.signature-inspect.boundary").to_string();
    if reports.is_empty() {
        message.push_str("\n\n");
        message.push_str(&crate::i18n::text("file.signature-inspect.none"));
        return message;
    }
    for (index, report) in reports.iter().take(8).enumerate() {
        let value = match report.signature_value {
            SignatureValueCheck::LegacySha1 { key_bits, matches } => crate::i18n::format(
                if matches { "file.signature-inspect.legacy-signature-match" } else { "file.signature-inspect.legacy-signature-differs" },
                &[("bits", &key_bits.to_string())]),
            SignatureValueCheck::MatchesUntrustedCertificate => crate::i18n::text("file.signature-inspect.matches").to_string(),
            SignatureValueCheck::Differs => crate::i18n::text("file.signature-inspect.differs").to_string(),
            SignatureValueCheck::Unsupported => crate::i18n::text("file.signature-inspect.unknown").to_string(),
        };
        let object = match report.package_object {
            ObjectDigest::LegacySha1 { matches: true } => "file.signature-inspect.legacy-match",
            ObjectDigest::LegacySha1 { matches: false } => "file.signature-inspect.legacy-differs",
            ObjectDigest::Matches => "file.signature-inspect.matches",
            ObjectDigest::Differs => "file.signature-inspect.differs",
            ObjectDigest::Unsupported => "file.signature-inspect.unknown",
        };
        let mut matching = 0usize;
        let mut legacy = 0usize;
        let mut legacy_differing = 0usize;
        let mut differing = 0usize;
        let mut unknown = 0usize;
        for reference in &report.manifest {
            match (&reference.digest, &reference.content_type) {
                (ManifestDigest::Matches, ContentTypeCheck::Matches) => matching += 1,
                (ManifestDigest::LegacySha1 { matches: true }, ContentTypeCheck::Matches) => legacy += 1,
                (ManifestDigest::LegacySha1 { matches: false }, _) => legacy_differing += 1,
                (ManifestDigest::Differs | ManifestDigest::MissingPart, _) | (_, ContentTypeCheck::Differs | ContentTypeCheck::Missing) => differing += 1,
                _ => unknown += 1,
            }
        }
        let number = (index + 1).to_string();
        let object = crate::i18n::text(object);
        let matching = matching.to_string(); let legacy = legacy.to_string(); let legacy_differing = legacy_differing.to_string(); let differing = differing.to_string(); let unknown = unknown.to_string();
        message.push_str("\n\n");
        message.push_str(&crate::i18n::format("file.signature-inspect.entry", &[("number", &number), ("value", &value), ("object", &object), ("matching", &matching), ("legacy", &legacy), ("legacy_differing", &legacy_differing), ("differing", &differing), ("unknown", &unknown)]));
        if !report.additional_signed_info_references.is_empty() {
            let mut matching = 0usize;
            let mut legacy = 0usize;
            let mut differing = 0usize;
            let mut legacy_differing = 0usize;
            let mut unknown = 0usize;
            for reference in &report.additional_signed_info_references {
                match reference.digest {
                    ObjectDigest::Matches => matching += 1,
                    ObjectDigest::Differs => differing += 1,
                    ObjectDigest::LegacySha1 { matches: true } => legacy += 1,
                    ObjectDigest::LegacySha1 { matches: false } => legacy_differing += 1,
                    ObjectDigest::Unsupported => unknown += 1,
                }
            }
            let matching = matching.to_string(); let legacy = legacy.to_string(); let differing = differing.to_string();
            let legacy_differing = legacy_differing.to_string(); let unknown = unknown.to_string();
            message.push_str("\n");
            message.push_str(&crate::i18n::format("file.signature-inspect.additional-refs", &[("matching", &matching), ("legacy", &legacy), ("differing", &differing), ("legacy_differing", &legacy_differing), ("unknown", &unknown)]));
        }
    }
    if reports.len() > 8 { message.push_str("\n\n"); message.push_str(&crate::i18n::text("file.signature-inspect.truncated")); }
    message
}
