// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Desktop GitHub App integration (OAuth Device Flow, REST API transport via `curl`,
//! session persistence, and `GIT_ASKPASS`-based `git` synchronization).

#[cfg(not(target_family = "wasm"))]
mod desktop {
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sorane_github::{
    DeviceCodeResponse, DeviceTokenPoll, GITHUB_API_BASE, GITHUB_API_VERSION, GITHUB_APP_CLIENT_ID,
    GITHUB_DEVICE_CODE_URL, GITHUB_OAUTH_TOKEN_URL, GitHubInstallation, GitHubRepo, GitHubUser,
    RepoCatalog, build_create_repo_body, build_init_index_put_body, evaluate_catalog,
    is_valid_repo_name, parse_created_repo_response, parse_device_code_response,
    parse_device_token_response, parse_installation_repos_response, parse_installations_response,
    parse_user_repos_response, parse_user_response,
};

const GITHUB_SESSION_FILE: &str = "sorane-github-session.json";
const USER_AGENT: &str = "soraneDocs-Desktop";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubSession {
    pub access_token: String,
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
}

fn session_file_path() -> Option<PathBuf> {
    let base = ibis_office_shell::app_data::base()?;
    Some(base.join("ibisOffice").join(GITHUB_SESSION_FILE))
}

pub fn load_session() -> Option<GitHubSession> {
    let path = session_file_path()?;
    let raw = fs::read_to_string(path).ok()?;
    let session: GitHubSession = serde_json::from_str(&raw).ok()?;
    if session.access_token.trim().is_empty() || session.login.trim().is_empty() {
        return None;
    }
    Some(session)
}

pub fn save_session(session: &GitHubSession) -> Result<(), String> {
    let Some(path) = session_file_path() else {
        return Err("アプリデータ保存先を取得できませんでした。".to_string());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("ディレクトリ作成に失敗しました: {e}"))?;
    }
    let json = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| format!("セッション保存に失敗しました: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn clear_session() {
    if let Some(path) = session_file_path() {
        let _ = fs::remove_file(path);
    }
}

pub fn default_clone_target(repo_name: &str) -> PathBuf {
    let safe_name = if is_valid_repo_name(repo_name) {
        repo_name.trim()
    } else {
        "soraneDocs"
    };
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(safe_name)
}

pub fn has_git_cli() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

struct HttpResponse {
    status: u16,
    body: String,
}

fn curl_json(
    method: &str,
    url: &str,
    bearer_token: Option<&str>,
    json_body: Option<&str>,
) -> Result<HttpResponse, String> {
    const STATUS_MARKER: &str = "\n__SORANE_HTTP_STATUS__:";
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "--connect-timeout",
        "10",
        "--max-time",
        "30",
        "-X",
        method,
        "-H",
        "Accept: application/json",
        "-H",
        &format!("User-Agent: {USER_AGENT}"),
        "-w",
        &format!("{STATUS_MARKER}%{{http_code}}"),
    ]);
    if let Some(token) = bearer_token {
        cmd.args(["-H", &format!("Authorization: Bearer {token}")]);
        cmd.args(["-H", &format!("X-GitHub-Api-Version: {GITHUB_API_VERSION}")]);
    }
    if let Some(body) = json_body {
        cmd.args(["-H", "Content-Type: application/json", "--data-binary", body]);
    }
    cmd.arg(url);

    let output = cmd
        .output()
        .map_err(|e| format!("HTTP クライアント (curl) の起動に失敗しました: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("通信エラーが発生しました: {}", err.trim()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(idx) = text.rfind(STATUS_MARKER) else {
        return Err("HTTP ステータスを取得できませんでした。".to_string());
    };
    let body = text[..idx].to_string();
    let status_str = text[idx + STATUS_MARKER.len()..].trim();
    let status: u16 = status_str
        .parse()
        .map_err(|_| format!("不正な HTTP ステータス: {status_str}"))?;
    Ok(HttpResponse { status, body })
}

