# Docs（masanork）と upstream ibisOffice（main）の差分

2026-11-21 時点。比較対象は upstream [`origin/main` の `a1079df1`](https://github.com/ibisjp/ibis-office/commit/a1079df1b1336286d242bd7d1cb8ba9bd32be277) と Docs [`masanork/masanork` の `6757a491`](https://github.com/masanork/ibis-office/commit/6757a491adfc07ba0dbf51eca27eb399d40db67e)。fork 側の `masanork/main` は比較時点で upstream より12コミット古いため、以降の `main` は `origin/main` を指す。

## 0.8.9 統合後の継承状況

[`8ed96736`](https://github.com/masanork/ibis-office/commit/8ed96736) は、統合直前の Docs（`2c25a475`）と upstream（`a1079df1`）の両方を親に持つ。確認時点の Docs は両方の子孫で、統合前の機能コミットは履歴に残っている。upstream 側だけにあるコミットは0件、Docs 側だけにあるコミットは825件（第1親で398件）。ソースツリーの差分は972ファイル。

履歴に残っていても、競合解消後のメニューやファイル操作で機能を呼べるかは別途確認が必要。統合前の Docs と現行ツリーの差分では、ODF コード・フィクスチャが `ibis-formats` から `ibis-odf` へ移動し、旧 CMYK シナリオ `cmyk_20261109.rs` は削除されている。現行の CMYK 印刷設定・プルーフには `cmyk_20261114.rs` がある。下表は第1親の統合履歴と現行ツリーをもとにした機能群の一覧。各機能の操作を一通り再実行した段階ではない。

### 機械可読性チェック

| 項目 | upstream main | Docs（masanork） |
| --- | --- | --- |
| 表計算の「ツール → 機械可読性チェック」 | ibis 標準パネル。複数表、見出し、数値文字列、結合セル、非表示行列、数式などを検出。リアルタイム再検査、ルール・参考ツールへのリンク、行列の挿入・削除に追従するセル移動を備える。 | Docs の Harunobu レビューパネルを開く。編集中のブックから結合セル・非表示行列・数式・文字種（私用文字を含む）の候補を表示し、手動で再検査できる。 |
| Harunobu 互換のファイル検査 | 標準パネルに Harunobu への参照リンクがある。 | デスクトップから Harunobu 0.2.2 を基にした Rust 実装を呼ぶ。XLSX/XLSM、保存済み CSV/TSV、L1・L1–L2・L1–L3、複数ファイルの L3-07、JSON/CSV 出力に対応。条件を満たす編集中の XLSX/XLSM は一時スナップショットで検査する。Web はブック内の候補表示まで。 |

**確認できた導線の欠落**。Docs 側にも upstream の `machine_readability_panel.rs`、`machine_readability.rs`、`tests/k0055_machine_readability.rs` が入っている。一方、`toolbar.rs` のメニュー登録と再接続はどちらも `harunobu_review.rs` の `open_machine_readability` に接続されている。`open_machine_readability_panel` に呼び出し元がなく、ibis 標準パネル固有の機能は通常メニューから利用できない。Docs 側のファイル検査は、レビュー画面から `harunobu.rs` を経て `ibis-formats` の Rust 検査へつながっている。Harunobu との比較範囲や制約は [機械可読性チェック](machine-readability.md) を参照。

### Docs 側の主な機能差分

| 領域 | Docs 側の追加・拡張 | 確認箇所 |
| --- | --- | --- |
| 一太郎 | JTD の読み取り専用プレビュー、論理レコード・縦書き・罫線・ルビ・画像・サムネイル・目次の復元、HTML/DOCX プレビュー出力、JTT/JTTC の取り込み。 | [JTD プレビュー](jtd-preview.md)、[互換性](jtd-layout-compatibility.md) |
| Markdown・Mermaid | 文書・スライド・表計算の Markdown 入出力、Markdown ソース編集、Mermaid の純 Rust 描画と OOXML 内の原文保持、数式・表・脚注・引用・ページ情報のラウンドトリップ。 | [Markdown リファレンス](markdown-reference-ja.md)、[Mermaid 互換性](mermaid-compatibility.md) |
| 表計算のデータ取り込み | ローカルの Parquet、JSON Lines、Arrow IPC、Avro、SQLite をクエリソースとして取り込む。Parquet の列選択・複数ファイル追記・Zstandard、CSV の文字コードを考慮した上書き保存を含む。 | `fc3597f1`、`6a0ec663`、`6f55caf5`、`dac772ff`、`42a31603`、`006bff04`、`b4fd8a57`、`9b43789d`、[CSV 保存](design/csv-in-place-encoding.md) |
| 名簿・日本語データ | FIELDMATCH 系の氏名・異体字・読み・住所・日付・識別番号等の突合、レコード単位の評価と候補の強調。日付レビュー、異体字辞書、行政 PUP・外字の確認と出力時の Unicode/IVS 正規化。 | `3f37f013`、`f574efdf`、`cd26c32d`、`894facba`、`7296b7eb`、[サンプル](samples/field-match.xlsx) |
| 文書形式・画像 | XDW、OFD、XPS/OXPS、PDF の閲覧・検査経路の拡張、ODF の入れ子・表・名前付き範囲の読み込み改善、Office 2007 Standard 暗号化の復号。HEIC、DPX、Cineon、OpenEXR、JPEG 2000 のプレビュー拡張。 | `2446ffe7`、`07748ac1`、`ac171716`、`88827c70`、`c635d645`、`dc0979d7`、[画像プレビュー](production-image-preview-ja.md) |
| 署名・来歴・デジタル資格情報 | 文書署名の独立検査、画像 C2PA のブラウザ・デスクトップでの検査、表計算からの SD-JWT・mDOC・OID4VP の作成・検査・提示支援。 | [文書署名](signature-inspection.md)、`e7bac33e`、`fd760e69`、[SD-JWT 計画と実装](xlsx-sd-jwt-plan.md) |
| 法令・医療 | e-Gov 法令 XML／Lawtext、改め文・新旧対照表等の支援。UKE/DPC、FHIR/CDA/HL7 v2、DICOM の取り込み・閲覧と関連ライブラリ。ROI マスクと PII 事前確認はライブラリの範囲。 | `abb0341d`、`8a06990d`、`83cedb3c`、[医療拡張](design/medical-domain-extension.md) |
| AI アシスタント | DOCX・XLSX・PPTX 共通の編集支援、編集提案と適用、デスクトップの OpenAI 互換 API と Web の Workers AI 経路。 | [アシスタント設計](ai-assistant-design.md)、`750ed99e`、`a18ca47e` |
| Web・配信 | Docs のホーム・合成サンプル・名簿照合デモ、サンプル管理、文書リポジトリ、HTML ダウンロード、埋め込み配布、Wasm の分割・検証付き読み込み、日本語フォントの動的サブセット。 | `ea145f20`、`ebd66919`、[HTML 出力](browser-html-export.md)、[埋め込み](web-embedding-ja.md)、[Wasm 配信](web-wasm-delivery.md)、[Web fonts](web-fonts.md) |
| デスクトップ・モバイル | Docs のブランドとアプリ識別子、デスクトップ CI とタグ付きビルド、署名付き更新メタデータの準備。Android/iOS 向け GPUI、タッチ操作、Android スマートフォン表示とサーフェス復帰の修正。 | [運用](fork-workflow.md)、[デスクトップ更新](sorane-desktop-updates.md)、`75ace0fc`、`514065b2`、`a65edffd`、`9dbdd1bd`、`2fe7ac1a` |

### 確認時点の CI

- [Desktop build CI](https://github.com/masanork/ibis-office/actions/runs/44779300538)、[Mermaid compatibility](https://github.com/masanork/ibis-office/actions/runs/44779300543)、[Lawtext compatibility](https://github.com/masanork/ibis-office/actions/runs/44779300544)、[Sorane update verifier](https://github.com/masanork/ibis-office/actions/runs/44779300430) は成功。
- [Harunobu Windows setup](https://github.com/masanork/ibis-office/actions/runs/44779300512) は成功。このジョブは Python 参照実装のセットアップと検査を検証する。Docs の Rust 検査の GUI 操作や全ファイルでの判定一致は今回未検証。
- [Web font tests](https://github.com/masanork/ibis-office/actions/runs/44779300723) は失敗。Chrome の HTML ダウンロードで編集結果のエスケープを確認するアサーションが失敗し、フォントの repertoire テストは `Note` の初期化で `custom_mark` が欠けてコンパイルに失敗。タッチのブラウザ試験とポータブルジェスチャーのジョブは成功。

## 更新方法

upstream を取り込んだら、比較 SHA、導線の欠落、検証結果を更新する。統合前の Docs の SHA は最新 upstream マージの第1親から取得する。

```sh
git fetch origin
git fetch masanork
git rev-parse origin/main masanork/masanork
git rev-list --left-right --count origin/main...masanork/masanork
git log --first-parent --oneline origin/main..masanork/masanork
git diff --stat origin/main masanork/masanork
```

統合前の Docs からのファイル移動・削除は `git diff --name-status -M <統合前のDocsのSHA> masanork/masanork` で確認する。両側に同名の機能がある場合は、メニュー登録と実行先、ファイルの開く・保存する経路、対象テストまで照合する。
