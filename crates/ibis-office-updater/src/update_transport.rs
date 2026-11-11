// SPDX-License-Identifier: AGPL-3.0-only
//! HTTPS-only bounded transport. The public API has no CA override argument.
use crate::signed_manifest::copy_bounded;
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn command(url: &str, limit: u64, seconds: u32) -> Command {
    let mut cmd = Command::new("curl");
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000);
    }
    // --disable must be first: a user's curl config cannot change this policy.
    cmd.args([
        "--disable",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "10",
        "--max-time",
        &seconds.to_string(),
        "--max-filesize",
        &limit.to_string(),
        "--url",
        url,
    ]);
    cmd
}

fn execute(mut cmd: Command, limit: u64, output: impl Write) -> Result<(), String> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let result = copy_bounded(
        child.stdout.take().ok_or("missing curl stdout")?,
        output,
        limit,
    );
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    result?;
    if !status.success() {
        return Err("download failed".into());
    }
    Ok(())
}

/// Only HTTPS including redirects, with system certificate verification.
/// Rejects responses larger than `limit`, even without Content-Length.
pub fn fetch_to(url: &str, limit: u64, seconds: u32, output: impl Write) -> Result<(), String> {
    execute(command(url, limit, seconds), limit, output)
}

/// Download to an exclusively-created temporary file, verify the signed
/// size and digest, and only then move it to the final staging name.
/// Callers must obtain the artifact from successfully verified metadata and
/// construct the URL from their trusted release origin.
pub fn download_verified(
    url: &str,
    artifact: &crate::signed_manifest::Artifact,
    directory: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    download_with(url, artifact, directory, |url, limit, out| {
        fetch_to(url, limit, 1800, out)
    })
}

