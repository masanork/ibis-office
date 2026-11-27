---
type: help
title: Workbook Markdown Syntax
description: Build multi-sheet spreadsheets with formulas using Markdown tables
tags: [help, workbook, xlsx, spreadsheet]
help_version: 1
profile: sorane-okf/0.1
---

# Workbook Markdown Syntax

[[help/index|← Back to Help Index]]

Setting `ibis-workbook: true` (or `type: workbook` / `workbook: true`) in the YAML frontmatter opens Markdown as a **Spreadsheet Workbook (XLSX compatible)** with live formula recalculation and bidirectional Split editing.

## 1. Basic Example (Formulas & Multiple Sheets)

```markdown
---
ibis-workbook: true
title: Quarterly Revenue Plan
---

<!-- ibis-sheet: Revenue -->

# Revenue

| Item | Q1 | Q2 | Total |
| --- | ---: | ---: | ---: |
| Licenses | 120000 | 150000 | =SUM(B2:C2) |
| Support | 40000 | 45000 | =SUM(B3:C3) |
| Total | =SUM(B2:B3) | =SUM(C2:C3) | =SUM(D2:D3) |

<!-- ibis-sheet: Summary -->

# Summary

| Metric | Value |
| --- | ---: |
| Annualized | =Revenue!D4*2 |
```

## 2. Key Conventions

- **`<!-- ibis-sheet: SheetName -->`** (or **`<!-- xlsx-sheet "SheetName" -->`**): Starts a new worksheet.
- **Formulas (`=...`)**: Standard formulas such as `=SUM(...)`, `=AVERAGE(...)`, `=IF(...)`, and cross-sheet references (`=Revenue!A1`) are evaluated by the built-in calculation engine.
- **Column Alignment (`:---`, `:---:`, `---:`)**: Right-align numeric columns with `---:` for clean formatting.
- **XLSX Export**: Export or Save As `.xlsx` at any time.
