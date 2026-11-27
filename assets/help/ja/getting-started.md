---
type: help
title: 基本操作ガイド
description: ibisOffice / soraneDocs の起動、編集、保存、GitHub Vault 連携の基本
tags: [help, getting-started]
help_version: 1
profile: sorane-okf/0.1
---

# 基本操作ガイド

[[help/index|← ヘルプ目次に戻る]]

## 1. 文書を開く・新規作成する

- **新規作成**: メニューの **ファイル > 新規作成** またはスタート画面のタイルから、文書（DOCX）・表計算（XLSX）・プレゼン（PPTX）・Markdown（`.md`）を作成できます。
- **ファイルを開く**: **ファイル > 開く**（`⌘O` / `Ctrl+O`）またはスタート画面のリスト・ドラッグ＆ドロップで開きます。
- **Vault から開く**: 左サイドバーのファイルツリー、またはクイックスイッチャーから Vault 内のファイルを選択します。

## 2. Source ↔ Visual 双方向 Split 編集

Markdown（`.md` / `.markdown`）、Marp スライド、Workbook Markdown、Lawtext（`.lawtext`）を開くと、エディタ上部に **[Source | Split | Visual]** の切替が表示されます。

| モード | 用途 |
| --- | --- |
| **Source** | テキストソースを集中して編集 |
| **Split** | 左でソース編集、右でレイアウト／スライド／表計算をリアルタイム同期 |
| **Visual** | 完成イメージを確認しながら直接編集 |

## 3. 保存とエクスポート

- **上書き保存 (`⌘S` / `Ctrl+S`)**: ローカルファイルや GitHub Vault から開いた文書を保存します。GitHub Vault 上のファイルを保存すると、確認ダイアログの後にリポジトリへコミットされます。
- **名前を付けて保存 (`⇧⌘S` / `Ctrl+Shift+S`)**: 別名や別フォルダに保存します（Web 版ではダウンロード）。
- **エクスポート**: **ファイル > エクスポート** から PDF / PNG / GIF / HTML などに書き出せます。

## 4. GitHub Vault 連携（Web / デスクトップ）

1. スタート画面または設定から **GitHub でログイン** します。
2. 新しい **Private リポジトリ（既定: `soraneDocs`）** を作成するか、既存のリポジトリを選択します。
3. リポジトリが接続されると、ファイルツリー・全文検索・タグ・Backlinks・Local Graph が利用できます。
4. 既存の Vault に最新のヘルプを取り込みたい場合は、GitHub リポジトリ設定画面の **「最新ヘルプを Vault に転記・更新」**（またはメニューの同期機能）から `help/` フォルダを更新できます。

## 5. 対応している主なファイル形式

| カテゴリ | 拡張子 | 主な機能 |
| --- | --- | --- |
| Markdown / ナレッジ | `.md`, `.markdown` | ノート、Marp スライド、Workbook 表計算、OKF |
| ワープロ文書 | `.docx`, `.odt`, `.rtf`, `.jtd`, `.wpd` | 編集・表示・PDF 書き出し（JTD は閲覧・DOCX 変換） |
| 表計算 | `.xlsx`, `.ods`, `.csv`, `.tsv` | 数式再計算、グラフ、294 列等の構造化 CSV ビュー |
| プレゼン・図面 | `.pptx`, `.odp`, `.vsdx` | スライド編集、アニメーション、Visio 図面表示 |
| 法令・構造化データ | `.lawtext`, `.xml`, `.jsonl`, `.json` | 法令 XML 双方向編集・Lint、AI Eval / SBOM ビューア |
