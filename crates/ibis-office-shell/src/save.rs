// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! ファイル > 保存 (⌘S) and 名前を付けて保存… (⌘⇧S).
//!
//! The file is written by `ibis_formats::Source::save`: the original file with
//! the edits made since it was opened written into it (see
//! `ibis-formats/src/save/mod.rs`). An edit the format cannot save yet is
//! reported in the status bar and nothing is written, so no edit is lost
//! silently. The file is written next to its destination and then moved
//! over it, so a failed save never leaves a half-written file.

use std::borrow::Cow;

use gpui_kit::*;
use ibis_doc_model::Presentation;

use crate::editor::SlideEditor;
use crate::actions::{Save, SaveAs};

/// What a refused save was asked as (`SlideEditor::refuse_as`): the words of the status differ.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefusedWay {
    Save,
    Export,
    Copy,
    Print,
}

impl SlideEditor {
    /// Prepare a same-file CSV save only when the source and tracked sheet
    /// are still the ones that were opened or last saved.
    #[cfg(not(target_family = "wasm"))]
    fn tracked_csv_bytes(&self, path: &std::path::Path) -> Result<Vec<u8>, String> {
        if self.is_csv_streaming { return Err(crate::i18n::text("file.csv-loading-blocked").to_string()); }
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() { return Err(crate::i18n::text("file.csv-error.symlink").to_string()); }
        if !metadata.is_file() { return Err(crate::i18n::text("file.csv-error.not-regular-file").to_string()); }
        if metadata.permissions().readonly() {
            return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).to_string());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() > 1 { return Err(crate::i18n::text("file.csv-error.hardlink").to_string()); }
        }
        let format = self.current_csv_format.ok_or_else(|| crate::i18n::text("file.csv-error.missing-format").to_string())?;
        let original = self.current_csv_original.as_deref().ok_or_else(|| crate::i18n::text("file.csv-error.missing-data").to_string())?;
        let slide = self.current_csv_slide.ok_or_else(|| crate::i18n::text("file.csv-error.missing-sheet").to_string())?;
        if self.current_slide != slide { return Err(crate::i18n::text("file.csv-error.wrong-sheet").to_string()); }
        let disk = std::fs::read(path).map_err(|e| e.to_string())?;
        ibis_formats::csv::rewrite_tracked_csv_checked(original, &disk, format, &self.document, slide)
            .map_err(|e| crate::i18n::text(e.i18n_key()).to_string())
    }
    /// Confirm before saving changes to a package with signatures, including
    /// when signature discovery is malformed or unsupported. A cancelled
    /// confirmation neither writes nor runs the caller's continuation.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn confirm_signature_output(&mut self, way: RefusedWay, window: &mut Window, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        let is_output = matches!(way, RefusedWay::Copy | RefusedWay::Export);
        let signature_state = (self.is_dirty() || is_output).then(|| self.source.as_ref().map(|source| source.has_package_signatures())).flatten();
        let message = match (signature_state, is_output) {
            (Some(Ok(true)), false) => "file.signature-save.message",
            (Some(Err(_)), false) => "file.signature-save.unknown",
            (Some(Ok(true)), true) => "file.signature-output.message",
            (Some(Err(_)), true) => "file.signature-output.unknown",
            _ => { then(self, window, cx); return; }
        };
        let handle = window.window_handle();
        let editor = cx.entity().downgrade();
        let action = std::rc::Rc::new(std::cell::RefCell::new(Some(then)));
        self.ask(crate::i18n::text("file.signature-save.title"), crate::i18n::text(message),
            std::rc::Rc::new(move |_, cx| {
                let Some(then) = action.borrow_mut().take() else { return };
                let editor = editor.clone();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        let _ = editor.update(cx, |this, cx| then(this, window, cx));
                    });
                });
            }), cx);
    }
    /// A copy made while viewing Standard encryption must stay password-protected.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn check_standard_copy_format(&self, path: &std::path::Path) -> Result<(), String> {
        if !self.standard_read_only { return Ok(()); }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let valid = if self.is_document() {
            matches!(ext.as_str(), "docx" | "docm" | "dotx" | "dotm")
        } else if self.is_workbook() {
            matches!(ext.as_str(), "xlsx" | "xlsm" | "xltx" | "xltm")
        } else {
            matches!(ext.as_str(), "pptx" | "pptm" | "ppsx" | "ppsm" | "potx" | "potm")
        };
        if valid { Ok(()) } else { Err(crate::i18n::text("file.standard-copy-format").to_string()) }
    }

    pub fn check_standard_output(&self, path: &std::path::Path) -> std::io::Result<()> {
        if self.is_csv_streaming {
            return Err(std::io::Error::other(crate::i18n::text("file.csv-loading-blocked").to_string()));
        }
        #[cfg(not(target_family = "wasm"))]
        if self.standard_source_path.as_deref().is_some_and(|original| same_file(original, path)) {
            return Err(std::io::Error::other(crate::i18n::text("file.standard-source-protected").to_string()));
        }
        #[cfg(not(target_family = "wasm"))]
        if self.current_csv_format.is_some() && self.path.as_deref().is_some_and(|original| same_file(original, path)) {
            return Err(std::io::Error::other(crate::i18n::text("file.csv-error.export-original").to_string()));
        }
        let _ = path;
        Ok(())
    }
    /// What is typed and not yet entered (a workbook's cell being edited, `FormatEditor::enter_pending_edit`) is entered, then the
    /// shell's own edit (a shape's text) is finished: what a save, a copy or an export that follows writes or shows is the document
    /// as it is now. Every way out of the window that writes the document starts here (⌘S used only `commit_edit`: the cell stayed
    /// open and was not in the saved file).
    pub(crate) fn commit_pending_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::format::with_format!(self, enter_pending_edit(self, cx));
        self.commit_edit(window, cx);
    }

    /// The model the file is written from: the open one, or the screen's copy of it with what it keeps outside the
    /// model put in (`FormatEditor::model_for_save`).
    pub(crate) fn presentation_for_save(&self) -> Cow<'_, Presentation> {
        crate::format::with_format!(self, model_for_save(self))
    }

    /// The format refuses a save for now (a presentation's master view is closed first): told in the status bar, before any
    /// dialog asks where to save (the master view's page is a file of its own with no path, so ⌘S would ask a name first and
    /// refuse only after it). True when refused.
    pub fn refuse_save(&mut self, cx: &mut Context<Self>) -> bool {
        self.refuse_as(RefusedWay::Save, cx)
    }

    /// `refuse_save`, told in the words of what was asked for (saving, exporting, saving a copy, printing).
    pub fn refuse_as(&mut self, way: RefusedWay, cx: &mut Context<Self>) -> bool {
        let refusal = if self.structured_preview {
            Some(crate::i18n::text("structured-preview.read-only").to_string())
        } else if self.is_csv_streaming { Some(crate::i18n::text("file.csv-loading-blocked").to_string()) }
            else { crate::format::with_format!(self, save_refused(self)) };
        match refusal {
            Some(why) => {
                let key = match way {
                    RefusedWay::Save => "file.save-failed",
                    RefusedWay::Export => "file.export-failed",
                    RefusedWay::Copy => "file.copy-failed",
                    RefusedWay::Print => "file.print-failed",
                };
                self.status = Some(crate::i18n::format(key, &[("error", &why)]).into());
                cx.notify();
                true
            }
            None => false,
        }
    }

    /// ⌘S: save to the file it was opened from (or ask where, for a file
    /// that has no path yet).
    pub fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_jtd_read_only() {
            self.set_status(crate::i18n::text("jtd.read-only").to_string(), cx);
            return;
        }
        self.commit_pending_edit(window, cx);
        if self.refuse_save(cx) {
            return;
        }
        // A signed file that was edited: asked first (キャンセル stops here; OK comes back).
        if crate::host::is_connected() && self.confirm_signature_loss(crate::signature::SignatureWay::Save, window, cx, |this, window, cx| this.save(&Save, window, cx)) {
            return;
        }
        // A host that keeps the file takes the save (`saveFile`, or the file window for a document that is not its own).
        if self.host_save(crate::host::SaveTrigger::User, window, cx, &mut None) {
            return;
        }
        #[cfg(not(target_family = "wasm"))]
        self.confirm_signature_output(RefusedWay::Save, window, cx, |this, window, cx| {
            // A file opened read-only for its password to modify is not overwritten.
            match this.path.clone().filter(|_| !this.is_modify_locked() && !this.standard_read_only) {
                Some(path) => { let result = this.save_to(path, window); this.report_save(result, cx); }
                None => this.save_as_then_confirmed(window, cx, |_, _, _| {}),
            }
        });
        #[cfg(target_family = "wasm")]
        self.save_web(false, window, cx, Box::new(|_, _, _| {}));
    }

    /// ⌘⇧S: ask where, and save there.
    pub fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        self.save_as_then(window, cx, |_, _, _| {});
    }

    /// ⌘⇧S, and then, once the file is written, `then`. A cancelled or failed save does not go on to `then`.
    pub(crate) fn save_as_then(&mut self, window: &mut Window, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        if self.is_jtd_read_only() {
            self.set_status(crate::i18n::text("jtd.read-only").to_string(), cx);
            return;
        }
        self.commit_pending_edit(window, cx);
        if self.refuse_save(cx) {
            return;
        }
        // A host that keeps the files: the file window asks where (in its storage), and the bytes go to `saveFileAs`.
        if crate::host::capabilities().list_folder && crate::host::capabilities().save_file_as {
            // A signed file that was edited: asked first; `then` goes on after the answer OK and the save.
            let then = std::rc::Rc::new(std::cell::RefCell::new(Some(then)));
            let later = then.clone();
            if crate::host::is_connected() && self.confirm_signature_loss(crate::signature::SignatureWay::Save, window, cx, move |this, window, cx| {
                if let Some(then) = later.borrow_mut().take() {
                    this.save_as_then(window, cx, then);
                }
            }) {
                return;
            }
            let Some(then) = then.borrow_mut().take() else { return };

                self.open_host_file_dialog(crate::host_file_dialog::Mode::SaveAs, Some(Box::new(then)), window, cx);
                return;
        }
        #[cfg(not(target_family = "wasm"))]
        { self.confirm_signature_output(RefusedWay::Save, window, cx, move |this, window, cx| this.save_as_then_confirmed(window, cx, then)); }
        #[cfg(target_family = "wasm")]
        self.save_web(true, window, cx, Box::new(then));
    }

    #[cfg(target_family = "wasm")]
    fn save_web(&mut self, save_as: bool, window: &mut Window, cx: &mut Context<Self>, then: Box<dyn FnOnce(&mut Self, &mut Window, &mut Context<Self>)>) {
        if self.structured_preview {
            self.set_status(crate::i18n::text("structured-preview.read-only").to_string(), cx);
            return;
        }
        let name = self.file_name.clone().map(|n| n.to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "document".to_string());
        let mime = crate::web::mime_for_file_name(&name).to_string();
        let bytes = match self.host_bytes(window, cx) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.set_status(crate::i18n::format("file.save-failed", &[("error", &error)]), cx);
                return;
            }
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = crate::web::save_local(&name, &mime, &bytes, save_as).await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Some(saved_name)) => {
                        if !saved_name.is_empty() {
                            this.file_name = Some(saved_name.into());
                        }
                        this.document.mark_clean();
                        this.cleanup_active_autorecover_file();
                        this.set_status(crate::i18n::text("_shell.saved").to_string(), cx);
                        then(this, window, cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.set_status(crate::i18n::format("file.save-failed", &[("error", &error)]), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    #[cfg(not(target_family = "wasm"))]
    fn save_as_then_confirmed(&mut self, window: &mut Window, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        if self.source.is_none() {
            self.status = Some(crate::i18n::text("_shell.saving-new-documents-is-not-yet-supported").into());
            return cx.notify();
        }
        #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
        if crate::scenario::button_probe::save_as_caught() {
            return;
        }
        {
            let directory = self.save_dialog_folder();
            let name = self.file_name.clone().unwrap_or_default();
            let picked = cx.prompt_for_new_path_with_formats(&directory, Some(&name), &self.save_formats());
            cx.spawn_in(window, async move |this, cx| {
                // The dialog failing to open is told in a message box (not taken for a cancel).
                let path = match picked.await {
                    Ok(Ok(Some(path))) => path,
                    Ok(Err(error)) => {
                        this.update(cx, |this, cx| this.show_alert(crate::i18n::text("shell.save-failed.title"), error.to_string().into(), cx)).ok();
                        return;
                    }
                    _ => return,
                };
                this.update_in(cx, |this, window, cx| {
                    let remembered = path.clone();
                    let result = this.save_to(path, window);
                    let saved = result.is_ok();
                    if saved {
                        crate::save_location::remember_save_folder(&remembered);
                    }
                    this.report_save(result, cx);
                    if saved {
                        then(this, window, cx);
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// Save (as ⌘S, asking where for a file that has no path yet) and then, once the file is written, `then` (the
    /// save confirmation's 保存: close afterwards). A cancelled or failed save does not go on to `then`. What is being
    /// typed in a cell is put in first, so it is saved too.
    pub fn save_then(&mut self, window: &mut Window, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        if self.is_jtd_read_only() {
            self.set_status(crate::i18n::text("jtd.read-only").to_string(), cx);
            return;
        }
        self.commit_pending_edit(window, cx);
        if self.refuse_save(cx) {
            return;
        }
        // A signed file that was edited: asked first; `then` goes on after the answer OK and the save.
        let then = std::rc::Rc::new(std::cell::RefCell::new(Some(then)));
        let later = then.clone();
        if crate::host::is_connected() && self.confirm_signature_loss(crate::signature::SignatureWay::Save, window, cx, move |this, window, cx| {
            if let Some(then) = later.borrow_mut().take() {
                this.save_then(window, cx, then);
            }
        }) {
            return;
        }
        let Some(then) = then.borrow_mut().take() else { return };
        // A host that keeps the file: `saveFile` (or the file window), and `then` once it accepted.
        let mut then: Option<Box<dyn FnOnce(&mut Self, &mut Window, &mut Context<Self>)>> = Some(Box::new(then));
        if self.host_save(crate::host::SaveTrigger::User, window, cx, &mut then) {
            return;
        }
        let then = then.take().expect("kept when the host did not take the save");
        #[cfg(not(target_family = "wasm"))]
        { self.confirm_signature_output(RefusedWay::Save, window, cx, move |this, window, cx| this.save_then_confirmed(window, cx, then)); }
        #[cfg(target_family = "wasm")]
        self.save_web(false, window, cx, then);
    }

    #[cfg(not(target_family = "wasm"))]
    fn save_then_confirmed(&mut self, window: &mut Window, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        {
            if let Some(path) = self.path.clone().filter(|_| !self.is_modify_locked() && !self.standard_read_only) {
                let result = self.save_to(path, window);
                let saved = result.is_ok();
                self.report_save(result, cx);
                if saved {
                    then(self, window, cx);
                }
                return;
            }
            if self.source.is_none() {
                self.status = Some(crate::i18n::text("_shell.saving-new-documents-is-not-yet-supported").into());
                return cx.notify();
            }
            let directory = self.new_file_save_folder();
            let name = self.file_name.clone().unwrap_or_default();
            let picked = cx.prompt_for_new_path_with_formats(&directory, Some(&name), &self.save_formats());
            cx.spawn_in(window, async move |this, cx| {
                // The dialog failing to open is told in a message box (not taken for a cancel).
                let path = match picked.await {
                    Ok(Ok(Some(path))) => path,
                    Ok(Err(error)) => {
                        this.update(cx, |this, cx| this.show_alert(crate::i18n::text("shell.save-failed.title"), error.to_string().into(), cx)).ok();
                        return;
                    }
                    _ => return,
                };
                this.update_in(cx, |this, window, cx| {
                    let remembered = path.clone();
                    let result = this.save_to(path, window);
                    let saved = result.is_ok();
                    if saved {
                        crate::save_location::remember_save_folder(&remembered);
                    }
                    this.report_save(result, cx);
                    if saved {
                        then(this, window, cx);
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// The folder 名前を付けて保存 opens in: the file's own folder, or (a new file) `new_file_save_folder`.
    #[cfg(not(target_family = "wasm"))]
    pub fn save_dialog_folder(&self) -> std::path::PathBuf {
        self.path.as_deref().and_then(|p| p.parent()).map(std::path::Path::to_path_buf).unwrap_or_else(|| self.new_file_save_folder())
    }

    /// The folder the save dialog opens in for a file with no folder: the one a workbook's setting names (ツール > 設定,
    /// the user's own choice), else the folder saved to last, else the OS's documents folder (`save_location.rs`).
    #[cfg(not(target_family = "wasm"))]
    pub fn new_file_save_folder(&self) -> std::path::PathBuf {
        if let Some(custom) = crate::format::with_format!(self, new_file_save_folder(self)) {
            return custom;
        }
        crate::save_location::default_save_folder()
    }

    /// The formats the save dialog offers for the kind of the open file (the pop-up of the dialog; a name typed with another
    /// extension is kept). Each is one `save_to` writes by its extension.
    #[cfg(not(target_family = "wasm"))]
    fn save_formats(&self) -> Vec<SaveFormat> {
        use crate::i18n::text;
        let format = |label: &str, extensions: &[&str]| SaveFormat { label: text(label), extensions: extensions.iter().map(|e| e.to_string()).collect() };
        if self.visio_draft {
            return vec![format("file.save-format.vsdx", &["vsdx"])];
        }
        if self.standard_read_only {
            return if self.is_workbook() {
                vec![format("file.save-format.xlsx", &["xlsx", "xlsm", "xltx", "xltm"])]
            } else if self.is_presentation() {
                vec![format("file.save-format.pptx", &["pptx", "pptm", "ppsx", "ppsm", "potx", "potm"])]
            } else {
                vec![format("file.save-format.docx", &["docx", "docm", "dotx", "dotm"])]
            };
        }
        self.format_info().save_formats.iter().map(|(label, extension)| SaveFormat { label: text(label), extensions: vec![extension.to_string()] }).collect()
    }

    /// Save to `path`, and make it the file's path.
    #[cfg(not(target_family = "wasm"))]
    pub fn save_to(&mut self, path: std::path::PathBuf, window: &mut Window) -> Result<(), String> {
        if self.structured_preview { return Err(crate::i18n::text("structured-preview.read-only").to_string()); }
        if self.is_csv_streaming { return Err(crate::i18n::text("file.csv-loading-blocked").to_string()); }
        if path.extension().and_then(|ext| ext.to_str()).is_some_and(ibis_formats::production_image::supports_extension) {
            return Err("Choose a document extension to save the imported image".into());
        }
        let visio_path = crate::visio::is_visio_path(&path);
        if self.visio_draft && !visio_path {
            return Err(crate::i18n::text("vsdx.export-extension").to_string());
        }
        if visio_path && !self.visio_draft {
            return Err(crate::i18n::text("vsdx.export-use-command").to_string());
        }
        self.check_standard_copy_format(&path)?;
        if self.standard_source_path.as_deref().is_some_and(|original| same_file(original, &path)) {
            return Err(crate::i18n::text("file.standard-source-protected").to_string());
        }
        if self.is_jtd_read_only() { return Err(crate::i18n::text("jtd.read-only").to_string()); }
        if self.jtd.source_path.as_deref().is_some_and(|original| same_file(original, &path) || path.extension().and_then(|ext| ext.to_str()).is_some_and(ibis_formats::jtd::is_native_extension)) {
            return Err(crate::i18n::text("jtd.source-protected").to_string());
        }
        // The format may refuse for now (a presentation's master view is closed first: what is saved is the file, with what was made in it).
        if let Some(why) = crate::format::with_format!(self, save_refused(self)) {
            return Err(why);
        }
        // The password to modify not given: the file itself is not written over (another name is).
        if self.is_modify_locked() && self.path.as_deref().is_some_and(|own| same_file(own, &path)) {
            return Err(crate::i18n::text("file.modify-locked-no-overwrite").to_string());
        }
        let saving_original = self.path.as_deref().is_some_and(|old| same_file(old, &path));
        let source = self.source.clone().ok_or(crate::i18n::text("_shell.saving-new-documents-is-not-yet-supported").to_string())?;
        // The format's own package, or what the format writes under this extension (a workbook saved as .csv is the sheet shown, as
        // Excel's 「CSV UTF-8」).
        let (bytes, may_protect) = if self.visio_draft {
            (self.visio_bytes()?, false)
        } else if saving_original && self.current_csv_format.is_some() {
            (self.tracked_csv_bytes(&path)?, false)
        } else {
            match crate::format::with_format!(self, bytes_for_path(self, &path, &source, false)) {
                Some(other) => {
                    let other = other?;
                    if let Some(note) = other.note {
                        self.save_note = note;
                    }
                    (other.bytes, other.protect)
                }
                None => (source.save(&self.presentation_for_save(), ibis_formats::SaveOptions::default()).map_err(|e| e.to_string())?, true),
            }
        };
        // A signed file that was edited (or is written as another kind of file): written without the signatures, as Office does.
        let other_kind = self.path.as_deref().and_then(|own| own.extension()).map(|e| e.to_ascii_lowercase()) != path.extension().map(|e| e.to_ascii_lowercase());
        let (bytes, signatures_removed) = self.without_signatures(bytes, other_kind);
        // 常にバックアップ コピーを作成する: what the file is now is kept, where the format has the option.
        crate::format::with_format!(self, backup_before_write(self, &path));
        // Protected with the password the file has (パスワードを使用して暗号化).
        let bytes = if let (Some(password), true) = (&self.protection_password, may_protect) { protect(&bytes, password)? } else { bytes };
        let written = if saving_original && self.current_csv_format.is_some() {
            let original = self.current_csv_original.as_deref().ok_or_else(|| crate::i18n::text("file.csv-error.missing-data").to_string())?;
            write_checked_csv(&path, original, &bytes)
        } else { write_replacing(&path, &bytes) };
        written.map_err(|e| crate::i18n::format("file.cannot-write", &[("path", &path.display()), ("error", &e)]))?;
        // A different output path is a new export, whose current serializer is
        // UTF-8 BOM. A later in-place save must remember that new format.
        if !saving_original {
            self.current_csv_format = path.extension().and_then(|x| x.to_str()).and_then(|ext| {
                let delimiter = if ext.eq_ignore_ascii_case("csv") { ',' }
                    else if ["tsv", "tab", "txt"].iter().any(|x| ext.eq_ignore_ascii_case(x)) { '\t' }
                    else { return None };
                Some(ibis_formats::csv::OriginalCsvFormat { encoding: ibis_formats::csv::OriginalEncoding::Utf8Bom, delimiter, record_ending: ibis_formats::csv::RecordEnding::CrLf, final_record_ending: true })
            });
        }
        self.current_csv_original = self.current_csv_format.map(|_| std::sync::Arc::from(bytes.as_slice()));
        self.current_csv_slide = self.current_csv_original.as_ref().map(|_| self.current_slide);
        if self.current_csv_original.is_some() { self.document.begin_csv_edit_tracking(); }
        if self.standard_read_only {
            self.standard_read_only = false;
            self.document.set_read_only(self.read_only || self.phone_view_only());
            self.save_note = Some(crate::i18n::text("file.standard-copy-saved"));
        }
        let name: SharedString = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().into();
        if self.manages_window_title() {
            window.set_window_title(&format!("{name} - Docs"));
        }
        self.file_name = Some(name);
        // A newly created file becomes recent as soon as its first save succeeds.
        self.recent_files = crate::recent_files::add(&path);
        self.set_file_path(Some(path.clone()));
        // This window now represents the saved document, not the verified image.
        self.image_provenance = None;
        // Written in a current format: no longer Compatibility Mode.
        if !path.extension().and_then(|e| e.to_str()).is_some_and(|x| ["xls", "ppt", "pps", "pot", "doc", "dot"].iter().any(|o| x.eq_ignore_ascii_case(o))) {
            self.opened_from_legacy = false;
        }
        // The file as written here: the watching for other apps' changes (shell/file_watch.rs) does not take it for one.
        self.watch.known = crate::file_watch::DiskStamp::after_write(&path, &bytes);
        self.watch.asked = None;
        if signatures_removed {
            self.signatures_removed();
        }
        self.document.mark_clean();
        self.cleanup_active_autorecover_file();
        self.is_recovered = false;
        self.is_repaired = false;
        Ok(())
    }

    #[cfg(not(target_family = "wasm"))]
    fn report_save(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        // A file saved for the first time is among the recent files: the system's menu lists them (macOS).
        if result.is_ok() {
            crate::app::set_app_menus(cx);
        }
        self.status = Some(match result {
            Ok(()) => self.save_note.take().unwrap_or_else(|| crate::i18n::text("_shell.saved").into()),
            Err(err) => crate::i18n::format("file.save-failed", &[("error", &err)]).into(),
        });
        cx.notify();
    }
}

/// Write to an exclusively created sibling and check the original bytes just
/// before replacing it. A concurrent writer can still race after the check;
/// this is not a filesystem compare-and-swap.
#[cfg(not(target_family = "wasm"))]
fn write_checked_csv(path: &std::path::Path, expected: &[u8], bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().ok_or_else(|| std::io::Error::other(crate::i18n::text("file.csv-error.no-filename").to_string()))?.to_string_lossy();
    let (temporary, mut file) = loop {
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_file_name(format!(".{name}.ibis-csv-{}-{nonce}", std::process::id()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary) {
            Ok(file) => break (temporary, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::other(crate::i18n::text("file.csv-error.type-changed").to_string()));
        }
        if metadata.permissions().readonly() {
            return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() > 1 { return Err(std::io::Error::other(crate::i18n::text("file.csv-error.hardlink").to_string())); }
        }
        if std::fs::read(path)? != expected { return Err(std::io::Error::other(crate::i18n::text("file.csv-error.external-change").to_string())); }
        file.set_permissions(metadata.permissions())?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temporary); }
    result
}

/// `package` protected with `password`: an OOXML package kept encrypted in a compound file.
#[cfg(not(target_family = "wasm"))]
pub(crate) fn protect(package: &[u8], password: &str) -> Result<Vec<u8>, String> {
    // The OS's random generator (getrandom: BCryptGenRandom on Windows, which has no /dev/urandom; #M146).
    let mut failed = false;
    let encrypted = ibis_formats::encrypt(package, password, &mut |bytes: &mut [u8]| {
        if getrandom::fill(bytes).is_err() {
            failed = true;
        }
    });
    if failed {
        return Err(crate::i18n::text("_shell.passwords-cannot-be-set-in-this-environment").to_string());
    }
    Ok(encrypted)
}

/// Write `bytes` to a new file next to `path`, then move it over `path`.
#[cfg(not(target_family = "wasm"))]
pub fn write_replacing(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = path.with_file_name(format!(".{name}.ibis-saving"));
    let moved = std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, path));
    let Err(error) = moved else {
        #[cfg(target_os = "android")]
        gpui_android::sync_saved_file(path);
        #[cfg(target_os = "ios")]
        gpui_ios::sync_saved_file(path);
        return Ok(());
    };
    let _ = std::fs::remove_file(&temporary);
    // Only "not found" is met by writing in place: a folder a sync client keeps (OneDrive) can take the new file away
    // before it is moved (os error 2, GitHub issue #144; the cause is a guess, not seen on OneDrive itself). Any other
    // error (a full disk, no permission, a lock) is returned: writing in place would start the file over.
    if !falls_back_to_writing_in_place(&error) {
        return Err(error);
    }
    write_in_place(path, bytes)?;
    #[cfg(target_os = "android")]
    gpui_android::sync_saved_file(path);
    #[cfg(target_os = "ios")]
    gpui_ios::sync_saved_file(path);
    Ok(())
}

/// Whether a failed save by a new file beside the target is tried again by writing the target itself.
#[cfg(not(target_family = "wasm"))]
fn falls_back_to_writing_in_place(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

/// Write `bytes` over `path` itself, after a copy of the file as it is (put back when the writing fails midway). When the
/// copy cannot be made nothing is written.
#[cfg(not(target_family = "wasm"))]
fn write_in_place(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let copy = path.with_file_name(format!(".{name}.ibis-previous"));
    let existed = path.exists();
    if existed {
        std::fs::copy(path, &copy)?;
    }
    match std::fs::write(path, bytes) {
        Ok(()) => {
            let _ = std::fs::remove_file(&copy);
            Ok(())
        }
        Err(error) => {
            // The copy stays when it cannot be put back.
            if existed && std::fs::copy(&copy, path).is_ok() {
                let _ = std::fs::remove_file(&copy);
            }
            Err(error)
        }
    }
}

/// Whether `a` and `b` are one file (the same path, or one that links to the other).
#[cfg(not(target_family = "wasm"))]
pub fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b || same_file::is_same_file(a, b).unwrap_or(false) || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

#[cfg(all(test, not(target_family = "wasm")))]
mod write_tests {
    use super::write_replacing;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/save-tests")).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn checked_csv_write_rejects_external_change_and_keeps_original() {
        let dir = folder("csv-checked-write");
        let path = dir.join("source.csv");
        std::fs::write(&path, b"old").unwrap();
        assert!(super::write_checked_csv(&path, b"different", b"new").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "failed write removes its temporary file");
        super::write_checked_csv(&path, b"old", b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn checked_csv_write_refuses_linked_files() {
        let dir = folder("csv-checked-links");
        let path = dir.join("source.csv");
        let alias = dir.join("alias.csv");
        std::fs::write(&path, b"old").unwrap();
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(super::write_checked_csv(&path, b"old", b"new").is_err());
        assert_eq!(std::fs::read(&alias).unwrap(), b"old");
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(super::write_checked_csv(&alias, b"old", b"new").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }

    #[test]
    fn a_file_is_replaced_in_a_folder_of_a_japanese_name() {
        let dir = folder("\u{66f8}\u{304d}\u{8fbc}\u{307f}/\u{30c9}\u{30ad}\u{30e5}\u{30e1}\u{30f3}\u{30c8}");
        let path = dir.join("\u{30d5}\u{30a1}\u{30a4}\u{30eb}.xlsx");
        write_replacing(&path, b"old").unwrap();
        write_replacing(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(!dir.join(".\u{30d5}\u{30a1}\u{30a4}\u{30eb}.xlsx.ibis-saving").exists());
    }

    #[test]
    fn only_not_found_is_written_in_place() {
        use std::io::{Error, ErrorKind};
        assert!(super::falls_back_to_writing_in_place(&Error::from(ErrorKind::NotFound)));
        for kind in [ErrorKind::PermissionDenied, ErrorKind::StorageFull, ErrorKind::ReadOnlyFilesystem, ErrorKind::AlreadyExists, ErrorKind::Other] {
            assert!(!super::falls_back_to_writing_in_place(&Error::from(kind)), "{kind:?}");
        }
    }

    #[test]
    fn a_file_is_written_in_place_and_its_copy_removed() {
        let dir = folder("write-in-place");
        let path = dir.join("book.xlsx");
        std::fs::write(&path, b"old").unwrap();
        super::write_in_place(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(!dir.join(".book.xlsx.ibis-previous").exists());
        // A file that is not there yet is written too.
        let fresh = dir.join("fresh.xlsx");
        super::write_in_place(&fresh, b"new").unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"new");
    }

    #[test]
    fn nothing_is_written_in_place_when_the_copy_cannot_be_made() {
        // The place of the copy is taken (by a folder): the file is left as it was and the error is returned.
        let dir = folder("write-in-place-no-copy");
        let path = dir.join("book.xlsx");
        std::fs::write(&path, b"old").unwrap();
        std::fs::create_dir(dir.join(".book.xlsx.ibis-previous")).unwrap();
        assert!(super::write_in_place(&path, b"new").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }

    #[test]
    fn another_error_leaves_the_file_as_it_was() {
        // The new file's place is taken (by a folder): the error is not "not found", so it is returned, the file stays.
        let dir = folder("write-other-error");
        let path = dir.join("book.xlsx");
        std::fs::write(&path, b"old").unwrap();
        std::fs::create_dir(dir.join(".book.xlsx.ibis-saving")).unwrap();
        assert!(write_replacing(&path, b"new").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod protect_tests {
    /// A password-protected save gets its random bytes from the OS on every OS (Windows has no /dev/urandom: #M146), and
    /// the result opens again with the password.
    #[test]
    fn a_protected_package_opens_with_its_password() {
        let package = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/editing/docx-report.docx")).expect("the sample");
        let locked = super::protect(&package, "ibis123").expect("random bytes from the OS");
        assert_ne!(locked, package);
        let opened = ibis_formats::decrypt(&locked, "ibis123").expect("opens with the password");
        assert_eq!(opened, package);
        assert!(ibis_formats::decrypt(&locked, "wrong").is_err());
    }
}
