// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Sorane Docs desktop updater (currently disabled until release keys and
//! platform signing are provisioned). Signed metadata and the downloaded
//! artifact are verified before the inherited replacement logic is entered.
//! See docs/sorane-desktop-updates.md for activation gates and limitations.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use gpui_kit::{App, AsyncApp, PromptButton, PromptLevel};
use ibis_office_updater::log::Log;
use ibis_office_updater::{paths, version};

use crate::shell::{i18n, settings};

/// This build's version (version.txt's line for this platform at build time).
const CURRENT: &str = env!("IBIS_VERSION");
// A release key must be reviewed and embedded before update UI is enabled.
// Never accept a trust key from the network, environment, or manifest.
const TRUSTED_UPDATE_KEY_HEX: &str = "";
const MANIFEST_URL: &str = "https://github.com/masanork/ibis-office/releases/latest/download/sorane-update.json";
const RELEASES_URL: &str = "https://github.com/masanork/ibis-office/releases";
/// Attempts at one version after which no more offers are made.
const MAX_ATTEMPTS: u32 = 2;
/// The Sorane Docs Apple developer team, configured before activation.
#[cfg(target_os = "macos")]
const TEAM_ID: &str = ""; // Set to the Sorane Docs Apple Developer team before enabling updates.

const CHECK_KEY: &str = "check-updates";
const ATTEMPT_KEY: &str = "update-attempt";
const NOTIFIED_KEY: &str = "update-notified";

/// Whether to look for updates at startup (settings; on unless turned off).
pub(crate) fn check_enabled() -> bool {
    settings::get(CHECK_KEY).as_deref() != Some("0")
}

pub(crate) fn set_check_enabled(on: bool) {
    settings::set(CHECK_KEY, if on { "1" } else { "0" });
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn log_for(version: &str) -> Option<Log> {
    Some(Log::new(paths::log_file()?, version))
}

/// ヘルプ > 更新のログを表示: show `updater.log` in Finder (Explorer).
pub(crate) fn reveal_log(cx: &mut App) {
    let Some(path) = paths::log_file() else { return };
    if !path.exists() {
        // Nothing logged yet: show the folder it will be in.
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
            cx.open_with_system(dir);
        }
        return;
    }
    cx.reveal_path(&path);
}

// ---- Start-up ----------------------------------------------------------

/// What the background check found.
#[derive(Clone)]
enum Found {
    /// This copy is the latest.
    Nothing,
    /// The check could not be made (no network, no version.txt line, the download failed) or this copy is a build that does
    /// not update itself (run out of `target/`). Silent at start-up; the user's own check says so.
    Failed,
    /// The zip of `version` is downloaded.
    Ready { version: String, zip: PathBuf, artifact: Option<ibis_office_updater::signed_manifest::Artifact> },
    /// There is a newer version this copy cannot update to by itself.
    Manual { version: String },
}

