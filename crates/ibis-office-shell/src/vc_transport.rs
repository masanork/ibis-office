// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Phase 2 Verifiable Credentials desktop keystore and OID4VP `direct_post` HTTP transport.
//!
//! Security invariants:
//! - Private ES256 keys (Holder & Issuer) are NEVER stored inside a Vault folder or Git repository.
//!   They are persisted in the OS keychain when available, or in an owner-only (`0600`) file under
//!   `ibis_office_base::app_data::base()/ibisOffice/vc-keys.json`.
//! - `direct_post` HTTP POSTs validate `response_uri` strictly (`https://`, or loopback
//!   `http://127.0.0.1` / `http://localhost` / `http://[::1]` for local verifier testing) and stream
//!   the `vp_token` body over `curl`'s stdin (`--data-binary @-`) so tokens never appear in process argv.

use ibis_doc_model::Es256KeyPair;

/// Purpose of a stored ES256 keypair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRole {
    /// Holder Key Binding key (`cnf.jwk` and RFC 9901 `kb+jwt`).
    Holder,
    /// Optional user-managed Issuer signing key for issuing `.sd-jwt` / `.mdoc`.
    Issuer,
}

impl KeyRole {
    fn account_name(self) -> &'static str {
        match self {
            Self::Holder => "holder_default",
            Self::Issuer => "issuer_default",
        }
    }
}

/// Where an active ES256 keypair was loaded from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStorageBackend {
    /// macOS Keychain (`security`) or Linux Secret Service (`secret-tool`).
    OsKeychain,
    /// Owner-only (`0600`) file in the application data directory (outside Vault/Git).
    LocalAppData,
    /// Deterministic built-in demo key (not for production trust).
    SandboxDemo,
}

impl KeyStorageBackend {
    pub fn is_sandbox(self) -> bool {
        matches!(self, Self::SandboxDemo)
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::OsKeychain => "os_keychain",
            Self::LocalAppData => "local_app_data",
            Self::SandboxDemo => "sandbox_demo",
        }
    }

    pub fn label_ja(self) -> &'static str {
        match self {
            Self::OsKeychain => "OS キーチェーン",
            Self::LocalAppData => "ローカル鍵 (0600)",
            Self::SandboxDemo => "Sandbox / テスト鍵",
        }
    }
}

/// A loaded ES256 keypair paired with its storage provenance and safe display thumbprint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedKey {
    pub key: Es256KeyPair,
    pub backend: KeyStorageBackend,
    pub thumbprint: String,
}

impl LoadedKey {
    fn new(key: Es256KeyPair, backend: KeyStorageBackend) -> Self {
        let thumbprint = key.thumbprint_short();
        Self {
            key,
            backend,
            thumbprint,
        }
    }

    pub fn sandbox_holder() -> Self {
        Self::new(Es256KeyPair::demo_holder_key(), KeyStorageBackend::SandboxDemo)
    }

    pub fn sandbox_issuer() -> Self {
        Self::new(Es256KeyPair::demo_issuer_key(), KeyStorageBackend::SandboxDemo)
    }
}

/// Validated OID4VP `response_uri`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedDirectPostUri {
    pub uri: String,
    pub is_loopback: bool,
}

/// Result of an OID4VP `response_mode=direct_post` HTTP POST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectPostOutcome {
    pub status: u16,
    pub response_uri: String,
    pub is_loopback: bool,
    pub body: String,
    pub redirect_uri: Option<String>,
}

/// Strictly validates an OID4VP `response_uri` before transmitting credentials.
///
/// Allows:
/// - `https://<host>...` (non-empty host, no whitespace/control chars/quotes)
/// - `http://127.0.0.1[:port][/...]`, `http://localhost[:port][/...]`, `http://[::1][:port][/...]`
///   strictly for local verifier testing (flagged with `is_loopback = true`).
pub fn validate_direct_post_uri(raw: &str) -> Result<ValidatedDirectPostUri, String> {
    let uri = raw.trim();
    if uri.is_empty() {
        return Err("送信先 response_uri が空です。".to_string());
    }
    if uri.len() > 2048 {
        return Err("response_uri が長すぎます。".to_string());
    }
    if uri
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
    {
        return Err("response_uri に使用できない文字が含まれています。".to_string());
    }

    if let Some(rest) = uri.strip_prefix("https://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.is_empty() || authority.contains('@') || authority.starts_with(':') {
            return Err("https:// のホスト名が不正です。".to_string());
        }
        return Ok(ValidatedDirectPostUri {
            uri: uri.to_string(),
            is_loopback: false,
        });
    }

    if let Some(rest) = uri.strip_prefix("http://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.contains('@') {
            return Err("不正な authority が含まれています。".to_string());
        }
        let is_loopback = is_strict_loopback_authority(authority);
        if is_loopback {
            return Ok(ValidatedDirectPostUri {
                uri: uri.to_string(),
                is_loopback: true,
            });
        }
        return Err(
            "暗号化されていない http:// への送信は禁止されています（https:// またはローカルテスト用の http://127.0.0.1 / http://localhost のみ許可）。"
                .to_string(),
        );
    }

    Err("response_uri は https://（またはローカルテスト用の http://127.0.0.1）で始まる必要があります。".to_string())
}