pub fn start_device_flow() -> Result<DeviceCodeResponse, String> {
    let body = serde_json::json!({
        "client_id": GITHUB_APP_CLIENT_ID,
    })
    .to_string();
    let res = curl_json("POST", GITHUB_DEVICE_CODE_URL, None, Some(&body))?;
    if res.status >= 400 {
        return Err(format!(
            "GitHub Device Flow の開始に失敗しました (HTTP {}): {}",
            res.status, res.body
        ));
    }
    parse_device_code_response(&res.body)
}

pub fn poll_device_token(device_code: &str) -> Result<DeviceTokenPoll, String> {
    let body = serde_json::json!({
        "client_id": GITHUB_APP_CLIENT_ID,
        "device_code": device_code,
        "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
    })
    .to_string();
    let res = curl_json("POST", GITHUB_OAUTH_TOKEN_URL, None, Some(&body))?;
    parse_device_token_response(&res.body)
}

pub fn fetch_user(access_token: &str) -> Result<GitHubUser, String> {
    let url = format!("{GITHUB_API_BASE}/user");
    let res = curl_json("GET", &url, Some(access_token), None)?;
    if res.status == 401 {
        return Err("GitHub の認証トークンが期限切れです。再ログインしてください。".to_string());
    }
    if res.status >= 400 {
        return Err(format!("ユーザー情報の取得に失敗しました (HTTP {})", res.status));
    }
    parse_user_response(&res.body)
}

pub fn fetch_repo_catalog(access_token: &str, login: &str) -> Result<RepoCatalog, String> {
    let inst_url = format!("{GITHUB_API_BASE}/user/installations?per_page=50");
    let inst_res = curl_json("GET", &inst_url, Some(access_token), None)?;
    if inst_res.status == 401 {
        return Err("GitHub の認証トークンが期限切れです。再ログインしてください。".to_string());
    }

    let mut installations: Vec<GitHubInstallation> = Vec::new();
    let mut repos: Vec<GitHubRepo> = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    if inst_res.status < 400 {
        installations = parse_installations_response(&inst_res.body)?;
        for inst in installations.iter().take(10) {
            let repo_url = format!(
                "{GITHUB_API_BASE}/user/installations/{}/repositories?per_page=100",
                inst.id
            );
            let repo_res = curl_json("GET", &repo_url, Some(access_token), None)?;
            if repo_res.status < 400 {
                if let Ok(list) = parse_installation_repos_response(&repo_res.body) {
                    for r in list {
                        if seen_names.insert(r.full_name.clone()) {
                            repos.push(r);
                        }
                    }
                }
            }
        }
    }

    if repos.is_empty() {
        let user_repos_url = format!("{GITHUB_API_BASE}/user/repos?sort=updated&per_page=50");
        let user_res = curl_json("GET", &user_repos_url, Some(access_token), None)?;
        if user_res.status == 401 {
            return Err("GitHub の認証トークンが期限切れです。再ログインしてください。".to_string());
        }
        if user_res.status < 400 {
            if let Ok(list) = parse_user_repos_response(&user_res.body) {
                for r in list {
                    if seen_names.insert(r.full_name.clone()) {
                        repos.push(r);
                    }
                }
            }
        }
    }

    Ok(evaluate_catalog(login, &installations, repos))
}

