// SPDX-License-Identifier: AGPL-3.0-only

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use super::{
    MAX_TEXT_DIFF_BYTES, Result, optional_blob, scoped_history, unified_diff, working_tree,
};

const STATUS_PAGE: usize = 50;
const DIFF_PAGE: usize = 24 * 1024;

pub(crate) fn run(folder: &Path, name: &str, args: &Value, offset: usize) -> Result<Value> {
    if folder.canonicalize().ok().as_deref() != Some(folder) || !folder.is_dir() {
        return Err("The reference folder moved. Open it again.".into());
    }
    let repo =
        gix::discover(folder).map_err(|_| "The open folder is not inside a Git working tree.")?;
    let root = repo
        .workdir()
        .ok_or("The Git repository has no working tree.")?
        .canonicalize()?;
    let scope = folder.strip_prefix(&root)?;
    let relative = match args.get("path") {
        Some(path) => allowed_path(path.as_str().ok_or("path must be a string")?)?,
        None if name != "git_diff" => PathBuf::new(),
        None => return Err("Provide a folder-relative file path.".into()),
    };
    let path = scope.join(&relative);
    match name {
        "git_status" => {
            let mut changes = working_tree::status(&root, Some(&path))?.into_iter()
                .filter_map(|change| {
                    let relative = change.path.strip_prefix(scope).ok()?;
                    let relative = relative.to_str()?.replace(std::path::MAIN_SEPARATOR, "/");
                    allowed_path(&relative).ok()?;
                    Some(json!({"path":relative,"index":char::from(change.index),"worktree":char::from(change.worktree)}))
                }).skip(offset);
            let entries: Vec<_> = changes.by_ref().take(STATUS_PAGE).collect();
            Ok(
                json!({"changes":entries,"next_offset":changes.next().map(|_| offset + entries.len())}),
            )
        }
        "git_log" => {
            let head = repo.head_id()?.detach();
            let shallow = repo.is_shallow()?;
            let start = match args.get("cursor") {
                Some(_) => commit_id(&args["cursor"])?,
                None => head,
            };
            let (revisions, older) = scoped_history(repo, start, &path, true)?;
            let commits: Vec<_> = revisions.iter().map(|r| json!({
                "commit":r.id.to_string(),"parent":r.parent.map(|id| id.to_string()),"date":r.date,"subject":r.title.chars().take(320).collect::<String>(),"exists":r.exists
            })).collect();
            Ok(
                json!({"path":args["path"],"head":head.to_string(),"history":"first-parent","shallow":shallow,
                "commits":commits,"next_cursor":older.map(|id| id.to_string())}),
            )
        }
        "git_diff" => {
            let old = commit_id(&args["old_commit"])?;
            let new = commit_id(&args["new_commit"])?;
            let old_bytes = optional_blob(&repo, old, &path)?;
            let new_bytes = optional_blob(&repo, new, &path)?;
            if old_bytes.is_none() && new_bytes.is_none() {
                return Err("The file is absent in both commits.".into());
            }
            let bytes_changed = old_bytes != new_bytes;
            let old_exists = old_bytes.is_some();
            let new_exists = new_bytes.is_some();
            let (old_text, format) = diff_text(&path, old_bytes)?;
            let (new_text, new_format) = diff_text(&path, new_bytes)?;
            let patch = unified_diff(&old_text, &new_text, &old.to_string(), &new.to_string());
            if !patch.is_char_boundary(offset) {
                return Err("Use the returned next_offset to continue the diff.".into());
            }
            let mut end = offset.saturating_add(DIFF_PAGE).min(patch.len());
            while !patch.is_char_boundary(end) {
                end -= 1;
            }
            Ok(
                json!({"path":args["path"],"old_commit":old.to_string(),"new_commit":new.to_string(),
                "format":if format == "office_markdown" || new_format == "office_markdown" { "office_markdown" } else { "text" },
                "old_exists":old_exists,"new_exists":new_exists,"bytes_changed":bytes_changed,
                "diff":&patch[offset..end],"next_offset":(end < patch.len()).then_some(end)}),
            )
        }
        _ => Err("Unknown Git tool.".into()),
    }
}

