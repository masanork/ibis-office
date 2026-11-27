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
pub const GITHUB_COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
pub const GITHUB_COPILOT_API_BASE_URL: &str = "https://api.githubcopilot.com";
pub const GITHUB_COPILOT_MODELS_URL: &str = "https://api.githubcopilot.com/models";
pub const GITHUB_COPILOT_CHAT_URL: &str = "https://api.githubcopilot.com/chat/completions";
pub const COPILOT_EDITOR_VERSION: &str = "vscode/1.99.0";
pub const COPILOT_EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.26.0";
pub const COPILOT_INTEGRATION_ID: &str = "vscode-chat";
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HelpLocale {
    Ja,
    En,
}

impl HelpLocale {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ja => "ja",
            Self::En => "en",
        }
    }

    pub fn parse(raw: &str) -> Self {
        if raw.trim().to_ascii_lowercase().starts_with("ja") {
            Self::Ja
        } else {
            Self::En
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpFile {
    pub path: &'static str,
    pub content: &'static str,
}

const COMMON_HELP_SAMPLES: &[HelpFile] = &[
    HelpFile {
        path: "help/samples/mynumber-act-ch1.xml",
        content: include_str!("../../../assets/help/samples/mynumber-act-ch1.xml"),
    },
    HelpFile {
        path: "help/samples/mynumber-act-ch1.lawtext",
        content: include_str!("../../../assets/help/samples/mynumber-act-ch1.lawtext"),
    },
    HelpFile {
        path: "help/samples/knowledge-note.md",
        content: include_str!("../../../assets/help/samples/knowledge-note.md"),
    },
    HelpFile {
        path: "help/samples/marp-deck.md",
        content: include_str!("../../../assets/help/samples/marp-deck.md"),
    },
    HelpFile {
        path: "help/samples/workbook-sheet.md",
        content: include_str!("../../../assets/help/samples/workbook-sheet.md"),
    },
    HelpFile {
        path: "help/samples/mermaid-diagrams.md",
        content: include_str!("../../../assets/help/samples/mermaid-diagrams.md"),
    },
    HelpFile {
        path: "help/samples/payment-demo.cdx.json",
        content: include_str!("../../../assets/help/samples/payment-demo.cdx.json"),
    },
    HelpFile {
        path: "help/samples/ai-eval-results.jsonl",
        content: include_str!("../../../assets/help/samples/ai-eval-results.jsonl"),
    },
    HelpFile {
        path: "help/samples/stations.xml",
        content: include_str!("../../../assets/help/samples/stations.xml"),
    },
    HelpFile {
        path: "help/samples/field-match-candidates.csv",
        content: include_str!("../../../assets/help/samples/field-match-candidates.csv"),
    },
];

const JA_HELP_FILES: &[HelpFile] = &[
    HelpFile {
        path: "help/index.md",
        content: include_str!("../../../assets/help/ja/index.md"),
    },
    HelpFile {
        path: "help/getting-started.md",
        content: include_str!("../../../assets/help/ja/getting-started.md"),
    },
    HelpFile {
        path: "help/shortcuts.md",
        content: include_str!("../../../assets/help/ja/shortcuts.md"),
    },
    HelpFile {
        path: "help/markdown-basics.md",
        content: include_str!("../../../assets/help/ja/markdown-basics.md"),
    },
    HelpFile {
        path: "help/marp-slides.md",
        content: include_str!("../../../assets/help/ja/marp-slides.md"),
    },
    HelpFile {
        path: "help/workbook-markdown.md",
        content: include_str!("../../../assets/help/ja/workbook-markdown.md"),
    },
    HelpFile {
        path: "help/mermaid-diagrams.md",
        content: include_str!("../../../assets/help/ja/mermaid-diagrams.md"),
    },
    HelpFile {
        path: "help/okf-and-vault.md",
        content: include_str!("../../../assets/help/ja/okf-and-vault.md"),
    },
    HelpFile {
        path: "help/office-and-lawtext.md",
        content: include_str!("../../../assets/help/ja/office-and-lawtext.md"),
    },
    HelpFile {
        path: "help/samples/index.md",
        content: include_str!("../../../assets/help/ja/samples/index.md"),
    },
];

const EN_HELP_FILES: &[HelpFile] = &[
    HelpFile {
        path: "help/index.md",
        content: include_str!("../../../assets/help/en/index.md"),
    },
    HelpFile {
        path: "help/getting-started.md",
        content: include_str!("../../../assets/help/en/getting-started.md"),
    },
    HelpFile {
        path: "help/shortcuts.md",
        content: include_str!("../../../assets/help/en/shortcuts.md"),
    },
    HelpFile {
        path: "help/markdown-basics.md",
        content: include_str!("../../../assets/help/en/markdown-basics.md"),
    },
    HelpFile {
        path: "help/marp-slides.md",
        content: include_str!("../../../assets/help/en/marp-slides.md"),
    },
    HelpFile {
        path: "help/workbook-markdown.md",
        content: include_str!("../../../assets/help/en/workbook-markdown.md"),
    },
    HelpFile {
        path: "help/mermaid-diagrams.md",
        content: include_str!("../../../assets/help/en/mermaid-diagrams.md"),
    },
    HelpFile {
        path: "help/okf-and-vault.md",
        content: include_str!("../../../assets/help/en/okf-and-vault.md"),
    },
    HelpFile {
        path: "help/office-and-lawtext.md",
        content: include_str!("../../../assets/help/en/office-and-lawtext.md"),
    },
    HelpFile {
        path: "help/samples/index.md",
        content: include_str!("../../../assets/help/en/samples/index.md"),
    },
];

pub fn help_files(locale: HelpLocale) -> Vec<HelpFile> {
    let localized = match locale {
        HelpLocale::Ja => JA_HELP_FILES,
        HelpLocale::En => EN_HELP_FILES,
    };
    let mut out = Vec::with_capacity(localized.len() + COMMON_HELP_SAMPLES.len());
    out.extend_from_slice(localized);
    out.extend_from_slice(COMMON_HELP_SAMPLES);
    out
}

pub fn help_file(locale: HelpLocale, path: &str) -> Option<&'static str> {
    let localized = match locale {
        HelpLocale::Ja => JA_HELP_FILES,
        HelpLocale::En => EN_HELP_FILES,
    };
    localized
        .iter()
        .chain(COMMON_HELP_SAMPLES.iter())
        .find(|f| f.path == path)
        .map(|f| f.content)
}

/// Generate the initial OKF-compliant help `index.md` for a newly created Vault repository.
pub fn build_initial_help_index_markdown(repo: &str) -> String {
    build_initial_help_index_markdown_for_locale(repo, HelpLocale::Ja)
}

/// Generate the initial OKF-compliant `index.md` for a newly created Vault repository in the given locale.
pub fn build_initial_help_index_markdown_for_locale(repo: &str, locale: HelpLocale) -> String {
    match locale {
        HelpLocale::Ja => [
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
            "## ヘルプ・記法ガイド",
            "",
            "すべての Markdown 記法や ibisOffice / soraneDocs の操作方法は `help/` フォルダにまとまっています。",
            "",
            "- [[help/index|ヘルプ目次]]",
            "- [[help/getting-started|基本操作ガイド]]",
            "- [[help/shortcuts|キーボードショートカット一覧]]",
            "- [[help/markdown-basics|Markdown 基本・拡張記法]]",
            "- [[help/marp-slides|Marp スライド記法]]",
            "- [[help/workbook-markdown|Workbook Markdown 記法]]",
            "- [[help/mermaid-diagrams|Mermaid 図表ガイド]]",
            "- [[help/okf-and-vault|OKF と Vault 活用]]",
            "- [[help/office-and-lawtext|Office 文書・Lawtext・構造化データ]]",
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
            "├── help/         # 操作ガイド・Markdown 記法リファレンス",
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
        .join("\n"),
        HelpLocale::En => [
            "---",
            "type: index",
            &format!("title: {repo} — soraneDocs Guide"),
            "description: Personal knowledge and documents managed with soraneDocs.",
            "profile: sorane-okf/0.1",
            "---",
            "",
            &format!("# {repo}"),
            "",
            "Welcome to your private document repository created with soraneDocs.",
            "This repository is your home for storing, editing, and committing notes, documents, spreadsheets, and slides directly from your browser or desktop.",
            "",
            "## Help & Syntax Reference",
            "",
            "All Markdown syntax guides and ibisOffice / soraneDocs instructions are included in the `help/` folder:",
            "",
            "- [[help/index|Help Index]]",
            "- [[help/getting-started|Getting Started]]",
            "- [[help/shortcuts|Keyboard Shortcuts]]",
            "- [[help/markdown-basics|Markdown Basics & Extensions]]",
            "- [[help/marp-slides|Marp Slide Syntax]]",
            "- [[help/workbook-markdown|Workbook Markdown Syntax]]",
            "- [[help/mermaid-diagrams|Mermaid Diagrams Guide]]",
            "- [[help/okf-and-vault|OKF & Vault Workflow]]",
            "- [[help/office-and-lawtext|Office Documents, Lawtext & Structured Data]]",
            "",
            "## 1. Quick Start",
            "",
            "1. **Open a document**: Click any file from the repository list on the start view.",
            "2. **Edit and save**: When you save a repository file, it is committed directly to this repository after confirmation.",
            "3. **Switch repository or subfolder**: Use the `GitHub: ...` button at the top of the start view.",
            "",
            "## 2. Supported Formats",
            "",
            "| Category | Extensions | Typical Use |",
            "| --- | --- | --- |",
            "| Markdown & Notes | `.md`, `.markdown` | Notes, meeting minutes, specs, OKF knowledge |",
            "| Office Documents | `.docx`, `.jtd`, `.xdw` | Proposals, contracts, Ichitaro / DocuWorks |",
            "| Spreadsheets & Data | `.xlsx`, `.csv`, `.tsv`, `.json` | Budgets, ledgers, structured data |",
            "| Slides & Diagrams | `.pptx`, `.vsdx` | Presentations, architecture diagrams |",
            "",
            "## 3. Suggested Folder Layout",
            "",
            "```text",
            &format!("{repo}/"),
            "├── index.md      # Repository guide & table of contents",
            "├── help/         # Usage guides & Markdown syntax reference",
            "├── notes/        # Daily notes & meeting minutes (Markdown)",
            "├── docs/         # Proposals, specs & handouts (DOCX / XLSX / PPTX)",
            "└── archive/      # Reference materials",
            "```",
            "",
        ]
        .join("\n"),
    }
}

/// Build all initial files (`index.md` and `help/*.md`) for a newly created Vault.
pub fn build_initial_vault_files(repo: &str, locale: HelpLocale) -> Vec<(String, String)> {
    let mut files = Vec::with_capacity(1 + help_files(locale).len());
    files.push((
        "index.md".to_string(),
        build_initial_help_index_markdown_for_locale(repo, locale),
    ));
    for help in help_files(locale) {
        files.push((help.path.to_string(), help.content.to_string()));
    }
    files
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
    build_init_index_put_body_for_locale(repo, branch, HelpLocale::Ja)
}

pub fn build_init_index_put_body_for_locale(
    repo: &str,
    branch: &str,
    locale: HelpLocale,
) -> String {
    let markdown = build_initial_help_index_markdown_for_locale(repo, locale);
    build_content_put_body(
        "Initialize soraneDocs repository with OKF help index.md",
        &markdown,
        branch,
        None,
    )
}

pub fn build_content_put_body(
    message: &str,
    content: &str,
    branch: &str,
    sha: Option<&str>,
) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
    let mut obj = serde_json::json!({
        "message": message,
        "content": encoded,
        "branch": branch,
    });
    if let Some(sha) = sha {
        obj["sha"] = serde_json::Value::String(sha.to_string());
    }
    obj.to_string()
}

/// Short-lived Copilot session token returned by `GET https://api.github.com/copilot_internal/v2/token`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopilotTokenResponse {
    pub token: String,
    pub expires_at: u64,
    pub api_base: String,
}

