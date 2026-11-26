---
ibis-workbook: true
---

<!-- ibis-sheet: Raw -->

# Raw

| record_id | source_ts | payload_json |
| --- | --- | --- |
| demo-001 | 2026-11-26T10:00:00Z | {"answers":{"name":"山田 太郎","party_size":3,"shelter":"第一小","needs":"アレルギー対応"},"submitted_at":"2026-11-26T10:00:00Z"} |
| demo-002 | 2026-11-26T10:05:00Z | {"answers":{"name":"佐藤 花子","party_size":2,"shelter":"第一小","needs":""},"submitted_at":"2026-11-26T10:05:00Z"} |
| demo-003 | 2026-11-26T10:12:00Z | {"answers":{"name":"髙橋 一郎","party_size":1,"shelter":"第二中","needs":"常備薬"},"submitted_at":"2026-11-26T10:12:00Z"} |

<!-- ibis-sheet: 一覧 -->

# 一覧

| 記録ID | 氏名 | 世帯人数 | 拠点 | 要配慮事項 | 提出時刻 |
| --- | --- | ---: | --- | --- | --- |
| record_id | answers.name | answers.party_size | answers.shelter | answers.needs | submitted_at |
| =FILTER(Raw!A2:A1000,Raw!A2:A1000<>"") | =SORANE.JSONTABLE(FILTER(Raw!C2:C1000,Raw!C2:C1000<>""),B2:F2) | | | | |

<!-- ibis-sheet: 突合・抽出 -->

# 突合・抽出

| 対象氏名 | 基準氏名 | 判定 |
| --- | --- | ---: |
| =FILTER(一覧!B3:B1000,一覧!B3:B1000<>"") | 高橋 一郎 | =SORANE.FIELDMATCH(A2:A4,B2,"name") |

<!-- ibis-sheet: 集計 -->

# 集計

| 拠点 | 件数 | 合計人数 |
| --- | ---: | ---: |
| =UNIQUE(FILTER(一覧!D3:D1000,一覧!D3:D1000<>"")) | =COUNTIF(一覧!D3:D1000,A2#) | =SUMIF(一覧!D3:D1000,A2#,一覧!C3:C1000) |
