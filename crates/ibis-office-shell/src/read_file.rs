// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Reading a file for editing (on a thread of its own), with its password if it has one.

#[cfg(not(target_family = "wasm"))]
use crate::timing;

/// The disk identity of the CycloneDX file used to generate a workbook.
#[cfg(not(target_family = "wasm"))]
#[derive(Clone)]
pub(crate) struct BomSource {
    pub path: std::path::PathBuf,
    pub digest: [u8; 32],
    pub has_crypto_assets: bool,
}

/// A file read for editing: its model, and what saving it needs.
#[cfg(not(target_family = "wasm"))]
pub struct Loaded {
    pub presentation: ibis_doc_model::Presentation,
    pub source: std::sync::Arc<ibis_formats::Source>,
    /// The password a protected file was opened with: saving protects it again.
    pub password: Option<String>,
    pub standard_encryption: bool,
    pub(crate) import_warning: Option<ibis_formats::ImportWarning>,
    pub(crate) editable_import: Option<EditableImport>,
    pub jtd_original: Option<Vec<u8>>,
    pub jtd_gallery: bool,
    pub structured_preview: bool,
    pub(crate) structured_bytes: Option<std::sync::Arc<Vec<u8>>>,
    pub(crate) bom_source: Option<BomSource>,
    pub(crate) image_provenance: Option<crate::image_provenance::Report>,
    pub(crate) exr_exposure: Option<ibis_formats::production_image::ExrExposure>,
    pub(crate) csv_stream: Option<std::sync::mpsc::Receiver<ibis_formats::CsvProgressiveChunk>>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) const LINKAGE_CSV_NEEDS_TREE: &str = "\u{1}linkage-csv-needs-tree";

#[cfg(not(target_family = "wasm"))]
fn read_only_preview(opened: ibis_formats::Opened, structured_bytes: Option<std::sync::Arc<Vec<u8>>>) -> Loaded {
    crate::render::preload::Preload::start_for_next(&opened.presentation);
    Loaded {
        presentation: opened.presentation, source: std::sync::Arc::new(opened.source),
        password: None, standard_encryption: false, editable_import: None, import_warning: None, csv_stream: None,
        jtd_original: None, jtd_gallery: false, structured_preview: true, structured_bytes, bom_source: None,
        exr_exposure: None, image_provenance: None,
    }
}

/// Identify municipal standard linkage files before the ordinary CSV importer infers cell types.
/// Shared between Desktop and Web builds so both choose the same tabular/tree path.
pub(crate) fn linkage_csv(data: &[u8]) -> Option<(&str, bool)> {
    let text = std::str::from_utf8(data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data)).ok()?;
    let mut records = ibis_formats::csv::CsvRecordIter::new(text, ',');
    let header = records.next()?;
    let signature = ["削除フラグ", "操作者ID", "操作年月日", "操作時刻"];
    if !signature.iter().all(|name| header.iter().any(|column| column == name)) {
        return None;
    }
    let needs_tree = header.len() > 256
        || header.iter().filter_map(|column| column.rsplit_once('_')).any(|(name, index)| {
            !name.is_empty() && index.trim_start_matches('0') == "1"
                && header.iter().filter_map(|column| column.rsplit_once('_'))
                    .any(|(other, index)| other == name && index.trim_start_matches('0') == "2")
        })
        || records.enumerate().any(|(index, fields)| fields.len() != header.len() || index >= 1_048_575);
    Some((text, needs_tree))
}

/// Format-specific document open result shared by Desktop (`read_file_data`) and
/// Web (`open_web_bytes`) so newly added importers never diverge between platforms.
pub(crate) enum CommonOpen {
    Standard {
        opened: ibis_formats::Opened,
        editable_import: Option<EditableImport>,
        bom_format: Option<ibis_formats::sbom_review::ReportFormat>,
        is_law: bool,
    },
    StructuredPreview(ibis_formats::Opened),
}

