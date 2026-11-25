// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Platform-agnostic GitHub App Vault models, OAuth Device Flow parsers,
//! repository catalog evaluation, and OKF help initialization helpers.
//!
//! Designed to compile cleanly on both desktop and `wasm32-unknown-unknown`.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

pub const GITHUB_APP_CLIENT_ID: &str = "Iv23lifyEQqiR8HPnQCc";
pub const GITHUB_APP_SLUG: &str = "soranedocs";
pub const GITHUB_APP_INSTALL_URL: &str =
    "https://github.com/apps/soranedocs/installations/new";
pub const GITHUB_API_BASE: &str = "https://api.github.com";
pub const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
pub const GITHUB_OAUTH_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
pub const DEFAULT_VAULT_REPO_NAME: &str = "soraneDocs";
pub const GITHUB_API_VERSION: &str = "2022-11-28";

/// Validate a repository name segment (`[A-Za-z0-9._-]{1,100}`).
pub fn is_valid_repo_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 100
        && trimmed != "."
        && trimmed != ".."
        && trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Generate the initial OKF-compliant help `index.md` for a newly created Vault repository.
pub fn build_initial_help_index_markdown(repo: &str) -> String {
    [
        "---",
        "type: index",
        &format!("title: {repo} — soraneDocs ガイド"),
        "description: Personal knowledge and documents managed with soraneDocs.",
        "profile: sorane-okf/0.1",
        "---",
        "",
        &format!("# {repo}"),
        "",
        "soraneDocs で作成したプライベートドキュメントリポジトリへようこそ。",
        "このリポジトリは、あなたの文書・メモ・表計算・スライドを安全に保管し、ブラウザやデスクトップから直接編集・保存するための拠点です。",
        "",
        "## 1. 基本的な使い方",
        "",
        "1. **文書を開く**: スタート画面のリポジトリ一覧からファイルをクリックして開きます。",
        "2. **編集して保存する**: リポジトリから開いたファイルを保存・ダウンロードすると、確認ダイアログのあとにこのリポジトリへ直接コミットされます。",
        "3. **リポジトリやフォルダを切り替える**: スタート画面上部の `GitHub: ...` ボタンから、別のリポジトリやサブフォルダ（例: `docs`）を指定できます。",
        "",
        "## 2. 対応しているファイル形式",
        "",
        "| カテゴリ | 対応拡張子 | 主な用途 |",
        "| --- | --- | --- |",
        "| マークダウン・文書 | `.md`, `.markdown` | メモ、議事録、設計書、OKF ナレッジ |",
        "| オフィス文書 | `.docx`, `.jtd`, `.xdw` | 企画書、契約書、一太郎・DocuWorks 文書 |",
        "| 表計算・データ | `.xlsx`, `.csv`, `.tsv`, `.json` | 集計表、台帳、データ一覧 |",
        "| プレゼン・図面 | `.pptx`, `.vsdx` | 発表資料、構成図、フロー図 |",
        "",
        "## 3. おすすめのフォルダ構成",
        "",
        "```text",
        &format!("{repo}/"),
        "├── index.md      # このガイド・全体の目次",
        "├── notes/        # 日々のメモ・議事録（Markdown）",
        "├── docs/         # 企画書・仕様書・配布資料（DOCX / XLSX / PPTX）",
        "└── archive/      # 保管用の参照資料",
        "```",
        "",
        "## 4. OKF メモの書き方ヒント",
        "",
        "Markdown ファイルの先頭に YAML フロントマター（`---` で囲んだメタデータ）を記述すると、文書の種別やタグを構造化して管理できます。",
        "",
        "```markdown",
        "---",
        "type: note",
        "title: 会議メモ",
        "tags: [meeting, project]",
        "---",
        "",
        "# 会議メモ",
        "```",
        "",
        "## 5. トラブルシューティング",
        "",
        "- **新しいリポジトリが見えない／保存できない場合**: GitHub App（soraneDocs）のインストール設定で、リポジトリアクセスが **All repositories** になっているか、対象リポジトリが許可されているか確認してください。",
        "- **ログインが切れた場合**: スタート画面上部の GitHub ボタンから再ログインすると、最新のトークンで再接続できます。",
        "",
    ]
    .join("\n")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubUser {
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubInstallation {
    pub id: u64,
    #[serde(default)]
    pub account_login: Option<String>,
    #[serde(default)]
    pub repository_selection: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub permissions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubRepo {
    pub owner: String,
    pub repo: String,
    pub full_name: String,
    pub default_branch: String,
    pub private: bool,
    pub clone_url: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoCatalog {
    pub repos: Vec<GitHubRepo>,
    pub installed: bool,
    pub can_create_repo: bool,
    pub repo_selection: Option<String>,
    pub install_settings_url: Option<String>,
    pub has_sorane_docs_repo: bool,
}

/// Evaluate installation permissions, `All repositories` requirement, and existing `soraneDocs` repo.
pub fn evaluate_catalog(
    login: &str,
    installations: &[GitHubInstallation],
    mut repos: Vec<GitHubRepo>,
) -> RepoCatalog {
    let installed = !installations.is_empty();
    let login_lower = login.to_ascii_lowercase();
    let personal_inst = installations
        .iter()
        .find(|inst| {
            inst.account_login
                .as_deref()
                .is_some_and(|acct| acct.to_ascii_lowercase() == login_lower)
        })
        .or_else(|| installations.first());

    let (can_create_repo, repo_selection, install_settings_url) = match personal_inst {
        Some(inst) => {
            let has_create_perm = inst
                .permissions
                .get("repository_creation")
                .is_some_and(|v| v == "write")
                || inst
                    .permissions
                    .get("administration")
                    .is_some_and(|v| v == "write");
            let is_selected = inst.repository_selection.as_deref() == Some("selected");
            (
                has_create_perm && !is_selected,
                inst.repository_selection.clone(),
                inst.html_url.clone(),
            )
        }
        None => (false, None, None),
    };

    repos.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let has_sorane_docs_repo = repos.iter().any(|r| {
        r.owner.to_ascii_lowercase() == login_lower && r.repo.eq_ignore_ascii_case(DEFAULT_VAULT_REPO_NAME)
    });

    RepoCatalog {
        repos,
        installed,
        can_create_repo,
        repo_selection,
        install_settings_url,
        has_sorane_docs_repo,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    #[serde(default = "default_device_interval")]
    pub interval: u64,
}

fn default_device_interval() -> u64 {
    5
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceTokenPoll {
    Pending,
    SlowDown,
    Authorized { access_token: String },
    Expired,
    AccessDenied,
    Error(String),
}

pub fn parse_device_code_response(json_str: &str) -> Result<DeviceCodeResponse, String> {
    #[derive(Deserialize)]
    struct Raw {
        device_code: Option<String>,
        user_code: Option<String>,
        verification_uri: Option<String>,
        expires_in: Option<u64>,
        interval: Option<u64>,
        error: Option<String>,
        error_description: Option<String>,
    }
    let raw: Raw = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    if let Some(err) = raw.error {
        return Err(raw.error_description.unwrap_or(err));
    }
    Ok(DeviceCodeResponse {
        device_code: raw.device_code.ok_or("missing device_code")?,
        user_code: raw.user_code.ok_or("missing user_code")?,
        verification_uri: raw.verification_uri.ok_or("missing verification_uri")?,
        expires_in: raw.expires_in.unwrap_or(900),
        interval: raw.interval.unwrap_or(5).max(1),
    })
}

pub fn parse_device_token_response(json_str: &str) -> Result<DeviceTokenPoll, String> {
    #[derive(Deserialize)]
    struct Raw {
        access_token: Option<String>,
        error: Option<String>,
        error_description: Option<String>,
    }
    let raw: Raw = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    if let Some(token) = raw.access_token {
        return Ok(DeviceTokenPoll::Authorized {
            access_token: token,
        });
    }
    match raw.error.as_deref() {
        Some("authorization_pending") => Ok(DeviceTokenPoll::Pending),
        Some("slow_down") => Ok(DeviceTokenPoll::SlowDown),
        Some("expired_token") => Ok(DeviceTokenPoll::Expired),
        Some("access_denied") => Ok(DeviceTokenPoll::AccessDenied),
        Some(other) => Ok(DeviceTokenPoll::Error(
            raw.error_description.unwrap_or_else(|| other.to_string()),
        )),
        None => Err("missing access_token and error in OAuth response".to_string()),
    }
}

pub fn parse_user_response(json_str: &str) -> Result<GitHubUser, String> {
    serde_json::from_str(json_str).map_err(|e| e.to_string())
}

pub fn parse_installations_response(json_str: &str) -> Result<Vec<GitHubInstallation>, String> {
    #[derive(Deserialize)]
    struct AccountRaw {
        login: Option<String>,
    }
    #[derive(Deserialize)]
    struct InstRaw {
        id: u64,
        account: Option<AccountRaw>,
        repository_selection: Option<String>,
        html_url: Option<String>,
        #[serde(default)]
        permissions: BTreeMap<String, String>,
    }
    #[derive(Deserialize)]
    struct Wrapper {
        #[serde(default)]
        installations: Vec<InstRaw>,
    }
    let wrapper: Wrapper = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    Ok(wrapper
        .installations
        .into_iter()
        .map(|inst| GitHubInstallation {
            id: inst.id,
            account_login: inst.account.and_then(|a| a.login),
            repository_selection: inst.repository_selection,
            html_url: inst.html_url,
            permissions: inst.permissions,
        })
        .collect())
}

#[derive(Deserialize)]
struct RepoOwnerRaw {
    login: String,
}

#[derive(Deserialize)]
struct RepoRaw {
    full_name: String,
    name: String,
    owner: RepoOwnerRaw,
    default_branch: Option<String>,
    #[serde(default)]
    private: bool,
    clone_url: Option<String>,
    updated_at: Option<String>,
}

impl From<RepoRaw> for GitHubRepo {
    fn from(r: RepoRaw) -> Self {
        let clone_url = r
            .clone_url
            .unwrap_or_else(|| format!("https://github.com/{}.git", r.full_name));
        Self {
            owner: r.owner.login,
            repo: r.name,
            full_name: r.full_name,
            default_branch: r.default_branch.unwrap_or_else(|| "main".to_string()),
            private: r.private,
            clone_url,
            updated_at: r.updated_at.unwrap_or_default(),
        }
    }
}

pub fn parse_installation_repos_response(json_str: &str) -> Result<Vec<GitHubRepo>, String> {
    #[derive(Deserialize)]
    struct Wrapper {
        #[serde(default)]
        repositories: Vec<RepoRaw>,
    }
    let wrapper: Wrapper = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    Ok(wrapper.repositories.into_iter().map(Into::into).collect())
}

pub fn parse_user_repos_response(json_str: &str) -> Result<Vec<GitHubRepo>, String> {
    let raw: Vec<RepoRaw> = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    Ok(raw.into_iter().map(Into::into).collect())
}

pub fn parse_created_repo_response(json_str: &str) -> Result<GitHubRepo, String> {
    let raw: RepoRaw = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    Ok(raw.into())
}

pub fn build_create_repo_body(name: &str, description: &str) -> String {
    serde_json::json!({
        "name": name,
        "description": description,
        "private": true,
        "auto_init": false,
    })
    .to_string()
}

pub fn build_init_index_put_body(repo: &str, branch: &str) -> String {
    let markdown = build_initial_help_index_markdown(repo);
    let encoded = base64::engine::general_purpose::STANDARD.encode(markdown.as_bytes());
    serde_json::json!({
        "message": "Initialize soraneDocs repository with OKF help index.md",
        "content": encoded,
        "branch": branch,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_name_validation_matches_worker_rules() {
        assert!(is_valid_repo_name("soraneDocs"));
        assert!(is_valid_repo_name("my-notes_2026.vault"));
        assert!(!is_valid_repo_name(""));
        assert!(!is_valid_repo_name(".."));
        assert!(!is_valid_repo_name("a/b"));
        assert!(!is_valid_repo_name("has space"));
    }

    #[test]
    fn initial_help_index_has_okf_frontmatter_and_sections() {
        let md = build_initial_help_index_markdown("soraneDocs");
        assert!(md.contains("profile: sorane-okf/0.1"));
        assert!(md.contains("## 1. 基本的な使い方"));
        assert!(md.contains("## 2. 対応しているファイル形式"));
        assert!(md.contains("## 3. おすすめのフォルダ構成"));
    }

    #[test]
    fn evaluate_catalog_requires_all_repositories_and_detects_existing_soranedocs() {
        let inst = GitHubInstallation {
            id: 10,
            account_login: Some("masanork".to_string()),
            repository_selection: Some("selected".to_string()),
            html_url: Some("https://github.com/settings/installations/10".to_string()),
            permissions: BTreeMap::from([(
                "repository_creation".to_string(),
                "write".to_string(),
            )]),
        };
        let repo = GitHubRepo {
            owner: "masanork".to_string(),
            repo: "soraneDocs".to_string(),
            full_name: "masanork/soraneDocs".to_string(),
            default_branch: "main".to_string(),
            private: true,
            clone_url: "https://github.com/masanork/soraneDocs.git".to_string(),
            updated_at: "2026-11-25T00:00:00Z".to_string(),
        };

        let selected = evaluate_catalog("masanork", std::slice::from_ref(&inst), vec![repo]);
        assert!(selected.installed);
        assert!(!selected.can_create_repo);
        assert!(selected.has_sorane_docs_repo);

        let mut all_inst = inst;
        all_inst.repository_selection = Some("all".to_string());
        let all = evaluate_catalog("masanork", &[all_inst], vec![]);
        assert!(all.can_create_repo);
        assert!(!all.has_sorane_docs_repo);
    }

    #[test]
    fn device_token_poll_parses_all_states() {
        assert_eq!(
            parse_device_token_response(r#"{"error":"authorization_pending"}"#).unwrap(),
            DeviceTokenPoll::Pending
        );
        assert_eq!(
            parse_device_token_response(r#"{"error":"slow_down"}"#).unwrap(),
            DeviceTokenPoll::SlowDown
        );
        assert_eq!(
            parse_device_token_response(r#"{"access_token":"ghu_abc"}"#).unwrap(),
            DeviceTokenPoll::Authorized {
                access_token: "ghu_abc".to_string()
            }
        );
    }
}
