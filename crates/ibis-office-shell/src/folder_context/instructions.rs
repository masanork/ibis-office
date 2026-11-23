// SPDX-License-Identifier: AGPL-3.0-only

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use super::{Folder, visible};

const INSTRUCTIONS_LIMIT: u64 = 24 * 1024;
const MEMORY_LIMIT: u64 = 16 * 1024;

impl Folder {
    pub(crate) fn agent_instructions(&self, document: Option<&Path>) -> Vec<Value> {
        let mut candidates = vec!["AGENTS.md".to_owned()];
        let document = document.and_then(|path| path.canonicalize().ok());
        if let Some(relative) = document
            .as_deref()
            .and_then(Path::parent)
            .and_then(|path| path.strip_prefix(&self.root).ok())
        {
            let mut directory = String::new();
            for name in relative.iter().map(|part| part.to_str()) {
                let Some(name) = name.filter(|name| visible(name) && !name.contains(['\\', ':']))
                else {
                    break;
                };
                directory.push_str(name);
                directory.push('/');
                candidates.push(format!("{directory}AGENTS.md"));
            }
        }

        let mut remaining = INSTRUCTIONS_LIMIT;
        let mut instructions = Vec::new();
        // Keep the most specific instructions first if the size budget is exhausted.
        for path in candidates.into_iter().rev() {
            if let Some(text) = self.read_agent_context(&path, remaining) {
                remaining -= text.len() as u64;
                instructions.push(json!({"path":path,"text":text}));
            }
        }
        instructions.reverse();
        instructions
    }

    pub(crate) fn agent_memory(&self) -> Option<String> {
        self.read_agent_context("MEMORY.md", MEMORY_LIMIT)
    }

    fn read_agent_context(&self, relative: &str, limit: u64) -> Option<String> {
        if limit == 0 {
            return None;
        }
        let path = self.resolve(relative).ok()?;
        let mut text = String::new();
        std::fs::File::open(path)
            .ok()?
            .take(limit + 1)
            .read_to_string(&mut text)
            .ok()?;
        (text.len() as u64 <= limit && !text.trim().is_empty()).then_some(text)
    }
}
