// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! GPUI PDF viewer state and rendering bridge over [`ibis_pdf`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui_kit::{App, Bounds, Pixels, Point, RenderImage, Window, px, size};
use ibis_ofd::OfdContainer;
use ibis_pdf::interpret::interpret_page_scene;
use ibis_pdf::scene::{PageInfo, PageScene, collect_pages};
use ibis_pdf::signature::{SignatureRecord, inspect_signatures};
use ibis_pdf::{PdfContainer, Result};

pub mod editor;
pub mod render;
pub use editor::PdfDocument;

enum ViewerSource {
    Empty,
    Pdf(Arc<Mutex<PdfContainer>>),
    Ofd(Arc<OfdContainer>),
}

/// Viewer state for an open PDF or OFD document (lazy page scene & GPU image cache).
pub struct PdfViewerState {
    source: ViewerSource,
    pub pages: Vec<PageInfo>,
    pub signatures: Vec<SignatureRecord>,
    ofd_signature_status: Option<String>,
    scene_cache: HashMap<usize, Arc<PageScene>>,
    image_cache: HashMap<(usize, usize), Option<Arc<RenderImage>>>,
    pub zoom: f32,
    pub current_page: usize,
}

impl PdfViewerState {
    /// Empty viewer state used as a non-panicking placeholder while loading.
    pub fn empty() -> Self {
        Self {
            source: ViewerSource::Empty,
            pages: Vec::new(),
            signatures: Vec::new(),
            ofd_signature_status: None,
            scene_cache: HashMap::new(),
            image_cache: HashMap::new(),
            zoom: 1.0,
            current_page: 0,
        }
    }

    /// Open a PDF or OFD container, resolving page metadata and signature counts
    /// without eagerly interpreting page content streams.
    pub fn open(bytes: impl Into<Arc<[u8]>>) -> Result<Self> {
        let raw: Arc<[u8]> = bytes.into();
        if ibis_ofd::is_ofd(&raw) {
            let ofd = OfdContainer::from_bytes(raw).map_err(|e| ibis_pdf::Error::new(e.to_string()))?;
            let pages = ofd.page_infos();
            let ofd_signature_status = ofd.signature_status_text();
            return Ok(Self {
                source: ViewerSource::Ofd(Arc::new(ofd)),
                pages,
                signatures: Vec::new(),
                ofd_signature_status,
                scene_cache: HashMap::new(),
                image_cache: HashMap::new(),
                zoom: 1.0,
                current_page: 0,
            });
        }
        let mut container = PdfContainer::from_bytes(raw)?;
        let pages = collect_pages(&mut container)?;
        let signatures = inspect_signatures(&mut container)?;
        Ok(Self {
            source: ViewerSource::Pdf(Arc::new(Mutex::new(container))),
            pages,
            signatures,
            ofd_signature_status: None,
            scene_cache: HashMap::new(),
            image_cache: HashMap::new(),
            zoom: 1.0,
            current_page: 0,
        })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn signature_status_text(&self) -> Option<String> {
        if let Some(status) = &self.ofd_signature_status {
            return Some(status.clone());
        }
        if self.signatures.is_empty() {
            None
        } else {
            Some(format!("署名: {}件", self.signatures.len()))
        }
    }

    /// Lazily interpret and cache `page_index`.
    pub fn page_scene(&mut self, page_index: usize) -> Result<Arc<PageScene>> {
        if let Some(scene) = self.scene_cache.get(&page_index) {
            return Ok(scene.clone());
        }
        let scene = match &self.source {
            ViewerSource::Empty => return Err(ibis_pdf::Error::new("empty pdf viewer")),
            ViewerSource::Pdf(container) => {
                let mut guard = container.lock().expect("pdf container lock poisoned");
                Arc::new(interpret_page_scene(&mut guard, page_index)?)
            }
            ViewerSource::Ofd(ofd) => Arc::new(
                ofd.interpret_page_scene(page_index)
                    .map_err(|e| ibis_pdf::Error::new(e.to_string()))?,
            ),
        };
        self.scene_cache.insert(page_index, scene.clone());
        Ok(scene)
    }

    /// Paint a single page at `origin` using `scale` (pixels per PDF point).
    pub fn paint_page(
        &mut self,
        page_index: usize,
        origin: Point<Pixels>,
        scale: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        let scene = self.page_scene(page_index)?;
        let bounds = Bounds::new(
            origin,
            size(
                px(scene.info.crop_box.width() * scale),
                px(scene.info.crop_box.height() * scale),
            ),
        );
        render::paint_page_scene(
            page_index,
            &scene,
            bounds,
            scale,
            &mut self.image_cache,
            window,
            cx,
        );
        Ok(())
    }
}
