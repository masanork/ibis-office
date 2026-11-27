---
type: help
title: soraneDocs / ibisOffice ヘルプ目次
description: soraneDocs と ibisOffice の操作方法・Markdown 記法の総合ガイド
tags: [help, index]
help_version: 1
profile: sorane-okf/0.1
---

# soraneDocs / ibisOffice ヘルプ

**soraneDocs / ibisOffice** は、Markdown（ノート・Marp スライド・Workbook 表計算・Lawtext 法令文書）と Office 文書（DOCX / XLSX / PPTX / JTD / XDW / VSDX 等）を、1つのワークスペースで閲覧・双方向 Split 編集できるドキュメント環境です。

メニューの **「ヘルプ」** からいつでも最新の内容を参照できるほか、GitHub Vault / ローカル Vault 内の `help/` フォルダでも同じ内容を検索・参照・実験できます。

## 目次

### 1. 基本操作とショートカット
- [[help/getting-started|基本操作ガイド]] — ファイルの開閉・保存、Source ↔ Visual Split 編集、GitHub Vault 連携、対応ファイル形式
- [[help/shortcuts|キーボードショートカット一覧]] — 編集・表示・ファイル操作・メニュー検索のショートカット

### 2. Markdown と構造化記法
- [[help/markdown-basics|Markdown 基本・拡張記法]] — 見出し、文字装飾、表、タスクリスト、Callout、脚注、数式
- [[help/marp-slides|Marp スライド記法]] — `marp: true` でプレゼンテーションを作成する
- [[help/workbook-markdown|Workbook Markdown 記法]] — `type: workbook` で表計算・数式・複数シートを扱う
- [[help/mermaid-diagrams|Mermaid 図表ガイド]] — フローチャート、シーケンス図、円グラフ、XY チャート等の記述例

### 3. ナレッジ管理・実務文書
- [[help/okf-and-vault|OKF と Vault 活用]] — YAML フロントマター、`#tag`、`[[wikilink]]`、Backlinks、Local Graph、検索
- [[help/office-and-lawtext|Office 文書・Lawtext・構造化データ]] — DOCX / XLSX / PPTX / 一太郎 (JTD) / Lawtext / JSONL / SBOM の扱い

### 4. 実データサンプル・ウォークスルー
- [[help/samples/index|サンプル・レンダリング・ウォークスルー]] — 番号利用法（e-Gov XML / Lawtext）・Marp・Workbook・SBOM・JSONL・CSV 等の実例を並べて確認

> [!TIP] Split 編集で記法をそのまま試す
> エディタ上部の **[Source | Split | Visual]** 切替から **Split** を選ぶと、左側に Markdown ソース、右側にリアルタイムのプレビュー／リッチ編集画面を並べて表示できます。
