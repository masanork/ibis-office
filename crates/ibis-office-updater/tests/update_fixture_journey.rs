// SPDX-License-Identifier: AGPL-3.0-only
//! Local fixture journey through shared verification and folder-swap code.
//! This does not exercise the app's HTTP transport, GUI, OS signing, updater
//! process or real application launch.
use ibis_office_updater::{signed_manifest, swap};
use ring::{
    digest,
    signature::{Ed25519KeyPair, KeyPair},
};
use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    key: Ed25519KeyPair,
    manifest: Vec<u8>,
    signature: Vec<u8>,
    now: u64,
}
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "sorane-journey-{}-{nonce}-{id}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("installed")).unwrap();
        fs::write(root.join("installed/app"), b"old").unwrap();
        let key = Ed25519KeyPair::from_seed_unchecked(&[27; 32]).unwrap();
        let now = (nonce / 1_000_000_000) as u64;
        let hash: String = digest::digest(&digest::SHA256, b"new")
            .as_ref()
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect();
        let manifest = format!(r#"{{"schema":1,"product":"Sorane Docs","channel":"stable","version":"2.0.0","issued_at":{},"expires_at":{},"artifacts":[{{"target":"windows-x64","filename":"new.zip","size":3,"sha256":"{hash}"}}]}}"#, now-1, now+300).into_bytes();
        let signature = key.sign(&manifest).as_ref().to_vec();
        Self {
            root,
            key,
            manifest,
            signature,
            now,
        }
    }
    fn prepare(&self, signature: &[u8], payload: &[u8]) -> Result<PathBuf, String> {
        let m = signed_manifest::verify_manifest(
            &self.manifest,
            signature,
            self.key.public_key().as_ref(),
            self.now,
        )
        .map_err(str::to_string)?;
        let a = m.artifact("windows-x64").ok_or("target missing")?;
        let mut received = Vec::new();
        signed_manifest::copy_bounded(Cursor::new(payload), &mut received, a.size)?;
        signed_manifest::verify_artifact(a, Cursor::new(&received)).map_err(str::to_string)?;
        let staged = self.root.join("verified");
        fs::create_dir(&staged).map_err(|e| e.to_string())?;
        fs::write(staged.join("app"), received).map_err(|e| e.to_string())?;
        Ok(staged)
    }
    fn installed(&self) -> Vec<u8> {
        fs::read(self.root.join("installed/app")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn verified_fixture_swaps_then_recovers_old_copy_on_simulated_launch_failure() {
    let f = Fixture::new();
    let staged = f.prepare(&f.signature, b"new").unwrap();
    let installed = f.root.join("installed");
    swap::install(&installed, &staged, &|dir| {
        if fs::read(dir.join("app")).map_err(|e| e.to_string())? == b"new" {
            Ok(())
        } else {
            Err("bad copy".into())
        }
    })
    .unwrap();
    assert_eq!(f.installed(), b"new");
    assert_eq!(
        fs::read(swap::backup_of(&installed).join("app")).unwrap(),
        b"old"
    );
    // The fixture simulates a failed launch; no process is started here.
    swap::rollback(&installed).unwrap();
    assert_eq!(f.installed(), b"old");
}

#[test]
fn untrusted_or_corrupt_fixture_never_reaches_swap() {
    let f = Fixture::new();
    assert!(f.prepare(&[0; 64], b"new").is_err());
    assert!(f.prepare(&f.signature, b"bad").is_err());
    assert!(f.prepare(&f.signature, b"new plus overflow").is_err());
    assert_eq!(f.installed(), b"old");
    assert!(!swap::backup_of(&f.root.join("installed")).exists());
    assert!(!f.root.join("verified").exists());
}
