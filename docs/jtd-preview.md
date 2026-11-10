# Experimental JTD preview

The browser can open `.jtd` files locally as a read-only text preview. An observed view-style pattern may enable vertical writing; one observed ruby pattern is imported. Some table rows are reconstructed as an approximate grid.
It reads the observed `DocumentText` stream into ibisOffice's document model;
most formatting, exact layout, pictures and some text controls are not reproduced.
Unsupported or encrypted variants may not open. `.jtt` and `.jttc` are not supported.
The source bytes remain in the browser; the optional embedding download action
returns the original bytes, not a re-created JTD. Saving or editing as JTD is
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
| Writing direction | `PageSetup.direction` and `flow_layout` page rotation | A bounded `DocumentViewStyles` record pattern selects vertical-rl. This is an observed candidate, not verified for every JTD variant. |
| Page size, margins and columns | `PageSetup` | Not decoded; Word defaults are used. |
| Paragraph and character formatting | `FlowParagraph`, `FlowRunStyle`, `RunFormat` | Not decoded; defaults are used. |
| Page breaks | `FlowRunKind::PageBreak` | An observed break immediately following a text run is imported. Other break forms are not decoded. |
| Ruby and tate-chu-yoko | `FlowRunKind::Ruby`, `FlowRunStyle` | A specific observed base/reading pair is mapped to ruby; font size and positioning use approximate Word defaults. Other ruby forms and tate-chu-yoko are not decoded. |
| Tables and borders | `FlowTable`, `FlowRow`, `FlowCell`, `FlowBorder` | Ordered cell-boundary candidates in a specific observed row form are reconstructed as tables. Column proportions and black rules are approximations, not decoded JTD dimensions, border presence, or styles. For one observed comparison-table pattern, narrow empty edge strips and completely empty two-unit internal strips are not treated as text columns, and display lines do not receive automatic row borders. Content-bearing narrow cells are preserved. Compatible adjacent observed rows with the same total width can share a union grid using Word cell spans; these spans represent the reconstructed boundaries, not decoded JTD merges. Other row forms remain undecoded; the layout may still differ substantially from the original. |
| Images and drawings | media, shapes | Not decoded. |
| Headers, footers, section breaks | `FlowDocument` and section/page layout | Not decoded. |

The browser's File > Export menu can download a `.docx` made from the
currently recognized preview model. This is a **lossy conversion**, not a
faithful representation of the source. The separate embedding download action
still returns the original JTD unchanged. JTD writing is not implemented;
no support for round-tripping unknown records is implied.
