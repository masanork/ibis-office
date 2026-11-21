# C2PA image verification acceptance checks

Issue: [#180](https://github.com/masanork/ibis-office/issues/180).
The desktop implementation landed in `0ccea7e1`; browser support followed in
`fd760e69`. This document records repeatable regression checks for that work.
Public sample and GUI checks are recorded in
[制作素材のプレビュー](production-image-preview-ja.md#原本の来歴).

## Scope and trust policy

JPEG, PNG and TIFF verification uses the original input bytes, independently of
preview conversion. The official Rust SDK (`c2pa 0.92.2`) uses
`default-features = false` and `rust_native_crypto`. Its dependency tree includes
neither OpenSSL nor HTTP clients. Verification is explicitly enabled; remote
manifests and OCSP fetches are disabled. An empty network host allow-list also
blocks SDK resolver traffic if HTTP features are enabled elsewhere in the future.

The shipped manifest and TSA anchors, their pinned upstream revision, hashes,
license and manual update procedure are documented in the
[trust bundle README](../crates/ibis-office-shell/data/c2pa/README.md).
No test anchor is installed in the production context. The UI says that online
revocation checking was not performed. Trusted credentials establish neither the
truth of the recorded history nor the absence of unrecorded edits.

## Automated regressions

```sh
cargo test --locked -p ibis-office-shell --lib --no-default-features image_provenance
```

The same command runs in Desktop build CI on macOS and Windows. It creates small
synthetic images and signs them with an explicitly test-only P-256 certificate.
The test root is injected into one SDK context, never a process-global trust
setting. All inputs are generated offline; no public photographs or private
media samples are committed. The synthetic certificate is valid until 2120.

The ten unit tests cover:

- JPEG, PNG and TIFF: the SDK reports `Trusted` with the test anchor, and the app
  maps it to the trusted state with the signer, action and ingredient titles.
  The identical image under production anchors is untrusted, without losing its
  recorded metadata. Signed and unsigned pixels agree, including PNG/TIFF transparency.
- A still-decodable JPEG with modified original bytes: the SDK reports `Invalid`
  and the app reports a hash mismatch. Unverified history is not displayed.
- A corrupted signing certificate and a malformed manifest: neither can be shown
  as verified. The image remains decodable.
- A remote-only manifest: it remains unverified and is not fetched.
- Legacy v1 manifests without action history: empty history remains explicit.
  Unsigned JPEG/PNG/TIFF images report no manifest.
- File loading, moving to an unsigned next frame, and saving/reopening DOCX:
  preview pixels survive; the original file is unchanged; DOCX has no carried
  verification report and its embedded preview has no C2PA manifest.
- Displayed metadata is limited to eight actions, eight ingredients and 48
  Unicode characters per caption; control / bidi override characters are removed.
  Unsupported formats have no C2PA report.

The shell's resource-list test also had a stale include path to an absent
`publish/docs` directory. It now uses the tracked `docs/resource-list.txt`;
all three resource-list tests pass.

The native editor scenario uses the normal asynchronous file-open path, switches
from a signed frame to an unsigned frame and back, refuses an attempted overwrite
of the source image, and saves DOCX. It checks that a successful save clears the
original image's report from the live editor as well as from the reopened file.
The original source bytes remain untouched. Run on macOS/Linux with:

```sh
cargo run --locked -p ibis-image-provenance-check
```

This runner uses production format drivers and only the shell's opt-in scenario
feature; it also runs in the macOS Desktop CI job. Test signing keys are only
compiled for unit tests or the opt-in shell `scenarios` feature, not normal app
builds.

The existing full `ibis-office-app --features scenarios` suite could not be built
at the inspected base (`79c01963`): the DOCX scenario
`scenario_issue179_multiple.rs` includes an absent
`tasks/doc/requests/2026-11-14-179-rest-many-patterns-result.tsv`, and XLSX scenarios
at lines 20833, 20838 and 20853 pass the removed `slide` field to
`SheetCommand::Insert/Delete` (the field is now `sheet`). These are separate
follow-ups. The focused runner does not enable those format-specific scenario
collections.


## Performance measurement

The opt-in benchmark generates 1920×1080 synthetic RGB images, signs them, then
measures verification and image preview conversion separately. Signing is outside
the measured interval. It reports the first verification and the median of eleven
subsequent runs. The fixture signing and verification use the same SDK as the app.

```sh
CARGO_PROFILE_RELEASE_INCREMENTAL=false CARGO_PROFILE_RELEASE_LTO=false \
  cargo test --release --locked -p ibis-office-shell --lib --no-default-features \
  measure_image_provenance_cost -- --ignored --nocapture --test-threads=1
```

Measured 2026-11-21 on Apple M1 / 16 GiB, macOS 27.2.1, Rust 1.100.0,
SDK 0.92.2. Source is `79c01963` plus this change. Release optimization uses
`CARGO_PROFILE_RELEASE_INCREMENTAL=false` and `CARGO_PROFILE_RELEASE_LTO=false`.
All eleven tests, including the opt-in measurement, passed in 2.50 seconds.

| Signed RGB image | Input bytes | First verification | Median verification | Median preview conversion |
| --- | ---: | ---: | ---: | ---: |
| JPEG | 253,951 | 1.927 ms | 1.883 ms | 76.343 ms |
| PNG | 1,822,230 | 2.930 ms | 3.081 ms | 72.135 ms |
| TIFF | 6,224,845 | 7.225 ms | 7.208 ms | 44.673 ms |

These are in-memory synthetic-image measurements, not end-to-end GUI startup or
archive-media latency. Verification uses production trust anchors, so the test
signer is untrusted, while integrity still validates. Preview conversion is
measured separately and does not include verification. The first measurement is
the first verification of that image within the test process, not a cold OS start.

### Local macOS executable size

A controlled comparison used the same checkout, Rust version, release settings
and package selection (`cargo build --release --locked -p ibis-office-app`). The
baseline temporarily returned `None` from the native `inspect` entry point, so
unused SDK code could be removed by the linker. No dependency versions, other
format implementations or packaging settings changed.

| Local executable | Bytes | MiB |
| --- | ---: | ---: |
| With C2PA verification | 169,006,512 | 161.177 |
| Without the verification call | 158,990,480 | 151.625 |
| Difference | 10,016,032 | 9.552 |

The marginal size is 6.30% of the baseline. This is the on-disk local
executable, not an app bundle, compressed CI artifact, resident memory size or
browser bundle delta. This comparison does not measure GUI startup time.
The ordinary source and previously verified executable were restored after the
measurement; their executable SHA-256 is
`13e9867945f18ce06e7d3e254b11cc9030b3b271165ece500abaeee3b89036e8`.
The verification-disabled binary is not a deliverable.