thread_local! {
    /// The newer version the start-up check found (kept for the About window's 「新しいバージョンがあります。」 button, which asks
    /// the question again whatever was answered, or the setting 起動時に更新を確かめる says, at start-up).
    static AVAILABLE: std::cell::RefCell<Option<Found>> = const { std::cell::RefCell::new(None) };
    /// For the tests: the versions the question was asked about again.
    /// A check of the user's own is running (a second click while it runs does nothing).
    static CHECKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// For the tests: how many times the user's own check was asked for.
    #[cfg(any(test, feature = "scenarios"))]
    static CHECKED_NOW: std::cell::RefCell<u32> = const { std::cell::RefCell::new(0) };
    #[cfg(any(test, feature = "scenarios"))]
    static ASKED_AGAIN: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The newer version the start-up check found, if it did.
pub(crate) fn available_version() -> Option<String> {
    AVAILABLE.with(|a| match a.borrow().as_ref() {
        Some(Found::Ready { version, .. } | Found::Manual { version }) => Some(version.clone()),
        _ => None,
    })
}

/// For the tests (no network): what the start-up check would have found.
#[cfg(any(test, feature = "scenarios"))]
pub(crate) fn set_available_for_test(version: Option<&str>) {
    AVAILABLE.with(|a| *a.borrow_mut() = version.map(|v| Found::Ready { version: v.to_string(), zip: PathBuf::new(), artifact: None }));
}

/// For the tests: how many times 更新を確認 was used.
#[cfg(any(test, feature = "scenarios"))]
pub(crate) fn checked_now_for_test() -> u32 {
    CHECKED_NOW.with(|c| *c.borrow())
}

/// For the tests: the versions the question was asked about again, oldest first.
#[cfg(any(test, feature = "scenarios"))]
pub(crate) fn asked_again_for_test() -> Vec<String> {
    ASKED_AGAIN.with(|a| a.borrow().clone())
}

/// Ask the question the start-up check asked (「新しいバージョンが見つかりました。更新しますか？」) again. A click asks
/// whatever was answered before and whatever the setting says: an update that was refused is still there.
pub(crate) fn offer_again(cx: &mut App) {
    let _ = &cx;
    let Some(found) = AVAILABLE.with(|a| a.borrow().clone()) else { return };
    #[cfg(any(test, feature = "scenarios"))]
    {
        if let Found::Ready { version, .. } | Found::Manual { version } = &found {
            ASKED_AGAIN.with(|a| a.borrow_mut().push(version.clone()));
        }
        return;
    }
    #[allow(unreachable_code)]
    cx.spawn(async move |cx: &mut AsyncApp| match found {
        Found::Ready { version, zip, artifact } => offer(cx, version, zip, artifact, true).await,
        Found::Manual { .. } => show_failed(cx).await,
        Found::Nothing | Found::Failed => {}
    })
    .detach();
}

/// ヘルプ > 更新を確認 / the start page's 更新を確認: the user's own check, whatever the setting 起動時に更新を確かめる says
/// and whether the start-up question was refused. A newer version asks the update question; none says so (「最新です」);
/// a check that could not be made says so too. Not in the browser (no updater there; the caller does not show it).
pub(crate) fn check_now(cx: &mut App) {
    #[cfg(any(test, feature = "scenarios"))]
    {
        let _ = &cx;
        CHECKED_NOW.with(|c| *c.borrow_mut() += 1);
        return;
    }
    #[allow(unreachable_code)]
    {
        if CHECKING.with(|c| c.replace(true)) {
            return;
        }
        cx.spawn(async move |cx: &mut AsyncApp| {
            // A version the start-up check found and downloaded is asked about at once.
            let known = AVAILABLE.with(|a| a.borrow().clone());
            let found = match known {
                Some(found @ (Found::Ready { .. } | Found::Manual { .. })) => found,
                _ => cx.background_executor().spawn(async { check() }).await,
            };
            if matches!(found, Found::Ready { .. } | Found::Manual { .. }) {
                AVAILABLE.with(|a| *a.borrow_mut() = Some(found.clone()));
            }
            match found {
                Found::Ready { version, zip, artifact } => offer(cx, version, zip, artifact, true).await,
                Found::Manual { .. } => show_failed(cx).await,
                Found::Nothing => {
                    let detail = i18n::format("fix.update.latest.detail", &[("current", &CURRENT)]);
                    prompt(cx, PromptLevel::Info, i18n::text("fix.update.latest").to_string(), detail, vec![PromptButton::ok("OK")]).await;
                }
                Found::Failed => {
                    let log = paths::log_file().map(|p| p.display().to_string()).unwrap_or_default();
                    let detail = format!("{}\n\n{}", i18n::text("fix.update.check-failed.detail"), i18n::format("fix.update.log-path", &[("log", &log)]));
                    prompt(cx, PromptLevel::Warning, i18n::text("fix.update.check-failed").to_string(), detail, vec![PromptButton::ok("OK")]).await;
                }
            }
            CHECKING.with(|c| c.set(false));
        })
        .detach();
    }
}

/// Called once the first window is open: look for an update without delaying anything. The check is made whatever the setting
/// 起動時に更新を確かめる says (the start window's 「ibisOffice を更新する」 button needs it); the setting only decides whether
/// the question 「更新しますか？」 is asked by itself.
pub(crate) fn start(cx: &mut App) {
    background_check(cx, check_enabled(), Duration::from_secs(3));
}

/// A start window was opened (at launch or by the logo): look for an update in the background, unless that was done lately.
/// Nothing is asked; the window shows 「ibisOffice を更新する」 once a newer version is known. Not in the browser.
pub(crate) fn check_for_start_window(cx: &mut App) {
    background_check(cx, false, Duration::ZERO);
}

/// Minutes during which a check that was made is not made again.
const RECHECK_MILLIS: u64 = 30 * 60 * 1000;

thread_local! {
    /// A background check is running.
    static BACKGROUND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// When the last background check was started (`clock::unix_millis`).
    static LAST_BACKGROUND: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

fn background_check(cx: &mut App, ask: bool, delay: Duration) {
    // No network in the tests.
    #[cfg(any(test, feature = "scenarios"))]
    {
        let _ = (&cx, ask, delay);
        return;
    }
    #[allow(unreachable_code)]
    {
        let now = crate::clock::unix_millis();
        // A newer version is already known; or the check was made lately, or is running (a start window opened at launch).
        if available_version().is_some() || BACKGROUND.with(|b| b.get()) {
            return;
        }
        if LAST_BACKGROUND.with(|l| l.get()).is_some_and(|last| now.saturating_sub(last) < RECHECK_MILLIS) {
            return;
        }
        BACKGROUND.with(|b| b.set(true));
        LAST_BACKGROUND.with(|l| l.set(Some(now)));
        cx.spawn(async move |cx: &mut AsyncApp| {
            if !delay.is_zero() {
                // Not while the app is starting.
                cx.background_executor().timer(delay).await;
            }
            let found = cx.background_executor().spawn(async { check() }).await;
            BACKGROUND.with(|b| b.set(false));
            if matches!(found, Found::Ready { .. } | Found::Manual { .. }) {
                AVAILABLE.with(|a| *a.borrow_mut() = Some(found.clone()));
                // The start window shows its button.
                cx.update(|cx| cx.refresh_windows());
            }
            if !ask {
                return;
            }
            match found {
                Found::Nothing | Found::Failed => {}
                Found::Manual { version } => {
                    if settings::get(NOTIFIED_KEY).as_deref() != Some(&version) {
                        settings::set(NOTIFIED_KEY, &version);
                        show_failed(cx).await;
                    }
                }
                Found::Ready { version, zip, artifact } => offer(cx, version, zip, artifact, false).await,
            }
        })
        .detach();
    }
}

/// The steps up to the downloaded zip (run off the main thread).
fn check() -> Found {
    cleanup_old_files();
    clear_attempts_if_updated();

    let Some(key) = decode_hex::<32>(TRUSTED_UPDATE_KEY_HEX) else { return Found::Failed };
    let manifest_text = match fetch_bounded(MANIFEST_URL, 64 * 1024) { Ok(v) => v, Err(_) => return Found::Failed };
    let sig_text = match fetch_bounded(&format!("{MANIFEST_URL}.sig"), 256) { Ok(v) => v, Err(_) => return Found::Failed };
    let Some(sig) = decode_hex::<64>(sig_text.trim()) else { return Found::Failed };
    let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) { Ok(v) => v.as_secs(), Err(_) => return Found::Failed };
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64", ("macos", "x86_64") => "macos-x64",
        ("windows", "x86_64") => "windows-x64", ("linux", "x86_64") => "linux-x64-appimage",
        ("linux", "aarch64") => "linux-arm64-appimage", _ => return Found::Failed,
    };
    let manifest = match ibis_office_updater::update_transport::select_update(manifest_text.as_bytes(), &sig, &key, now, target, CURRENT) {
        Ok(Some(v)) => v, Ok(None) => return Found::Nothing, Err(_) => return Found::Failed,
    };
    let Some(artifact) = manifest.artifact(target) else { return Found::Failed };
    let latest = manifest.version.clone();
    let Some(log) = log_for(&latest) else { return Found::Failed };
    log.ok("check", &format!("a newer version: {latest} (current {CURRENT})"));

    if attempts(&latest) >= MAX_ATTEMPTS {
        log.info("check", &format!("{MAX_ATTEMPTS} attempts at {latest} did not end in it running: no more offers"));
        return Found::Manual { version: latest };
    }
    if let Err(reason) = installed_copy().map(|_| ()) {
        log.fail("check", &format!("this copy cannot update itself: {reason}"));
        return if reason.starts_with("a build run") { Found::Failed } else { Found::Manual { version: latest } };
    }

    match download_signed(&log, &latest, artifact) {
        Ok(zip) => Found::Ready { version: latest, zip, artifact: Some(artifact.clone()) },
        Err(err) => {
            log.fail("download", &err);
            Found::Failed
        }
    }
}

