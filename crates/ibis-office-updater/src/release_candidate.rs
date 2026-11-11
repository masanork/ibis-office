// SPDX-License-Identifier: AGPL-3.0-only
//! Offline inspection of signed metadata and every named artifact.
//! The caller provides the trusted key out-of-band; this proves byte integrity,
//! not build provenance, operating-system signing, or safe installation.
use crate::signed_manifest::{self, Manifest};
use std::{collections::HashSet, fs, io::Read, path::Path};

fn small_file(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut result = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut result)
        .map_err(|e| e.to_string())?;
    if result.len() as u64 > max {
        return Err(format!("{}: file too large", path.display()));
    }
    Ok(result)
}

pub fn decode_hex<const N: usize>(input: &str) -> Result<[u8; N], String> {
    let input = input.trim();
    if input.len() != N * 2 || !input.is_ascii() {
        return Err("invalid hex length".into());
    }
    let mut out = [0; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&input[i * 2..i * 2 + 2], 16).map_err(|_| "invalid hex")?;
    }
    Ok(out)
}

/// Verify metadata, required targets and all artifacts beneath `directory`.
/// The metadata and artifacts are read from a local staging directory which
/// must not be modified concurrently during verification or publication.
pub fn verify_directory(
    directory: &Path,
    public_key: &[u8; 32],
    now: u64,
    required: &[&str],
) -> Result<Manifest, String> {
    let data = small_file(&directory.join("sorane-update.json"), 64 * 1024)?;
    let signature = small_file(&directory.join("sorane-update.json.sig"), 256)?;
    let sig = decode_hex::<64>(
        std::str::from_utf8(&signature).map_err(|_| "invalid signature encoding")?,
    )?;
    let manifest =
        signed_manifest::verify_manifest(&data, &sig, public_key, now).map_err(str::to_string)?;
    let mut required_seen = HashSet::new();
    for target in required {
        if !required_seen.insert(*target) {
            return Err(format!("duplicate required target: {target}"));
        }
        if manifest.artifact(target).is_none() {
            return Err(format!("missing required target: {target}"));
        }
    }
    for artifact in &manifest.artifacts {
        let path = directory.join(&artifact.filename);
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !meta.file_type().is_file() {
            return Err(format!("{}: not a regular file", path.display()));
        }
        let file = fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        signed_manifest::verify_artifact(artifact, file)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn candidate_rejects_missing_corrupt_wrong_key_and_oversized_metadata() {
        let path =
            std::env::temp_dir().join(format!("sorane-candidate-test-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let data = format!(
            r#"{{"schema":1,"product":"Sorane Docs","channel":"stable","version":"1.2.3","issued_at":{},"expires_at":{},"artifacts":[{{"target":"linux-x64-appimage","filename":"candidate.AppImage","size":3,"sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}}]}}"#,
            now - 1,
            now + 1000
        );
        let sig: String = key
            .sign(data.as_bytes())
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        fs::write(path.join("sorane-update.json"), &data).unwrap();
        fs::write(path.join("sorane-update.json.sig"), sig).unwrap();
        fs::write(path.join("candidate.AppImage"), b"abc").unwrap();
        let public: [u8; 32] = key.public_key().as_ref().try_into().unwrap();
        assert!(verify_directory(&path, &public, now, &["linux-x64-appimage"]).is_ok());
        assert!(verify_directory(&path, &public, now, &["macos-arm64"]).is_err());
        assert!(verify_directory(&path, &[8; 32], now, &[]).is_err());
        fs::write(path.join("candidate.AppImage"), b"abd").unwrap();
        assert!(verify_directory(&path, &public, now, &[]).is_err());
        fs::remove_file(path.join("candidate.AppImage")).unwrap();
        assert!(verify_directory(&path, &public, now, &[]).is_err());
        fs::write(path.join("sorane-update.json.sig"), vec![b'0'; 257]).unwrap();
        assert!(verify_directory(&path, &public, now, &[]).is_err());
        fs::remove_dir_all(path).unwrap();
    }
}
