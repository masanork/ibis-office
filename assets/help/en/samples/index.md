---
type: help
title: Samples & Rendering Walkthrough
description: Walkthrough of real Japanese statutory data (My Number Act) and machine-readable formats
tags: [help, samples, walkthrough]
help_version: 1
profile: sorane-okf/0.1
---

# Samples & Rendering Walkthrough

[[help/index|← Back to Help Index]]

The `help/samples/` folder contains **real statutory data (Japan's My Number Act) and representative machine-readable formats**.
Open each file below and switch **[Source | Split | Visual]** to **Split** to inspect the raw source alongside the live rendered output.

---

## Step 1. Statutory Law Formats (My Number Act, Act No. 27 of 2013)

Real excerpt (Chapter 1: General Provisions) fetched from the official e-Gov Law API (`425AC0000000027`):

1. **e-Gov Legal Standard XML**: [[help/samples/mynumber-act-ch1.xml|mynumber-act-ch1.xml]]
   * **What to look for**: `<Law>`, `<Article>`, and `<Paragraph>` elements are rendered into structured Japanese statutory layout in the Visual pane while preserving surgical XML source editing on the left.
2. **Lawtext (Human-Editable Legal Text)**: [[help/samples/mynumber-act-ch1.lawtext|mynumber-act-ch1.lawtext]]
   * **What to look for**: The same statute in clean indentation-based **Lawtext** syntax, with bidirectional preview and live legislative lint checks.

---

## Step 2. Structured Markdown Formats

3. **OKF Knowledge Note**: [[help/samples/knowledge-note.md|knowledge-note.md]]
   * **What to look for**: YAML frontmatter, `#tag`, `[[wikilink]]`, callouts (`[!TIP]`), task lists, tables, math, and footnotes rendered live.
4. **Marp Presentation Deck**: [[help/samples/marp-deck.md|marp-deck.md]]
   * **What to look for**: `marp: true` and `---` separators turn Markdown into an editable multi-slide deck with embedded Mermaid diagrams.
5. **Workbook Markdown (Spreadsheet)**: [[help/samples/workbook-sheet.md|workbook-sheet.md]]
   * **What to look for**: `ibis-workbook: true` builds multi-sheet tables with live formula recalculation (`=SUM(B2:B3)`).
6. **Mermaid Diagrams**: [[help/samples/mermaid-diagrams.md|mermaid-diagrams.md]]
   * **What to look for**: Flowcharts, sequence diagrams, pie charts, and XY charts rendered directly from fenced code blocks.

---

## Step 3. Machine-Readable Data & Open Standards

7. **CycloneDX 1.6 CBOM / SBOM (JSON)**: [[help/samples/payment-demo.cdx.json|payment-demo.cdx.json]]
   * **What to look for**: Structured tabular inspection of cryptographic/software bill-of-materials components.
8. **AI Evaluation Results (JSONL)**: [[help/samples/ai-eval-results.jsonl|ai-eval-results.jsonl]]
   * **What to look for**: Line-delimited JSON evaluation records rendered as a structured table with Pass/Fail and scores.
9. **FDSN StationXML**: [[help/samples/stations.xml|stations.xml]]
   * **What to look for**: Hierarchical seismic station metadata XML rendered in the structured data viewer.
10. **Field-Matching Candidate Roster (CSV)**: [[help/samples/field-match-candidates.csv|field-match-candidates.csv]]
    * **What to look for**: Japanese name/address CSV loaded directly into the spreadsheet grid for inspection and matching.
