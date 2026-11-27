// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Modal dialog for downloading and managing optional NDLOCR-Lite v2 models
//! (modeled on [`super::font_setup_dialog`]).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use ibis_ocr::models::{CORE_MODELS, MODEL_LICENSE_NAME};

use super::chrome;
use super::editor::SlideEditor;
use super::font_setup::{self, DownloadError};
use super::i18n::text;
use super::ocr_setup;

const MEMORY_KEY: &str = "ocr-setup.dialog";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Downloading {
        index: usize,
        count: usize,
        name: String,
        total: u64,
    },
    Done,
}

struct OcrSetupState {
    phase: Phase,
    message: Option<SharedString>,
    received: u64,
    sidebar: bool,
    cancel: Arc<AtomicBool>,
}

type Shared = Rc<RefCell<OcrSetupState>>;

impl SlideEditor {
    pub(crate) fn ocr_setup_is_open(&self) -> bool {
        self.ocr_setup_state().is_some()
    }

    fn ocr_setup_state(&self) -> Option<Shared> {
        self.document_memory.get::<Option<Shared>>(MEMORY_KEY).flatten()
    }

    /// Help > OCR モデルのセットアップ
    pub(crate) fn open_ocr_setup(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.ocr_setup_state().is_some() {
            return;
        }
        let state = OcrSetupState {
            phase: Phase::Idle,
            message: None,
            received: 0,
            sidebar: self.sidebar_open,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        self.document_memory.set(MEMORY_KEY, Some(Rc::new(RefCell::new(state))));
        self.sidebar_open = true;
        self.dialog = Default::default();
        cx.notify();
    }

    pub(crate) fn close_ocr_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = self.ocr_setup_state() {
            self.sidebar_open = state.borrow().sidebar;
        }
        self.ocr_setup_discard();
        self.dialog = Default::default();
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn ocr_setup_discard(&mut self) {
        if let Some(state) = self.ocr_setup_state() {
            state.borrow().cancel.store(true, Ordering::SeqCst);
        }
        self.document_memory.set(MEMORY_KEY, None::<Shared>);
    }

    fn ocr_setup_cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.ocr_setup_state() {
            state.borrow().cancel.store(true, Ordering::SeqCst);
        }
        cx.notify();
    }

    fn ocr_setup_remove(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.ocr_setup_state() else {
            return;
        };
        match ocr_setup::remove_models() {
            Ok(()) => {
                state.borrow_mut().message = Some(text("shell.ocr-setup.removed"));
            }
            Err(err) => {
                state.borrow_mut().message =
                    Some(SharedString::from(crate::i18n::format("shell.ocr-setup.failed", &[("error", &err)])));
            }
        }
        cx.notify();
    }

