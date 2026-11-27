---
type: help
title: Marp スライド記法
description: Markdown からスライド資料を作成し、PowerPoint と双方向編集する方法
tags: [help, marp, slides, pptx]
help_version: 1
profile: sorane-okf/0.1
---

# Marp スライド記法

[[help/index|← ヘルプ目次に戻る]]

Markdown の先頭に `marp: true` を記述すると、ファイルが **プレゼンテーション（スライド）** として認識され、右側の Visual ペインでスライド編集・再生・PPTX/PDF 書き出しができるようになります。

## 1. 基本構成

```markdown
---
marp: true
title: プロジェクト報告
theme: default
paginate: true
size: 16:9
---

# タイトルスライド

サブタイトル・発表者名

<!-- スピーカーノートは HTML コメントで記述します -->

---

## 2枚目のスライド

- 箇条書き項目 1
- 箇条書き項目 2

---

## 表や Mermaid 図もそのままスライド化

| 項目 | 状況 |
| --- | --- |
| 設計 | 完了 |
| 実装 | 進行中 |
```

## 2. 主な機能とディレクティブ

| 記法 / 設定 | 説明 |
| --- | --- |
| `marp: true` | Marp プレゼンテーションモードを有効化 |
| `---` | スライドの区切り（水平線） |
| `size: 16:9` / `4:3` | スライドのアスペクト比 |
| `paginate: true` | スライド番号を表示 |
| `header: "..."` / `footer: "..."` | ヘッダー・フッター文字列 |
| `<!-- ノート -->` | 発表者用スピーカーノート |

## 3. PPTX との双方向同期

- Split ビューでは、左の Markdown を編集すると右のスライドが即座に更新されます。
- 右のスライド上でテキストや図形・表・グラフの位置や内容を編集しても、左の Markdown へ同期されます。
- そのまま `.pptx` や `.pdf` として書き出すことも可能です。
