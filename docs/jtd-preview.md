# Experimental JTD preview

This fork's published editor is available at <https://ibis.sorane.dev>.
`wrangler.jsonc` attaches that Custom Domain to the existing
`ibis-office-jtd-layout-20261111` Worker. Its `workers.dev` URL remains
available for previously shared links. Deploy this branch with the
project-local `node_modules/.bin/wrangler deploy` after building and checking
the assets; keep the Custom Domain in the configuration on subsequent deploys.

**Layout follow-up (2026-11-11):** agreeing explicit default-font references
are imported, DOCX grid pitch survives half-point size rounding, body ruby
keeps its base formatting, and uncovered full or mixed border segments are
drawn with their line styles. Actual browser table screenshots have now been
checked against the corresponding local PDF. Cell text distribution and
DOCX row-segment export remain incomplete. See
[validation and limitations](jtd-layout-compatibility.md).

**Structured decoding (2026-11-10):** the defects recorded in the
[decoding audit](jtd-decoding-audit.md) are fixed: the block directory and
piece lists are reconstructed, inline footers resume ordinary text, and
paragraph headers are parsed as TLV lists. Independent text and display-row
qualification covers 22 local documents. This does not establish visual
fidelity. See [implementation and remaining gaps](jtd-structured-import.md).

The browser and desktop app can open `.jtd`, uncompressed `.jtt`, and observed LH5-compressed `.jttc` files locally as a read-only preview when their content is supported. The original is never saved as JTD, JTT or JTTC; recognized content can be exported separately as DOCX. Recognized view-style direction flags enable vertical writing; one observed ruby pattern is imported. Some table rows and supported 008f border records are reconstructed approximately.
File > Export > Convert to Word and edit creates an editable DOCX from the recognized
preview in memory. Save it separately; the source file remains unchanged. The
conversion is lossy, including missing images and some table rules, and the
result is not a substitute for the original. Explicit read-only mode remains
read-only. Editing or writing JTD, JTT or JTTC is not implemented.
It reads the observed `DocumentText` stream into ibisOffice's document model;
most formatting, exact layout, pictures and some text controls are not reproduced.
Unsupported or encrypted variants may not open. JTTC support is limited to the
observed `JustCompressedDocument` / LH5 wrapper, checked with an output size
limit and CRC. Two local JTTC files decompress and their inner text structures
can be inspected, but both contain no readable body text and embedded objects.
For this specific no-readable-body case, the app can instead show a read-only,
explicitly unplaced list of renderable embedded WMF candidates. The list is
not the original document: pages, position, size, order and visual fidelity
are unknown. Unsupported candidates are omitted, with inspection capped at
32 candidates and 8 MiB. Word conversion/export is disabled in this mode;
the original source remains available through the original download action.
Images are not placed in ordinary text previews or converted documents.
When a document has readable text, View > Show Ichitaro embedded images opens
the same explicitly unplaced image list separately; View > Return to Ichitaro
body preview returns to the read-only body and its normal DOCX conversion.
Only the observed `EmbeddedPress` WMF candidates are shown; other embedded
content can still be omitted. If no candidate is displayable, the body preview
stays open with a message.
Each image is labelled with its observed embedding index, which is not a page
number or evidence of source order. The list reports how many recognized stream
candidates were displayed; it does not count other embedded formats.

For format investigation, `cargo run -p ibis-pptx --example jtd_frame_links -- INPUT`
reports candidate links between `EmbeddingInfo` references and `Frame` IDs.
It exposes duplicate matches, missing matches, and raw size comparisons rather
than selecting a position. It reads the source without writing to it. Values,
coordinate units, page association and paint order remain unverified; the tool
does not place images or convert the image list to DOCX.
Three local JTT files were checked against the independent diagnostic for
normalized text, ruby and ruled display-row counts. This is not a general JTT
compatibility or visual-fidelity claim; the files remain local.
The preview reports detected additional `ObjectSheets/DocSheet/DOCS_*`
document streams and embedded-object storages in its status message; those
contents are not imported. This inventory covers observed container patterns
only and is not proof that all unsupported content has been detected.
The source bytes remain in the browser; while the JTD, JTT or JTTC preview is open, the
optional embedding download action returns the original bytes, not a re-created
source file. After conversion to an editable Word document, it returns the DOCX instead.
Saving or editing as JTD, JTT or JTTC is
not supported; exporting a lossy DOCX preview is available.

The independent importer uses format observations in OpenJTD RFC 0003 and
0009, revision `cfb44c9ac53126e00808c49709fb4dade8a4daae`:
https://github.com/OpenJTD/rjtd/tree/cfb44c9ac53126e00808c49709fb4dade8a4daae/openjtd-spec/rfc

Table-row observations additionally refer to RFC 0009 in the local OpenJTD
checkout at revision `28927abb6b103dc70453f6443a7da961acfccb92`. That revision
was not present in the public upstream checked for this work.
These draft observations do not establish every record's semantics. Unknown
constructs are omitted from the preview; it is not a faithful document rendering.
Synthetic fixtures are used in tests. Local samples must not be redistributed
without permission.

## Mapping to Word layout

| JTD feature | ibisOffice Word model / layout | Current JTD import |
| --- | --- | --- |
| Writing direction | `PageSetup.direction` and `flow_layout` page rotation | Recognized `DocumentViewStyles` flags select vertical-rl. The paper-record heuristic was removed. Unknown layouts remain unsupported. |
| Page size, margins and grid | `PageSetup` | Known paper size, margins, character and line grids are imported, using Ichitaro defaults for omitted fields. Table width follows the text axis. Multi-column sections remain incomplete. |
| Paragraph and character formatting | `FlowParagraph`, `FlowRunStyle`, `RunFormat` | Known alignment, indents, feed, size, bold, italic, underline and colour are imported. Font selectors and further properties remain incomplete. |
| Page breaks | `FlowRunKind::PageBreak` | An observed break immediately following a text run is imported. Other break forms are not decoded. |
| Ruby and tate-chu-yoko | `FlowRunKind::Ruby`, `FlowRunStyle` | A specific observed base/reading pair is mapped to ruby; font size and positioning use approximate Word defaults. Other ruby forms and tate-chu-yoko are not decoded. |
| Tables and borders | `FlowTable`, `FlowRow`, `FlowCell`, `FlowBorder` | Validated 008f records supply full-height cell edges and some full-width bottom rules; other geometry uses the earlier approximate grid. Empty ruled rows can preserve a preceding table edge. Some half-height, mid-line and partial rules are displayed as row-relative segments. DOCX export currently omits these segments. Widths, line styles, pagination and logical merges remain incomplete. See [format notes](jtd-format-notes.md). |
| Images and drawings | media, shapes | Not decoded. |
| Headers, footers, section breaks | `FlowDocument` and section/page layout | Not decoded. |

The browser's File > Export menu can download a `.docx` made from the
currently recognized preview model. This is a **lossy conversion**, not a
faithful representation of the source. The separate embedding download action
still returns the original JTD unchanged. JTD writing is not implemented;
no support for round-tripping unknown records is implied.
