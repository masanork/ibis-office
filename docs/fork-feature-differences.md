# soraneDocs と upstream ibisOffice（main）の差分

2026-11-25 時点。比較対象は upstream [`ibisjp/ibis-office` (`main`)](https://github.com/ibisjp/ibis-office)（[`a82e96c3`](https://github.com/ibisjp/ibis-office/commit/a82e96c3)）と [`sorane-dev/docs` (`sorane`)](https://github.com/sorane-dev/docs)（[`b20a603`](https://github.com/sorane-dev/docs/commit/b20a603) 時点）。

## upstream 統合後の継承状況

`sorane` ブランチは `04907ce`（`origin/main` マージ）以降も upstream の修正を取り込みつつ、`sorane-dev/docs` 側の拡張を統合している。

### 機械可読性チェック（Harunobu 0.2.3 相当）

| 項目 | upstream main | soraneDocs (`sorane`) |
| --- | --- | --- |
| 表計算の「校閲 → デジタル庁: 機械可読性チェック…」 | 標準パネルで複数表、見出し、数値文字列、結合セル、非表示行列、数式などを検出。リアルタイム再検査と、行列の挿入・削除に追従するセル移動に対応。 | upstream と同じパネルを開く（[machine-readability.md](machine-readability.md)）。 |

### 重複実装の確認・整理

表計算の機械可読性チェックは upstream のパネルに統一し、fork 独自のファイル検査・JSON/CSV 出力は廃止済み。また、旧 R2 動的フォントサブセット配信も `631403f` で廃止し、upstream 由来の `tools/ibis-font-subsetter` による静的フォントチャンク配信へ統一している。

| 比較した機能 | soraneDocs 側で引き続き維持・拡張している処理 |
| --- | --- |
| HTML 書き出し | upstream の表計算メニューは現在のシートを出力する。Web 用処理は可視シートをまとめて出力し、スライドも構造化 HTML に変換する（[browser-html-export.md](browser-html-export.md)）。 |
| Markdown・Mermaid・PlantUML | 複数形式での Markdown 入出力、ブロック単位 `SourceMap` 双方向同期、Mermaid 原文保持、および Mermaid IR を共有した PlantUML 18 図種のオフライン SVG 描画を扱う。 |
| ODF 読み込み | 入れ子になった領域の読み込み、繰り返し列と非表示行列の処理を追加している。 |

### soraneDocs 側の主な機能差分

| 領域 | soraneDocs 側の追加・拡張 | 確認箇所 |
| --- | --- | --- |
| ナレッジワークスペース | Obsidian 風デスクトップシェル `SoraneWorkspace`（ペイン分割、タブ固定・履歴、コマンドパレット、デイリーノート、Wiki リンク、OKF v0.3、タグブラウザ、Outgoing Links、Local Graph、Outline、Office インスペクタ、Git ドック）。 | `crates/sorane-workspace`、[設計書](design/desktop-workspace-ui.md) |
| Markdown・Mermaid・PlantUML | CommonMark 0.31.2 全 652 例検証、ブロック単位 `SourceMap` と WYSIWYG 外科的書き戻し、双方向カーソル/スクロール同期。Mermaid 描画資産を共有した PlantUML（UML 全般・Mindmap/WBS/Gantt・C4/JSON/YAML/EBNF/nwdiag・軽量テーマ）のオフライン SVG 描画。 | [Markdown リファレンス](markdown-reference-ja.md)、[Mermaid 互換性](mermaid-compatibility.md)、[PlantUML 変換・対応境界](plantuml-sequence-conversion.md) |
| Visio・UOF・WordPerfect・EPUB | Visio (`.vsdx`) の閲覧・新規作成・編集・ステンシルパレット・Wasm 対応。Unified Office Format (`.uot`) 読み込み、WordPerfect (`.wpd`) + WPG 2 画像読み込み、EPUB 3 ネイティブ読み込み。 | `crates/ibis-vsdx`、`crates/ibis-formats/src/{uot,wpd,epub}.rs` |
| 一太郎・DocuWorks・Hancom | 一太郎（`.jtd`/`.jtt`/`.jttc`）の読み取り専用プレビュー・DOCX/HTML 出力、DocuWorks（`.xdw`/`.xbd`）閲覧、Hancom（`.hwp`/`.hwpx`）の DOCX 変換読み込み。 | [JTD プレビュー](jtd-preview.md)、[互換性](jtd-layout-compatibility.md)、[Hancom 読み込み](hangul-import.md) |
| BI・外部データ・分析 | Power Query M 言語インタプリタ（`crates/ibis-m-lang`）、Turso/SQLite による CSV・Parquet・Arrow IPC・JSON Lines・Avro・SQLite の結合クエリ・Excel テーブル書き出し・一括更新・スキーマ変更検知・品質チェックゲート。コホート分析、統計パネル、予測シート、ソルバー、文字コード保持 CSV 上書き保存。 | `crates/ibis-m-lang`、`crates/ibis-doc-service/src/tools_sqlite.rs`、[CSV 保存](design/csv-in-place-encoding.md) |
| 名簿・日本語データ | `FIELDMATCH` 系の氏名・異体字・読み・住所・日付・識別番号等の突合、レコード単位の評価と候補強調、日付レビュー、異体字辞書、行政 PUP/外字確認と Unicode/IVS 正規化。 | [サンプル](samples/field-match.xlsx)、[異体字調査](design/variant-review-upstream-assessment.md) |
| 制作素材画像・暗号化 | HEIC、DPX、Cineon、OpenEXR、JPEG 2000 プレビュー、ODF/OOXML 拡張。 | [画像プレビュー](production-image-preview-ja.md) |
| 署名・資格情報・サプライチェーン | 文書電子署名検査、画像 C2PA 検査、表計算からの SD-JWT・mDOC・OID4VP 作成・検査・提示、CycloneDX SBOM/CBOM・OSV-Scanner レビュー。 | [文書署名](signature-inspection.md)、[C2PA](c2pa-verification.md)、[SD-JWT 計画](xlsx-sd-jwt-plan.md) |
| 法令・医療 | e-Gov 法令 XML／Lawtext 双方向変換・新旧対照表。UKE/DPC、FHIR/CDA/HL7 v2、非診断用 DICOM（Non-SaMD 範囲）の取り込み・閲覧。 | [法令五点セット](lawtext-five-set-ja.md)、[医療拡張](design/medical-domain-extension.md) |
| AI アシスタント・MCP 拡張 | デスクトップ OpenAI 互換 API 連携の Assistant サイドバー。`ibis-mcp` / `ibis-doc-service` での SQLite/データセット・ピボット・チャート・プレフィル・ウォレット MCP ツール群とライブ Assistant 再接続ブリッジ。 | [アシスタント設計](ai-assistant-design.md)、[MCP ガイド](mcp-ja.md) |
| Web・PWA・配信 | `docs.sorane.dev` / `ibis.sorane.dev` での PWA オフラインローカルファイル対応、GitHub App リポジトリ（Vault）ログイン・作成・編集、HTML ダウンロード、Wasm Brotli 単一/分割配信、静的日本語フォントサブセット。 | [PWA](pwa-ja.md)、[HTML 出力](browser-html-export.md)、[埋め込み](web-embedding-ja.md)、[Wasm 配信](web-wasm-delivery.md) |
| デスクトップ・モバイル・CI | `soraneDocs` ブランディング、Android（`gpui-android`、Play 内部トラック自動配信）/ iOS（`gpui-ios`）移植、CycloneDX 1.6 SBOM/CBOM 生成、Linux AppImage の GitHub Artifact Attestations 来歴署名。 | [運用](fork-workflow.md)、[デスクトップ更新](sorane-desktop-updates.md)、[サプライチェーン検証](supply-chain-fit-gap.md) |

## 確認方法

upstream を取り込んだら、比較 SHA、導線の欠落、CI 結果を確認して本ドキュメントを更新する。

```sh
git fetch origin
git fetch upstream
git rev-parse upstream/main origin/sorane
git rev-list --left-right --count upstream/main...origin/sorane
git log --first-parent --oneline upstream/main..origin/sorane
git diff --stat upstream/main origin/sorane
```

統合前のコードからのファイル移動・削除は `git diff --name-status -M <統合前のSHA> origin/sorane` で確認する。両側に同名の機能がある場合は、メニュー登録と実行先、ファイルの開く・保存する経路、対象テストまで照合する。
