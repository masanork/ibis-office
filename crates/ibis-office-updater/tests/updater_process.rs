// SPDX-License-Identifier: AGPL-3.0-only
//! Opt-in process tests use synthetic executables in an isolated temp folder.
//! No Sorane Docs installation, signing key, update feed, or network is used.
#![cfg(any(target_os = "macos", windows))]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    #[cfg(target_os = "macos")]
    bundle_id: String,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "sorane-updater-process-{}-{n}-{id}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        #[cfg(target_os = "macos")]
        {
            fs::create_dir(root.join("real")).unwrap();
            std::os::unix::fs::symlink("real", root.join("alias")).unwrap();
        }
        Self {
            root,
            #[cfg(target_os = "macos")]
            bundle_id: format!("dev.sorane.updaterfixture.p{}n{n}i{id}", std::process::id()),
        }
    }
    fn executable(&self, path: &Path, mode: &str, marker: &Path) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/process_app.rs");
        let status = Command::new("rustc")
            .args(["--edition", "2024"])
            .arg(source)
            .arg("-o")
            .arg(path)
            .env("SORANE_FIXTURE_MODE", mode)
            .env("SORANE_FIXTURE_MARKER", marker)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "rustc: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }
    fn app(&self, name: &str, mode: &str, marker: &Path) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            // LaunchServices resolves this alias; the updater must detect the
            // canonical path reported by the OS, not just the input spelling.
            let app = self.root.join("alias").join(format!("{name}.app"));
            let bin = app.join("Contents/MacOS");
            fs::create_dir_all(&bin).unwrap();
            self.executable(&bin.join("ibisOffice"), mode, marker);
            let identifier = &self.bundle_id;
            let plist = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{identifier}</string><key>CFBundleName</key><string>SoraneUpdaterFixture</string><key>CFBundleExecutable</key><string>ibisOffice</string><key>CFBundlePackageType</key><string>APPL</string><key>LSUIElement</key><true/></dict></plist>"#
            );
            fs::write(app.join("Contents/Info.plist"), plist).unwrap();
            let out = Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-"])
                .arg(&app)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "codesign: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            app
        }
        #[cfg(windows)]
        {
            let dir = self.root.join(name);
            fs::create_dir(&dir).unwrap();
            self.executable(&dir.join("ibisOffice.exe"), mode, marker);
            dir
        }
    }
    fn app_binary(app: &Path) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            app.join("Contents/MacOS/ibisOffice")
        }
        #[cfg(windows)]
        {
            app.join("ibisOffice.exe")
        }
    }
    fn run(&self, installed: &Path, candidate: &Path) -> std::process::ExitStatus {
        let parent = self.root.join(if cfg!(windows) {
            "parent.exe"
        } else {
            "parent"
        });
        self.executable(&parent, "parent", &self.root.join("parent-marker"));
        // This child belongs to the test; keep its handle open until the
        // updater has finished so an unrelated process cannot reuse its PID.
        let mut child = Command::new(parent)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_ibisOfficeUpdater"))
            .args([
                "--pid",
                &child.id().to_string(),
                "--version",
                "2.0.0",
                "--old",
            ])
            .arg(installed)
            .arg("--new")
            .arg(candidate)
            .arg("--log")
            .arg(self.root.join("updater.log"))
            .args(["--wait-secs", "15"])
            .status()
            .unwrap();
        let _ = child.wait();
        result
    }
    fn wait_marker(&self, name: &str) {
        let path = self.root.join(name);
        for _ in 0..100 {
            if path.is_file() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("missing fixture marker {name}");
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        for name in ["Installed.app", "Candidate.app"] {
            let _ = Command::new("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister")
                .arg("-u").arg(self.root.join("alias").join(name))
                .stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
        // The successful fixture has a finite lifetime; on Windows its image
        // cannot be removed while it is running.
        for _ in 0..100 {
            if fs::remove_dir_all(&self.root).is_ok() || !self.root.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("fixture cleanup failed: {}", self.root.display());
    }
}

#[test]
fn updater_process_installs_and_keeps_running_fixture() {
    if std::env::var_os("SORANE_UPDATER_PROCESS_TEST").as_deref() != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }
    let f = Fixture::new();
    let old = f.app("Installed", "old", &f.root.join("old-marker"));
    let new = f.app("Candidate", "new-running", &f.root.join("new-marker"));
    let expected = fs::read(Fixture::app_binary(&new)).unwrap();
    let status = f.run(&old, &new);
    let log = fs::read_to_string(f.root.join("updater.log")).unwrap_or_default();
    assert!(status.success(), "updater status {status}: {log}");
    f.wait_marker("new-marker");
    assert_eq!(fs::read(Fixture::app_binary(&old)).unwrap(), expected);
    assert!(!ibis_office_updater::swap::backup_of(&old).exists());
    let log = fs::read_to_string(f.root.join("updater.log")).unwrap();
    assert!(log.contains(" check-running ok "), "{log}");
    // The synthetic process exits after seven seconds; don't leave it running.
    std::thread::sleep(Duration::from_secs(8));
}

#[test]
fn updater_process_rolls_back_after_fixture_exits() {
    if std::env::var_os("SORANE_UPDATER_PROCESS_TEST").as_deref() != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }
    let f = Fixture::new();
    let old = f.app("Installed", "old", &f.root.join("old-marker"));
    let new = f.app("Candidate", "new-failing", &f.root.join("new-marker"));
    let expected = fs::read(Fixture::app_binary(&old)).unwrap();
    assert!(!f.run(&old, &new).success());
    f.wait_marker("new-marker");
    f.wait_marker("old-marker");
    assert_eq!(fs::read(Fixture::app_binary(&old)).unwrap(), expected);
    assert!(!ibis_office_updater::swap::backup_of(&old).exists());
    let log = fs::read_to_string(f.root.join("updater.log")).unwrap();
    assert!(log.contains(" rollback ok "), "{log}");
}
