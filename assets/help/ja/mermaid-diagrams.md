---
type: help
title: Mermaid 図表ガイド
description: Markdown 内にフローチャート・シーケンス図・円グラフ・XYチャート等を埋め込む方法
tags: [help, mermaid, diagram, chart]
help_version: 1
profile: sorane-okf/0.1
---

# Mermaid 図表ガイド

[[help/index|← ヘルプ目次に戻る]]

コードブロックの言語に `mermaid` を指定すると、図表やグラフが直接レンダリングされます。Office 文書（DOCX / PPTX）への変換時にはネイティブ図形やグラフとしても出力できます。

## 1. フローチャート (`flowchart LR` / `TD`)

```mermaid
flowchart LR
  Input["Markdown / Office 文書"] --> Engine["ibisOffice エンジン"]
  Engine --> Split["双方向 Split 編集"]
  Engine --> Export["PDF / DOCX / XLSX / PPTX"]
```

## 2. シーケンス図 (`sequenceDiagram`)

```mermaid
sequenceDiagram
  participant User as ユーザー
  participant Editor as soraneDocs
  participant GitHub as GitHub Vault
  User->>Editor: ファイルを編集・保存 (⌘S)
  Editor->>GitHub: コミットを作成
  GitHub-->>User: 変更履歴を保存完了
```

## 3. 円グラフ (`pie`) と XY チャート (`xychart-beta`)

```mermaid
pie title ファイル形式の利用割合
  "Markdown" : 45
  "Excel (XLSX)" : 25
  "Word / 一太郎" : 20
  "PowerPoint" : 10
```

```mermaid
xychart-beta
  title "月次ドキュメント作成数"
  x-axis [1月, 2月, 3月, 4月]
  y-axis "件数" 0 --> 60
  bar [18, 29, 42, 55]
  line [15, 25, 38, 50]
```

## 4. その他の対応図種

- `stateDiagram-v2`（状態遷移図）
- `classDiagram`（クラス図）
- `erDiagram`（ER 図）
- `gantt`（ガントチャート）

> [!TIP] ラベルに括弧や記号を含める場合
> `id["ラベル (補足)"]` のようにダブルクォートで囲むと構文エラーを防げます。
