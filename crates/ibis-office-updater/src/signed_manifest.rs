// SPDX-License-Identifier: AGPL-3.0-only
//! Verification of an exact-byte Ed25519 signed update manifest.
//!
//! The trust key is supplied by the application, never by the manifest.
//! Callers must use a fixed HTTPS origin for artifact names and must not run
//! downloaded code until `verify_artifact` succeeds and platform checks pass.

use ring::{digest, signature};
use serde::Deserialize;
use std::io::Read;

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_LIFETIME: u64 = 31 * 24 * 60 * 60;
const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub product: String,
    pub channel: String,
    pub version: String,
    pub issued_at: u64,
    pub expires_at: u64,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub target: String,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
}

pub(crate) fn safe_filename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 180
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Verify signature and policy before exposing any release information.
/// `now` is Unix time in seconds; an incorrect system clock fails closed.
pub fn verify_manifest(
    bytes: &[u8],
    sig: &[u8],
    public_key: &[u8],
    now: u64,
) -> Result<Manifest, &'static str> {
    if bytes.len() > MAX_MANIFEST_BYTES || sig.len() != 64 || public_key.len() != 32 {
        return Err("invalid metadata size");
    }
    signature::UnparsedPublicKey::new(&signature::ED25519, public_key)
        .verify(bytes, sig)
        .map_err(|_| "invalid manifest signature")?;
    let m: Manifest = serde_json::from_slice(bytes).map_err(|_| "invalid manifest")?;
    if m.schema != 1
        || m.product != "Sorane Docs"
        || m.channel != "stable"
        || !crate::version::valid(&m.version)
    {
        return Err("unexpected release identity");
    }
    if m.issued_at > now
        || m.expires_at <= now
        || m.expires_at <= m.issued_at
        || m.expires_at - m.issued_at > MAX_LIFETIME
    {
        return Err("invalid or expired release metadata");
    }
    if m.artifacts.is_empty() || m.artifacts.len() > 16 {
        return Err("invalid artifact count");
    }
    let mut targets = std::collections::HashSet::new();
    let mut filenames = std::collections::HashSet::new();
    for a in &m.artifacts {
        if !matches!(
            a.target.as_str(),
            "macos-arm64"
                | "macos-x64"
                | "windows-x64"
                | "linux-x64-appimage"
                | "linux-arm64-appimage"
        ) || !targets.insert(a.target.as_str())
            || !filenames.insert(a.filename.as_str())
            || !safe_filename(&a.filename)
            || a.size == 0
            || a.size > MAX_ARTIFACT_BYTES
            || a.sha256.len() != 64
            || !a
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid artifact metadata");
        }
    }
    Ok(m)
}

impl Manifest {
    pub fn artifact(&self, target: &str) -> Option<&Artifact> {
        self.artifacts.iter().find(|a| a.target == target)
    }
}

/// Bound both metadata and artifact downloads even when a server omits
/// Content-Length (older curl versions do not enforce --max-filesize then).
pub fn copy_bounded(
    mut input: impl std::io::Read,
    mut output: impl std::io::Write,
    limit: u64,
) -> Result<(), String> {
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(());
        }
        total = total
            .checked_add(count as u64)
            .ok_or("download too large")?;
        if total > limit {
            return Err("download exceeded size limit".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
    }
}