/// Model metadata parsed from `GET {copilot_api_base}/models`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopilotModel {
    pub id: String,
    pub name: String,
    pub vendor: Option<String>,
    pub model_picker_enabled: bool,
    pub supports_streaming: bool,
    pub supports_tool_calls: bool,
    pub max_prompt_tokens: Option<usize>,
    pub max_context_window_tokens: Option<usize>,
}

impl CopilotModel {
    pub fn display_label(&self) -> String {
        if self.name.trim().is_empty() || self.name.eq_ignore_ascii_case(&self.id) {
            format!("GitHub Copilot: {}", self.id)
        } else {
            format!("GitHub Copilot: {} ({})", self.name.trim(), self.id)
        }
    }

    pub fn max_input_chars(&self) -> usize {
        let tokens = self
            .max_prompt_tokens
            .or(self.max_context_window_tokens)
            .unwrap_or(16_000);
        ((tokens as f64) * 2.5).round().clamp(8_000.0, 120_000.0) as usize
    }
}

pub fn parse_copilot_token_response(json_str: &str) -> Result<CopilotTokenResponse, String> {
    #[derive(Deserialize)]
    struct EndpointsRaw {
        api: Option<String>,
    }
    #[derive(Deserialize)]
    struct Raw {
        token: Option<String>,
        expires_at: Option<u64>,
        endpoints: Option<EndpointsRaw>,
        message: Option<String>,
    }
    let raw: Raw = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    let token = raw
        .token
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| raw.message.unwrap_or_else(|| "missing copilot token".to_string()))?;
    let api_base = raw
        .endpoints
        .and_then(|ep| ep.api)
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| s.starts_with("https://"))
        .unwrap_or_else(|| GITHUB_COPILOT_API_BASE_URL.to_string());
    Ok(CopilotTokenResponse {
        token,
        expires_at: raw.expires_at.unwrap_or(0),
        api_base,
    })
}

