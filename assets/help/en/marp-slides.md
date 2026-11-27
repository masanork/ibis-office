---
type: help
title: Marp Slide Syntax
description: Create presentations from Markdown with bidirectional PowerPoint editing
tags: [help, marp, slides, pptx]
help_version: 1
profile: sorane-okf/0.1
---

# Marp Slide Syntax

[[help/index|← Back to Help Index]]

Adding `marp: true` in the YAML frontmatter enables **Presentation (Slides)** mode, allowing live slide preview, editing, slideshow playback, and `.pptx` / `.pdf` export.

## 1. Basic Structure

```markdown
---
marp: true
title: Project Update
theme: default
paginate: true
size: 16:9
---

# Title Slide

Subtitle & Presenter

<!-- Speaker notes go inside HTML comments -->

---

## Slide 2

- Bullet point 1
- Bullet point 2

---

## Tables & Mermaid Diagrams on Slides

| Item | Status |
| --- | --- |
| Design | Done |
| Implementation | In Progress |
```

## 2. Directives & Options

| Syntax / Option | Description |
| --- | --- |
| `marp: true` | Enables Marp presentation mode |
| `---` | Slide separator |
| `size: 16:9` / `4:3` | Slide aspect ratio |
| `paginate: true` | Shows slide numbers |
| `header: "..."` / `footer: "..."` | Header and footer text |
| `<!-- note -->` | Speaker notes |

## 3. Bidirectional PPTX Sync

- Edits in the left Markdown pane update the slide deck immediately.
- Edits to text, shapes, tables, or charts in the right Visual pane sync back to Markdown.
- You can export the deck directly to `.pptx` or `.pdf`.
