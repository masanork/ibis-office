---
type: help
title: Office 文書・Lawtext・構造化データ
description: DOCX/XLSX/PPTX/一太郎(JTD)/DocuWorks/Visio や Lawtext・JSONL・SBOM の扱い
tags: [help, office, lawtext, formats]
help_version: 1
profile: sorane-okf/0.1
---

# Office 文書・Lawtext・構造化データ

[[help/index|← ヘルプ目次に戻る]]

## 1. Office 互換フォーマット

- **Word / ワープロ (`.docx`, `.odt`, `.rtf`, `.wpd`)**: 本文、表、図形、脚注、コメント、変更履歴の表示・編集と PDF エクスポートに対応しています。
- **Excel / 表計算 (`.xlsx`, `.ods`, `.csv`, `.tsv`)**: 複数シート、数式再計算、グラフ描画、ピボット・条件付き書式に対応しています。
- **PowerPoint / プレゼン (`.pptx`, `.odp`)**: スライド編集、アニメーション、画面切り替え、スライドショー再生に対応しています。
- **一太郎 (`.jtd`)**: 縦書き条文や罫線表を含む一太郎文書を端末内で閲覧し、`.docx` や HTML へ変換できます。
- **DocuWorks (`.xdw`) / Visio (`.vsdx`)**: ページの閲覧や図面表示に対応しています。

## 2. Lawtext / e-Gov 法令 XML (`.lawtext`, `.xml`)

法令・規程・条例のドラフトを記述する **Lawtext** 形式と **e-Gov 法令標準 XML** の双方向 Split 編集に対応しています。

- 条・項・号の構造化プレビュー
- ライブ法制執務 Lint（参照条文や表記のチェック）

## 3. 構造化データビューア

- **AI 評価データ (`.jsonl`)**: プロンプト・出力・Eval スコア・Pass/Fail を構造化テーブルとして検証できます。
- **CycloneDX SBOM (`.cdx.json`)**: ソフトウェア部品表（SBOM）を表計算形式で閲覧できます。
- **大規模・繰り返し列 CSV**: 自治体標準連携などの多列 CSV を構造化ツリーで展開・検索できます。
