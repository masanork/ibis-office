// SPDX-License-Identifier: AGPL-3.0-only
//! AppImage replacement for the standalone Linux updater. Stale state is never
//! discarded automatically. A hard link preserves the original inode while an
//! atomic rename installs the staged file on the same filesystem.
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn adjacent(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}
pub fn backup_of(path: &Path) -> PathBuf {
    adjacent(path, ".update-backup")
}
pub fn stage_of(path: &Path) -> PathBuf {
    adjacent(path, ".update-new")
}
fn regular(path: &Path) -> Result<fs::Metadata, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.file_type().is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    Ok(meta)
}
fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(format!(
            "{} already exists; preserve it for recovery",
            path.display()
        )),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}
/// Stage and check the image, then preserve and replace the installed file.
pub fn install(
    old: &Path,
    new: &Path,
    expected: &crate::signed_manifest::Artifact,
) -> Result<(), String> {
    let metadata = regular(old)?;
    regular(new)?;
    let backup = backup_of(old);
    let stage = stage_of(old);
    absent(&backup)?;
    absent(&stage)?;
    let mut input = File::open(new).map_err(|e| format!("{}: {e}", new.display()))?;
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stage)
        .map_err(|e| format!("{}: {e}", stage.display()))?;
    let staged = (|| -> Result<(), String> {
        crate::signed_manifest::copy_bounded(&mut input, &mut out, expected.size)?;
        out.flush().map_err(|e| e.to_string())?;
        out.set_permissions(fs::Permissions::from_mode(metadata.permissions().mode()))
            .map_err(|e| e.to_string())?;
        out.sync_all().map_err(|e| e.to_string())?;
        let staged = File::open(&stage).map_err(|e| e.to_string())?;
        crate::signed_manifest::verify_artifact(expected, staged).map_err(str::to_string)?;
        let mut header = [0; 11];
        File::open(&stage)
            .and_then(|mut f| f.read_exact(&mut header))
            .map_err(|e| e.to_string())?;
        if header[..4] != *b"\x7fELF" || header[8..] != *b"AI\x02" {
            return Err("candidate is not an AppImage".into());
        }
        Ok(())
    })();
    drop(out);
    if let Err(err) = staged {
        let _ = fs::remove_file(&stage);
        return Err(err);
    }
    // hard_link fails if the backup exists, and never replaces it.
    if let Err(err) = fs::hard_link(old, &backup) {
        let _ = fs::remove_file(&stage);
        return Err(format!("{}: {err}", backup.display()));
    }
    // On failure leave both backup and stage for inspection and recovery.
    fs::rename(&stage, old).map_err(|e| format!("{}: {e}", old.display()))
}
pub fn rollback(old: &Path) -> Result<(), String> {
    let backup = backup_of(old);
    regular(&backup)?;
    match fs::symlink_metadata(old) {
        Ok(m) if !m.file_type().is_file() => {
            return Err(format!("{} is not a regular file", old.display()));
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        _ => (),
    }
    fs::rename(&backup, old).map_err(|e| format!("{}: {e}", old.display()))
}
pub fn cleanup(old: &Path) -> Result<(), String> {
    let backup = backup_of(old);
    regular(&backup)?;
    fs::remove_file(&backup).map_err(|e| format!("{}: {e}", backup.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!(
                "sorane-image-swap-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn expected(bytes: &[u8]) -> crate::signed_manifest::Artifact {
        let hash = ring::digest::digest(&ring::digest::SHA256, bytes);
        crate::signed_manifest::Artifact {
            target: "linux-x64-appimage".into(),
            filename: "fixture.AppImage".into(),
            size: bytes.len() as u64,
            sha256: hash.as_ref().iter().map(|b| format!("{b:02x}")).collect(),
        }
    }
    #[test]
    fn install_and_restore_original_bytes() {
        let t = Temp::new();
        let old = t.0.join("app");
        let new = t.0.join("new");
        fs::write(&old, b"original").unwrap();
        fs::write(&new, b"\x7fELF\0\0\0\0AI\x02new").unwrap();
        install(&old, &new, &expected(&fs::read(&new).unwrap())).unwrap();
        assert_eq!(fs::read(backup_of(&old)).unwrap(), b"original");
        rollback(&old).unwrap();
        assert_eq!(fs::read(&old).unwrap(), b"original");
    }
    #[test]
    fn invalid_and_stale_candidates_do_not_replace_original() {
        let t = Temp::new();
        let old = t.0.join("app");
        let new = t.0.join("new");
        fs::write(&old, b"original").unwrap();
        fs::write(&new, b"invalid").unwrap();
        assert!(install(&old, &new, &expected(b"other")).is_err());
        assert!(install(&old, &new, &expected(b"invalid")).is_err());
        assert!(!stage_of(&old).exists());
        fs::write(stage_of(&old), b"stale").unwrap();
        assert!(install(&old, &new, &expected(b"other")).is_err());
        assert_eq!(fs::read(stage_of(&old)).unwrap(), b"stale");
        assert_eq!(fs::read(old).unwrap(), b"original");
    }
}
