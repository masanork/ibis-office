// SPDX-License-Identifier: AGPL-3.0-only

//! Local rendezvous for explicitly shared Assistant windows.

use std::{
    fs::{self, DirBuilder, File},
    io::{self, Read},
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub struct Workspace {
    root: PathBuf,
    runtime: PathBuf,
}

const SESSION_ID_LENGTH: usize = 12;

pub const SOCKET_NAME: &str = "socket";
pub const REGISTRATION_NAME: &str = "workspace.json";

impl Workspace {
    pub fn open(path: &Path) -> io::Result<Self> {
        let root = path.canonicalize()?;
        if !root.is_dir() {
            return Err(io::Error::other("Assistant workspace must be a directory"));
        }
        // SAFETY: geteuid takes no arguments and does not access Rust memory.
        let uid = unsafe { libc::geteuid() };
        let runtime = PathBuf::from(format!("/tmp/ibis-assistant-{uid}"));
        match DirBuilder::new().mode(0o700).create(&runtime) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        if !private_directory(&runtime, uid) {
            return Err(io::Error::other(
                "Assistant runtime directory must be private and owned by this user",
            ));
        }
        Ok(Self { root, runtime })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Own the registration until the GUI window adopts it.
    pub fn register(&self) -> io::Result<tempfile::TempDir> {
        let dir = tempfile::Builder::new()
            .prefix("s-")
            .rand_bytes(SESSION_ID_LENGTH)
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(&self.runtime)?;
        let record = serde_json::to_vec(&self.root)?;
        fs::write(dir.path().join(REGISTRATION_NAME), record)?;
        Ok(dir)
    }

    pub(crate) fn socket(&self, id: &str) -> Option<PathBuf> {
        let suffix = id.strip_prefix("s-")?;
        if suffix.len() != SESSION_ID_LENGTH
            || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return None;
        }
        let dir = self.runtime.join(id);
        let uid = fs::symlink_metadata(&self.runtime).ok()?.uid();
        if !private_directory(&dir, uid) {
            return None;
        }
        let registration = dir.join(REGISTRATION_NAME);
        if !fs::symlink_metadata(&registration).ok()?.is_file() {
            return None;
        }
        let mut record = Vec::new();
        File::open(registration)
            .ok()?
            .take(16 * 1024)
            .read_to_end(&mut record)
            .ok()?;
        if serde_json::from_slice::<PathBuf>(&record).ok()? != self.root {
            return None;
        }
        let socket = dir.join(SOCKET_NAME);
        fs::symlink_metadata(&socket)
            .ok()?
            .file_type()
            .is_socket()
            .then_some(socket)
    }

    pub(crate) fn sessions(&self) -> io::Result<impl Iterator<Item = (String, PathBuf)> + '_> {
        Ok(fs::read_dir(&self.runtime)?.filter_map(|entry| {
            let id = entry.ok()?.file_name().into_string().ok()?;
            let socket = self.socket(&id)?;
            Some((id, socket))
        }))
    }
}

fn private_directory(path: &Path, uid: u32) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_dir() && metadata.uid() == uid && metadata.permissions().mode() & 0o077 == 0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn registration_is_scoped_and_requires_a_socket() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(first.path()).unwrap();
        let other = Workspace::open(second.path()).unwrap();
        let registration = workspace.register().unwrap();
        let id = registration.path().file_name().unwrap().to_str().unwrap();
        let path = registration.path().join(SOCKET_NAME);
        assert!(workspace.socket(id).is_none());
        let listener = UnixListener::bind(&path).unwrap();
        assert_eq!(workspace.socket(id).as_deref(), Some(path.as_path()));
        assert!(other.socket(id).is_none());
        assert!(workspace.socket("../workspace.json").is_none());
        assert_eq!(workspace.sessions().unwrap().count(), 1);
        drop(listener);
        registration.close().unwrap();
        assert!(workspace.sessions().unwrap().next().is_none());
    }
}
