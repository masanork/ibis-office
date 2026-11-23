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
