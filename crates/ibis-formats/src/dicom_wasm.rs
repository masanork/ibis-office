// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Lightweight wasm stub for DICOM types so the web build does not bundle
//! `dicom-object`, `dicom-dictionary-std`, `pure_jpegls`, or `jxl-oxide`.

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tag(pub u16, pub u16);

impl Tag {
    pub const fn group(self) -> u16 {
        self.0
    }

    pub const fn element(self) -> u16 {
        self.1
    }
}

pub const DEFAULT_PII_TAGS: &[(Tag, &str)] = &[
    (Tag(0x0010, 0x0010), "PatientName"),
    (Tag(0x0010, 0x0020), "PatientID"),
    (Tag(0x0010, 0x0030), "PatientBirthDate"),
    (Tag(0x0010, 0x0040), "PatientSex"),
    (Tag(0x0010, 0x1000), "OtherPatientIDs"),
    (Tag(0x0010, 0x1001), "OtherPatientNames"),
    (Tag(0x0010, 0x1040), "PatientAddress"),
    (Tag(0x0010, 0x2154), "PatientTelephoneNumbers"),
    (Tag(0x0008, 0x0050), "AccessionNumber"),
    (Tag(0x0008, 0x0080), "InstitutionName"),
    (Tag(0x0008, 0x0081), "InstitutionAddress"),
    (Tag(0x0008, 0x0090), "ReferringPhysicianName"),
    (Tag(0x0008, 0x1048), "PhysiciansOfRecord"),
    (Tag(0x0008, 0x1050), "PerformingPhysicianName"),
    (Tag(0x0008, 0x1070), "OperatorsName"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedactionRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct DicomAnonymizationProfile {
    pub extra_strip_tags: Vec<Tag>,
    pub strip_dates: bool,
    pub roi_masks: Vec<RedactionRect>,
}

impl Default for DicomAnonymizationProfile {
    fn default() -> Self {
        Self {
            extra_strip_tags: Vec::new(),
            strip_dates: true,
            roi_masks: Vec::new(),
        }
    }
}

pub fn parse_tag_list(_text: &str) -> (Vec<Tag>, Vec<String>) {
    (Vec::new(), Vec::new())
}

pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    Some((
        u32::from_be_bytes(png[16..20].try_into().ok()?),
        u32::from_be_bytes(png[20..24].try_into().ok()?),
    ))
}

#[derive(Debug, Clone)]
pub struct DicomNonDiagnosticAsset {
    pub anonymized_dcm: Vec<u8>,
    pub stripped_tags: Vec<String>,
    pub metadata_rows: Vec<(String, String)>,
    pub png_bytes: Option<Vec<u8>>,
    pub warnings: Vec<String>,
    pub burned_in: bool,
}

pub fn is_dicom(_data: &[u8]) -> bool {
    false
}

pub fn anonymize_and_extract(
    _data: &[u8],
    _profile: &DicomAnonymizationProfile,
) -> Result<DicomNonDiagnosticAsset, Error> {
    Err(Error::new("DICOM 処理は Web 版では利用できません"))
}

pub fn input_to_markdown(_data: &[u8]) -> Result<Option<String>, Error> {
    Ok(None)
}

pub fn format_asset_markdown(_asset: &DicomNonDiagnosticAsset) -> String {
    String::new()
}

pub fn embed_dicom_into_pptx_slide(
    _presentation: &mut ibis_doc_model::Presentation,
    _slide_idx: usize,
    _dcm_bytes: &[u8],
    _profile: &DicomAnonymizationProfile,
) -> Result<DicomNonDiagnosticAsset, Error> {
    Err(Error::new("DICOM 処理は Web 版では利用できません"))
}

pub fn embed_dicom_into_docx_flow(
    _presentation: &mut ibis_doc_model::Presentation,
    _dcm_bytes: &[u8],
    _profile: &DicomAnonymizationProfile,
) -> Result<DicomNonDiagnosticAsset, Error> {
    Err(Error::new("DICOM 処理は Web 版では利用できません"))
}
