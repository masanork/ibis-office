# CycloneDX cryptographic inventory demo

This synthetic CycloneDX 1.6 JSON describes cryptographic metadata for a fictional payment service.

1. Open `payment-demo.cdx.json` in the desktop app. Inspect **Cryptographic assets** for algorithms, the certificate, TLS, and the key metadata.
2. In **Crypto relationships**, compare the service's `dependsOn` references with the provider's `provides` references. These are BOM declarations.
3. Save the workbook as XLSX and reopen it. The **Input artifact** sheet records the source filename and SHA-256.

Alternatively, run:

```sh
ibisOffice --export sbom-table payment-demo.cdx.json --out-file payment-crypto-inventory.xlsx
```

The related-material `value` field is excluded from the workbook. The sample illustrates inventory review and does not establish runtime use or PQC readiness.

## Compare two releases

In the desktop app, open `payment-before.cdx.json`. Choose **File → Compare with another CBOM…**, select `payment-after.cdx.json`, and save the new workbook as `payment-crypto-changes.xlsx`. The first file stays open. If it has changed on disk, reopen it before comparing.

The same comparison is available from the command line.

```sh
ibisOffice --export cbom-compare \
  --old-sbom payment-before.cdx.json --new-sbom payment-after.cdx.json \
  --out-file payment-crypto-changes.xlsx
```

Open the XLSX in ibisOffice. **Asset changes** shows RSA-2048 and its related key record removed, and ML-DSA-65 added. **Changed asset fields** shows the certificate declaration before and after. **Relationship changes** shows the certificate dependency and provider implementation changing from RSA-2048 to ML-DSA-65. The payment service dependencies on TLS and the certificate are unchanged.

Keep `bom-ref` values stable for the same asset or endpoint across releases. An asset with a missing reference is listed as unmatched. Changes to edge targets appear as removal and addition. **Input artifacts** records the two filenames and SHA-256 values. This fixture describes fictional BOM declarations for an inventory comparison.

**Dependency impact** traces declared `dependsOn` edges back to dependent components and services. The payment service reaches RSA-2048 through its certificate in the old snapshot and ML-DSA-65 through that certificate in the new snapshot. Each row shows one shortest path in that snapshot. **Asset providers** lists the crypto library’s direct `provides` declarations separately.

**System impact summary** groups the affected applications and services by BOM reference. For the sample, `payments` has one row with the changed certificate and RSA-2048 on the old side, and the changed certificate and ML-DSA-65 on the new side. **Dependency impact** retains the individual paths.
