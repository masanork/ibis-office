// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! GPUI PDF viewer state and rendering bridge over [`ibis_pdf`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui_kit::{Bounds, Pixels, Point, RenderImage, Window, px, size};
use ibis_pdf::interpret::interpret_page_scene;
use ibis_pdf::scene::{PageInfo, PageScene, collect_pages};
use ibis_pdf::signature::{SignatureRecord, inspect_signatures};
use ibis_pdf::{PdfContainer, Result};

pub mod render;

/// Viewer state for an open PDF document (lazy page scene & GPU image cache).
pub struct PdfViewerState {
    container: Arc<Mutex<PdfContainer>>,
    pub pages: Vec<PageInfo>,
    pub signatures: Vec<SignatureRecord>,
    scene_cache: HashMap<usize, Arc<PageScene>>,
    image_cache: HashMap<(usize, usize), Option<Arc<RenderImage>>>,
    pub zoom: f32,
    pub current_page: usize,
}

impl PdfViewerState {
    /// Open a PDF container, resolving the page tree and signature metadata without
    /// eagerly interpreting page content streams.
    pub fn open(bytes: impl Into<Arc<[u8]>>) -> Result<Self> {
        let mut container = PdfContainer::from_bytes(bytes)?;
        let pages = collect_pages(&mut container)?;
        let signatures = inspect_signatures(&mut container)?;
        Ok(Self {
            container: Arc::new(Mutex::new(container)),
            pages,
            signatures,
            scene_cache: HashMap::new(),
            image_cache: HashMap::new(),
            zoom: 1.0,
            current_page: 0,
        })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Lazily interpret and cache `page_index`.
    pub fn page_scene(&mut self, page_index: usize) -> Result<Arc<PageScene>> {
        if let Some(scene) = self.scene_cache.get(&page_index) {
            return Ok(scene.clone());
        }
        let scene = {
            let mut guard = self.container.lock().expect("pdf container lock poisoned");
            Arc::new(interpret_page_scene(&mut guard, page_index)?)
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
        );
        Ok(())
    }
}
