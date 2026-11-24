# Visio sample drawings

TopLocalAI's [vsdx-sample-files](https://github.com/TopLocalAI/vsdx-sample-files/tree/21ea96891525399521b2a623f0c7d1d3821c2bc2/samples)
provides these clean-room VSDX drawings under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
The copies are unmodified.

- `01-basic-flowchart.vsdx` — Customer Refund Process
- `06-grouped-architecture.vsdx` — Event Analytics Architecture
- `07-zh-flowchart.vsdx` — Employee onboarding and access provisioning
- `08-multi-page-handbook.vsdx` — Architecture Handbook
- `09-embedded-media-icons.vsdx` — Architecture diagram with embedded PNG icons

## Microsoft Visio files for local testing

Apache POI publishes [two Microsoft Visio 15 drawings](https://github.com/apache/poi/tree/732120980140d5ed64b482c470e0b625cdb1ab15/test-data/diagram)
that can be tested locally. The producer and version are recorded in each file's
`docProps/app.xml`. Download the unmodified files into one directory. Their
SHA-256 digests at this revision are listed below.

| Filename | SHA-256 |
| --- | --- |
| `test.vsdx` | `558c09c518848c98a815c9684f511d5260585fa0c893f2d570b6284d6a9b29a8` |
| `test_text_extraction.vsdx` | `5a5117a6c41313a6e7253689d7ae09c7e46bfbb958a670f60f7f5d0bbaef27f9` |

```sh
VSDX_VISIO_CORPUS_DIR=/path/to/samples cargo test --locked -p ibis-vsdx --test visio_saved -- --ignored
```


The `test.vsdx` package's `docProps/thumbnail.emf` stores Visio's raster preview.
It shows a white container body, with blue limited to the header and outline.
Use this preview to verify which region is filled. Its low resolution limits
comparisons of typefaces and thin outlines.

## Fixed connection points saved by Microsoft Visio

The libvisio corpus includes a [Visio-authored flowchart](https://github.com/LibreOffice/libvisio/blob/bc054fcd9de142eaa513ebaf5689e0bc84958f46/src/test/data/testfile3.vsdx)
with inherited connection points and an orthogonal connector whose transform
uses signed dimensions. The original SHA-256 is
`9ea7dd2a88ecc4e7f29ff5b022712db18baa5b607e44b32d73f2b79bd4029387`.
The test reconnects connector 17 to point 2 on shape 1, saves a separate copy,
and checks the rendered endpoints and saved references after reopening.

```sh
VSDX_VISIO_GLUE_SAMPLE=/path/to/testfile3.vsdx \
VSDX_VISIO_GLUE_OUTPUT=/path/to/testfile3-reconnected.vsdx \
cargo test --locked -p ibis-vsdx --test visio_native_glue -- --ignored
```

Omit `VSDX_VISIO_GLUE_OUTPUT` to use and remove a temporary copy.

## Circular line jumps

The Aspose.Diagram examples include [modified.vsdx](https://github.com/aspose-diagram/Aspose.Diagram-for-Java/blob/4c46d5e3ab5ba8939b43d845633490794276a8e7/Examples/src/main/resources/com/aspose/diagram/examples/OLEObjectsinVisioDiagram/ManipulateEmbeddedOLEObjects/modified.vsdx),
a Visio-origin drawing with two circular line jumps in one connector. The
SHA-256 is `9c140154d9ff810cf72e5ced28d12072756cfccb004e9532b833b499182ad543`.
The drawing has no explicit connection records for those endpoints, so this
fixture tests rendering. The geometry edit suite uses a separate adapted
flowchart to verify reconnection and save/reopen.

```sh
VSDX_VISIO_ARC_SAMPLE=/path/to/modified.vsdx \
cargo test --locked -p ibis-vsdx --test visio_arc -- --ignored
```

## Glued circular line jump saved by Microsoft Visio

The [Data Warehouse & Analytics Platform drawing](https://github.com/bhdicaire/visioStencils/blob/f87f1f5b12f867149de92894778c9c056eb68340/Documents/IT%20Vendors/Data%20Warehouse%20%26%20Analytics%20Platform.vsdx)
reports Microsoft Visio 16 in its application properties. Its SHA-256 is
`b634396dc794d1e797f882e8e8429317b6f050fb499cfa0bb668fb14a05e2bf0`.
Connector 614 has an `ArcTo` row and glue records for both endpoints. The
test attaches its end to shape 253 without preparing or modifying the input,
saves a copy, and verifies the arc, endpoints, and glue on reopening.

```sh
VSDX_VISIO_GLUED_ARC_SAMPLE=/path/to/original.vsdx \
VSDX_VISIO_GLUED_ARC_OUTPUT=/path/to/reconnected.vsdx \
cargo test --locked -p ibis-vsdx --test visio_arc reconnects_a_circular -- --ignored
```

Omit `VSDX_VISIO_GLUED_ARC_OUTPUT` to use and remove a temporary copy.

The desktop scenario uses the same original drawing to apply a shape move and
a reconnect in one editing session, save a copy, and compare the untouched
shapes and package parts after reopening.

```sh
VSDX_VISIO_GLUED_ARC_SAMPLE=/path/to/original.vsdx \
cargo test --locked -p ibis-office-app --test scenarios -- shell::vsdx_visio_authored_drawing_edit_mode
```