/// Dispatch specialized document formats (Lawtext, DICOM, FHIR, CycloneDX/OSV JSON,
/// Markdown, Hancom/ODF editable imports, and DocxConvert formats) from raw bytes.
/// Returns `Ok(None)` when the caller should fall through to the primary OOXML/CSV/image path.
pub(crate) fn open_document_bytes(
    name: &str,
    base_dir: Option<&std::path::Path>,
    data: &[u8],
) -> Result<Option<CommonOpen>, String> {
    use ibis_formats::docx_convert::DocxConvertFormat;
    if let Some(opened) = ibis_formats::xmlss::try_open(data).map_err(|e| e.to_string())? {
        return Ok(Some(CommonOpen::Standard {
            opened,
            editable_import: Some(EditableImport::XmlSpreadsheet),
            bom_format: None,
            is_law: false,
        }));
    }
    let virtual_path = std::path::Path::new(name);
    let extension = virtual_path.extension().and_then(|ext| ext.to_str());
    let external_document_format = DocxConvertFormat::from_path(virtual_path);
    let editable_import = EditableImport::detect(extension, data).filter(|format| {
        matches!(format, EditableImport::Hancom(_) | EditableImport::Uot | EditableImport::Epub | EditableImport::WordPerfect)
            || matches!(external_document_format, None | Some(DocxConvertFormat::Odt))
    });

    if let Some(law_md) = ibis_lawtext::path_input_to_markdown(virtual_path, data)
        .map_err(|e| e.to_string())?
    {
        let opened = open_markdown_bytes(law_md.into_bytes(), base_dir, false)?;
        return Ok(Some(CommonOpen::Standard { opened, editable_import: None, bom_format: None, is_law: true }));
    }
    if let Some(dicom_md) = ibis_formats::dicom::input_to_markdown(data).map_err(|e| e.to_string())? {
        let opened = open_markdown_bytes(dicom_md.into_bytes(), base_dir, false)?;
        return Ok(Some(CommonOpen::Standard { opened, editable_import: None, bom_format: None, is_law: false }));
    }
    if let Some(fhir_md) = ibis_formats::fhir::input_to_markdown(data).map_err(|e| e.to_string())? {
        let opened = open_markdown_bytes(fhir_md.into_bytes(), base_dir, false)?;
        return Ok(Some(CommonOpen::Standard { opened, editable_import: None, bom_format: None, is_law: false }));
    }
    if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        && let Some((opened, format)) = ibis_formats::sbom_review::open_table_with_format((name, data))?
    {
        return Ok(Some(CommonOpen::Standard {
            opened,
            editable_import: None,
            bom_format: Some(format),
            is_law: false,
        }));
    }
    if let Some(ext) = extension.filter(|ext| ibis_formats::structured_preview::supports_extension(ext)) {
        #[cfg(not(target_family = "wasm"))]
        let definition = base_dir.zip(virtual_path.file_name())
            .map(|(directory, file_name)| directory.join(file_name).with_extension("view.json"));
        #[cfg(target_family = "wasm")]
        let definition: Option<std::path::PathBuf> = {
            let _ = base_dir;
            None
        };
        let opened = crate::structured_preview::open(ext, data, definition.as_deref())?;
        return Ok(Some(CommonOpen::StructuredPreview(opened)));
    }
    if external_document_format == Some(DocxConvertFormat::Markdown) {
        match crate::markdown_split::detect_text_content_kind(data) {
            crate::markdown_split::TextContentKind::Marp => {
                let opened = open_markdown_bytes(data.to_vec(), base_dir, true)?;
                return Ok(Some(CommonOpen::Standard { opened, editable_import: None, bom_format: None, is_law: false }));
            }
            crate::markdown_split::TextContentKind::Workbook => {
                let opened = ibis_formats::markdown::open_markdown_workbook(data.to_vec())
                    .map_err(|e| e.to_string())?;
                return Ok(Some(CommonOpen::Standard { opened, editable_import: None, bom_format: None, is_law: false }));
            }
            crate::markdown_split::TextContentKind::Lawtext => {
                if let Ok(text) = std::str::from_utf8(data)
                    && let Ok(law) = ibis_lawtext::parse_lawtext(text)
                {
                    let law_md = ibis_lawtext::to_markdown(&law);
                    let opened = open_markdown_bytes(law_md.into_bytes(), base_dir, false)?;
                    return Ok(Some(CommonOpen::Standard { opened, editable_import: None, bom_format: None, is_law: true }));
                }
                let opened = open_markdown_bytes(data.to_vec(), base_dir, false)?;
                return Ok(Some(CommonOpen::Standard { opened, editable_import: None, bom_format: None, is_law: false }));
            }
            crate::markdown_split::TextContentKind::Markdown => {
                let opened = open_markdown_bytes(data.to_vec(), base_dir, false)?;
                return Ok(Some(CommonOpen::Standard { opened, editable_import: None, bom_format: None, is_law: false }));
            }
        }
    }
    if let Some(format) = editable_import {
        let opened = format.open(data, base_dir).map_err(|e| e.to_string())?;
        return Ok(Some(CommonOpen::Standard { opened, editable_import: Some(format), bom_format: None, is_law: false }));
    }
    if let Some(format) = external_document_format {
        let opened = ibis_formats::docx_convert::import_docx_format(data, format).map_err(|e| e.to_string())?;
        return Ok(Some(CommonOpen::Standard { opened, editable_import: None, bom_format: None, is_law: false }));
    }
    Ok(None)
}

