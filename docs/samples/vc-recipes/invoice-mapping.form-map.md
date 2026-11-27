# 様式対応表：請求書テンプレート（XLSX / PDF AcroForm）

```yaml
form_id: qualified-invoice-v1
target_format: xlsx
fields:
  - target: "請求書!B4"
    claim: "company.name"
    label: "発行事業者名"
  - target: "請求書!B5"
    claim: "invoice.registration_number"
    label: "適格請求書発行事業者登録番号"
  - target: "請求書!F3"
    claim: "invoice.issue_date"
    label: "発行日"
    format: "era_or_iso_date"
  - target: "請求書!F18"
    claim: "invoice.total_amount"
    label: "請求金額（税込）"
```
