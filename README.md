<p align="center"><b>English</b> | <a href="docs/README-ja.md">日本語</a></p>

<p align="center">
  <img src="crates/ibis-office-app/branding/sorane-docs.svg" alt="soraneDocs" width="84">
</p>

<h1 align="center">soraneDocs</h1>

<p align="center">
  <b>An experimental, feature-extended fork of <a href="https://github.com/ibisjp/ibis-office">ibisOffice</a> built with Rust and GPUI.</b><br>
  Office documents, Markdown knowledge workspaces, diagrams, and civic/enterprise format extensions across Desktop, Web (Wasm/WebGPU), and Mobile.
</p>

<p align="center">
  <a href="https://docs.sorane.dev">Try the Web App (docs.sorane.dev)</a> ·
  <a href="docs/fork-feature-differences.md">Feature Differences</a> ·
  <a href="docs/fork-workflow.md">Fork Workflow</a>
</p>

---

<!-- TODO: Add soraneDocs screenshots here -->

## What is soraneDocs?

**soraneDocs** is an active fork of [ibisOffice](https://github.com/ibisjp/ibis-office) maintained in `sorane-dev/docs`. While inheriting upstream's Rust + GPUI engine, pixel-accurate OOXML (`.docx`, `.xlsx`, `.pptx`) rendering, and MCP support, this fork serves two purposes:

1. **Prototyping & Upstream Contributions**: Developing bug fixes, Japanese typography improvements, and format extensions as verifiable proof-of-concept commits and proposing them to `ibisjp/ibis-office` via Issues.
2. **Extended Document & Knowledge Workspace**: Integrating Markdown-first knowledge workflows, diagramming, Japanese civic/legal/medical formats, and PWA/GitHub Vault capabilities for personal and organizational use.

For core ibisOffice features, benchmarks, and commercial licensing, please refer to the [upstream repository](https://github.com/ibisjp/ibis-office).

## Contributions to ibisOffice

Because upstream `ibisjp/ibis-office` restricts pull requests to repository collaborators, contributions from this fork are proposed through GitHub Issues backed by working PoC commits. The following proposals and fixes have been adopted into upstream:

- **Japanese Typography & Font Rendering**
  - IVS (Ideographic Variation Sequences) and Japanese Government Standard Characters support, plus macOS supplementary glyph fix ([#158](https://github.com/ibisjp/ibis-office/issues/158), [#251](https://github.com/ibisjp/ibis-office/issues/251))
  - Fix MS Mincho font substitution on non-Windows platforms ([#248](https://github.com/ibisjp/ibis-office/issues/248))
  - Japanese symbol/kana advance width and GPOS positioning improvements for fallback fonts and Web ([#230](https://github.com/ibisjp/ibis-office/issues/230))
  - Vertical-text emphasis mark placement, vertical table cell direction/merging, and justified bullet list layout fixes in DOCX ([#160](https://github.com/ibisjp/ibis-office/issues/160), [#222](https://github.com/ibisjp/ibis-office/issues/222), [#193](https://github.com/ibisjp/ibis-office/issues/193))
- **Document Formats, Compatibility & Civic Tech**
  - Pure-Rust PDF, XPS/OpenXPS, and OFD viewing and signature inspection ([#235](https://github.com/ibisjp/ibis-office/issues/235))
  - Native Markdown round-trip conversion, clipboard/CLI support, and pure-Rust Mermaid rendering ([#226](https://github.com/ibisjp/ibis-office/issues/226))
  - Office 2007 Standard encryption decryption for DOCX/XLSX/PPTX ([#190](https://github.com/ibisjp/ibis-office/issues/190))
  - Preservation of DOCX paragraph IDs (`w14:paraId`) on edit/save ([#221](https://github.com/ibisjp/ibis-office/issues/221))
  - Spreadsheet machine-readability checker (Digital Agency guidelines) ([#194](https://github.com/ibisjp/ibis-office/issues/194))
- **Web, Mobile & Linux Desktop**
  - Chunked Wasm delivery for CDN file-size limits ([#152](https://github.com/ibisjp/ibis-office/issues/152)) and on-demand web font glyph fetching ([#157](https://github.com/ibisjp/ibis-office/issues/157))
  - Web two-finger pinch-to-zoom and touch timestamp panic fix ([#212](https://github.com/ibisjp/ibis-office/issues/212), [#207](https://github.com/ibisjp/ibis-office/issues/207))
  - Android file-picker return crash fix ([#256](https://github.com/ibisjp/ibis-office/issues/256)) and Ubuntu AppImage dock icon fix ([#147](https://github.com/ibisjp/ibis-office/issues/147))

## Current Differences from Upstream

Detailed tracking of upstream sync status and feature parity is maintained in [docs/fork-feature-differences.md](docs/fork-feature-differences.md). Key additions currently in `soraneDocs`:

- **Knowledge Workspace (`SoraneWorkspace`) & Markdown / PlantUML**
  - Obsidian-style desktop shell (`crates/sorane-workspace`) with split panes, tab pinning, session persistence, command palette, daily notes, Wiki links, Tag Browser, Outgoing Links, Local Graph, Outline, Office Inspector, and Git dock ([#283](https://github.com/ibisjp/ibis-office/issues/283)).
  - Block-level Markdown `SourceMap` with surgical WYSIWYG writeback and bidirectional cursor/scroll sync ([#286](https://github.com/ibisjp/ibis-office/issues/286)).
  - Offline pure-Rust PlantUML SVG rendering sharing Mermaid's diagram IR and assets across UML and non-UML diagrams ([#298](https://github.com/ibisjp/ibis-office/issues/298)).
- **Additional Document, Diagram & Data Formats**
  - **Visio (`.vsdx`)**: Viewing, creation, editing, stencil palette, flowchart shapes, and Wasm support ([#292](https://github.com/ibisjp/ibis-office/issues/292)).
  - **Ichitaro (`.jtd`, `.jtt`, `.jttc`)**: Read-only preview with vertical text, ruby, borders, images, and DOCX/HTML export ([docs/jtd-preview.md](docs/jtd-preview.md)).
  - **DocuWorks (`.xdw`, `.xbd`) & Hancom (`.hwp`, `.hwpx`)**: Viewing and editable DOCX import ([#254](https://github.com/ibisjp/ibis-office/issues/254), [#288](https://github.com/ibisjp/ibis-office/issues/288)).
  - **EPUB 3 & Production Images**: Native EPUB 3 import, plus HEIC, DPX, Cineon, OpenEXR, and JPEG 2000 preview ([docs/production-image-preview-ja.md](docs/production-image-preview-ja.md)).
  - **External Data Sources**: Spreadsheet ingestion for Parquet, JSON Lines, Arrow IPC, Avro, and SQLite, plus encoding-aware CSV save ([#263](https://github.com/ibisjp/ibis-office/issues/263)).
- **Japanese Civic, Legal, Medical & Trust Extensions**
  - **Roster & Name Matching**: `FIELDMATCH` functions, variant character dictionaries, date review, and administrative PUP/Gaiji normalization ([#257](https://github.com/ibisjp/ibis-office/issues/257)).
  - **Legal & Medical**: e-Gov XML / Lawtext bidirectional conversion and comparison tables ([#229](https://github.com/ibisjp/ibis-office/issues/229)); pure-Rust medical document (UKE/DPC, FHIR/CDA/HL7 v2) and non-diagnostic DICOM interoperability ([#246](https://github.com/ibisjp/ibis-office/issues/246)).
  - **Signatures, Credentials & Supply Chain**: Document signature inspection, image C2PA verification, SD-JWT / mDOC / OID4VP spreadsheet tooling, and CycloneDX SBOM/CBOM + OSV-Scanner review ([#232](https://github.com/ibisjp/ibis-office/issues/232), [#279](https://github.com/ibisjp/ibis-office/issues/279), [#280](https://github.com/ibisjp/ibis-office/issues/280)).
- **Web, PWA, Mobile & AI Assistant**
  - **Web (`docs.sorane.dev`)**: Installable PWA with offline local-file support, GitHub App repository (Vault) login/creation/editing, HTML export ([#208](https://github.com/ibisjp/ibis-office/issues/208)), and static Japanese font subset chunks.
  - **Mobile & Performance**: Android/iOS native GPUI builds ([#247](https://github.com/ibisjp/ibis-office/issues/247)) and hot-path rendering/geometry caching optimizations ([#299](https://github.com/ibisjp/ibis-office/issues/299), [#300](https://github.com/ibisjp/ibis-office/issues/300)).
  - **AI Assistant**: Cross-format editing sidebar via OpenAI-compatible APIs (Desktop) and Cloudflare Workers AI (Web) ([#231](https://github.com/ibisjp/ibis-office/issues/231)).
- **Branding, Provenance & Packaging**
  - Distinct `soraneDocs` branding and app identifiers so builds can coexist with upstream `ibisOffice`.
  - Upstream update checks and Windows automatic "Open With" registration are disabled to avoid conflicts.
  - CI generates CycloneDX 1.6 SBOM/CBOM artifacts and GitHub Artifact Attestations for Linux AppImage builds ([#285](https://github.com/ibisjp/ibis-office/issues/285)).

## Building & Development

```bash
git clone --recurse-submodules https://github.com/sorane-dev/docs.git
cd docs
cargo build --release
```

- **Web edition**: `scripts/build-web.sh --release` (serves `www/` via WebAssembly + WebGPU).
- **Desktop bundles**: `scripts/bundle-macos.sh --release`, `scripts/bundle-linux.sh --release`, or `cargo build --release -p ibis-office-app` on Windows.
- **Branch & CI workflow**: `sorane` is the integration branch. See [docs/fork-workflow.md](docs/fork-workflow.md) for testing, CI verification, and deployment details.

## License & Attribution

- **Powered by [ibisOffice](https://github.com/ibisjp/ibis-office)** (in accordance with [docs/TRADEMARK.md](docs/TRADEMARK.md)).
- Licensed under **AGPL-3.0-only** — see [LICENSE](LICENSE).
- Original copyrights in ibisOffice are held by ibis inc. Third-party notices are listed in [docs/THIRD-PARTY-NOTICES.md](docs/THIRD-PARTY-NOTICES.md).
