---
type: help
title: Getting Started
description: Opening, editing, saving documents, and connecting a GitHub Vault in ibisOffice / soraneDocs
tags: [help, getting-started]
help_version: 1
profile: sorane-okf/0.1
---

# Getting Started

[[help/index|← Back to Help Index]]

## 1. Opening & Creating Documents

- **New document**: Use **File > New** or the start-view tiles to create a Document (DOCX), Workbook (XLSX), Presentation (PPTX), or Markdown (`.md`) file.
- **Open file**: Use **File > Open** (`⌘O` / `Ctrl+O`), select an item from the start view, or drag and drop a file into the window.
- **Open from Vault**: Pick any file from the left Vault file tree or Quick Switcher.

## 2. Source ↔ Visual Bidirectional Split Editing

When opening Markdown (`.md` / `.markdown`), Marp slides, Workbook Markdown, or Lawtext (`.lawtext`), the editor header displays **[Source | Split | Visual]**.

| Mode | Purpose |
| --- | --- |
| **Source** | Focus on editing raw text source |
| **Split** | Edit source on the left with real-time synced rich/slide/sheet view on the right |
| **Visual** | Edit directly in the rendered document view |

## 3. Saving & Exporting

- **Save (`⌘S` / `Ctrl+S`)**: Saves changes to local files or commits directly to your connected GitHub Vault repository after confirmation.
- **Save As (`⇧⌘S` / `Ctrl+Shift+S`)**: Saves to a new path or downloads a copy in the browser.
- **Export**: Use **File > Export** to export to PDF, PNG, GIF, or HTML.

## 4. GitHub Vault Integration (Web & Desktop)

1. Click **Sign in with GitHub** on the start view or workspace settings.
2. Create a new **Private repository (default: `soraneDocs`)** or select an existing repository.
3. Once connected, you can use the Vault file tree, full-text search, `#tag` filtering, Backlinks, and Local Graph.
4. To copy or update the latest `help/` files into an existing Vault, use **Sync Latest Help to Vault** in the GitHub repository settings modal or workspace.

## 5. Supported File Formats

| Category | Extensions | Highlights |
| --- | --- | --- |
| Markdown / Knowledge | `.md`, `.markdown` | Notes, Marp slides, Workbook sheets, OKF |
| Word Processing | `.docx`, `.odt`, `.rtf`, `.jtd`, `.wpd` | Editing, viewing, PDF export (JTD viewing & DOCX conversion) |
| Spreadsheets | `.xlsx`, `.ods`, `.csv`, `.tsv` | Formula recalculation, charts, structured wide-CSV viewer |
| Presentations & Diagrams | `.pptx`, `.odp`, `.vsdx` | Slide editing, animations, Visio diagram viewing |
| Legal & Structured Data | `.lawtext`, `.xml`, `.jsonl`, `.json` | Lawtext bidirectional editing & lint, AI Eval / SBOM viewer |
