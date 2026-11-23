// SPDX-License-Identifier: AGPL-3.0-only

use std::io::Read;

use ibis_formats::agent_skill::{self, Metadata, Skill};

use super::{Folder, read_frontmatter};

const DIRECTORY: &str = ".agents/skills";
const FILE_LIMIT: u64 = 64 * 1024;
const SCAN_LIMIT: usize = 256;

impl Folder {
    pub(super) fn scan_skills(&self) -> Vec<Metadata> {
        let agents = self.root.join(".agents");
        let directory = self.root.join(DIRECTORY);
        if [&agents, &directory]
            .into_iter()
            .any(|path| !path.symlink_metadata().is_ok_and(|info| info.is_dir()))
        {
            return Vec::new();
        }
        let Ok(entries) = directory.read_dir() else {
            return Vec::new();
        };
        let mut names: Vec<_> = entries
            .take(SCAN_LIMIT)
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort_unstable();
        names
            .into_iter()
            .filter_map(|name| {
                let path = self.skill_path(&name).ok()?;
                let frontmatter = read_frontmatter(&path)?;
                agent_skill::parse(&name, &frontmatter)
                    .ok()
                    .map(|skill| skill.metadata)
            })
            .collect()
    }

    pub(crate) fn load_skill(&self, name: &str) -> Result<Skill, String> {
        if !self.skills.iter().any(|skill| skill.name == name) {
            return Err(format!("Skill not found in the open folder: {name}"));
        }
        let path = self.skill_path(name)?;
        let mut text = String::new();
        std::fs::File::open(&path)
            .and_then(|file| file.take(FILE_LIMIT + 1).read_to_string(&mut text))
            .map_err(|error| error.to_string())?;
        if text.len() as u64 > FILE_LIMIT {
            return Err("SKILL.md exceeds 64 KiB".into());
        }
        agent_skill::parse(name, &text).map_err(|error| error.to_string())
    }

    fn skill_path(&self, name: &str) -> Result<std::path::PathBuf, String> {
        if !agent_skill::valid_name(name) {
            return Err("Invalid skill name".into());
        }
        self.resolve_file(&format!("{DIRECTORY}/{name}/SKILL.md"))
    }
}

pub(super) fn is_reference(name: &str, path: &str) -> bool {
    path.strip_prefix(&format!("{DIRECTORY}/{name}/references/"))
        .is_some_and(|tail| {
            !tail.contains(['\\', ':'])
                && tail
                    .split('/')
                    .all(|part| !part.is_empty() && super::visible(part))
        })
}

pub(super) fn is_skill_file(name: &str, path: &str) -> bool {
    path == format!("{DIRECTORY}/{name}/SKILL.md") || is_reference(name, path)
}
