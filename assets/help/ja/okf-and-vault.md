---
type: help
title: OKF と Vault 活用
description: YAML フロントマター、#tag、[[wikilink]]、Backlinks、Local Graph、検索の使い方
tags: [help, okf, vault, obsidian]
help_version: 1
profile: sorane-okf/0.1
---

# OKF と Vault 活用

[[help/index|← ヘルプ目次に戻る]]

soraneDocs は **Open Knowledge Format (OKF)** と Obsidian 互換のナレッジ管理記法を採用しており、人間が読みやすい Markdown と、検索・AI・グラフ解析が扱いやすい構造化データを両立します。

## 1. YAML フロントマター

ファイルの先頭に `---` で囲んだメタデータを記述します。

```yaml
---
type: note
title: プロジェクト定例メモ
tags: [meeting, project-a]
aliases: [定例メモ]
status: active
profile: sorane-okf/0.1
---
```

## 2. タグ (`#tag`)

本文中に `#meeting` や `#project/phase1` のように記述すると、Vault 内のタグ一覧から関連ノートを即座に絞り込めます（見出し行の `# 見出し` とは自動で区別されます）。

## 3. Wiki リンク (`[[wikilink]]`)

Vault 内の別ファイルを `[[ファイル名]]` または `[[フォルダ/ファイル名|表示名]]` でリンクできます。

- リンクをクリックすると対象ノートへジャンプします。
- 相手のノート側の **Backlinks（被リンク）** パネルに、どこから参照されているかが自動表示されます。
- **Local Graph** でノート同士のつながりを視覚的にたどれます。

## 4. 検索・クイックスイッチャー

- **Vault 内全文検索**: 左サイドバーの検索パネルからすべての `.md` / `.lawtext` 等を横断検索できます。
- **クイックスイッチャー**: ファイル名・エイリアス・`#tag` から素早くノートを開けます。

## 5. 既存 Vault への最新ヘルプ転記

すでに作成済みの GitHub Vault やローカル Vault にも最新の `help/` を取り込みたい場合は、**GitHub リポジトリ設定画面（またはワークスペース）の「最新ヘルプを Vault に転記・更新」** を実行してください。既存の他フォルダや別名のメモには触れず、`help/` 配下を現在の UI 言語の最新版に更新します。