pub fn create_private_vault_repo(
    access_token: &str,
    login: &str,
    repo_name: &str,
) -> Result<GitHubRepo, String> {
    let trimmed = repo_name.trim();
    if !is_valid_repo_name(trimmed) {
        return Err("リポジトリ名には英数字・ハイフン・アンダースコア・ドットのみ使用できます。".to_string());
    }

    let create_url = format!("{GITHUB_API_BASE}/user/repos");
    let create_body = build_create_repo_body(trimmed, "Personal documents repository for soraneDocs");
    let create_res = curl_json("POST", &create_url, Some(access_token), Some(&create_body))?;
    if create_res.status == 401 {
        return Err("GitHub の認証トークンが期限切れです。再ログインしてください。".to_string());
    }
    if create_res.status == 422 {
        return Err(format!(
            "リポジトリ {login}/{trimmed} はすでに存在するため新規作成できません。既存リポジトリ一覧から選択してください。"
        ));
    }
    if create_res.status >= 400 {
        return Err(format!(
            "Private リポジトリの作成に失敗しました (HTTP {}): {}",
            create_res.status, create_res.body
        ));
    }

    let created = parse_created_repo_response(&create_res.body)?;
    let init_url = format!(
        "{GITHUB_API_BASE}/repos/{}/{}/contents/index.md",
        created.owner, created.repo
    );
    let init_body = build_init_index_put_body(&created.repo, &created.default_branch);

    let retry_delays = [Duration::from_millis(300), Duration::from_millis(700)];
    let mut last_status = 0u16;
    for attempt in 0..=retry_delays.len() {
        let init_res = curl_json("PUT", &init_url, Some(access_token), Some(&init_body))?;
        last_status = init_res.status;
        if init_res.status < 400 {
            return Ok(created);
        }
        if !matches!(init_res.status, 404 | 502 | 503) {
            break;
        }
        if let Some(delay) = retry_delays.get(attempt) {
            std::thread::sleep(*delay);
        }
    }

    Err(format!(
        "リポジトリ {} は作成されましたが、初期ファイル index.md の書き込みに失敗しました (HTTP {last_status})。GitHub App の設定で「All repositories」が許可されているか確認してください。",
        created.full_name
    ))
}

/// Ephemeral `GIT_ASKPASS` helper (VSCode / Zed style) so tokens are never embedded in `.git/config`.
struct AskpassGuard {
    path: PathBuf,
}

impl AskpassGuard {
    fn create() -> Result<Self, String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        #[cfg(windows)]
        let (filename, contents) = (
            format!("sorane-askpass-{pid}-{nanos}.cmd"),
            "@echo off\r\necho %~1 | findstr /I \"Username\" >nul\r\nif %errorlevel%==0 (\r\n  echo x-access-token\r\n) else (\r\n  echo %SORANE_GH_ASKPASS_TOKEN%\r\n)\r\n",
        );
        #[cfg(not(windows))]
        let (filename, contents) = (
            format!("sorane-askpass-{pid}-{nanos}.sh"),
            "#!/bin/sh\ncase \"$1\" in\n  *Username*) printf '%s\\n' 'x-access-token' ;;\n  *) printf '%s\\n' \"$SORANE_GH_ASKPASS_TOKEN\" ;;\nesac\n",
        );

        let path = std::env::temp_dir().join(filename);
        fs::write(&path, contents).map_err(|e| format!("askpass スクリプト作成に失敗しました: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("askpass 権限設定に失敗しました: {e}"))?;
        }
        Ok(Self { path })
    }
}