/// Folders of finished or older updates left in the update folder (an updater on Windows cannot remove the folder it runs from).
fn cleanup_old_files() {
    let Some(root) = paths::update_root() else { return };
    let Ok(entries) = std::fs::read_dir(&root) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if version::valid(&name) && !version::is_newer(&name, CURRENT) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

// ---- Attempts ----------------------------------------------------------

/// How many times an update to `version` was started.
fn attempts(version: &str) -> u32 {
    let stored = settings::get(ATTEMPT_KEY).unwrap_or_default();
    match stored.split_once(':') {
        Some((v, n)) if v == version => n.parse().unwrap_or(0),
        _ => 0,
    }
}

fn count_attempt(version: &str) {
    settings::set(ATTEMPT_KEY, &format!("{version}:{}", attempts(version) + 1));
}

/// Running the version an attempt was for (or a newer one): that attempt worked.
fn clear_attempts_if_updated() {
    let stored = settings::get(ATTEMPT_KEY).unwrap_or_default();
    if let Some((v, _)) = stored.split_once(':')
        && !version::is_newer(v, CURRENT)
    {
        settings::set(ATTEMPT_KEY, "");
    }
}

// ---- Where this copy is installed --------------------------------------

/// The installed copy to replace: the `.app` (macOS), the folder of `ibisOffice.exe` (Windows) or the AppImage (Linux), or why it cannot be.
fn installed_copy() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe = crate::shell::plain_path::canonical(&exe).unwrap_or(exe);
    let text = exe.to_string_lossy().replace('\\', "/");
    if text.contains("/target/release/") || text.contains("/target/debug/") || text.contains("/target/dist/") {
        return Err("a build run from the source tree".into());
    }
    #[cfg(target_os = "macos")]
    let app = {
        if text.contains("/AppTranslocation/") {
            return Err("running from macOS's App Translocation (move ibisOffice.app to Applications and open it from there)".into());
        }
        // Contents/MacOS/ibisOffice -> the .app.
        let app = exe.parent().and_then(Path::parent).and_then(Path::parent).map(Path::to_path_buf);
        match app {
            Some(app) if app.extension().is_some_and(|e| e == "app") => app,
            _ => return Err(format!("{} is not inside an .app", exe.display())),
        }
    };
    // The AppImage its runtime started this copy from.
    #[cfg(target_os = "linux")]
    let app = match env("APPIMAGE") {
        Some(appimage) => PathBuf::from(appimage),
        None => return Err("a build run without its AppImage (APPIMAGE is not set)".into()),
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let app = exe.parent().map(Path::to_path_buf).ok_or("no folder")?;
    let parent = app.parent().ok_or("no parent folder")?;
    let probe = parent.join(format!(".ibis-update-probe-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(app)
        }
        Err(err) => Err(format!("{} is not writable: {err}", parent.display())),
    }
}

// ---- Downloading -------------------------------------------------------

fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// Run a command, with its output as the error when it fails.
fn run(command: &mut Command) -> Result<String, String> {
    let out = command.output().map_err(|e| format!("{:?}: {e}", command.get_program()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        return Ok(stdout);
    }
    Err(format!("{:?} {}: {} {}", command.get_program(), out.status, stdout.trim(), String::from_utf8_lossy(&out.stderr).trim()))
}

fn decode_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() { return None; }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() { *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?; }
    Some(out)
}

fn fetch_bounded(url: &str, limit: u64) -> Result<String, String> {
    let mut bytes = Vec::new();
    ibis_office_updater::update_transport::fetch_to(url, limit, 20, &mut bytes)?;
    String::from_utf8(bytes).map_err(|_| "metadata is not UTF-8".into())
}

fn download_signed(log: &Log, version: &str, artifact: &ibis_office_updater::signed_manifest::Artifact) -> Result<PathBuf, String> {
    let dir = paths::update_dir(version).ok_or("no folder for update")?;
    let url = format!("{RELEASES_URL}/download/v{version}/{}", artifact.filename);
    let file = ibis_office_updater::update_transport::download_verified(&url, artifact, &dir)?;
    log.ok("verify", "signed size and SHA-256 match");
    Ok(file)
}

/// The error stream of a command (codesign writes its report there), whether or not it succeeds.
#[cfg(target_os = "macos")]
fn run_stderr(command: &mut Command) -> String {
    command.output().map(|o| String::from_utf8_lossy(&o.stderr).into_owned()).unwrap_or_default()
}

/// Unpacked and checked: the new copy and the updater inside it.
struct Unpacked {
    /// The new `.app` (macOS), folder (Windows) or AppImage (Linux).
    new: PathBuf,
    /// Empty on Linux, where the helper comes from the currently running AppImage.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    updater: PathBuf,
}

/// Check the AppImage downloaded (run off the main thread): an ELF program
/// with AppImage's mark, nothing to unpack.
#[cfg(target_os = "linux")]
fn unpack(log: &Log, _version: &str, appimage: &Path) -> Result<Unpacked, String> {
    use std::io::Read as _;
    log.start("verify-app", &appimage.display().to_string());
    let mut head = [0u8; 11];
    std::fs::File::open(appimage).and_then(|mut f| f.read_exact(&mut head)).map_err(|e| format!("{}: {e}", appimage.display()))?;
    if head[..4] != *b"\x7fELF" || head[8..11] != *b"AI\x02" {
        return Err(format!("{} is not an AppImage", appimage.display()));
    }
    log.ok("verify-app", "an AppImage");
    Ok(Unpacked { new: appimage.to_path_buf(), updater: PathBuf::new() })
}

/// Unpack `zip` and check what is in it (run off the main thread).
#[cfg(not(target_os = "linux"))]
fn unpack(log: &Log, version: &str, zip: &Path) -> Result<Unpacked, String> {
    let dir = zip.parent().ok_or("no folder")?.join("unpacked");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    log.start("unpack", &zip.display().to_string());
    #[cfg(target_os = "macos")]
    run(command("ditto").args(["-x", "-k"]).arg(zip).arg(&dir))?;
    #[cfg(not(target_os = "macos"))]
    run(command("tar").arg("-xf").arg(zip).arg("-C").arg(&dir))?;
    log.ok("unpack", &dir.display().to_string());

    #[cfg(target_os = "macos")]
    {
        let app = std::fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or("the zip has no .app")?;
        log.start("verify-app", &app.display().to_string());
        let plist = app.join("Contents/Info.plist");
        let found = run(command("/usr/libexec/PlistBuddy").args(["-c", "Print :CFBundleShortVersionString"]).arg(&plist))?;
        if found.trim() != version {
            return Err(format!("the new app is version {}, not {version}", found.trim()));
        }
        if TEAM_ID.is_empty() { return Err("Sorane Docs signing team is not configured".into()); }
        run(command("codesign").args(["--verify", "--deep", "--strict"]).arg(&app)).map_err(|e| format!("codesign --verify: {e}"))?;
        let info = run_stderr(command("codesign").args(["-dv", "--verbose=2"]).arg(&app));
        if !info.contains(&format!("TeamIdentifier={TEAM_ID}")) { return Err("unexpected signing team".into()); }
        run(command("spctl").args(["-a", "-t", "exec"]).arg(&app)).map_err(|e| format!("spctl: {e}"))?;
        let updater = app.join("Contents/MacOS/ibisOfficeUpdater");
        if !updater.is_file() {
            return Err("the new app has no ibisOfficeUpdater".into());
        }
        log.ok("verify-app", &format!("version {version}"));
        Ok(Unpacked { new: app, updater })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = version;
        // The zip holds one folder, ibisOffice\ (ibisOffice.exe, ibisOfficeUpdater.exe, Resources\).
        let folder = std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.join("ibisOffice.exe").is_file()).ok_or("the zip has no folder with ibisOffice.exe")?;
        let updater = folder.join("ibisOfficeUpdater.exe");
        if !updater.is_file() {
            return Err("the new folder has no ibisOfficeUpdater.exe".into());
        }
        log.ok("verify-app", &folder.display().to_string());
        Ok(Unpacked { new: folder, updater })
    }
}