fn download_with<F>(
    url: &str,
    artifact: &crate::signed_manifest::Artifact,
    directory: &std::path::Path,
    fetch: F,
) -> Result<std::path::PathBuf, String>
where
    F: FnOnce(&str, u64, &mut std::fs::File) -> Result<(), String>,
{
    if !crate::signed_manifest::safe_filename(&artifact.filename) {
        return Err("invalid artifact filename".into());
    }
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let part = directory.join(format!("{}.part-{}", artifact.filename, std::process::id()));
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .map_err(|e| e.to_string())?;
    let final_path = directory.join(&artifact.filename);
    let result = (|| {
        fetch(url, artifact.size, &mut out)?;
        drop(out);
        let file = std::fs::File::open(&part).map_err(|e| e.to_string())?;
        crate::signed_manifest::verify_artifact(artifact, file).map_err(str::to_string)?;
        std::fs::rename(&part, &final_path).map_err(|e| e.to_string())?;
        Ok(final_path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// Verify signed metadata and select a newer release for this target.
pub fn select_update(
    bytes: &[u8],
    signature: &[u8],
    key: &[u8],
    now: u64,
    target: &str,
    current: &str,
) -> Result<Option<crate::signed_manifest::Manifest>, &'static str> {
    let manifest = crate::signed_manifest::verify_manifest(bytes, signature, key, now)?;
    if manifest.artifact(target).is_none() {
        return Err("target missing");
    }
    if !crate::version::valid(current) {
        return Err("invalid current version");
    }
    if !crate::version::is_newer(&manifest.version, current) {
        return Ok(None);
    }
    Ok(Some(manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curl_configuration_is_not_loaded_and_protocol_is_restricted() {
        let cmd = command("https://example.invalid", 20, 5);
        let args: Vec<_> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args.first().map(String::as_str), Some("--disable"));
        assert!(args.windows(2).any(|v| v == ["--proto", "=https"]));
        assert!(args.windows(2).any(|v| v == ["--proto-redir", "=https"]));
    }
    #[test]
    fn unsafe_filename_is_rejected_before_creating_directory() {
        let artifact = crate::signed_manifest::Artifact {
            target: "windows-x64".into(),
            filename: "../escape".into(),
            size: 1,
            sha256: "0".repeat(64),
        };
        let path = std::env::temp_dir().join(format!("sorane-unsafe-name-{}", std::process::id()));
        assert!(download_verified("https://example.invalid", &artifact, &path).is_err());
        assert!(!path.exists());
    }

    /// Uses a throwaway local TLS certificate and an ephemeral signing key.
    /// The certificate override is accessible only to this unit-test module;
    /// the public transport uses system trust and HTTPS-only redirects.
    #[cfg(unix)]
    #[test]
    fn local_https_serves_signed_update_and_rejects_failures() {
        let Some(openssl) = std::env::var_os("SORANE_TEST_OPENSSL") else {
            return;
        };
        use ring::{
            digest,
            signature::{Ed25519KeyPair, KeyPair},
        };
        use std::{
            fs,
            net::{TcpListener, TcpStream},
            path::PathBuf,
            process::Child,
            time::{Duration, SystemTime, UNIX_EPOCH},
        };
        struct Server {
            child: Child,
            dir: PathBuf,
        }
        impl Drop for Server {
            fn drop(&mut self) {
                let _ = self.child.kill();
                let _ = self.child.wait();
                let _ = fs::remove_dir_all(&self.dir);
            }
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sorane-tls-{}-{nonce}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let key = dir.join("tls-key.pem");
        let cert = dir.join("tls-cert.pem");
        let status = Command::new(&openssl)
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost",
                "-keyout",
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "cannot create test TLS certificate");
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(&openssl)
            .args(["s_server", "-quiet", "-WWW", "-accept"])
            .arg(format!("127.0.0.1:{port}"))
            .arg("-cert")
            .arg(&cert)
            .arg("-key")
            .arg(&key)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let _server = Server {
            child,
            dir: dir.clone(),
        };
        let mut ready = false;
        for _ in 0..100 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(ready, "test TLS server did not start");
        let base = format!("https://localhost:{port}");
        let local_fetch = |url: &str, limit: u64, output: &mut dyn Write| {
            let mut cmd = command(url, limit, 10);
            cmd.args(["--noproxy", "*", "--cacert"]).arg(&cert);
            execute(cmd, limit, output)
        };
        fs::write(dir.join("payload"), b"new").unwrap();
        // A self-signed endpoint is rejected by the unmodified production API.
        assert!(fetch_to(&format!("{base}/payload"), 32, 10, Vec::new()).is_err());
        let now = (nonce / 1_000_000_000) as u64;
        let signing = Ed25519KeyPair::from_seed_unchecked(&[42; 32]).unwrap();
        let hash: String = digest::digest(&digest::SHA256, b"new")
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let meta = format!(
            r#"{{"schema":1,"product":"Sorane Docs","channel":"stable","version":"2.0.0","issued_at":{},"expires_at":{},"artifacts":[{{"target":"windows-x64","filename":"payload","size":3,"sha256":"{hash}"}}]}}"#,
            now - 1,
            now + 300
        );
        let sig = signing.sign(meta.as_bytes());
        fs::write(dir.join("manifest"), meta.as_bytes()).unwrap();
        fs::write(
            dir.join("signature"),
            sig.as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        )
        .unwrap();
        let mut data = Vec::new();
        let mut signature = Vec::new();
        local_fetch(&format!("{base}/manifest"), 64 * 1024, &mut data).unwrap();
        local_fetch(&format!("{base}/signature"), 256, &mut signature).unwrap();
        let signature =
            crate::release_candidate::decode_hex::<64>(std::str::from_utf8(&signature).unwrap())
                .unwrap();
        let public = signing.public_key().as_ref();
        let release = select_update(&data, &signature, public, now, "windows-x64", "1.0.0")
            .unwrap()
            .unwrap();
        assert!(select_update(&data, &[0; 64], public, now, "windows-x64", "1.0.0").is_err());
        assert!(
            select_update(&data, &signature, public, now, "windows-x64", "2.0.0")
                .unwrap()
                .is_none()
        );
        assert!(
            select_update(&data, &signature, public, now + 300, "windows-x64", "1.0.0").is_err()
        );
        let artifact = release.artifact("windows-x64").unwrap();
        let dest = dir.join("staged");
        let file = download_with(
            &format!("{base}/payload"),
            artifact,
            &dest,
            |url, limit, out| local_fetch(url, limit, out),
        )
        .unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"new");
        // Treat the verified synthetic payload as an already-unpacked fixture;
        // this does not exercise a real archive, OS signature, or process.
        let installed = dir.join("installed");
        let unpacked = dir.join("unpacked-fixture");
        fs::create_dir(&installed).unwrap();
        fs::create_dir(&unpacked).unwrap();
        fs::write(installed.join("app"), b"old").unwrap();
        fs::copy(&file, unpacked.join("app")).unwrap();
        crate::swap::install(&installed, &unpacked, &|path| {
            if fs::read(path.join("app")).map_err(|e| e.to_string())? == b"new" {
                Ok(())
            } else {
                Err("unexpected fixture".into())
            }
        })
        .unwrap();
        assert_eq!(fs::read(installed.join("app")).unwrap(), b"new");
        crate::swap::rollback(&installed).unwrap();
        assert_eq!(fs::read(installed.join("app")).unwrap(), b"old");
        fs::write(dir.join("payload"), b"bad").unwrap();
        let rejected = dir.join("rejected");
        assert!(
            download_with(
                &format!("{base}/payload"),
                artifact,
                &rejected,
                |url, limit, out| local_fetch(url, limit, out)
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(&rejected).unwrap().count(), 0);
        fs::write(dir.join("payload"), b"ne").unwrap();
        assert!(
            download_with(
                &format!("{base}/payload"),
                artifact,
                &rejected,
                |url, limit, out| local_fetch(url, limit, out)
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(&rejected).unwrap().count(), 0);
        fs::write(dir.join("payload"), b"toolong").unwrap();
        assert!(
            download_with(
                &format!("{base}/payload"),
                artifact,
                &rejected,
                |url, limit, out| local_fetch(url, limit, out)
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(&rejected).unwrap().count(), 0);
        let mut too_large = Vec::new();
        assert!(local_fetch(&format!("{base}/manifest"), 8, &mut too_large).is_err());
        assert!(too_large.len() <= 8);
        assert!(
            fetch_to(
                &format!("http://127.0.0.1:{port}/payload"),
                100,
                3,
                Vec::new()
            )
            .is_err()
        );
        // A real HTTPS redirect must not fall back to plain HTTP.
        struct ChildGuard(Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let http_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        http_listener.set_nonblocking(true).unwrap();
        let http_port = http_listener.local_addr().unwrap().port();
        fs::write(dir.join("redirect"), format!("HTTP/1.0 302 Found\r\nLocation: http://127.0.0.1:{http_port}/payload\r\nContent-Length: 0\r\n\r\n")).unwrap();
        let redirect_port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(&openssl)
            .args(["s_server", "-quiet", "-HTTP", "-accept"])
            .arg(format!("127.0.0.1:{redirect_port}"))
            .arg("-cert")
            .arg(&cert)
            .arg("-key")
            .arg(&key)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let _redirect_server = ChildGuard(child);
        let mut ready = false;
        for _ in 0..100 {
            if TcpStream::connect(("127.0.0.1", redirect_port)).is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(ready, "redirect TLS server did not start");
        assert!(
            local_fetch(
                &format!("https://localhost:{redirect_port}/redirect"),
                1024,
                &mut Vec::new()
            )
            .is_err()
        );
        assert_eq!(
            http_listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
