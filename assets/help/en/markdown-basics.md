---
type: help
title: Markdown Basics & Extensions
description: Headings, text styling, lists, tables, callouts, footnotes, and math syntax
tags: [help, markdown, gfm]
help_version: 1
profile: sorane-okf/0.1
---

# Markdown Basics & Extensions

[[help/index|← Back to Help Index]]

soraneDocs / ibisOffice supports CommonMark and GitHub Flavored Markdown (GFM) along with callouts, footnotes, math, wikilinks, and Mermaid diagrams.

## 1. Headings & Paragraphs

```markdown
# Heading 1 (H1)
## Heading 2 (H2)
### Heading 3 (H3)
```

You can attach a custom anchor ID to a heading using `{#custom-id}`.

## 2. Text Formatting

| Syntax | Rendered | Usage |
| --- | --- | --- |
| `**Bold**` | **Bold** | Strong emphasis |
| `*Italic*` | *Italic* | Italic text |
| `~~Strikethrough~~` | ~~Strikethrough~~ | Deleted or revised text |
| `` `Inline code` `` | `code` | Commands and identifiers |
| `==Highlight==` | ==Highlight== | Highlighted text |
| `H~2~O` / `x^2^` | H~2~O / x^2^ | Subscript and superscript |

## 3. Lists & Task Lists

```markdown
- Bullet item 1
  - Nested item
1. Numbered item 1
2. Numbered item 2

- [x] Completed task
- [ ] Pending task
```

Rendered example:
- [x] Completed task
- [ ] Pending task

## 4. Tables

```markdown
| Item | Left | Center | Right |
| :--- | :--- | :---: | ---: |
| Budget | Plan A | ○ | 120,000 |
| Actual | Plan B | △ | 98,500 |
```

Use `:` in the separator row to control column alignment.

## 5. Blockquotes & Callouts

Standard blockquotes use `>`. Adding `[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, or `[!CAUTION]` on the first line turns the blockquote into a styled callout.

```markdown
> [!NOTE] Reference
> Highlight helpful context or background notes clearly.

> [!WARNING] Caution
> Remind readers before performing destructive operations.
```

> [!TIP] Supported Callouts
> `NOTE`, `TIP`, `IMPORTANT`, `WARNING`, and `CAUTION` are supported.

## 6. Code Blocks, Footnotes & Dividers

Wrap code blocks in triple backticks (```` ```language ````). Horizontal rules use `---` surrounded by blank lines.

Footnotes are referenced with `[^1]` and defined as `[^1]: Footnote text`[^1].

[^1]: Sample footnote. Footnotes are preserved during `.docx` round-trip conversion.

## 7. Math (LaTeX / KaTeX)

Use `$...$` for inline math and `$$...$$` for display math:

```markdown
The Pythagorean theorem is $a^2 + b^2 = c^2$.

$$
E = mc^2
$$
```