fn is_strict_loopback_authority(authority: &str) -> bool {
    if authority == "localhost" || authority == "127.0.0.1" || authority == "[::1]" {
        return true;
    }
    for prefix in ["localhost:", "127.0.0.1:", "[::1]:"] {
        if let Some(port) = authority.strip_prefix(prefix)
            && !port.is_empty()
            && port.chars().all(|c| c.is_ascii_digit())
            && port.parse::<u16>().is_ok()
        {
            return true;
        }
    }
    false
}

#[cfg(not(target_family = "wasm"))]
mod native {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    use ibis_doc_model::{Es256KeyPair, base64url_encode};
    use serde::{Deserialize, Serialize};

    use super::{
        DirectPostOutcome, KeyRole, KeyStorageBackend, LoadedKey, validate_direct_post_uri,
    };

    const KEYSTORE_FILENAME: &str = "vc-keys.json";
    const KEYCHAIN_SERVICE: &str = "dev.sorane.docs.vc-keys";
    const USER_AGENT: &str = "soraneDocs-Desktop";

    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    struct LocalKeystoreFile {
        version: u32,
        keys: BTreeMap<String, String>,
    }

    fn keystore_file_path() -> Option<PathBuf> {
        let base = ibis_office_base::app_data::base()?;
        Some(base.join("ibisOffice").join(KEYSTORE_FILENAME))
    }

    fn use_os_keychain() -> bool {
        if cfg!(test) || std::env::var_os("SORANE_VC_KEYSTORE_FILE_ONLY").is_some() {
            return false;
        }
        #[cfg(target_os = "macos")]
        {
            true
        }
        #[cfg(target_os = "linux")]
        {
            std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            false
        }
    }

    fn generate_csprng_p256_key() -> Result<Es256KeyPair, String> {
        for _ in 0..32 {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(|e| format!("CSPRNG エラー: {e}"))?;
            if let Some(kp) = Es256KeyPair::from_secret_bytes(bytes) {
                return Ok(kp);
            }
        }
        Err("有効な P-256 秘密鍵を生成できませんでした。".to_string())
    }

    pub fn generate_random_nonce() -> String {
        let mut bytes = [0u8; 16];
        if getrandom::fill(&mut bytes).is_ok() {
            base64url_encode(&bytes)
        } else {
            "n-fallback-nonce".to_string()
        }
    }

    #[cfg(target_os = "macos")]
    fn os_keychain_load(role: KeyRole) -> Option<Es256KeyPair> {
        if !use_os_keychain() {
            return None;
        }
        let output = Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                role.account_name(),
                "-w",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let jwk = String::from_utf8(output.stdout).ok()?;
        Es256KeyPair::from_jwk_json(jwk.trim())
    }