    fn ocr_setup_install(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.ocr_setup_state() else {
            return;
        };
        let missing = ocr_setup::missing_models();
        if missing.is_empty() {
            state.borrow_mut().phase = Phase::Done;
            state.borrow_mut().message = Some(text("shell.ocr-setup.done"));
            cx.notify();
            return;
        }

        let cancel = {
            let mut s = state.borrow_mut();
            s.cancel.store(false, Ordering::SeqCst);
            s.message = None;
            s.received = 0;
            s.phase = Phase::Downloading {
                index: 1,
                count: missing.len(),
                name: missing[0].filename.to_string(),
                total: missing[0].approx_size_bytes,
            };
            s.cancel.clone()
        };
        cx.notify();

        let total_count = missing.len();
        let names: Vec<(&'static str, u64)> =
            missing.iter().map(|m| (m.filename, m.approx_size_bytes)).collect();

        cx.spawn(async move |this, cx| {
            let current_index = Arc::new(AtomicUsize::new(1));
            let progress_bytes = Arc::new(AtomicU64::new(0));
            let (sender, receiver) = std::sync::mpsc::channel();
            {
                let (idx, prog, cancel) = (current_index.clone(), progress_bytes.clone(), cancel.clone());
                std::thread::spawn(move || {
                    sender.send(ocr_setup::download_missing(&idx, &prog, &cancel)).ok();
                });
            }

            let result = loop {
                cx.background_executor().timer(Duration::from_millis(120)).await;
                let idx = current_index.load(Ordering::SeqCst).clamp(1, total_count);
                let received = progress_bytes.load(Ordering::SeqCst);
                let (name, total) = names[idx - 1];
                let alive = this.update(cx, |this, cx| {
                    if let Some(state) = this.ocr_setup_state() {
                        let mut s = state.borrow_mut();
                        s.received = received;
                        s.phase = Phase::Downloading {
                            index: idx,
                            count: total_count,
                            name: name.to_string(),
                            total,
                        };
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    cancel.store(true, Ordering::SeqCst);
                }
                match receiver.try_recv() {
                    Ok(res) => break res,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        break Err(DownloadError::Failed("the download thread ended".into()));
                    }
                }
            };

            this.update(cx, |this, cx| {
                if let Some(state) = this.ocr_setup_state() {
                    let mut s = state.borrow_mut();
                    match result {
                        Ok(()) => {
                            s.phase = Phase::Done;
                            s.message = Some(text("shell.ocr-setup.done"));
                        }
                        Err(DownloadError::Cancelled) => {
                            s.phase = Phase::Idle;
                            s.message = Some(text("shell.ocr-setup.cancelled"));
                        }
                        Err(DownloadError::Failed(err)) => {
                            s.phase = Phase::Idle;
                            s.message = Some(SharedString::from(crate::i18n::format(
                                "shell.ocr-setup.failed",
                                &[("error", &err)],
                            )));
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn ocr_setup_dialog(&self, cx: &mut Context<Self>) -> Option<chrome::SidebarPanel> {
        let state = self.ocr_setup_state()?;
        let (phase, message, received) = {
            let s = state.borrow();
            (s.phase.clone(), s.message.clone(), s.received)
        };

        let is_ja = crate::i18n::language() == ibis_office_base::i18n::Language::Japanese;
        let missing = ocr_setup::missing_models();
        let all_installed = missing.is_empty();
        let missing_names: std::collections::HashSet<&'static str> =
            missing.iter().map(|m| m.filename).collect();

        let total_size = font_setup::size_text(ocr_setup::total_model_bytes());
        let heading = SharedString::from(crate::i18n::format(
            "shell.ocr-setup.heading",
            &[("size", &total_size)],
        ));
        let license_note = SharedString::from(crate::i18n::format(
            "shell.ocr-setup.license",
            &[("license", &MODEL_LICENSE_NAME)],
        ));

        let mut model_rows = v_flex().gap_1().w_full();
        for spec in CORE_MODELS {
            let installed = !missing_names.contains(spec.filename);
            let desc = if is_ja { spec.description_ja } else { spec.description_en };
            let size = font_setup::size_text(spec.approx_size_bytes);
            let status_label = if installed {
                text("shell.ocr-setup.installed")
            } else {
                text("shell.ocr-setup.not-installed")
            };
            model_rows = model_rows.child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .bg(rgb(0xf1f3f4))
                    .child(
                        v_flex()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(desc))
                            .child(div().text_xs().text_color(chrome::text_secondary()).child(format!("{} ({size})", spec.filename))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if installed { rgb(0x1a73e8).into() } else { chrome::text_secondary() })
                            .child(status_label),
                    ),
            );
        }

        let (ndl_license_url, dinov3_license_url) = ocr_setup::license_urls();
        let license_links = h_flex()
            .gap_3()
            .child(
                div()
                    .id("ocr-license-ndl")
                    .text_xs()
                    .text_color(rgb(0x1a73e8))
                    .cursor_pointer()
                    .child("NDLOCR-Lite (CC BY 4.0)")
                    .on_click(cx.listener(move |_, _, _, cx| crate::external::open_url(ndl_license_url, cx))),
            )
            .child(
                div()
                    .id("ocr-license-dinov3")
                    .text_xs()
                    .text_color(rgb(0x1a73e8))
                    .cursor_pointer()
                    .child("DINOv3 License")
                    .on_click(cx.listener(move |_, _, _, cx| crate::external::open_url(dinov3_license_url, cx))),
            );

        let status = match &phase {
            Phase::Downloading {
                index,
                count,
                name,
                total,
            } => {
                let received = font_setup::size_text(received.max(1));
                let total = font_setup::size_text(*total);
                Some(SharedString::from(crate::i18n::format(
                    "shell.ocr-setup.progress",
                    &[
                        ("name", name),
                        ("index", index),
                        ("count", count),
                        ("received", &received),
                        ("total", &total),
                    ],
                )))
            }
            _ => message,
        };

        let body = v_flex()
            .gap_2()
            .w_full()
            .child(div().id("ocr-setup-heading").text_sm().font_weight(FontWeight::MEDIUM).child(heading))
            .child(model_rows)
            .child(div().text_xs().text_color(chrome::text_secondary()).child(license_note))
            .child(license_links)
            .children(status.map(|m| div().id("ocr-setup-status").text_sm().child(m)));

        let mut actions = chrome::action_bar();
        match phase {
            Phase::Downloading { .. } => {
                actions = actions.action(
                    chrome::ActionDefinition::new(
                        "ocr-setup-cancel",
                        text("shell.ocr-setup.cancel"),
                        chrome::ActionButtonKind::Normal,
                    )
                    .on_click(cx.listener(|e, _, _, cx| e.ocr_setup_cancel(cx))),
                );
            }
            Phase::Idle if !all_installed => {
                actions = actions
                    .action(
                        chrome::ActionDefinition::new(
                            "ocr-setup-install",
                            text("shell.ocr-setup.install"),
                            chrome::ActionButtonKind::Primary,
                        )
                        .on_click(cx.listener(|e, _, _, cx| e.ocr_setup_install(cx))),
                    )
                    .action(
                        chrome::ActionDefinition::new(
                            "ocr-setup-close",
                            text("_shell.close"),
                            chrome::ActionButtonKind::Normal,
                        )
                        .on_click(cx.listener(|e, _, w, cx| e.close_ocr_setup(w, cx))),
                    );
            }
            Phase::Idle if all_installed => {
                actions = actions
                    .action(
                        chrome::ActionDefinition::new(
                            "ocr-setup-remove",
                            text("shell.ocr-setup.remove"),
                            chrome::ActionButtonKind::Normal,
                        )
                        .on_click(cx.listener(|e, _, _, cx| e.ocr_setup_remove(cx))),
                    )
                    .action(
                        chrome::ActionDefinition::new(
                            "ocr-setup-close",
                            text("_shell.close"),
                            chrome::ActionButtonKind::Primary,
                        )
                        .on_click(cx.listener(|e, _, w, cx| e.close_ocr_setup(w, cx))),
                    );
            }
            _ => {
                actions = actions.action(
                    chrome::ActionDefinition::new(
                        "ocr-setup-close",
                        text("_shell.close"),
                        chrome::ActionButtonKind::Primary,
                    )
                    .on_click(cx.listener(|e, _, w, cx| e.close_ocr_setup(w, cx))),
                );
            }
        }

        Some(chrome::SidebarPanel::with_footer(
            text("shell.ocr-setup.title"),
            body.into_any_element(),
            actions,
        ))
    }
}