// ---- Asking, then handing over -----------------------------------------

/// Whether any window has edits not yet saved (the same sign the close confirmation uses).
fn any_unsaved(cx: &mut App) -> bool {
    use crate::shell::editor::SlideEditor;
    let mut unsaved = false;
    for window in cx.windows() {
        if let Some(root) = window.downcast::<gpui_kit::component::Root>() {
            let _ = root.update(cx, |root, _, app| {
                if let Ok(editor) = root.view().clone().downcast::<SlideEditor>() {
                    let editor = editor.read(app);
                    unsaved |= editor.has_document && editor.document.can_undo();
                }
            });
        }
    }
    unsaved
}

/// Show a prompt over the active window; the index of the button chosen.
async fn prompt(cx: &mut AsyncApp, level: PromptLevel, message: String, detail: String, buttons: Vec<PromptButton>) -> Option<usize> {
    let receiver = cx.update(|cx| {
        let window = cx.active_window().or_else(|| cx.windows().first().copied())?;
        window.update(cx, |_, window, cx| window.prompt(level, &message, Some(&detail), &buttons, cx)).ok()
    })?;
    receiver.await.ok()
}

async fn show_failed(cx: &mut AsyncApp) {
    let log = paths::log_file().map(|p| p.display().to_string()).unwrap_or_default();
    let detail = format!("{}\n\n{}", i18n::format("fix.update.failed.detail", &[("url", &RELEASES_URL)]), i18n::format("fix.update.log-path", &[("log", &log)]));
    let buttons = vec![PromptButton::ok(i18n::text("fix.update.open-site")), PromptButton::cancel("OK")];
    if prompt(cx, PromptLevel::Warning, i18n::text("fix.update.failed").to_string(), detail, buttons).await == Some(0) {
        cx.update(|cx| cx.open_url(RELEASES_URL));
    }
}

