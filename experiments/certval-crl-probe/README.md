# Offline CRL backend experiment

This isolated experiment evaluates `certval` 0.2.0-rc.3 against synthetic, caller-provided DER certificates and CRLs. It is not linked to ibisOffice or its signature inspector and produces **no revocation or trust decision**. The separate path probe remains independent of this crate. This is original experimental code; it does not connect to pverify.

Only an explicitly supplied anchor, one directly issued end certificate, an optional supplied CRL, and an explicit nonzero Unix time are used. Path validation is reported separately and must succeed before the local CRL function is attempted. Disabled checking is reported as `NotRequested`, not success; a byte-identical anchor and target are rejected. No intermediate, fetched data, system trust store, OCSP response, or network access is configured. Only the `rsa` and `revocation` features are enabled; `remote` and `std` are not enabled on certval. Input sizes are bounded. Inputs are untrusted; the limits do not establish parser security.

`EngineOk`, `EngineRevoked`, `EngineUndetermined`, and `EngineError` are **raw backend observations**, not conclusions about the document or certificate. In particular, `EngineOk` does not prove that an applicable, sufficient CRL was evaluated: the backend has bypasses such as OCSP no-check for responder certificates, and this experiment has not independently qualified CRL scope, issuer authorization, indirect/delta CRLs, all extension semantics, freshness policy, historical validation, or full path revocation. Trust anchor acceptance is an input, not an assessment of trust. There is no FPKI, eIDAS, or Japanese PKI qualification here.

The backend's own [documentation](https://docs.rs/crate/certval/0.2.0-rc.3/source/README.md) identifies CRL profile gaps including missing CRLNumber, critical CRLNumber, and duplicate serials. Our synthetic fixtures reproduce `EngineOk` for all three. The critical-CRLNumber case is rejected by the tested LibreSSL verifier. [RFC 5280 section 5.2.3](https://www.rfc-editor.org/rfc/rfc5280.html#section-5.2.3) specifies a non-critical CRL number extension for conforming CRL issuers. A successful backend return must therefore never be promoted to an application-level good status. These cases are regression observations of an unsuitable backend boundary, not endorsement of acceptance.

## Reproduce

```sh
cargo test --offline --locked --manifest-path experiments/certval-crl-probe/Cargo.toml
python3 experiments/certval-crl-probe/verify_reference.py
cargo tree --offline --locked --manifest-path experiments/certval-crl-probe/Cargo.toml -e features
```

The reference script uses an installed `openssl` CLI, a temporary directory, and the fixed test time 2026-12-01 UTC. It was checked with LibreSSL 3.3.6; messages and behavior can differ with another implementation. It checks ordinary empty and revoked CRLs, bad signature, validity bounds, and unknown/critical extensions. It does not establish full conformance. Tests separately cover missing CRL and malformed data, explicit no-check and failed-path gates, and the three known backend gaps.

Fixtures are synthetic RSA-2048 test certificates generated using Python `cryptography`; private keys were discarded. Root and leaf are valid 2026-01-01 through 2029-01-01. CRLs are signed by the test root except `wrong-key.der`; the target serial is 301, and the duplicate-entry fixture repeats unrelated serial 302. The ordinary CRL validity is 2026-01-01 through 2027-01-01; expiry and future fixtures use their named intervals. These inputs have no production authority.