pub(crate) fn open_markdown_bytes(
    bytes: Vec<u8>,
    base_dir: Option<&std::path::Path>,
    presentation: bool,
) -> Result<ibis_formats::Opened, String> {
    #[cfg(not(target_family = "wasm"))]
    let result = {
        let system_dir = ibis_office_base::app_data::design_systems_dir();
        if presentation {
            ibis_formats::markdown::open_markdown_presentation_with_design_system_dirs(
                bytes,
                base_dir,
                system_dir.as_deref(),
            )
        } else {
            ibis_formats::markdown::open_markdown_with_design_system_dirs(
                bytes,
                base_dir,
                system_dir.as_deref(),
            )
        }
    };
    #[cfg(target_family = "wasm")]
    let result = {
        let _ = base_dir;
        if presentation {
            ibis_formats::markdown::open_markdown_presentation(bytes)
        } else {
            ibis_formats::markdown::open_markdown(bytes)
        }
    };
    result.map_err(|e| e.to_string())
}

/// What `read_file` says when the file is protected and needs its password.
pub const NEEDS_PASSWORD: &str = "\u{1}needs-password";
/// What it says when the password given is not the file's.
pub const WRONG_PASSWORD: &str = "\u{1}wrong-password";

/// The file at `path`, read and parsed. Its pictures and fonts start
/// decoding on other threads (`preload`).
#[cfg(not(target_family = "wasm"))]
pub fn read_file(path: &std::path::Path) -> Result<Loaded, String> {
    read_file_with(path, None)
}

/// As `read_file`; `password` opens a protected file (パスワードを使用して暗号化).
#[cfg(not(target_family = "wasm"))]
pub fn read_file_with(path: &std::path::Path, password: Option<&str>) -> Result<Loaded, String> {
    read_file_with_stack(path, password, ibis_formats::csv::TextEncoding::Auto)
}

/// Re-read a CSV after the user has selected its encoding.
#[cfg(not(target_family = "wasm"))]
pub fn read_file_with_csv_encoding(path: &std::path::Path, encoding: ibis_formats::csv::TextEncoding) -> Result<Loaded, String> {
    read_file_with_stack(path, None, encoding)
}

/// ZIP creation during CSV import needs more stack than GPUI workers provide.
/// Both automatic and explicitly selected reads can run on those workers.
#[cfg(not(target_family = "wasm"))]
fn read_file_with_stack(path: &std::path::Path, password: Option<&str>, encoding: ibis_formats::csv::TextEncoding) -> Result<Loaded, String> {
    let path = path.to_owned();
    let password = password.map(str::to_owned);
    std::thread::Builder::new().name("file-read".into()).stack_size(8 * 1024 * 1024)
        .spawn(move || catch_read_panic(&path, || read_file_unguarded(&path, password.as_deref(), encoding)))
        .map_err(|error| error.to_string())?
        .join().map_err(|_| "File reader thread failed".to_string())?
}

#[cfg(not(target_family = "wasm"))]
pub fn read_dpx_with_color(path: &std::path::Path, color: ibis_formats::production_image::DpxColor) -> Result<Loaded, String> {
    catch_read_panic(path, || {
        let data = std::fs::read(path).map_err(|error| error.to_string())?;
        let (opened, exposure) = ibis_formats::production_image::open_with_dpx_color(Some("dpx"), &data, Some(color))?
            .ok_or("DPX preview is unavailable")?;
        Ok(load_production_image(opened, exposure, None))
    })
}

#[cfg(not(target_family = "wasm"))]
fn load_production_image(
    opened: ibis_formats::Opened,
    exr_exposure: Option<ibis_formats::production_image::ExrExposure>,
    image_provenance: Option<crate::image_provenance::Report>,
) -> Loaded {
    crate::render::preload::Preload::start_for_next(&opened.presentation);
    Loaded { presentation: opened.presentation, source: std::sync::Arc::new(opened.source), password: None, standard_encryption: false, editable_import: None, import_warning: None, csv_stream: None, jtd_original: None, jtd_gallery: false, structured_preview: false, structured_bytes: None, bom_source: None, exr_exposure, image_provenance }
}

