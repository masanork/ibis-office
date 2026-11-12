# Sorane Docs desktop updates

The fork does not yet ship an enabled update channel. The code now has an
Ed25519 verifier for a stable-channel manifest and checks the exact signed
artifact length and SHA-256. A fixed GitHub Releases origin supplies both
metadata and artifacts; the signed manifest contains only filenames, never
arbitrary download URLs. No key is learned from the network or environment.

## Preparing metadata

Keep the Ed25519 signing key outside this repository and CI logs. Given signed,
release-ready platform packages, create a new output directory:

```sh
python3 scripts/sorane-update-manifest.py --version 1.2.3 \
  --private-key /secure/path/sorane-update.pem --out /tmp/sorane-release-metadata \
  macos-arm64=/path/to/Sorane-1.2.3-macos-arm64.zip \
  windows-x64=/path/to/Sorane-1.2.3-windows-x64.zip \
  linux-x64-appimage=/path/to/Sorane-1.2.3-linux-x64.AppImage
```

Use OpenSSL 3; on macOS, pass `--openssl /path/to/openssl` if the system
command is LibreSSL. It emits `sorane-update.json` and a hex-encoded detached signature over its
exact UTF-8 bytes. It does not upload or publish anything. Only the listed
AppImage target is supported for Linux; Flatpak updates belong to its repository.
Publish all package assets before the signed metadata, and verify every asset
and signature after publishing. Metadata expires after at most 31 days and must
be refreshed even if no release is made; system clocks outside its validity
window cause a failed check. Signed metadata can still be replayed until it
expires; the installed version check rejects downgrades but does not prevent a
freeze on an older still-valid newer release. Key rotation, revocation,
persistent anti-rollback and an offline recovery procedure remain to be designed.

## Offline candidate verification

Put the generated `sorane-update.json`, its `.sig`, and each listed package
in one staging directory. Obtain the expected 32-byte Ed25519 public key as
64 hex digits through a separately trusted channel, and verify the exact
staged bytes with the same verifier as the application:

```sh
cargo run --locked -p ibis-office-updater --example verify_sorane_candidate -- \
  /path/to/staging PUBLIC_KEY_HEX macos-arm64 windows-x64 linux-x64-appimage
```

The command requires at least one expected target, checks the signature,
validity period and all listed files, and fails if a required target is absent,
a named package is missing, a package is not a regular file, or any hash or
size is wrong. Keep the staging directory unchanged during verification and
publication. This does not establish the source or provenance of the packages,
OS code signing or notarization, availability on GitHub, or successful updates
on installed apps. The staging key is caller-supplied; it is not a production
trust anchor. Distinct targets must have distinct package filenames.

## Recovery boundary

For the macOS and Windows folder-swap implementation, an existing
`.update-backup` or `.update-new` beside the install now causes a retry to
stop without deleting either path. An interruption after the old folder has
been moved aside can leave the original location absent. Preserve both paths;
inspect and recover the old copy manually rather than starting another update
or deleting them. This is a fail-closed preservation measure, not automated
crash recovery. The standalone Linux updater stages an AppImage beside the
installed file, keeps a hard-linked backup, and restores it if startup fails.
It refuses stale stage or backup paths, symlinked files, and filesystems that
cannot create the backup hard link. An interruption can leave recovery files;
preserve and inspect them manually. File syncing and rename do not establish
power-loss recovery. The application starts the helper bundled with the current AppImage and passes
the size and SHA-256 from verified metadata. The helper bounds the copy and
checks the staged file against those values immediately before replacement.
The helper trusts the parent-provided values; this is an integrity handoff, not
independent signature verification or a defense against a compromised local
account. Older AppImages without the helper cannot perform this update.

A local HTTPS fixture exercises the app's shared curl transport, signed metadata
selection, bounded download, artifact hash verification, and folder swap and
rollback using synthetic bytes. Its CA override is private to the unit test;
the public transport has no CA override parameter, disables curl configuration
files, and enforces HTTPS for initial requests and redirects. The test checks
untrusted TLS, downgrade redirects, malformed signatures and altered, truncated
or oversized downloads. The fixture simulates extraction and launch failure. It does not test a real
archive, GUI, OS code-signing checks, updater process, actual application
launch, machine-interruption recovery, or a public release service. On Unix,
the local HTTPS test runs when `SORANE_TEST_OPENSSL` names OpenSSL 3; the
update-verifier CI sets it to `openssl`.

## Isolated updater-process fixture

On macOS, Windows, and Linux, run the real updater executable against synthetic app
copies under a new temporary directory:

```sh
SORANE_UPDATER_PROCESS_TEST=1 cargo test --locked -p ibis-office-updater --test updater_process
```

The test starts a child owned by the test, waits for it to exit, replaces a
synthetic application, launches a short-lived fixture and checks success or
rollback after an immediate exit. The macOS fixture is ad-hoc signed and uses
a symlinked install path to check the path reported by LaunchServices. Desktop
CI opts into this test on all three platforms. The Linux fixture marks synthetic
ELF executables as AppImages and also checks stale-backup preservation and that
a live parent is never terminated. Three seconds of process liveness is not an
application health check. This does not test a real Sorane Docs install, a
Developer ID signature or notarization, a download or archive, user-interface
confirmation, machine interruption, or a real AppImage update.

The Linux desktop build also extracts the updater from its actual built AppImage
and tests it against isolated copies. The test alters the candidate ELF machine
type to cause an execution failure, then checks that the bundled helper restores
the original AppImage bytes. It supplies the candidate hash directly: it does
not verify a signature, exercise the application UI or app-to-helper handoff,
or establish a successful GUI restart or power-loss recovery.

## Gates before activation

- Establish and protect a production signing key, review the public key, embed
  it in the client, and define key rotation and incident recovery. No test key
  in this repository is a production trust anchor.
- Set up release packaging and publication for each supported architecture,
  including platform-specific integrity checks and independent verification.
- macOS requires the Sorane Docs Developer ID team and signed/notarized app;
  the inherited ibis inc. team identifier must never be used as the fork's.
  Windows release signing and verification also require a distribution policy.
- Validate the complete Linux app-to-helper handoff on installed AppImages,
  including upgrades from versions that contain the helper.
- Exercise the entire download, verification, replacement, rollback and restart
  flow on installed copies of each platform, including interruptions, untrusted
  keys, corrupted files and unwritable installs. CI packaging alone is not proof.
- Only after those gates restore the check and settings UI. The current updater
  entry points remain disabled and the embedded trust key is empty, so it fails
  closed. CI artifacts are not release artifacts. No release feed is published
  by these changes.
