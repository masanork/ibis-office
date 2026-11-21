# Synthetic C2PA signing identity

These certificates and the **publicly disclosed test private key** are generated
solely for automated regression tests. They identify no real person or media.
Never use this key for production signing or add this root to the shipped trust
bundles. The tests inject this anchor into an isolated SDK context only.

The P-256 root and signing certificate are valid from 2020-01-01 to 2120-01-01.
The leaf has digitalSignature key usage and emailProtection extended key usage.
The root private key is not retained. No timestamps or OCSP responses are issued.
Each test creates its own small synthetic JPEG, PNG or TIFF and signs it with the
SDK's pure Rust backend, without fetching assets or invoking OpenSSL.

These synthetic fixtures use the repository's AGPL-3.0-only license and NOTICE.
