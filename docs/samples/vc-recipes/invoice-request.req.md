# 提出先定義：適格請求書（インボイス）確認

- `client_id`: `https://verifier.example.jp/invoice-intake`
- `response_uri`: `https://verifier.example.jp/oid4vp/direct-post`
- `response_mode`: `direct_post`
- `dcql_credential_id`: `qualified_invoice_issuer`
- `requested_claims`:
  - `company.name`
  - `invoice.registration_number`
  - `invoice.issue_date`
  - `invoice.total_amount`