/// Verify the entire downloaded file, including its signed length, before use.
pub fn verify_artifact(a: &Artifact, mut reader: impl Read) -> Result<(), &'static str> {
    let mut ctx = digest::Context::new(&digest::SHA256);
    let mut size = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|_| "artifact read failed")?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n as u64).ok_or("artifact too large")?;
        if size > a.size {
            return Err("artifact length mismatch");
        }
        ctx.update(&buf[..n]);
    }
    if size != a.size {
        return Err("artifact length mismatch");
    }
    let actual: String = ctx
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != a.sha256 {
        return Err("artifact hash mismatch");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn key() -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[7u8; 32]).unwrap()
    }
    fn manifest() -> String {
        r#"{"schema":1,"product":"Sorane Docs","channel":"stable","version":"1.2.3","issued_at":1000,"expires_at":2000,"artifacts":[{"target":"linux-x64-appimage","filename":"Sorane-1.2.3.AppImage","size":3,"sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}]}"#.into()
    }
    fn verify(text: &str, now: u64) -> Result<Manifest, &'static str> {
        let k = key();
        verify_manifest(
            text.as_bytes(),
            k.sign(text.as_bytes()).as_ref(),
            k.public_key().as_ref(),
            now,
        )
    }
    #[test]
    fn bounded_copy_rejects_more_than_limit() {
        let mut output = Vec::new();
        assert!(copy_bounded(&b"abc"[..], &mut output, 3).is_ok());
        assert_eq!(output, b"abc");
        let mut output = Vec::new();
        assert!(copy_bounded(&b"abcd"[..], &mut output, 3).is_err());
        assert!(output.len() <= 3);
    }
    #[test]
    fn verifies_signed_metadata_and_actual_bytes() {
        let m = verify(&manifest(), 1500).unwrap();
        let a = m.artifact("linux-x64-appimage").unwrap();
        assert!(verify_artifact(a, &b"abc"[..]).is_ok());
        assert!(verify_artifact(a, &b"abd"[..]).is_err());
        assert!(verify_artifact(a, &b"abcd"[..]).is_err());
        assert!(verify_artifact(a, &b"ab"[..]).is_err());
        assert!(m.artifact("windows-x64").is_none());
    }
    #[test]
    fn rejects_tampering_wrong_key_and_expiry() {
        let k = key();
        let m = manifest();
        let sig = k.sign(m.as_bytes());
        assert!(
            verify_manifest(
                m.replace("1.2.3", "1.2.4").as_bytes(),
                sig.as_ref(),
                k.public_key().as_ref(),
                1500
            )
            .is_err()
        );
        assert!(verify_manifest(m.as_bytes(), sig.as_ref(), &[8u8; 32], 1500).is_err());
        assert!(verify(&m, 2000).is_err());
        assert!(verify(&m, 999).is_err());
        assert!(verify(&m.replace("2000", "9999999"), 1500).is_err());
    }
    #[test]
    fn rejects_signed_unsafe_or_ambiguous_metadata() {
        let m = manifest();
        for bad in [
            m.replace("Sorane-1.2.3.AppImage", "../evil"),
            m.replace("stable", "beta"),
            m.replace("1.2.3", "../x"),
            m.replace("linux-x64-appimage", "linux-x64-flatpak"),
            m.replace("\"schema\":1", "\"schema\":1,\"schema\":1"),
            m.replace("\"size\":3", "\"size\":3,\"other\":1"),
        ] {
            assert!(verify(&bad, 1500).is_err(), "{bad}");
        }
    }
    #[test]
    fn rejects_duplicate_artifact_names_across_targets() {
        let mut json: serde_json::Value = serde_json::from_str(&manifest()).unwrap();
        let mut second = json["artifacts"][0].clone();
        second["target"] = "windows-x64".into();
        json["artifacts"].as_array_mut().unwrap().push(second);
        assert!(verify(&json.to_string(), 1500).is_err());
    }
    /// Optional cross-tool test: set SORANE_TEST_OPENSSL to an OpenSSL 3 binary.
    #[test]
    fn signer_output_interoperates_with_verifier() {
        let Some(openssl) = std::env::var_os("SORANE_TEST_OPENSSL") else {
            return;
        };
        use std::process::Command;
        let root = std::env::temp_dir().join(format!("sorane-signer-test-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let key = root.join("key.pem");
        let public = root.join("public.der");
        let payload = root.join("fixture.AppImage");
        let out = root.join("out");
        std::fs::write(&payload, b"abc").unwrap();
        assert!(
            Command::new(&openssl)
                .args(["genpkey", "-algorithm", "Ed25519", "-out"])
                .arg(&key)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new(&openssl)
                .args(["pkey", "-in"])
                .arg(&key)
                .args(["-pubout", "-outform", "DER", "-out"])
                .arg(&public)
                .status()
                .unwrap()
                .success()
        );
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/sorane-update-manifest.py");
        assert!(
            Command::new("python3")
                .arg(script)
                .arg("--openssl")
                .arg(&openssl)
                .args(["--version", "1.2.3", "--private-key"])
                .arg(&key)
                .arg("--out")
                .arg(&out)
                .arg(format!("linux-x64-appimage={}", payload.display()))
                .status()
                .unwrap()
                .success()
        );
        let der = std::fs::read(public).unwrap();
        assert_eq!(
            &der[..12],
            &[
                0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00
            ]
        );
        let sig_hex = std::fs::read_to_string(out.join("sorane-update.json.sig")).unwrap();
        let sig: Vec<u8> = sig_hex
            .trim()
            .as_bytes()
            .chunks_exact(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect();
        let data = std::fs::read(out.join("sorane-update.json")).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let m = verify_manifest(&data, &sig, &der[12..], now).unwrap();
        verify_artifact(m.artifact("linux-x64-appimage").unwrap(), &b"abc"[..]).unwrap();
        std::fs::copy(&payload, out.join("fixture.AppImage")).unwrap();
        let public_key: [u8; 32] = der[12..].try_into().unwrap();
        assert!(
            crate::release_candidate::verify_directory(
                &out,
                &public_key,
                now,
                &["linux-x64-appimage"]
            )
            .is_ok()
        );
        std::fs::write(out.join("fixture.AppImage"), b"abd").unwrap();
        assert!(
            crate::release_candidate::verify_directory(
                &out,
                &public_key,
                now,
                &["linux-x64-appimage"]
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