/// Run `read` for the file at `path`. One file's reader going wrong must close that file only: every window shares this
/// process, so a panic here would take the other open documents with it (review 0.8.5, C1). The panic is caught and becomes
/// this file's open error. (No `panic = "abort"` is set anywhere in the workspace, so the unwind reaches this frame.)
#[cfg(not(target_family = "wasm"))]
pub fn catch_read_panic<T>(path: &std::path::Path, read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|payload| {
        let what = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_default();
        log::error!("{}: the reader panicked: {what}", path.display());
        note_open_failed(path);
        Err(crate::i18n::text("file.read-crashed").to_string())
    })
}

#[cfg(all(test, not(target_family = "wasm")))]
mod stack_tests {
    #[test]
    fn imported_opendocument_uses_detected_type_for_save() {
        use super::EditableImport;
        use ibis_formats::Kind;
        use ibis_odf::Format;

        type WriteOdf = fn(&ibis_doc_model::Presentation) -> Result<Vec<u8>, ibis_formats::Error>;
        let cases: [(Kind, Format, &str, WriteOdf); 3] = [
            (Kind::Document, Format::Text, "docx", ibis_odf::export_docx_to_odt),
            (Kind::Workbook, Format::Spreadsheet, "xlsx", ibis_odf::write_ods),
            (Kind::Presentation, Format::Presentation, "pptx", ibis_odf::write_odp),
        ];
        for (kind, format, extension, write) in cases {
            let blank = ibis_formats::open(ibis_formats::blank_file(kind).unwrap()).unwrap();
            let bytes = write(&blank.presentation).unwrap();
            let detected = EditableImport::detect(None, &bytes).unwrap();
            assert_eq!(detected, EditableImport::OpenDocument(format));
            assert_eq!(detected.save_extension(), extension);
            assert_eq!(detected.open(&bytes, None).unwrap().source.kind(), kind);
        }
    }

