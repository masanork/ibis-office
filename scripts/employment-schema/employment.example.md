---
type: credential-definition
title: 就労証明書の就労データ
description: 会社が証明する就労項目と公式Excel様式の対応
x-ibis-credential:
  version: 1
  approved_mapping: approved.json
  sd_jwt_vc:
    vct: https://credentials.example.org/employment-pilot/v1
    paths:
      certification_date: [certification_date]
      work_schedule.total: [work_schedule, total]
  mdoc:
    doctype: org.example.employment.pilot.1
    paths:
      certification_date: [org.example.employment.pilot.1, certification_date]
      work_schedule.total: [org.example.employment.pilot.1, work_schedule_total]
---

# 証明する範囲

会社が確認する事業所、本人、証明日、雇用形態、就労時間と日数を扱う。変則勤務では、時間と日数の単位を別々に保持する。No.19の保護者記載欄は申請側で扱う。

出典は [こども家庭庁の就労証明書の標準様式](https://www.cfa.go.jp/policies/hoiku/shugyoshomeisho)。
