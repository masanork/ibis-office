---
type: help
title: Mermaid Diagrams Guide
description: Embed flowcharts, sequence diagrams, pie charts, and XY charts in Markdown
tags: [help, mermaid, diagram, chart]
help_version: 1
profile: sorane-okf/0.1
---

# Mermaid Diagrams Guide

[[help/index|← Back to Help Index]]

Use a fenced code block with the `mermaid` language tag to render diagrams and charts directly. When exporting to DOCX or PPTX, supported charts and diagrams can also be converted to native Office shapes and charts.

## 1. Flowchart (`flowchart LR` / `TD`)

```mermaid
flowchart LR
  Input["Markdown / Office File"] --> Engine["ibisOffice Engine"]
  Engine --> Split["Bidirectional Split"]
  Engine --> Export["PDF / DOCX / XLSX / PPTX"]
```

## 2. Sequence Diagram (`sequenceDiagram`)

```mermaid
sequenceDiagram
  participant User
  participant Editor as soraneDocs
  participant GitHub as GitHub Vault
  User->>Editor: Edit & Save (⌘S)
  Editor->>GitHub: Create Commit
  GitHub-->>User: Revision Saved
```

## 3. Pie Chart (`pie`) & XY Chart (`xychart-beta`)

```mermaid
pie title Document Formats
  "Markdown" : 45
  "Excel (XLSX)" : 25
  "Word / JTD" : 20
  "PowerPoint" : 10
```

```mermaid
xychart-beta
  title "Monthly Documents Created"
  x-axis [Jan, Feb, Mar, Apr]
  y-axis "Count" 0 --> 60
  bar [18, 29, 42, 55]
  line [15, 25, 38, 50]
```

## 4. Other Supported Diagrams

- `stateDiagram-v2` (State diagrams)
- `classDiagram` (Class diagrams)
- `erDiagram` (Entity-relationship diagrams)
- `gantt` (Gantt charts)

> [!TIP] Labels with parentheses or special characters
> Wrap node labels in quotes, e.g. `id["Label (Detail)"]`, to avoid syntax errors.
