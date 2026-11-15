// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Display bounded observations about the package that was opened.

use gpui_kit::*;
use ibis_pptx::ibis_ooxml::signatures::{ContentTypeCheck, ManifestDigest, ObjectDigest, SignatureInspection, SignatureValueCheck, XadesIssuerSerial, RelationshipReference};
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
            SignatureValueCheck::RsaPssSha256 { matches } => crate::i18n::text(if matches { "file.signature-inspect.pss-match" } else { "file.signature-inspect.pss-differs" }).to_string(),
            SignatureValueCheck::EcdsaP384 { matches } => crate::i18n::text(if matches { "file.signature-inspect.ecdsa384-match" } else { "file.signature-inspect.ecdsa384-differs" }).to_string(),
            SignatureValueCheck::EcdsaP256 { matches } => crate::i18n::text(if matches { "file.signature-inspect.ecdsa-match" } else { "file.signature-inspect.ecdsa-differs" }).to_string(),
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
        let inventory = &report.manifest_inventory;
        let entries = inventory.entries.to_string(); let unlisted = inventory.unlisted.to_string();
        let ambiguous = inventory.ambiguous_entries.to_string(); let unresolved = inventory.unresolved_references.to_string();
        message.push_str("\n");
        message.push_str(&crate::i18n::format("file.signature-inspect.inventory", &[("entries", &entries), ("unlisted", &unlisted), ("ambiguous", &ambiguous), ("unresolved", &unresolved)]));
        let mut selected = 0usize; let mut unselected = 0usize; let mut unmatched = 0usize; let mut whole = 0usize; let mut unavailable = 0usize; let mut filtered = 0usize;
        for reference in &report.manifest {
            match &reference.relationships {
                Some(RelationshipReference::Filtered(selection)) => {
                    filtered += 1;
                    selected = selected.saturating_add(selection.selected);
                    unselected = unselected.saturating_add(selection.unselected);
                    unmatched = unmatched.saturating_add(selection.unmatched_id_selectors).saturating_add(selection.unmatched_type_selectors);
                }
                Some(RelationshipReference::WholePart) => whole += 1,
                Some(RelationshipReference::Unsupported) => unavailable += 1,
                None => (),
            }
        }
        if filtered + whole + unavailable > 0 {
            let filtered = filtered.to_string(); let selected = selected.to_string(); let unselected = unselected.to_string(); let unmatched = unmatched.to_string(); let whole = whole.to_string(); let unavailable = unavailable.to_string();
            message.push_str("\n");
            message.push_str(&crate::i18n::format("file.signature-inspect.relationships", &[("filtered", &filtered), ("selected", &selected), ("unselected", &unselected), ("unmatched", &unmatched), ("whole", &whole), ("unavailable", &unavailable)]));
        }
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
            let mut cert_matching = 0usize; let mut cert_differing = 0usize; let mut cert_weak = 0usize; let mut cert_weak_differing = 0usize; let mut cert_unknown = 0usize;
            for reference in &report.additional_signed_info_references {
                match reference.xades_certificate_digest {
                    Some(ObjectDigest::Matches) => cert_matching += 1,
                    Some(ObjectDigest::Differs) => cert_differing += 1,
                    Some(ObjectDigest::LegacySha1 { matches: true }) => cert_weak += 1,
                    Some(ObjectDigest::LegacySha1 { matches: false }) => cert_weak_differing += 1,
                    Some(ObjectDigest::Unsupported) => cert_unknown += 1,
                    None => (),
                }
            }
            if cert_matching + cert_differing + cert_weak + cert_weak_differing + cert_unknown > 0 {
                let matching = cert_matching.to_string(); let differing = cert_differing.to_string(); let weak = cert_weak.to_string(); let weak_differing = cert_weak_differing.to_string(); let unknown = cert_unknown.to_string();
                message.push_str("\n");
                message.push_str(&crate::i18n::format("file.signature-inspect.xades-cert", &[("matching", &matching), ("differing", &differing), ("weak", &weak), ("weak_differing", &weak_differing), ("unknown", &unknown)]));
            }
            let mut exact = 0usize; let mut name_diff = 0usize; let mut serial_diff = 0usize; let mut absent = 0usize; let mut unsupported = 0usize;
            for reference in &report.additional_signed_info_references {
                match &reference.xades_issuer_serial {
                    Some(XadesIssuerSerial::Compared { issuer_der_equal: true, serial_equal: true }) => exact += 1,
                    Some(XadesIssuerSerial::Compared { issuer_der_equal: false, .. }) => name_diff += 1,
                    Some(XadesIssuerSerial::Compared { issuer_der_equal: true, serial_equal: false }) => serial_diff += 1,
                    Some(XadesIssuerSerial::Absent) => absent += 1,
                    Some(XadesIssuerSerial::Unsupported) => unsupported += 1,
                    None => (),
                }
            }
            if exact + name_diff + serial_diff + absent + unsupported > 0 {
                let exact = exact.to_string(); let name_diff = name_diff.to_string(); let serial_diff = serial_diff.to_string(); let absent = absent.to_string(); let unsupported = unsupported.to_string();
                message.push_str("\n");
                message.push_str(&crate::i18n::format("file.signature-inspect.xades-issuer", &[("exact", &exact), ("name_diff", &name_diff), ("serial_diff", &serial_diff), ("absent", &absent), ("unsupported", &unsupported)]));
            }
        }
    }
    if reports.len() > 8 { message.push_str("\n\n"); message.push_str(&crate::i18n::text("file.signature-inspect.truncated")); }
    message
}