impl Drop for AskpassGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn run_git_with_token(
    access_token: &str,
    cwd: Option<&Path>,
    args: &[&str],
) -> Result<String, String> {
    let guard = AskpassGuard::create()?;
    let mut cmd = Command::new("git");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.args(args)
        .env("GIT_ASKPASS", &guard.path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("SORANE_GH_ASKPASS_TOKEN", access_token);

    let output = cmd
        .output()
        .map_err(|e| format!("git コマンドの実行に失敗しました (git がインストールされているか確認してください): {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let msg = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        return Err(format!("git {} に失敗しました: {msg}", args.first().copied().unwrap_or("")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn clone_or_open_repo(
    access_token: &str,
    repo: &GitHubRepo,
    target_dir: &Path,
) -> Result<PathBuf, String> {
    if target_dir.join(".git").exists() {
        let _ = run_git_with_token(access_token, Some(target_dir), &["fetch", "--prune"]);
        return Ok(target_dir.to_path_buf());
    }
    if target_dir.exists() {
        let is_empty = fs::read_dir(target_dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            return Err(format!(
                "保存先フォルダ {} はすでに存在し、空ではありません。",
                target_dir.display()
            ));
        }
    } else if let Some(parent) = target_dir.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("フォルダ作成に失敗しました: {e}"))?;
    }

    let target_str = target_dir.to_string_lossy().to_string();
    run_git_with_token(
        access_token,
        None,
        &["clone", &repo.clone_url, &target_str],
    )?;
    Ok(target_dir.to_path_buf())
}

pub fn git_pull(access_token: &str, repo_root: &Path) -> Result<String, String> {
    run_git_with_token(access_token, Some(repo_root), &["pull", "--ff-only"])
}

pub fn git_commit_and_push(
    access_token: &str,
    repo_root: &Path,
    login: &str,
    message: &str,
) -> Result<String, String> {
    run_git_with_token(access_token, Some(repo_root), &["add", "-A"])?;
    let status = run_git_with_token(access_token, Some(repo_root), &["status", "--porcelain"])?;
    if !status.is_empty() {
        let commit_msg = if message.trim().is_empty() {
            "Update documents via soraneDocs"
        } else {
            message.trim()
        };
        let has_name = Command::new("git")
            .current_dir(repo_root)
            .args(["config", "user.name"])
            .output()
            .is_ok_and(|o| o.status.success() && !o.stdout.is_empty());
        if has_name {
            run_git_with_token(access_token, Some(repo_root), &["commit", "-m", commit_msg])?;
        } else {
            let email = format!("{login}@users.noreply.github.com");
            run_git_with_token(
                access_token,
                Some(repo_root),
                &[
                    "-c",
                    &format!("user.name={login}"),
                    "-c",
                    &format!("user.email={email}"),
                    "commit",
                    "-m",
                    commit_msg,
                ],
            )?;
        }
    }
    run_git_with_token(access_token, Some(repo_root), &["push"])?;
    Ok("GitHub へプッシュしました。".to_string())
}
}

#[cfg(not(target_family = "wasm"))]
pub use desktop::*;

#[cfg(target_family = "wasm")]
mod wasm_stub {
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Serialize};
    use sorane_github::{DeviceCodeResponse, DeviceTokenPoll, GitHubRepo, GitHubUser, RepoCatalog};

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct GitHubSession {
        pub access_token: String,
        pub login: String,
        #[serde(default)]
        pub name: Option<String>,
    }

    pub fn load_session() -> Option<GitHubSession> {
        None
    }

    pub fn save_session(_session: &GitHubSession) -> Result<(), String> {
        Ok(())
    }

    pub fn clear_session() {}

    pub fn default_clone_target(repo_name: &str) -> PathBuf {
        PathBuf::from(repo_name)
    }

    pub fn start_device_flow() -> Result<DeviceCodeResponse, String> {
        Err("Web 版ではページ側の GitHub ログインを利用してください。".to_string())
    }

    pub fn poll_device_token(_device_code: &str) -> Result<DeviceTokenPoll, String> {
        Err("Web 版ではページ側の GitHub ログインを利用してください。".to_string())
    }

    pub fn fetch_user(_access_token: &str) -> Result<GitHubUser, String> {
        Err("Web 版ではページ側の GitHub ログインを利用してください。".to_string())
    }

    pub fn fetch_repo_catalog(_access_token: &str, _login: &str) -> Result<RepoCatalog, String> {
        Err("Web 版ではページ側の GitHub ログインを利用してください。".to_string())
    }

    pub fn create_private_vault_repo(
        _access_token: &str,
        _login: &str,
        _name: &str,
    ) -> Result<GitHubRepo, String> {
        Err("Web 版ではページ側の GitHub ログインを利用してください。".to_string())
    }

    pub fn clone_or_open_repo(
        _access_token: &str,
        _repo: &GitHubRepo,
        _target_dir: &Path,
    ) -> Result<PathBuf, String> {
        Err("Web 版ではローカル git クローンは不要です。".to_string())
    }

    pub fn git_pull(_access_token: &str, _repo_root: &Path) -> Result<String, String> {
        Err("Web 版ではページ側の GitHub 同期を利用してください。".to_string())
    }

    pub fn git_commit_and_push(
        _access_token: &str,
        _repo_root: &Path,
        _login: &str,
        _message: &str,
    ) -> Result<String, String> {
        Err("Web 版ではページ側の GitHub 保存を利用してください。".to_string())
    }
}

#[cfg(target_family = "wasm")]
pub use wasm_stub::*;