    #[test]
    fn imported_preview_formats_use_the_shared_routes() {
        use super::{EditableImport, FixedLayoutFormat, PreviewImport};
        assert_eq!(PreviewImport::detect(Some("XDW"), b""), Some(PreviewImport::FixedLayout(FixedLayoutFormat::Xdw)));
        assert_eq!(PreviewImport::detect(Some("JTT"), b""), Some(PreviewImport::Jtd));
        assert_eq!(EditableImport::detect(Some("HWPX"), b""), Some(EditableImport::Hancom(ibis_formats::hwp::Format::Hwpx)));
        assert_eq!(EditableImport::detect(Some("HWP"), b"").unwrap().save_extension(), "docx");
        assert_eq!(EditableImport::detect(Some("UOT"), b""), Some(EditableImport::Uot));
        assert_eq!(EditableImport::Uot.save_extension(), "docx");
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    #[test]
    fn plain_cyclonedx_workbook_retains_source_without_enabling_cbom_comparison() {
        let path = std::env::temp_dir().join(format!("ibis-plain-sbom-{}.json", std::process::id()));
        std::fs::write(&path, br#"{"bomFormat":"CycloneDX","specVersion":"1.6","components":[{"name":"demo","type":"library"}]}"#).unwrap();
        let loaded = super::read_file(&path);
        std::fs::remove_file(&path).unwrap();
        let loaded = loaded.unwrap();
        let source = loaded.bom_source.unwrap();
        assert_eq!(source.path, path);
        assert!(!source.has_crypto_assets);
        assert!(loaded.presentation.workbook.is_some());
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    #[test]
    fn linkage_csv_distinguishes_plain_tabular_and_structured_inputs() {
        use super::linkage_csv;
        let header = "操作時刻,削除フラグ,操作者ID,操作年月日";
        assert!(linkage_csv("削除フラグ,操作者ID,操作年月日\n0,u,20261124".as_bytes()).is_none());
        assert!(!linkage_csv(format!("\u{feff}{header}\r\n090000,0,000001,20261124").as_bytes()).unwrap().1);
        assert!(linkage_csv(format!("{header},項目_01,項目_02\n090000,0,000001,20261124,x,y").as_bytes()).unwrap().1);
        assert!(linkage_csv(format!("{header}\n090000,0,000001").as_bytes()).unwrap().1);
        let wide = (1..=253).map(|index| format!(",列{index}")).collect::<String>();
        assert!(linkage_csv(format!("{header}{wide}").as_bytes()).unwrap().1);
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    #[test]
    fn linkage_csv_opens_as_read_only_literal_cells() {
        let path = std::env::temp_dir().join(format!("ibis-linkage-csv-{}.csv", std::process::id()));
        std::fs::write(&path, "削除フラグ,操作者ID,操作年月日,操作時刻,備考\r\n0,000001,20261124,090000,=1+2\r\n").unwrap();
        let loaded = super::read_file(&path);
        let definition = path.with_extension("definition.json");
        std::fs::write(&definition, "{}").unwrap();
        let with_definition = super::read_file(&path);
        std::fs::remove_file(definition).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(with_definition.err().as_deref(), Some(super::LINKAGE_CSV_NEEDS_TREE));
        let loaded = loaded.unwrap();
        assert!(loaded.structured_preview);
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&loaded.presentation.slides[0]).unwrap();
        for (column, expected) in [(1, "000001"), (2, "20261124"), (3, "090000"), (4, "=1+2")] {
            let cell = sheet.cell(1, column).unwrap();
            assert_eq!(cell.value, ibis_doc_model::CellValue::Text(expected.into()));
            assert!(cell.formula.is_none());
        }
    }

    #[test]
    fn osv_json_opens_as_an_editable_workbook() {
        let path = std::env::temp_dir().join(format!("ibis-review-{}.osv.json", std::process::id()));
        std::fs::write(&path, br#"{"results":[]}"#).unwrap();
        let loaded = super::read_file(&path);
        std::fs::remove_file(path).unwrap();
        let loaded = loaded.unwrap();
        assert_eq!(ibis_formats::Kind::of(&loaded.presentation), ibis_formats::Kind::Workbook);
        assert!(ibis_formats::sbom_review::can_refresh(&loaded.presentation));
        assert!(!loaded.structured_preview);
    }

    #[test]
    fn automatic_csv_read_works_from_a_small_stack() {
        let path = std::env::temp_dir().join(format!("ibis-csv-small-stack-{}-{}.csv", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let bytes = ibis_formats::csv::encode_sjis("名前,数量\r\nりんご,00123\r\n");
        std::fs::write(&path, &bytes).unwrap();
        let read_path = path.clone();
        let loaded = std::thread::Builder::new().stack_size(512 * 1024).spawn(move || super::read_file(&read_path)).unwrap().join().unwrap().unwrap();
        assert_eq!(loaded.source.original_csv_bytes().unwrap().as_ref(), bytes);
        std::fs::remove_file(path).unwrap();
    }
}

/// Imports that can be edited and saved to a new OOXML file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditableImport {
    Hancom(ibis_formats::hwp::Format),
    OpenDocument(ibis_odf::Format),
    Uot,
    Epub,
    WordPerfect,
    XmlSpreadsheet,
}

impl EditableImport {
    pub(crate) fn detect(extension: Option<&str>, data: &[u8]) -> Option<Self> {
        if let Some(format) = ibis_formats::hwp::Format::detect(extension, data) {
            return Some(Self::Hancom(format));
        }
        if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("epub")) || ibis_formats::epub::is_epub(data) {
            return Some(Self::Epub);
        }
        if extension.is_some_and(ibis_formats::uot::is_extension) || ibis_formats::uot::is_uot(data) {
            return Some(Self::Uot);
        }
        if extension.is_some_and(ibis_formats::wpd::is_extension) || ibis_formats::wpd::is_wpd(data) {
            return Some(Self::WordPerfect);
        }
        // A text-document extension also routes malformed input to its importer.
        if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("odt") || ext.eq_ignore_ascii_case("ott")) {
            return Some(Self::OpenDocument(ibis_odf::Format::Text));
        }
        ibis_odf::Format::detect(data).map(Self::OpenDocument)
    }

    pub(crate) fn open(self, data: &[u8], base_dir: Option<&std::path::Path>) -> Result<ibis_formats::Opened, ibis_formats::Error> {
        match self {
            Self::Hancom(format) => format.open(data),
            Self::Epub => ibis_formats::epub::open(data),
            Self::Uot => ibis_formats::uot::open_with_base_dir(data, base_dir),
            Self::WordPerfect => ibis_formats::wpd::open(data),
            Self::XmlSpreadsheet => ibis_formats::xmlss::try_open(data)?
                .ok_or_else(|| ibis_formats::Error::new("Invalid Excel XML Spreadsheet 2003")),
            Self::OpenDocument(format) => ibis_formats::open(format.to_ooxml(data)?),
        }
    }

    pub(crate) fn save_extension(self) -> &'static str {
        match self {
            Self::Hancom(_) | Self::Uot | Self::Epub | Self::WordPerfect | Self::OpenDocument(ibis_odf::Format::Text) => "docx",
            Self::OpenDocument(ibis_odf::Format::Spreadsheet) | Self::XmlSpreadsheet => "xlsx",
            Self::OpenDocument(ibis_odf::Format::Presentation) => "pptx",
        }
    }
}

/// Imports displayed through a preview instead of an editable OOXML source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewImport {
    FixedLayout(FixedLayoutFormat),
    /// The JTD document model can subsequently be exported or reopened as DOCX.
    Jtd,
}

impl PreviewImport {
    pub(crate) fn detect(extension: Option<&str>, data: &[u8]) -> Option<Self> {
        if let Some(format) = FixedLayoutFormat::detect(extension, data) {
            Some(Self::FixedLayout(format))
        } else if extension.is_some_and(ibis_formats::jtd::is_extension) || ibis_formats::jtd::is_jtd(data) {
            Some(Self::Jtd)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FixedLayoutFormat {
    Pdf,
    Ofd,
    Xps,
    Xdw,
    Vsdx,
}

impl FixedLayoutFormat {
    pub(crate) fn detect(extension: Option<&str>, data: &[u8]) -> Option<Self> {
        let extension = extension.map(str::to_ascii_lowercase);
        if matches!(extension.as_deref(), Some("xdw" | "xbd")) || ibis_xdw::is_xdw(data) {
            return Some(Self::Xdw);
        }
        if data.starts_with(b"%PDF-") {
            return Some(Self::Pdf);
        }
        if ibis_ofd::is_ofd(data) {
            return Some(Self::Ofd);
        }
        if ibis_xps::is_xps(data) {
            return Some(Self::Xps);
        }
        if ibis_vsdx::is_vsdx(data) {
            return Some(Self::Vsdx);
        }
        match extension.as_deref() {
            Some("vsdx") => Some(Self::Vsdx),
            Some("pdf") => Some(Self::Pdf),
            Some("ofd") => Some(Self::Ofd),
            Some("xps" | "oxps") => Some(Self::Xps),
            _ => None,
        }
    }

    pub(crate) fn read(self, raw: std::sync::Arc<[u8]>) -> Result<ibis_doc_model::Presentation, String> {
        let pages = match self {
            Self::Pdf => {
                let mut container = ibis_pdf::PdfContainer::from_bytes(raw.clone()).map_err(|e| e.to_string())?;
                ibis_pdf::scene::collect_pages(&mut container).map_err(|e| e.to_string())?
            }
            Self::Ofd => ibis_ofd::OfdContainer::from_bytes(raw.clone()).map_err(|e| e.to_string())?.page_infos(),
            Self::Xps => ibis_xps::XpsContainer::from_bytes(raw.clone()).map_err(|e| e.to_string())?.page_infos(),
            Self::Xdw => ibis_xdw::XdwContainer::from_bytes(raw.clone()).map_err(|e| e.to_string())?.page_infos(),
            Self::Vsdx => ibis_vsdx::VsdxContainer::from_bytes(raw.clone()).map_err(|e| e.to_string())?.page_infos(),
        };
        let page_boxes: Vec<_> = pages.iter().map(|page| (page.crop_box.width(), page.crop_box.height())).collect();
        Ok(fixed_layout(raw, &page_boxes))
    }
}

fn fixed_layout(raw: std::sync::Arc<[u8]>, page_boxes: &[(f32, f32)]) -> ibis_doc_model::Presentation {
    let first_size = page_boxes
        .first()
        .map(|&(w, h)| ibis_doc_model::EmuSize::new((w as i64) * ibis_doc_model::EMU_PER_POINT, (h as i64) * ibis_doc_model::EMU_PER_POINT))
        .unwrap_or_else(|| ibis_doc_model::EmuSize::new(595 * ibis_doc_model::EMU_PER_POINT, 842 * ibis_doc_model::EMU_PER_POINT));
    let mut presentation = ibis_doc_model::Presentation::empty(first_size);
    for (w, h) in page_boxes {
        let mut slide = ibis_doc_model::Slide::new(presentation.new_slide_id());
        slide.size = Some(ibis_doc_model::EmuSize::new(
            (*w as i64) * ibis_doc_model::EMU_PER_POINT,
            (*h as i64) * ibis_doc_model::EMU_PER_POINT,
        ));
        presentation.slides.push(slide);
    }
    presentation.pdf_bytes = Some(raw);
    presentation
}

#[cfg(not(target_family = "wasm"))]
fn read_file_unguarded(path: &std::path::Path, password: Option<&str>, csv_encoding: ibis_formats::csv::TextEncoding) -> Result<Loaded, String> {
    timing::mark_read_start();
    let read_t0 = crate::clock::Instant::now();
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    read_file_data(path, password, csv_encoding, data, read_t0)
}

/// Validate a citation against the same snapshot passed to the editor's reader.
#[cfg(not(target_family = "wasm"))]
pub fn read_file_at_ai_citation(path: &std::path::Path, query_path: &std::path::Path) -> Result<(Loaded, u32), String> {
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("docx")) {
        return Err("Opening an AI citation requires a .docx document".into());
    }
    let path = path.to_owned();
    let query_path = query_path.to_owned();
    std::thread::Builder::new().name("file-read".into()).stack_size(8 * 1024 * 1024)
        .spawn(move || catch_read_panic(&path, || {
            timing::mark_read_start();
            let read_t0 = crate::clock::Instant::now();
            let data = std::fs::read(&path).map_err(|e| e.to_string())?;
            let query = std::fs::read(&query_path).map_err(|e| e.to_string())?;
            let query = serde_json::from_slice(&query).map_err(|e| e.to_string())?;
            let paragraph = ibis_formats::markdown::locate_ai_paragraph(&data, &query).map_err(|e| e.to_string())?;
            let loaded = read_file_data(&path, None, ibis_formats::csv::TextEncoding::Auto, data, read_t0)?;
            Ok((loaded, paragraph))
        }))
        .map_err(|error| error.to_string())?
        .join().map_err(|_| "File reader thread failed".to_string())?
}

#[cfg(not(target_family = "wasm"))]
fn read_file_data(
    path: &std::path::Path,
    password: Option<&str>,
    csv_encoding: ibis_formats::csv::TextEncoding,
    mut data: Vec<u8>,
    read_t0: crate::clock::Instant,
) -> Result<Loaded, String> {
    let extension = path.extension().and_then(|ext| ext.to_str());
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
        && let Some((text, needs_tree)) = linkage_csv(&data)
    {
        if needs_tree || path.with_extension("definition.json").is_file() {
            return Err(LINKAGE_CSV_NEEDS_TREE.into());
        }
        let encoding = if data.starts_with(b"\xef\xbb\xbf") {
            ibis_formats::csv::OriginalEncoding::Utf8Bom
        } else {
            ibis_formats::csv::OriginalEncoding::Utf8
        };
        ibis_formats::csv::raw_csv_record_ranges(&data, encoding, ',', 0)
            .map_err(|error| error.to_string())?;
        let font = crate::settings_dialog::csv_font();
        let workbook = ibis_formats::csv::workbook_from_literal_fields(text, "Sheet1", font.as_ref())
            .map_err(|error| error.to_string())?;
        let opened = ibis_formats::open(workbook).map_err(|error| error.to_string())?;
        return Ok(read_only_preview(opened, None));
    }
    let preview_import = PreviewImport::detect(extension, &data);
    if let Some(PreviewImport::FixedLayout(format)) = preview_import {
        let presentation = format.read(data.into())?;
        let blank = ibis_formats::blank_file(ibis_formats::Kind::Document).map_err(|e| e.to_string())?;
        let source = ibis_formats::open(blank).map_err(|e| e.to_string())?.source;
        return Ok(Loaded { presentation, source: std::sync::Arc::new(source), password: None, standard_encryption: false, editable_import: None, import_warning: None, csv_stream: None, jtd_original: None, jtd_gallery: false, structured_preview: false, structured_bytes: None, bom_source: None, exr_exposure: None, image_provenance: None });
    }
    let extension = path.extension().and_then(|e| e.to_str());
    if let Some((opened, exr_exposure)) = ibis_formats::production_image::open_with_exposure(extension, &data)? {
        let image_provenance = extension.and_then(|ext| crate::image_provenance::inspect(ext, &data));
        return Ok(load_production_image(opened, exr_exposure, image_provenance));
    }
    if preview_import == Some(PreviewImport::Jtd) {
        let (presentation, jtd_gallery) = crate::jtd_preview::read(&data)?;
        let blank = ibis_formats::blank_file(ibis_formats::Kind::Document).map_err(|e| e.to_string())?;
        let source = ibis_formats::open(blank).map_err(|e| e.to_string())?.source;
        crate::render::preload::Preload::start_for_next(&presentation);
        return Ok(Loaded { presentation, source: std::sync::Arc::new(source), password: None, standard_encryption: false, editable_import: None, import_warning: None, csv_stream: None, jtd_original: Some(data), jtd_gallery, structured_preview: false, structured_bytes: None, bom_source: None, exr_exposure: None, image_provenance: None });
    }
    let standard_encryption = ibis_formats::ibis_ooxml::encryption::is_standard_encrypted(&data);
    let read_fs_ms = read_t0.elapsed().as_secs_f64() * 1000.0;
    timing::mark("read");
    let mut used = None;
    if ibis_formats::is_encrypted(&data) {
        let Some(password) = password else { return Err(NEEDS_PASSWORD.to_string()) };
        data = ibis_formats::decrypt(&data, password).map_err(|e| if e == ibis_formats::DecryptError::WrongPassword { WRONG_PASSWORD.to_string() } else { e.to_string() })?;
        used = Some(password.to_string());
    }
    let open_t0 = crate::clock::Instant::now();
    // An old binary file (.xls, .ppt, .doc): read by its content, not by its extension.
    let is_legacy_binary = ibis_formats::is_legacy_binary(&data);
    let mut opened_import = None;
    let mut bom_source = None;
    let file_name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
    let common = ibis_calc::eval::convert::with_opening_file(Some(path.to_string_lossy().into_owned()), || {
        open_document_bytes(&file_name, path.parent(), &data)
    }).map_err(|e| {
        note_open_failed(path);
        e
    })?;
    let mut structured_bytes = None;
    let (opened, csv_stream) = match common {
        Some(CommonOpen::StructuredPreview(opened)) => {
            return Ok(read_only_preview(opened, Some(std::sync::Arc::new(data))));
        }
        Some(CommonOpen::Standard { opened, editable_import, bom_format, .. }) => {
            opened_import = editable_import;
            let is_md = extension.is_some_and(|e| {
                e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown")
            });
            if bom_format.is_some() || is_md {
                structured_bytes = Some(std::sync::Arc::new(data.clone()));
            }
            if bom_format == Some(ibis_formats::sbom_review::ReportFormat::CycloneDx) {
                use sha2::{Digest, Sha256};
                bom_source = Some(BomSource {
                    path: path.to_owned(),
                    digest: Sha256::digest(&data).into(),
                    has_crypto_assets: ibis_formats::sbom_review::is_cbom_inventory(&opened.presentation),
                });
            }
            (opened, None)
        }
        None => {
            // The volatile cells calculated again on open (CELL("filename") among them) need the file's path.
            ibis_calc::eval::convert::with_opening_file(Some(path.to_string_lossy().into_owned()), || {
                ibis_formats::open_progressive_with_options(data, 100_000, crate::settings_dialog::csv_font().as_ref(), csv_encoding)
            })
            .map_err(|e| {
                note_open_failed(path);
                e.to_string()
            })?
        }
    };
    for (_, driver) in crate::format::drivers() {
        driver.note_opened(path);
    }
    let mut opened = opened;
    opened.presentation.opened_from_legacy = is_legacy_binary;
    let open_ms = open_t0.elapsed().as_secs_f64() * 1000.0;
    timing::mark("parsed");
    let total_file_ms = read_t0.elapsed().as_secs_f64() * 1000.0;
    eprintln!("ibisOffice timing: read_file total: {total_file_ms:.1} ms (fs::read: {read_fs_ms:.1} ms, open: {open_ms:.1} ms)");
    crate::render::preload::Preload::start_for_next(&opened.presentation);
    Ok(Loaded { presentation: opened.presentation, source: std::sync::Arc::new(opened.source), password: used, csv_stream, standard_encryption, editable_import: opened_import, import_warning: opened.import_warning, jtd_original: None, jtd_gallery: false, structured_preview: false, structured_bytes, bom_source, exr_exposure: None, image_provenance: None })
}

/// Tell every format that the file at `path` could not be parsed (in the order of their registration, as they were told
/// before the shell was a crate of its own: the workbook's, then the document's).
#[cfg(not(target_family = "wasm"))]
fn note_open_failed(path: &std::path::Path) {
    for (_, driver) in crate::format::drivers() {
        driver.note_open_failed(path);
    }
}

/// Let the queries of a workbook read the files their sources name (`Excel.Workbook(File.Contents(path))`): the model has no file reader of
/// its own, so it is given this one (the app's, which reads every format). Called once at start; the browser reads no files.
#[cfg(not(target_family = "wasm"))]
pub fn register_query_workbook_loader() {
    ibis_doc_model::query::set_workbook_loader(|path| read_file(std::path::Path::new(path)).map(|loaded| loaded.presentation));
}
