# Origin and contribution

The subsetting engine was imported from `masanork/gjs`, commit
`9173a4d90addf8b6c7e3563473c9195e3acbdb5e`,
`crates/font-subsetter/src/lib.rs` on 2026-11-11.

Masanori Kusunoki (@masanork) authorized this donation and its distribution
under the ibisOffice project license (AGPL-3.0-only). The original project
notice below is retained for provenance. This does not relicense Allsorts,
Brotli, wasm-bindgen or any input/output font. Their own licenses and notices
remain applicable. No claim of an upstream CLA acceptance or copyright
assignment is made by this import.

The GJS font-merging CLI/module was not imported. The subsetting APIs retain
IVS handling and font attribution/license records. Generated subsets use the
independent `IBS Subset` family. No GJS checkout or hosted service is needed.

## Original GJS notice

```text
MIT License

Copyright (c) 2026 Digital Agency, Government of Japan / Masanori Kusunoki

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

```
