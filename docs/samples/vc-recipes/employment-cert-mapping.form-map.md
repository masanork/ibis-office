# 様式対応表：就労証明書（標準様式 XLSX / PDF AcroForm）

```yaml
form_id: employment-certificate-ja-v1
target_format: xlsx
fields:
  - target: "就労証明書!C4"
    claim: "employer.name"
    label: "事業所名"
  - target: "就労証明書!C8"
    claim: "employee.name"
    label: "本人氏名"
  - target: "就労証明書!C9"
    claim: "employee.birth_date"
    label: "生年月日"
    format: "era_or_iso_date"
  - target: "就労証明書!C12"
    claim: "employment.status"
    label: "就労状況"
  - target: "就労証明書!C13"
    claim: "employment.start_date"
    label: "採用年月日"
    format: "era_or_iso_date"
```
