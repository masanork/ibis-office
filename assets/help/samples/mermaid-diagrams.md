---
type: note
title: Mermaid 図表サンプル
tags: [help, sample, mermaid]
profile: sorane-okf/0.1
---

# Mermaid 図表サンプル

[[help/samples/index|← サンプル一覧・ウォークスルーに戻る]]

## フローチャート (`flowchart LR`)

```mermaid
flowchart LR
  XML["e-Gov 法令 XML / Lawtext"] --> Engine["ibisOffice エンジン"]
  MD["Markdown / Marp / Workbook"] --> Engine
  Engine --> Split["双方向 Split 編集"]
  Engine --> Export["DOCX / XLSX / PPTX / PDF"]
```

## シーケンス図 (`sequenceDiagram`)

```mermaid
sequenceDiagram
  participant User as 利用者
  participant App as ibisOffice / soraneDocs
  participant Vault as GitHub Vault
  User->>App: サンプルを開く (Split)
  App-->>User: リアルタイムにレンダリング
  User->>Vault: 変更を保存・コミット
```

## 円グラフ (`pie`) と XY チャート (`xychart-beta`)

```mermaid
pie title サンプルカテゴリ
  "法令 (Lawtext / XML)" : 25
  "Markdown / Marp / Workbook" : 40
  "SBOM / JSONL / CSV / XML" : 35
```

```mermaid
xychart-beta
  title "月別処理件数サンプル"
  x-axis [1月, 2月, 3月, 4月]
  y-axis "件数" 0 --> 100
  bar [32, 48, 65, 84]
  line [28, 45, 60, 80]
```
