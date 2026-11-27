// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! The variant dictionary that is not bundled: a larger edition downloaded with セットアップ (a line of `font-list.txt` with
//! `"kind":"variant-dictionary"`, [`crate::font_setup::EntryKind::VariantDictionary`]), or one the user placed by hand.
//!
//! * **Why it is not bundled**: the bundled dictionary (`ibis_doc_model::variant_clusters`) is the GJS basic edition, whose
//!   sources allow redistribution without share-alike terms. The GJS full edition adds the MJ縮退マップ of IPA and is
//!   CC BY-SA 2.1 JP: it is a work of its own, distributed by GJS beside its licence, as the fonts are; ibisOffice reads it.
//! * **Where it is kept**: `ibis_office_base::app_data::datasets_dir()/<id>/<file>.zst` with its licence beside it (desktop),
//!   or in the browser's IndexedDB with the fonts (`www/main.ts`, which gives it to the app before the app starts).
//! * **Placed by hand** (a PC that cannot reach the network, e.g. on LGWAN): put the `.zst` file (and its NOTICE/licence) in a
//!   folder of its own under `datasets/`. It is checked as a dictionary when read; a file that is not one is ignored.
//! * **When it is used**: at the start, before a window opens (the formulas calculated from then on use it), and right after
//!   セットアップ downloads it (a workbook calculated before is calculated again with 今すぐ計算).

/// The setting that names the dictionary file chosen last (`<id>/<file>` under `datasets/`), used when there are several.
pub const CHOSEN_KEY: &str = "variant-dictionary.file";

/// The dictionary files under `datasets/` (desktop): every `*.zst` one folder down, the chosen one (setting) first, then the
/// newest first.
#[cfg(not(target_family = "wasm"))]
pub fn candidates() -> Vec<std::path::PathBuf> {
    let Some(root) = ibis_office_base::app_data::datasets_dir() else { return Vec::new() };
    let Ok(folders) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut found: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    for folder in folders.flatten().filter(|f| f.path().is_dir()) {
        let Ok(inside) = std::fs::read_dir(folder.path()) else { continue };
        for file in inside.flatten() {
            let path = file.path();
            if path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zst")) {
                let modified = file.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                found.push((modified, path));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    let mut paths: Vec<std::path::PathBuf> = found.into_iter().map(|(_, p)| p).collect();
    if let Some(chosen) = crate::settings::get(CHOSEN_KEY) {
        let chosen = root.join(chosen);
        if let Some(i) = paths.iter().position(|p| *p == chosen) {
            let p = paths.remove(i);
            paths.insert(0, p);
        }
    }
    paths
}

/// Use the first dictionary under `datasets/` that reads as one (at the start, before a window opens). Returns its path and its
/// number of characters; `None` (the bundled one stays) when there is none or none reads.
#[cfg(not(target_family = "wasm"))]
pub fn load_installed() -> Option<(std::path::PathBuf, usize)> {
    for path in candidates() {
        match std::fs::read(&path).ok().and_then(|bytes| ibis_doc_model::variant_clusters::decode(&bytes)) {
            Some(dict) => {
                let len = dict.len();
                ibis_doc_model::variant_clusters::install(dict);
                log::info!("variant dictionary: {} ({len} characters)", path.display());
                return Some((path, len));
            }
            None => log::warn!("variant dictionary: {} is not a dictionary; ignored", path.display()),
        }
    }
    None
}

/// Use the dictionary `entry` that セットアップ has just downloaded (desktop), and remember it as the chosen one.
#[cfg(not(target_family = "wasm"))]
pub fn install_downloaded(entry: &crate::font_setup::FontEntry) -> Result<usize, String> {
    let dir = crate::font_setup::font_folder(entry).ok_or("no folder for the data sets")?;
    let path = dir.join(&entry.file);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    install_bytes(&bytes, Some(&format!("{}/{}", entry.id, entry.file)))
}

/// Use the dictionary in `bytes` (a `.zst` file); `chosen` is remembered as the one to use at the next start.
pub fn install_bytes(bytes: &[u8], chosen: Option<&str>) -> Result<usize, String> {
    let dict = ibis_doc_model::variant_clusters::decode(bytes).ok_or("the file is not a variant dictionary")?;
    let len = dict.len();
    ibis_doc_model::variant_clusters::install(dict);
    log::info!("variant dictionary in use: {} ({len} characters)", chosen.unwrap_or("-"));
    if let Some(chosen) = chosen {
        crate::settings::set(CHOSEN_KEY, chosen);
    }
    Ok(len)
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;

    #[test]
    fn a_dictionary_placed_under_datasets_is_found_and_read_and_a_broken_one_is_skipped() {
        let root = ibis_office_base::app_data::datasets_dir().expect("a data folder in the tests");
        let (good, bad) = (root.join("t-placed"), root.join("t-broken"));
        let _ = std::fs::remove_dir_all(&good);
        let _ = std::fs::remove_dir_all(&bad);
        std::fs::create_dir_all(&good).unwrap();
        std::fs::create_dir_all(&bad).unwrap();
        // The bundled basic edition stands in for a downloaded one (the full edition is not kept in this repository).
        std::fs::write(good.join("variant-clusters.zst"), include_bytes!("../../ibis-doc-model/resources/gjs-variant-clusters.zst")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(bad.join("broken.zst"), b"not a dictionary").unwrap();
        let found = candidates();
        assert!(found.iter().any(|p| p.ends_with("t-placed/variant-clusters.zst")));
        assert!(found.iter().any(|p| p.ends_with("t-broken/broken.zst")));
        // The newest (the broken one) is tried first and skipped.
        let (path, len) = load_installed().expect("the placed dictionary");
        assert!(path.ends_with("t-placed/variant-clusters.zst"), "{}", path.display());
        assert_eq!(len, ibis_doc_model::variant_clusters::embedded().map_or(0, |d| d.len()));
        ibis_doc_model::variant_clusters::reset_to_embedded();
        let _ = std::fs::remove_dir_all(&good);
        let _ = std::fs::remove_dir_all(&bad);
    }

    #[test]
    fn bytes_that_are_not_a_dictionary_change_nothing() {
        let before = ibis_doc_model::variant_clusters::generation();
        assert!(install_bytes(b"not a dictionary", None).is_err());
        assert_eq!(ibis_doc_model::variant_clusters::generation(), before);
    }
}