pub fn parse_copilot_models_response(json_str: &str) -> Result<Vec<CopilotModel>, String> {
    #[derive(Deserialize)]
    struct LimitsRaw {
        max_prompt_tokens: Option<usize>,
        max_context_window_tokens: Option<usize>,
    }
    #[derive(Deserialize)]
    struct SupportsRaw {
        streaming: Option<bool>,
        tool_calls: Option<bool>,
    }
    #[derive(Deserialize)]
    struct CapabilitiesRaw {
        #[serde(rename = "type")]
        kind: Option<String>,
        limits: Option<LimitsRaw>,
        supports: Option<SupportsRaw>,
    }
    #[derive(Deserialize)]
    struct PolicyRaw {
        state: Option<String>,
    }
    #[derive(Deserialize)]
    struct ModelRaw {
        id: Option<String>,
        name: Option<String>,
        vendor: Option<String>,
        model_picker_enabled: Option<bool>,
        policy: Option<PolicyRaw>,
        capabilities: Option<CapabilitiesRaw>,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wrapper {
        Object { data: Vec<ModelRaw> },
        Array(Vec<ModelRaw>),
    }

    let items = match serde_json::from_str::<Wrapper>(json_str).map_err(|e| e.to_string())? {
        Wrapper::Object { data } => data,
        Wrapper::Array(data) => data,
    };

    let mut all_chat = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for raw in items {
        let Some(id) = raw.id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let policy_disabled = raw
            .policy
            .as_ref()
            .and_then(|p| p.state.as_deref())
            .is_some_and(|state| {
                let s = state.to_ascii_lowercase();
                s == "disabled" || s == "unconfigured" || s == "blocked"
            });
        if policy_disabled {
            continue;
        }
        let kind = raw
            .capabilities
            .as_ref()
            .and_then(|c| c.kind.as_deref())
            .map(str::to_ascii_lowercase);
        if let Some(kind) = kind.as_deref()
            && kind != "chat"
        {
            continue;
        }
        let limits = raw.capabilities.as_ref().and_then(|c| c.limits.as_ref());
        let supports = raw.capabilities.as_ref().and_then(|c| c.supports.as_ref());
        all_chat.push(CopilotModel {
            name: raw
                .name
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| id.clone()),
            id,
            vendor: raw.vendor.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            model_picker_enabled: raw.model_picker_enabled.unwrap_or(true),
            supports_streaming: supports.and_then(|s| s.streaming).unwrap_or(true),
            supports_tool_calls: supports.and_then(|s| s.tool_calls).unwrap_or(false),
            max_prompt_tokens: limits.and_then(|l| l.max_prompt_tokens),
            max_context_window_tokens: limits.and_then(|l| l.max_context_window_tokens),
        });
    }

    let picker_models: Vec<CopilotModel> = all_chat
        .iter()
        .filter(|m| m.model_picker_enabled)
        .cloned()
        .collect();
    if picker_models.is_empty() {
        Ok(all_chat)
    } else {
        Ok(picker_models)
    }
}

