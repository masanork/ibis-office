---
ibis-workbook: true
---

<!-- ibis-sheet: Raw -->

# Raw

| record_id | source_ts | payload_json |
| --- | --- | --- |
| demo-001 | 2026-11-26T10:00:00Z | {"answers":{"name":"山田 太郎","birth_date":"1985-04-12","address":"東京都千代田区霞が関1-1-1","shelter":"第一小","needs":"アレルギー対応","input_helper":"","group_id":"grp-a1b2"},"verification":{"method":"self_asserted","trust_status":"unverified"},"submitted_at":"2026-11-26T10:00:00Z"} |
| demo-002 | 2026-11-26T10:02:00Z | {"answers":{"name":"山田 花子","birth_date":"2018-09-03","address":"東京都千代田区霞が関1-1-1","shelter":"第一小","needs":"","input_helper":"山田 太郎","group_id":"grp-a1b2"},"verification":{"method":"self_asserted","trust_status":"unverified"},"submitted_at":"2026-11-26T10:02:00Z"} |
| demo-003 | 2026-11-26T10:12:00Z | {"answers":{"name":"髙橋 一郎","birth_date":"1952-11-20","address":"東京都千代田区永田町2-2-2","shelter":"第二中","needs":"常備薬","input_helper":"","group_id":""},"verification":{"method":"mdoc_dc_api","trust_status":"verified"},"submitted_at":"2026-11-26T10:12:00Z"} |

<!-- ibis-sheet: 一覧 -->

# 一覧

| 記録ID | 氏名 | 生年月日 | 住所 | 拠点 | 要配慮事項 | 入力補助者 | グループID | 本人確認 | 提出時刻 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| record_id | answers.name | answers.birth_date | answers.address | answers.shelter | answers.needs | answers.input_helper | answers.group_id | verification.trust_status | submitted_at |
| =FILTER(Raw!A2:A1000,Raw!A2:A1000<>"") | =SORANE.JSONTABLE(FILTER(Raw!C2:C1000,Raw!C2:C1000<>""),B2:J2) | | | | | | | | |

<!-- ibis-sheet: 突合・抽出 -->

# 突合・抽出

| 対象氏名 | 基準氏名 | 判定 |
| --- | --- | ---: |
| =FILTER(一覧!B3:B1000,一覧!B3:B1000<>"") | 高橋 一郎 | =SORANE.FIELDMATCH(A2:A4,B2,"name") |

<!-- ibis-sheet: 集計 -->

# 集計

| 拠点 | 人数 | 本人確認済 |
| --- | ---: | ---: |
| =UNIQUE(FILTER(一覧!E3:E1000,一覧!E3:E1000<>"")) | =COUNTIF(一覧!E3:E1000,A2#) | =COUNTIFS(一覧!E3:E1000,A2#,一覧!I3:I1000,"verified") |
