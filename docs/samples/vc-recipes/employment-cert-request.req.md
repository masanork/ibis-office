# 提出先定義：就労証明書（自治体・保育所提出用サンプル）

- `client_id`: `https://city.example.jp/childcare/employment-cert`
- `response_uri`: `https://city.example.jp/oid4vp/direct-post`
- `response_mode`: `direct_post`
- `dcql_credential_id`: `employment_certificate`
- `requested_claims`:
  - `employer.name`
  - `employee.name`
  - `employee.birth_date`
  - `employment.status`
  - `employment.start_date`