async fn offer(cx: &mut AsyncApp, version: String, zip: PathBuf, artifact: Option<ibis_office_updater::signed_manifest::Artifact>, asked_by_user: bool) {
    // Not while there is work not saved: the app quits when the update starts. (Not at the user's own click: the question is
    // shown at once, and saying はい with work unsaved is turned away below.)
    while !asked_by_user && cx.update(|cx| any_unsaved(cx)) {
        cx.background_executor().timer(Duration::from_secs(30)).await;
    }
    let buttons = vec![PromptButton::ok(i18n::text("fix.update.yes")), PromptButton::cancel(i18n::text("fix.update.no"))];
    let detail = i18n::format("fix.update.found.detail", &[("version", &version), ("current", &CURRENT)]);
    let answer = prompt(cx, PromptLevel::Info, i18n::text("fix.update.found").to_string(), detail, buttons).await;
    if answer != Some(0) {
        if let Some(log) = log_for(&version) {
            log.info("offer", "the user chose not to update now");
        }
        return;
    }
    if cx.update(|cx| any_unsaved(cx)) {
        // Edited while the question was open: ask again next time.
        if let Some(log) = log_for(&version) {
            log.info("offer", "there is unsaved work: not updating now");
        }
        return;
    }
    let zip_for_task = zip.clone();
    let version_for_task = version.clone();
    let unpacked = cx
        .background_executor()
        .spawn(async move {
            let log = log_for(&version_for_task).ok_or("no log")?;
            unpack(&log, &version_for_task, &zip_for_task).inspect_err(|err| log.fail("unpack", err))
        })
        .await;
    let Ok(unpacked) = unpacked else {
        // Counts as an attempt: a release that cannot be unpacked or checked must not be offered at every start.
        count_attempt(&version);
        show_failed(cx).await;
        return;
    };
    let version_for_start = version.clone();
    let started = cx.update(|cx| {
        let log = log_for(&version_for_start)?;
        match start_updater(&log, &version_for_start, &unpacked, artifact.as_ref()) {
            Ok(()) => {
                // The updater waits for this process to end.
                cx.quit();
                Some(())
            }
            Err(err) => {
                log.fail("start-updater", &err);
                None
            }
        }
    });
    if started.is_none() {
        show_failed(cx).await;
    }
}