pub fn is_fast_completion_model(model_id: &str) -> bool {
    let lower = model_id.to_ascii_lowercase();
    lower.contains("mini")
        || lower.contains("flash")
        || lower.contains("haiku")
        || lower.contains("nano")
        || lower.contains("lite")
        || lower.contains("small")
}

/// Dynamically select a model from the fetched Copilot model catalog.
/// Never invents a model ID that is absent from `models`.
pub fn select_copilot_model<'a>(
    models: &'a [CopilotModel],
    preferred: Option<&str>,
    prefer_fast: bool,
) -> Option<&'a CopilotModel> {
    if let Some(pref) = preferred.map(str::trim).filter(|s| !s.is_empty())
        && let Some(found) = models.iter().find(|m| m.id == pref)
    {
        return Some(found);
    }
    if prefer_fast {
        if let Some(fast) = models
            .iter()
            .find(|m| m.supports_streaming && is_fast_completion_model(&m.id))
        {
            return Some(fast);
        }
    } else if let Some(standard) = models
        .iter()
        .find(|m| m.supports_streaming && m.supports_tool_calls && !is_fast_completion_model(&m.id))
    {
        return Some(standard);
    }
    models
        .iter()
        .find(|m| m.supports_streaming)
        .or_else(|| models.first())
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
        assert!(md.contains("[[help/index|"));
        assert!(md.contains("## 1. 基本的な使い方"));
        assert!(md.contains("## 2. 対応しているファイル形式"));
        assert!(md.contains("## 3. おすすめのフォルダ構成"));

        let en_md = build_initial_help_index_markdown_for_locale("soraneDocs", HelpLocale::En);
        assert!(en_md.contains("profile: sorane-okf/0.1"));
        assert!(en_md.contains("[[help/index|Help Index]]"));
    }

    #[test]
    fn builtin_help_files_exist_for_ja_and_en() {
        for locale in [HelpLocale::Ja, HelpLocale::En] {
            let files = help_files(locale);
            assert_eq!(files.len(), 20);
            assert!(help_file(locale, "help/index.md").is_some());
            assert!(help_file(locale, "help/shortcuts.md").is_some());
            assert!(help_file(locale, "help/markdown-basics.md").is_some());
            assert!(help_file(locale, "help/marp-slides.md").is_some());
            assert!(help_file(locale, "help/workbook-markdown.md").is_some());
            assert!(help_file(locale, "help/mermaid-diagrams.md").is_some());
            assert!(help_file(locale, "help/okf-and-vault.md").is_some());
            assert!(help_file(locale, "help/office-and-lawtext.md").is_some());
            assert!(help_file(locale, "help/getting-started.md").is_some());
            assert!(help_file(locale, "help/samples/index.md").is_some());
            assert!(help_file(locale, "help/samples/mynumber-act-ch1.xml").is_some());
            assert!(help_file(locale, "help/samples/mynumber-act-ch1.lawtext").is_some());
            assert!(help_file(locale, "help/samples/payment-demo.cdx.json").is_some());
            assert!(help_file(locale, "help/samples/ai-eval-results.jsonl").is_some());
            assert!(help_file(locale, "help/samples/stations.xml").is_some());
            assert!(help_file(locale, "help/samples/field-match-candidates.csv").is_some());
            let initial = build_initial_vault_files("soraneDocs", locale);
            assert_eq!(initial.len(), 21);
            assert_eq!(initial[0].0, "index.md");
        }
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

    #[test]
    fn copilot_token_and_models_parse_and_select_dynamically() {
        let token = parse_copilot_token_response(
            r#"{"token":"tid=abc","expires_at":1800000000,"endpoints":{"api":"https://api.individual.githubcopilot.com/"}}"#,
        )
        .unwrap();
        assert_eq!(token.token, "tid=abc");
        assert_eq!(token.api_base, "https://api.individual.githubcopilot.com");

        let models = parse_copilot_models_response(
            r#"{
              "object": "list",
              "data": [
                {"id": "text-embedding-3-small", "capabilities": {"type": "embeddings"}},
                {"id": "blocked-model", "policy": {"state": "disabled"}, "capabilities": {"type": "chat"}},
                {"id": "gpt-4.1", "name": "GPT-4.1", "model_picker_enabled": true, "capabilities": {"type": "chat", "supports": {"streaming": true, "tool_calls": true}, "limits": {"max_prompt_tokens": 64000}}},
                {"id": "gpt-5-mini", "name": "GPT-5 mini", "model_picker_enabled": true, "capabilities": {"type": "chat", "supports": {"streaming": true, "tool_calls": true}, "limits": {"max_prompt_tokens": 32000}}}
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(select_copilot_model(&models, None, false).unwrap().id, "gpt-4.1");
        assert_eq!(select_copilot_model(&models, None, true).unwrap().id, "gpt-5-mini");
        assert_eq!(select_copilot_model(&models, Some("gpt-5-mini"), false).unwrap().id, "gpt-5-mini");
        assert_eq!(select_copilot_model(&models, Some("nonexistent"), true).unwrap().id, "gpt-5-mini");
    }
}
