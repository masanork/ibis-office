---
type: note
title: OKF 機械可読ナレッジサンプル
tags: [help, sample, okf, obsidian-compatible]
aliases: [Knowledge Note Sample]
status: active
profile: sorane-okf/0.1
---

# OKF 機械可読ナレッジサンプル

[[help/samples/index|← サンプル一覧・ウォークスルーに戻る]]

#sorane #okf #sample

このファイルは、**Open Knowledge Format (OKF)** と Obsidian 互換記法（YAML フロントマター、`#tag`、`[[wikilink]]`、Callout、脚注、数式、表）のレンダリングを確認するためのサンプルです。

> [!TIP] Split ビューでの確認ポイント
> エディタ上部の **[Source | Split | Visual]** で **Split** を選ぶと、左側の Markdown ソースと右側のリッチ文書表示を同時に見比べられます。

## 1. 関連サンプルへの Wiki リンク

- 番号利用法（Lawtext）: [[help/samples/mynumber-act-ch1.lawtext|mynumber-act-ch1.lawtext]]
- Marp スライド: [[help/samples/marp-deck.md|marp-deck.md]]
- Workbook 表計算: [[help/samples/workbook-sheet.md|workbook-sheet.md]]

## 2. タスクリストと表

- [x] YAML フロントマターによるメタデータ定義
- [x] `#tag` と `[[wikilink]]` による知識グラフ構築
- [ ] GitHub Vault へのコミット確認

| 項目 | 記法例 | 用途 |
| --- | --- | --- |
| 太字・ハイライト | **強調** / ==ハイライト== | 重要語句の明示 |
| 数式 | $E = mc^2$ | インライン数式[^1] |
| コード | `soraneDocs` | コマンド・識別子 |

[^1]: 脚注は Word (`.docx`) エクスポート時にも保持されます。
