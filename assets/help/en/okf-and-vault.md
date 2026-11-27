---
type: help
title: OKF & Vault Workflow
description: YAML frontmatter, #tag, [[wikilink]], Backlinks, Local Graph, and search
tags: [help, okf, vault, obsidian]
help_version: 1
profile: sorane-okf/0.1
---

# OKF & Vault Workflow

[[help/index|← Back to Help Index]]

soraneDocs uses **Open Knowledge Format (OKF)** and Obsidian-compatible knowledge syntax so your notes stay human-readable while remaining structured for search, AI tools, and link graphs.

## 1. YAML Frontmatter

Add metadata at the top of a Markdown file between `---` fences:

```yaml
---
type: note
title: Weekly Sync Notes
tags: [meeting, project-a]
aliases: [Weekly Sync]
status: active
profile: sorane-okf/0.1
---
```

## 2. Tags (`#tag`)

Write `#meeting` or `#project/phase1` in the body text to filter related notes from the Vault tag list (headings like `# Heading` are automatically ignored).

## 3. Wiki Links (`[[wikilink]]`)

Link to another note in your Vault with `[[filename]]` or `[[folder/filename|Display Label]]`.

- Clicking a link opens the target note.
- The **Backlinks** panel automatically shows all notes referencing the current note.
- **Local Graph** visualizes connections around the active note.

## 4. Search & Quick Switcher

- **Full-Text Search**: Search across all `.md` and `.lawtext` files in the Vault from the left sidebar.
- **Quick Switcher**: Jump to any note by filename, alias, or `#tag`.

## 5. Syncing Latest Help to an Existing Vault

If you already created a GitHub Vault or local Vault and want to add or update the `help/` directory, click **Sync Latest Help to Vault** in the GitHub repository settings modal or workspace. Only files under `help/` are updated in your current UI language, leaving your other notes untouched.