/// Start the helper from the current AppImage, never from the candidate.
#[cfg(target_os = "linux")]
fn start_updater(log: &Log, version: &str, unpacked: &Unpacked, artifact: Option<&ibis_office_updater::signed_manifest::Artifact>) -> Result<(), String> {
    let artifact = artifact.ok_or("missing verified artifact metadata")?;
    let old = installed_copy()?;
    let workdir = paths::update_dir(version).ok_or("no update folder")?;
    let appdir = std::env::var_os("APPDIR").ok_or("APPDIR is not set")?;
    let helper = PathBuf::from(appdir).join("usr/bin/ibisOfficeUpdater");
    if !std::fs::symlink_metadata(&helper).map_err(|e| format!("{}: {e}", helper.display()))?.file_type().is_file() { return Err("updater is not a regular file".into()); }
    count_attempt(version);
    log.start("start-updater", &format!("{} for {} -> {}", helper.display(), unpacked.new.display(), old.display()));
    let mut updater = command(&helper.to_string_lossy());
    updater.args(["--pid", &std::process::id().to_string(), "--version", version, "--old"])
        .arg(&old).arg("--new").arg(&unpacked.new).arg("--log").arg(log.path())
        .arg("--workdir").arg(workdir).args(["--size", &artifact.size.to_string(), "--sha256", &artifact.sha256])
        .current_dir(std::env::temp_dir()).stdout(Stdio::null()).stderr(Stdio::null());
    updater.spawn().map_err(|e| format!("{}: {e}", helper.display()))?;
    log.ok("start-updater", "started; ibisOffice quits now");
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn start_updater(log: &Log, version: &str, unpacked: &Unpacked, _artifact: Option<&ibis_office_updater::signed_manifest::Artifact>) -> Result<(), String> {
    let old = installed_copy()?;
    let workdir = paths::update_dir(version).ok_or("no update folder")?;
    count_attempt(version);
    log.start("start-updater", &format!("{} for {} -> {}", unpacked.updater.display(), unpacked.new.display(), old.display()));
    let mut updater = command(&unpacked.updater.to_string_lossy());
    updater
        .args(["--pid", &std::process::id().to_string(), "--version", version, "--old"])
        .arg(&old)
        .arg("--new")
        .arg(&unpacked.new)
        .arg("--log")
        .arg(log.path())
        .arg("--workdir")
        .arg(&workdir)
        // Not inside the folder being replaced (Windows cannot rename a folder a process is in) or the one being removed.
        .current_dir(std::env::temp_dir())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    updater.spawn().map_err(|e| format!("{}: {e}", unpacked.updater.display()))?;
    log.ok("start-updater", "started; ibisOffice quits now");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::decode_hex;
    #[test]
    fn hex_requires_exact_length_and_valid_digits() {
        assert_eq!(decode_hex::<2>("00ff"), Some([0, 255]));
        assert!(decode_hex::<2>("0f").is_none());
        assert!(decode_hex::<2>("xxff").is_none());
        assert!(decode_hex::<2>("éff").is_none());
    }
}
