// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Reading a file for editing (on a thread of its own), with its password if it has one.

#[cfg(not(target_family = "wasm"))]
use crate::timing;

/// A file read for editing: its model, and what saving it needs.
#[cfg(not(target_family = "wasm"))]
pub struct Loaded {
    pub presentation: ibis_doc_model::Presentation,
    pub source: std::sync::Arc<ibis_formats::Source>,
    /// The password a protected file was opened with: saving protects it again.
    pub password: Option<String>,
    pub standard_encryption: bool,
    pub jtd_original: Option<Vec<u8>>,
    pub jtd_gallery: bool,
    pub structured_preview: bool,
    pub(crate) image_provenance: Option<crate::image_provenance::Report>,
    pub(crate) exr_exposure: Option<ibis_formats::production_image::ExrExposure>,
    pub(crate) csv_stream: Option<std::sync::mpsc::Receiver<ibis_formats::CsvProgressiveChunk>>,
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

pub(crate) enum FixedLayoutFormat {
    Pdf,
    Ofd,
    Xps,
    Xdw,
}

impl FixedLayoutFormat {
    pub(crate) fn detect(extension: Option<&str>, data: &[u8]) -> Option<Self> {
        let extension = extension.map(str::to_ascii_lowercase);
        if extension.as_deref() == Some("xdw") || ibis_xdw::is_xdw(data) {
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
        match extension.as_deref() {
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
    if let Some(format) = FixedLayoutFormat::detect(extension, &data) {
        let presentation = format.read(data.into())?;
        let blank = ibis_formats::blank_file(ibis_formats::Kind::Document).map_err(|e| e.to_string())?;
        let source = ibis_formats::open(blank).map_err(|e| e.to_string())?.source;
        return Ok(Loaded { presentation, source: std::sync::Arc::new(source), password: None, standard_encryption: false, csv_stream: None, jtd_original: None, jtd_gallery: false, structured_preview: false, exr_exposure: None, image_provenance: None });
    }
    let extension = path.extension().and_then(|e| e.to_str());
    if let Some((opened, exr_exposure)) = ibis_formats::production_image::open_with_exposure(extension, &data)? {
        let image_provenance = extension.and_then(|ext| crate::image_provenance::inspect(ext, &data));
        crate::render::preload::Preload::start_for_next(&opened.presentation);
        return Ok(Loaded { presentation: opened.presentation, source: std::sync::Arc::new(opened.source), password: None, standard_encryption: false, csv_stream: None, jtd_original: None, jtd_gallery: false, structured_preview: false, exr_exposure, image_provenance });
    }
    if path.extension().and_then(|e| e.to_str()).is_some_and(ibis_formats::jtd::is_extension) || ibis_formats::jtd::is_jtd(&data) {
        let (presentation, jtd_gallery) = crate::jtd_preview::read(&data)?;
        let blank = ibis_formats::blank_file(ibis_formats::Kind::Document).map_err(|e| e.to_string())?;
        let source = ibis_formats::open(blank).map_err(|e| e.to_string())?.source;
        crate::render::preload::Preload::start_for_next(&presentation);
        return Ok(Loaded { presentation, source: std::sync::Arc::new(source), password: None, standard_encryption: false, csv_stream: None, jtd_original: Some(data), jtd_gallery, structured_preview: false, exr_exposure: None, image_provenance: None });
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
    let external_document_format = ibis_formats::docx_convert::DocxConvertFormat::from_path(path);
    let open_result = if let Some(law_md) = ibis_formats::lawtext::path_input_to_markdown(path, &data).map_err(|e| {
        note_open_failed(path);
        e.to_string()
    })? {
        ibis_formats::markdown::open_markdown_with_base_dir(law_md.into_bytes(), path.parent()).map(|opened| (opened, None))
    } else if let Some(dicom_md) = ibis_formats::dicom::input_to_markdown(&data).map_err(|e| {
        note_open_failed(path);
        e.to_string()
    })? {
        ibis_formats::markdown::open_markdown_with_base_dir(dicom_md.into_bytes(), path.parent()).map(|opened| (opened, None))
    } else if let Some(fhir_md) = ibis_formats::fhir::input_to_markdown(&data).map_err(|e| {
        note_open_failed(path);
        e.to_string()
    })? {
        ibis_formats::markdown::open_markdown_with_base_dir(fhir_md.into_bytes(), path.parent()).map(|opened| (opened, None))
    } else if let Some(extension) = extension.filter(|ext| ibis_formats::structured_preview::supports_extension(ext)) {
        let opened = crate::structured_preview::open(extension, &data)?;
        crate::render::preload::Preload::start_for_next(&opened.presentation);
        return Ok(Loaded {
            presentation: opened.presentation, source: std::sync::Arc::new(opened.source),
            password: None, standard_encryption: false, csv_stream: None,
            jtd_original: None, jtd_gallery: false, structured_preview: true,
            exr_exposure: None, image_provenance: None,
        });
    } else if external_document_format == Some(ibis_formats::docx_convert::DocxConvertFormat::Markdown) {
        if ibis_formats::markdown::is_presentation_markdown(&data) {
            ibis_formats::markdown::open_markdown_presentation_with_base_dir(data, path.parent()).map(|opened| (opened, None))
        } else {
            ibis_formats::markdown::open_markdown_with_base_dir(data, path.parent()).map(|opened| (opened, None))
        }
    } else if let Some(format) = external_document_format {
        (if format == ibis_formats::docx_convert::DocxConvertFormat::Odt { ibis_odf::import_odt(&data) } else { ibis_formats::docx_convert::import_docx_format(&data, format) }).map(|opened| (opened, None))
    } else {
        // The volatile cells calculated again on open (CELL("filename") among them) need the file's path.
        ibis_calc::eval::convert::with_opening_file(Some(path.to_string_lossy().into_owned()), || {
            let data = ibis_odf::to_ooxml(&data)?.unwrap_or(data);
            ibis_formats::open_progressive_with_options(data, 100_000, crate::settings_dialog::csv_font().as_ref(), csv_encoding)
        })
    };
    let (opened, csv_stream) = open_result.map_err(|e| {
        note_open_failed(path);
        e.to_string()
    })?;
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
    Ok(Loaded { presentation: opened.presentation, source: std::sync::Arc::new(opened.source), password: used, csv_stream, standard_encryption, jtd_original: None, jtd_gallery: false, structured_preview: false, exr_exposure: None, image_provenance: None })
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
