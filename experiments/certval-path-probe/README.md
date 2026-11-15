# Offline certificate path experiment

This separate Cargo workspace evaluates `certval` 0.2.0-rc.3 with its default
features disabled and its `rsa` feature enabled. It is **not linked to
ibisOffice**, is not a production trust decision, and does not use pverify.
No system trust store, trust-list provider, network retrieval, OCSP or CRL
checking is enabled. The experiment accepts a single explicitly supplied anchor
as an input to a single explicitly constructed path; it does not build paths.
The reference time is an explicit test value. The caller must not substitute an
untrusted document certificate as an anchor. `set_require_ta_store(false)` is
used only because the test itself supplies the anchor; no trust-store membership
check is performed.

From the repository root:

```sh
cargo test --locked --manifest-path experiments/certval-path-probe/Cargo.toml -- --nocapture
cargo +nightly-2026-09-17 check --locked \
  --manifest-path experiments/certval-path-probe/Cargo.toml --target wasm32-unknown-unknown
```

The two tests exercise 11 combinations: a direct valid path, an existing
RSA-SHA384 fixture, a path with an explicit intermediate, a different issuer,
a same-name/different-key issuer, an expired target, a changed signature,
an unknown critical extension, a missing intermediate, a non-CA intermediate,
and an intermediate without keyCertSign. Rejection tests check their respective
error categories. The DER files under `data/` were created with Python
cryptography for this experiment; keys were discarded. Two other certificates
are existing independently created ibis-ooxml synthetic fixtures. All OIDs and
names are synthetic.

The pinned crate uses Apache-2.0 OR MIT licensing. Its own
[security warning](https://docs.rs/crate/certval/0.2.0-rc.3/source/README.md)
says it has not been independently audited and documents remaining corpus
mismatches, including CRL cases. A path success here says nothing about
revocation, trusted-list authenticity, document signature validity, timestamp,
coverage, qualified status, FPKI, eIDAS or Japanese PKI compliance. The Wasm
command checks compilation only. These few synthetic cases are not a standards
or security qualification and do not establish production readiness.

Before connecting a path result to a document signature, a separate design and
broader tests must cover explicit anchor selection and provenance, path-building
limits, policy inputs and mappings, signer key usage and purpose, historical
time and trusted timestamps, offline revocation and freshness, unsupported
algorithms and critical extensions, resource limits, and distinct indeterminate
versus rejection results. Any platform and trust-community decisions must be
made explicitly; no default trust stores or PKI membership are inferred here.
