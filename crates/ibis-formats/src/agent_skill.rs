// SPDX-License-Identifier: AGPL-3.0-only

//! The portable Agent Skills format (https://agentskills.io/specification).

use serde_json::Value;

#[derive(Clone)]
pub struct Metadata {
    pub name: String,
    pub description: String,
}

pub struct Skill {
    pub metadata: Metadata,
    pub instructions: String,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub fn parse(directory: &str, text: &str) -> Result<Skill, crate::Error> {
    let (fields, body) = crate::okf::markdown_frontmatter(text)?
        .ok_or_else(|| crate::Error::new("SKILL.md must have YAML frontmatter"))?;
    let name = fields
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !valid_name(name) || name != directory {
        return Err(crate::Error::new(
            "Skill name must match its directory and follow the Agent Skills naming rules",
        ));
    }
    let description = fields
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.chars().count() <= 1024)
        .ok_or_else(|| crate::Error::new("Skill description must contain 1–1024 characters"))?;
    Ok(Skill {
        metadata: Metadata {
            name: name.into(),
            description: description.into(),
        },
        instructions: body.trim().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_standard_yaml_and_markdown() {
        let text = "---\r\nname: report-check\r\ndescription: >-\r\n  Check the monthly report.\r\nallowed-tools: Bash\r\n---\r\n\r\nRead the checklist.\r\n";
        let skill = parse("report-check", text).unwrap();
        assert_eq!(skill.metadata.description, "Check the monthly report.");
        assert_eq!(skill.instructions, "Read the checklist.");
        assert!(parse("other-name", text).is_err());
        assert!(!valid_name("report--check"));
        assert!(parse("report-check", "---\nname: report-check\n---\n").is_err());
    }
}
