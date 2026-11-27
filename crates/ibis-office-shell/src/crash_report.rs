// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Explicit sharing/dismissal of the previous session's `last-panic.log`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

use gpui_kit::{App, ClipboardItem};

const GITHUB_ISSUE_URL: &str = "https://github.com/sorane-dev/docs/issues/new";
const MAX_REPORT_BYTES: usize = 16 * 1024;

// 0 = unchecked, 1 = none, 2 = available
static PENDING_CACHE: AtomicU8 = AtomicU8::new(0);

pub fn last_panic_path() -> Option<PathBuf> {
    crate::app_data::base().map(|base| base.join("ibisOffice").join("crash-logs").join("last-panic.log"))
}

pub fn has_pending_report() -> bool {
    match PENDING_CACHE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let exists = last_panic_path().is_some_and(|p| p.is_file());
            PENDING_CACHE.store(if exists { 2 } else { 1 }, Ordering::Relaxed);
            exists
        }
    }
}

pub fn read_pending_report() -> Option<String> {
    let path = last_panic_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(truncate_utf8(trimmed, MAX_REPORT_BYTES))
}

fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n... (truncated)", &s[..end])
}

pub fn dismiss_report() {
    if let Some(path) = last_panic_path() {
        let _ = std::fs::remove_file(path);
    }
    PENDING_CACHE.store(1, Ordering::Relaxed);
}

pub fn share_report(cx: &mut App) -> Option<String> {
    let report = read_pending_report()?;
    cx.write_to_clipboard(ClipboardItem::new_string(report.clone()));
    #[cfg(target_os = "android")]
    {
        gpui_android::share_text("soraneDocs Crash Report", &report);
    }
    #[cfg(not(target_os = "android"))]
    {
        crate::external::open_url(GITHUB_ISSUE_URL, cx);
    }
    dismiss_report();
    Some(crate::i18n::text("crash-report.copied-issue").to_string())
}

#[cfg(test)]
mod tests {
    use super::truncate_utf8;

    #[test]
    fn truncate_utf8_respects_char_boundaries() {
        let s = "あいうえお";
        let out = truncate_utf8(s, 7);
        assert!(out.starts_with("あい"));
        assert!(out.ends_with("... (truncated)"));
    }
}