fn allowed_path(path: &str) -> Result<PathBuf> {
    if path.contains(['\\', ':'])
        || path
            .split('/')
            .any(|part| part.is_empty() || !crate::folder_context::visible(part))
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("Use a visible, folder-relative path with forward slashes.".into());
    }
    Ok(PathBuf::from(path))
}

fn commit_id(value: &Value) -> Result<gix::ObjectId> {
    let hex = value
        .as_str()
        .ok_or("Provide a full commit SHA from git_log.")?;
    Ok(gix::ObjectId::from_hex(hex.as_bytes())?)
}

fn diff_text(path: &Path, bytes: Option<Vec<u8>>) -> Result<(String, &'static str)> {
    let Some(bytes) = bytes else {
        return Ok((String::new(), "text"));
    };
    let extension = crate::folder_context::extension(path);
    let result = if crate::folder_context::OFFICE.contains(&extension.as_str()) {
        let opened = ibis_odf::open(bytes)?;
        let markdown =
            ibis_formats::markdown::export_presentation_to_markdown(&opened.presentation)?;
        (markdown, "office_markdown")
    } else {
        if bytes.len() > MAX_TEXT_DIFF_BYTES || bytes.contains(&0) {
            return Err("Text diff requires UTF-8 input up to 512 KiB per revision.".into());
        }
        (String::from_utf8(bytes)?, "text")
    };
    if result.0.len() > MAX_TEXT_DIFF_BYTES {
        return Err("The extracted text exceeds 512 KiB per revision.".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("ibis-git-context-{}-{nonce}", std::process::id()));
            std::fs::create_dir_all(root.join("docs")).unwrap();
            let repo = Self(root.canonicalize().unwrap());
            repo.git(&["init", "-q"]);
            repo
        }

        fn git(&self, args: &[&str]) {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(&self.0)
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn write(&self, path: &str, contents: impl AsRef<[u8]>) {
            std::fs::write(self.0.join(path), contents).unwrap();
        }

        fn commit(&self, message: &str) {
            self.git(&["add", "."]);
            self.git(&["commit", "-qm", message]);
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn git_context_scopes_status_history_and_office_diff_to_open_folder() {
        let repo = TempRepo::new();
        let folder = repo.0.join("docs");
        let word = ibis_formats::markdown::import_markdown_to_docx_bytes;
        repo.write("outside.md", "outside");
        repo.write(
            "docs/report.docx",
            word(b"# Report\n\nInitial estimate.").unwrap(),
        );
        repo.commit("Initial documents");
        repo.write("outside.md", "outside-only update");
        repo.commit("Outside update");
        repo.write(
            "docs/report.docx",
            word(b"# Report\n\nFinal estimate.").unwrap(),
        );
        repo.commit("Update report");
        repo.write("docs/pending.md", "pending");
        repo.write("unrelated.md", "unrelated");
        let status = run(&folder, "git_status", &json!({}), 0).unwrap();
        assert_eq!(
            status["changes"],
            json!([{"path":"pending.md","index":"?","worktree":"?"}])
        );
        let log = run(&folder, "git_log", &json!({}), 0).unwrap();
        let commits = log["commits"].as_array().unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0]["subject"], "Update report");
        assert_eq!(commits[1]["subject"], "Initial documents");
        assert_ne!(commits[0]["parent"], commits[1]["commit"]);
        let diff = run(
            &folder,
            "git_diff",
            &json!({"path":"report.docx",
            "old_commit":commits[1]["commit"],"new_commit":commits[0]["commit"]}),
            0,
        )
        .unwrap();
        assert_eq!(diff["format"], "office_markdown");
        assert_eq!(diff["bytes_changed"], true);
        let patch = diff["diff"].as_str().unwrap();
        assert!(
            patch.contains("-Initial estimate.") && patch.contains("+Final estimate."),
            "{patch}"
        );
        assert!(run(&folder, "git_log", &json!({"path":"../outside.md"}), 0).is_err());
        assert!(
            run(
                &folder,
                "git_diff",
                &json!({"path":"../outside.md",
            "old_commit":commits[1]["commit"],"new_commit":commits[0]["commit"]}),
                0
            )
            .is_err()
        );
    }
}
