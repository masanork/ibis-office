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

## Gates before activation

- Establish and protect a production signing key, review the public key, embed
  it in the client, and define key rotation and incident recovery. No test key
  in this repository is a production trust anchor.
- Set up release packaging and publication for each supported architecture,
  including platform-specific integrity checks and independent verification.
- macOS requires the Sorane Docs Developer ID team and signed/notarized app;
  the inherited ibis inc. team identifier must never be used as the fork's.
  Windows release signing and verification also require a distribution policy.
- Exercise the entire download, verification, replacement, rollback and restart
  flow on installed copies of each platform, including interruptions, untrusted
  keys, corrupted files and unwritable installs. CI packaging alone is not proof.
- Only after those gates restore the check and settings UI. The current updater
  entry points remain disabled and the embedded trust key is empty, so it fails
  closed. CI artifacts are not release artifacts. No release feed is published
  by these changes.