    #[cfg(target_os = "linux")]
    fn os_keychain_load(role: KeyRole) -> Option<Es256KeyPair> {
        if !use_os_keychain() {
            return None;
        }
        let output = Command::new("secret-tool")
            .args([
                "lookup",
                "service",
                KEYCHAIN_SERVICE,
                "role",
                role.account_name(),
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let jwk = String::from_utf8(output.stdout).ok()?;
        Es256KeyPair::from_jwk_json(jwk.trim())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn os_keychain_load(_role: KeyRole) -> Option<Es256KeyPair> {
        None
    }

    #[cfg(target_os = "macos")]
    fn os_keychain_save(role: KeyRole, jwk_json: &str) -> bool {
        if !use_os_keychain() {
            return false;
        }
        // Pass command via `security -i` stdin so private JWK never appears in `ps` argv.
        let Ok(mut child) = Command::new("security")
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let escaped = jwk_json.replace('\\', "\\\\").replace('"', "\\\"");
        let cmd_line = format!(
            "add-generic-password -U -s {KEYCHAIN_SERVICE} -a {} -w \"{escaped}\"\n",
            role.account_name()
        );
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(cmd_line.as_bytes());
        }
        child.wait().is_ok_and(|s| s.success())
    }

    #[cfg(target_os = "linux")]
    fn os_keychain_save(role: KeyRole, jwk_json: &str) -> bool {
        if !use_os_keychain() {
            return false;
        }
        let label = format!("soraneDocs VC Key ({})", role.account_name());
        let Ok(mut child) = Command::new("secret-tool")
            .args([
                "store",
                "--label",
                &label,
                "service",
                KEYCHAIN_SERVICE,
                "role",
                role.account_name(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(jwk_json.as_bytes());
        }
        child.wait().is_ok_and(|s| s.success())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn os_keychain_save(_role: KeyRole, _jwk_json: &str) -> bool {
        false
    }

    #[cfg(target_os = "macos")]
    fn os_keychain_delete(role: KeyRole) {
        if !use_os_keychain() {
            return;
        }
        let _ = Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                role.account_name(),
            ])
            .output();
    }

    #[cfg(target_os = "linux")]
    fn os_keychain_delete(role: KeyRole) {
        if !use_os_keychain() {
            return;
        }
        let _ = Command::new("secret-tool")
            .args([
                "clear",
                "service",
                KEYCHAIN_SERVICE,
                "role",
                role.account_name(),
            ])
            .output();
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn os_keychain_delete(_role: KeyRole) {}

    fn file_keystore_load_all() -> LocalKeystoreFile {
        let Some(path) = keystore_file_path() else {
            return LocalKeystoreFile::default();
        };
        let Ok(raw) = fs::read_to_string(path) else {
            return LocalKeystoreFile::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    fn file_keystore_save_all(store: &LocalKeystoreFile) -> Result<(), String> {
        let Some(path) = keystore_file_path() else {
            return Err("アプリデータ保存先を取得できませんでした。".to_string());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("鍵保存ディレクトリの作成に失敗しました: {e}"))?;
        }
        let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| format!("鍵ファイルの保存に失敗しました: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    fn save_key_pair(role: KeyRole, kp: &Es256KeyPair) -> Result<KeyStorageBackend, String> {
        let jwk_json = kp.to_private_jwk_json();
        if os_keychain_save(role, &jwk_json) {
            return Ok(KeyStorageBackend::OsKeychain);
        }
        let mut store = file_keystore_load_all();
        store.version = 1;
        store
            .keys
            .insert(role.account_name().to_string(), jwk_json);
        file_keystore_save_all(&store)?;
        Ok(KeyStorageBackend::LocalAppData)
    }

    /// Loads an existing persisted keypair for `role`, if any (without generating one).
    pub fn load_persisted_key(role: KeyRole) -> Option<LoadedKey> {
        if let Some(kp) = os_keychain_load(role) {
            return Some(LoadedKey::new(kp, KeyStorageBackend::OsKeychain));
        }
        let store = file_keystore_load_all();
        let jwk = store.keys.get(role.account_name())?;
        let kp = Es256KeyPair::from_jwk_json(jwk)?;
        Some(LoadedKey::new(kp, KeyStorageBackend::LocalAppData))
    }

    /// Loads the persistent Holder keypair, generating and storing a fresh CSPRNG P-256 key
    /// outside the Vault on first use. Falls back to `SandboxDemo` only if storage fails.
    pub fn load_or_create_holder_key() -> LoadedKey {
        if let Some(existing) = load_persisted_key(KeyRole::Holder) {
            return existing;
        }
        match generate_and_save_key(KeyRole::Holder) {
            Ok(created) => created,
            Err(err) => {
                log::warn!("Failed to initialize persistent VC holder key: {err}");
                LoadedKey::sandbox_holder()
            }
        }
    }

    /// Loads the persistent Issuer keypair if the user generated/imported one.
    pub fn load_issuer_key() -> Option<LoadedKey> {
        load_persisted_key(KeyRole::Issuer)
    }

    /// Generates a fresh CSPRNG ES256 keypair for `role` and persists it outside the Vault.
    pub fn generate_and_save_key(role: KeyRole) -> Result<LoadedKey, String> {
        let kp = generate_csprng_p256_key()?;
        let backend = save_key_pair(role, &kp)?;
        Ok(LoadedKey::new(kp, backend))
    }

    /// Imports an ES256 private JWK JSON for `role` and persists it outside the Vault.
    pub fn import_jwk_key(role: KeyRole, jwk_json: &str) -> Result<LoadedKey, String> {
        let kp = Es256KeyPair::from_jwk_json(jwk_json.trim())
            .ok_or_else(|| "有効な ES256 (P-256) 秘密鍵 JWK JSON ではありません。".to_string())?;
        let backend = save_key_pair(role, &kp)?;
        Ok(LoadedKey::new(kp, backend))
    }

    /// Removes the persisted keypair for `role` from both OS keychain and local keystore.
    pub fn clear_key(role: KeyRole) -> Result<(), String> {
        os_keychain_delete(role);
        let mut store = file_keystore_load_all();
        if store.keys.remove(role.account_name()).is_some() {
            file_keystore_save_all(&store)?;
        }
        Ok(())
    }

    /// Sends an OpenID4VP `response_mode=direct_post` HTTP POST (`application/x-www-form-urlencoded`)
    /// using system `curl` with the payload piped over stdin (`--data-binary @-`).
    pub fn send_oid4vp_direct_post(
        response_uri: &str,
        form_body: &str,
    ) -> Result<DirectPostOutcome, String> {
        const STATUS_MARKER: &str = "\n__SORANE_HTTP_STATUS__:";
        let validated = validate_direct_post_uri(response_uri)?;

        let mut child = Command::new("curl")
            .args([
                "-sS",
                "--connect-timeout",
                "10",
                "--max-time",
                "30",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/x-www-form-urlencoded",
                "-H",
                "Accept: application/json",
                "-H",
                &format!("User-Agent: {USER_AGENT}"),
                "-w",
                &format!("{STATUS_MARKER}%{{http_code}}"),
                "--data-binary",
                "@-",
                "--",
                &validated.uri,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("HTTP クライアント (curl) の起動に失敗しました: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(form_body.as_bytes())
                .map_err(|e| format!("direct_post ペイロードの送信に失敗しました: {e}"))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("curl の待機に失敗しました: {e}"))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("OID4VP direct_post 通信エラー: {}", err.trim()));
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

        let redirect_uri = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("redirect_uri").and_then(|r| r.as_str()).map(str::to_string));

        Ok(DirectPostOutcome {
            status,
            response_uri: validated.uri,
            is_loopback: validated.is_loopback,
            body,
            redirect_uri,
        })
    }
}

#[cfg(target_family = "wasm")]
mod wasm {
    use super::{DirectPostOutcome, KeyRole, LoadedKey};

    pub fn generate_random_nonce() -> String {
        "n-wasm-sandbox".to_string()
    }

    pub fn load_persisted_key(_role: KeyRole) -> Option<LoadedKey> {
        None
    }

    pub fn load_or_create_holder_key() -> LoadedKey {
        LoadedKey::sandbox_holder()
    }

    pub fn load_issuer_key() -> Option<LoadedKey> {
        None
    }

    pub fn generate_and_save_key(_role: KeyRole) -> Result<LoadedKey, String> {
        Err("永続鍵管理はデスクトップ版で利用できます。".to_string())
    }

    pub fn import_jwk_key(_role: KeyRole, _jwk_json: &str) -> Result<LoadedKey, String> {
        Err("永続鍵管理はデスクトップ版で利用できます。".to_string())
    }

    pub fn clear_key(_role: KeyRole) -> Result<(), String> {
        Ok(())
    }

    pub fn send_oid4vp_direct_post(
        _response_uri: &str,
        _form_body: &str,
    ) -> Result<DirectPostOutcome, String> {
        Err("OID4VP direct_post 送信はデスクトップ版で利用できます。".to_string())
    }
}

#[cfg(not(target_family = "wasm"))]
pub use native::*;
#[cfg(target_family = "wasm")]
pub use wasm::*;

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    use ibis_doc_model::{
        BoundClaim, BindingSource, ClaimValue, Oid4vpRequest, SdJwtOptions, SheetClaimBindings,
        SlideId, issue_sd_jwt, present_existing_sd_jwt,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn validates_direct_post_uris_strictly() {
        let https = validate_direct_post_uri("https://verifier.example.org/oid4vp/post").unwrap();
        assert!(!https.is_loopback);

        let local_ip = validate_direct_post_uri("http://127.0.0.1:8443/post").unwrap();
        assert!(local_ip.is_loopback);

        let local_name = validate_direct_post_uri("http://localhost:3000/cb").unwrap();
        assert!(local_name.is_loopback);

        let local_v6 = validate_direct_post_uri("http://[::1]:9000/cb").unwrap();
        assert!(local_v6.is_loopback);

        assert!(validate_direct_post_uri("http://verifier.example.org/post").is_err());
        assert!(validate_direct_post_uri("http://localhost.evil.example/post").is_err());
        assert!(validate_direct_post_uri("http://127.0.0.1.evil.example/post").is_err());
        assert!(validate_direct_post_uri("file:///etc/passwd").is_err());
        assert!(validate_direct_post_uri("https://verifier.example.org/post\n-o /tmp/x").is_err());
    }

    static TEST_KEYSTORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn keystore_generates_persists_imports_and_clears_keys_with_0600_perms() {
        let _lock = TEST_KEYSTORE_LOCK.lock().unwrap();
        let base = ibis_office_base::app_data::base().expect("test app_data::base");
        let path = base.join("ibisOffice").join("vc-keys.json");
        let _ = std::fs::remove_file(&path);

        let holder1 = load_or_create_holder_key();
        assert_eq!(holder1.backend, KeyStorageBackend::LocalAppData);
        assert!(!holder1.backend.is_sandbox());
        assert_eq!(holder1.thumbprint.len(), 12);

        // Second load returns the exact same persisted keypair
        let holder2 = load_or_create_holder_key();
        assert_eq!(holder1.key, holder2.key);

        // Issuer starts empty until generated or imported
        assert!(load_issuer_key().is_none());
        let issuer = generate_and_save_key(KeyRole::Issuer).unwrap();
        assert_ne!(issuer.key, holder1.key);
        assert_eq!(load_issuer_key().unwrap().key, issuer.key);

        // Verify 0600 permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        // Import custom JWK and clear
        let demo = Es256KeyPair::demo_issuer_key();
        let imported = import_jwk_key(KeyRole::Issuer, &demo.to_private_jwk_json()).unwrap();
        assert_eq!(imported.key, demo);
        clear_key(KeyRole::Issuer).unwrap();
        assert!(load_issuer_key().is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn direct_post_end_to_end_roundtrip_verified_by_presentation_compare() {
        let _lock = TEST_KEYSTORE_LOCK.lock().unwrap();
        let base = ibis_office_base::app_data::base().expect("test app_data::base");
        let path = base.join("ibisOffice").join("vc-keys.json");
        let _ = std::fs::remove_file(&path);

        // 1. Generate real CSPRNG Issuer & Holder keys via keystore
        let holder = load_or_create_holder_key();
        let issuer = generate_and_save_key(KeyRole::Issuer).unwrap();

        // 2. Issue an SD-JWT bound to the persistent Holder key
        let bindings = SheetClaimBindings {
            slide: SlideId(1),
            claims: vec![
                BoundClaim {
                    path: vec!["given_name".into()],
                    path_str: "given_name".into(),
                    selective: true,
                    disclose: true,
                    value: ClaimValue::String("花子".into()),
                    source: BindingSource::Cell { row: 1, column: 1 },
                },
                BoundClaim {
                    path: vec!["family_name".into()],
                    path_str: "family_name".into(),
                    selective: true,
                    disclose: true,
                    value: ClaimValue::String("山田".into()),
                    source: BindingSource::Cell { row: 2, column: 1 },
                },
                BoundClaim {
                    path: vec!["address".into(), "region".into()],
                    path_str: "address.region".into(),
                    selective: true,
                    disclose: true,
                    value: ClaimValue::String("東京都".into()),
                    source: BindingSource::Cell { row: 3, column: 1 },
                },
            ],
            disclosures: Default::default(),
        };
        let mut opts = SdJwtOptions::default();
        opts.default_vct = Some("urn:eu.europa.ec.eudi:pid:1".into());
        opts.holder_public_jwk = Some(holder.key.to_public_jwk());
        let issued = issue_sd_jwt(&bindings, &opts, &issuer.key);

        // 3. Spin up a local mock OID4VP Verifier HTTP server on 127.0.0.1:0
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback verifier");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel::<String>();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept curl connection");
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            // Read HTTP headers + Content-Length body
            let mut header_end = None;
            let mut content_length = 0usize;
            loop {
                let n = stream.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if header_end.is_none()
                    && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
                {
                    header_end = Some(pos + 4);
                    let headers = String::from_utf8_lossy(&buf[..pos]);
                    for line in headers.lines() {
                        if let Some((k, v)) = line.split_once(':')
                            && k.trim().eq_ignore_ascii_case("content-length")
                        {
                            content_length = v.trim().parse().unwrap_or(0);
                        }
                    }
                }
                if let Some(end) = header_end
                    && buf.len() >= end + content_length
                {
                    break;
                }
            }
            let body_bytes = match header_end {
                Some(end) => &buf[end..(end + content_length).min(buf.len())],
                None => &[][..],
            };
            let body = String::from_utf8_lossy(body_bytes).into_owned();
            let _ = tx.send(body);

            let resp_body = r#"{"redirect_uri":"https://verifier.example.org/done"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            let _ = stream.write_all(resp.as_bytes());
        });

        // 4. Parse OID4VP DCQL request, selectively disclose ONLY `given_name` & `address.region`, and POST!
        let nonce = generate_random_nonce();
        let state = "st-e2e-99";
        let response_uri = format!("http://127.0.0.1:{port}/oid4vp/direct_post");
        let dcql = json!({
            "credentials": [{
                "id": "pid_cred",
                "format": "dc+sd-jwt",
                "meta": { "vct_values": ["urn:eu.europa.ec.eudi:pid:1"] },
                "claims": [
                    { "path": ["given_name"] },
                    { "path": ["address", "region"] }
                ]
            }]
        });
        let req_json = json!({
            "client_id": "https://verifier.example.org",
            "response_mode": "direct_post",
            "response_uri": response_uri,
            "nonce": nonce,
            "state": state,
            "dcql_query": dcql,
        })
        .to_string();
        let req = Oid4vpRequest::parse(&req_json).expect("parse request JSON");
        let presented = present_existing_sd_jwt(
            &issued.sd_jwt_all,
            Some(&req.requested_claims),
            &req.client_id,
            &req.nonce,
            1_760_000_000,
            &holder.key,
        )
        .expect("re-present SD-JWT");
        assert_eq!(presented.holder_cnf_matches, Some(true));

        let form_body = req.build_direct_post_body(&presented.kb.vp_token);
        let outcome = send_oid4vp_direct_post(&response_uri, &form_body).expect("direct_post HTTP");
        assert_eq!(outcome.status, 200);
        assert!(outcome.is_loopback);
        assert_eq!(
            outcome.redirect_uri.as_deref(),
            Some("https://verifier.example.org/done")
        );
        let received_body = rx.recv().expect("verifier received body");
        server.join().expect("server thread finished");

        // 5. Verify the exact wire-received `direct_post` body using ibis-doc-service's `presentation_compare`!
        let issuer_jwk: serde_json::Value =
            serde_json::from_str(&issuer.key.to_public_jwk_json()).unwrap();
        let mut service =
            ibis_doc_service::Service::new(&[], true, ibis_doc_service::Limits::default());
        let result = service
            .call(
                "presentation_compare",
                &json!({
                    "format": "sd-jwt",
                    "direct_post_body": received_body,
                    "expected_state": state,
                    "dcql_query": dcql,
                    "issuer_jwk": issuer_jwk,
                    "nonce": nonce,
                    "audience": "https://verifier.example.org",
                }),
            )
            .expect("presentation_compare accepts direct_post wire body");

        assert_eq!(result["vp_token_credential_id"], json!("pid_cred"));
        assert_eq!(result["state_matches"], json!(true));
        assert_eq!(result["issuer_signature_valid"], json!(true));
        assert_eq!(result["supplied_disclosure_digests_match"], json!(true));
        assert_eq!(result["holder_signature_valid"], json!(true));
        assert_eq!(result["holder_sd_hash_valid"], json!(true));
        assert_eq!(result["nonce_matches"], json!(true));
        assert_eq!(result["audience_matches"], json!(true));
        assert_eq!(result["dcql"]["credential_type_matches"], json!(true));
        assert_eq!(result["missing_paths"], json!([]));
        assert_eq!(result["unexpected_paths"], json!([]));

        let _ = std::fs::remove_file(&path);
    }
}
