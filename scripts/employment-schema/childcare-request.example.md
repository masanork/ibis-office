---
type: presentation-request
title: 保育利用申込での就労証明の提示
x-ibis-presentation-request:
  version: 1
  credential: employment.md
  purpose: 保育利用申込における就労状況の確認
  intent_to_retain: false
  claims:
    - certification_date
    - work_schedule.total
---

# 試作で要求する項目

証明日と、単位・時間・分・うち休憩分を含む就労時間の合計を要求する。この項目構成は変換の検証用。自治体の実際の提出要件は提出先が定める。
