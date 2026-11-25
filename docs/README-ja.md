<p align="center"><a href="../README.md">English</a> | <b>日本語</b></p>

<p align="center">
  <img src="../crates/ibis-office-app/branding/sorane-docs.svg" alt="soraneDocs" width="84">
</p>

<h1 align="center">soraneDocs</h1>

<p align="center">
  <b>Rust と GPUI で作られた <a href="https://github.com/ibisjp/ibis-office">ibisOffice</a> の機能拡張フォーク</b><br>
  デスクトップ・Web（Wasm/WebGPU）・モバイルで動く、Office 文書・Markdown ナレッジワークスペース・図表・行政/実務向け拡張環境。
</p>

<p align="center">
  <a href="https://docs.sorane.dev">Web 版を試す (docs.sorane.dev)</a> ·
  <a href="fork-feature-differences.md">upstream との機能差分</a> ·
  <a href="fork-workflow.md">フォーク運用</a>
</p>

---

<!-- TODO: 追って soraneDocs の画面ショットを追加 -->

## soraneDocs とは

**soraneDocs** は、[ibisOffice](https://github.com/ibisjp/ibis-office) をベースに `sorane-dev/docs` で開発・運用しているフォークです。upstream の Rust + GPUI エンジン、高精度な OOXML（`.docx`・`.xlsx`・`.pptx`）描画、MCP 対応などを引き継ぎつつ、次の 2 点を目的としています。

1. **プロトタイピングと upstream への貢献**：バグ修正、日本語組版の改善、フォーマット拡張などを検証可能な PoC コミットとして実装し、Issue を通じて `ibisjp/ibis-office` へ提案・還元すること。
2. **拡張ドキュメント／ナレッジワークスペース**：Markdown 中心のナレッジ管理、図表描画、日本の行政・法令・医療向け拡張、PWA / GitHub Vault 連携などを統合した環境を提供すること。

ibisOffice 本来の機能一覧、ベンチマーク、商用ライセンスについては [upstream リポジトリ](https://github.com/ibisjp/ibis-office) をご覧ください。

## これまでの ibisOffice への貢献

upstream `ibisjp/ibis-office` はコラボレーター以外の Pull Request を受け付けていないため、このフォークでの PoC コミットを添えて GitHub Issue から提案・報告を行い、以下の機能・修正が upstream に採用・統合されています。

- **日本語組版・フォント描画**
  - 行政事務標準文字と異体字シーケンス（IVS）の表示・編集対応、macOS での追加字形欠落修正（[#158](https://github.com/ibisjp/ibis-office/issues/158), [#251](https://github.com/ibisjp/ibis-office/issues/251)）
  - 非 Windows 環境における「ＭＳ 明朝」フォント代替ルールの修正（[#248](https://github.com/ibisjp/ibis-office/issues/248)）
  - 代替フォント・Web 版における和文記号・仮名文字送りと GPOS 配置オフセットの改善（[#230](https://github.com/ibisjp/ibis-office/issues/230)）
  - DOCX 縦書きの傍線位置、縦書き表のセル文字方向と新規表の縦結合保持、両端揃え箇条書きのはみ出し修正（[#160](https://github.com/ibisjp/ibis-office/issues/160), [#222](https://github.com/ibisjp/ibis-office/issues/222), [#193](https://github.com/ibisjp/ibis-office/issues/193)）
- **文書形式・互換性・シビックテック**
  - Pure-Rust による PDF・XPS/OpenXPS・OFD の閲覧および署名検査対応（[#235](https://github.com/ibisjp/ibis-office/issues/235)）
  - Markdown ネイティブ対応強化（往復変換・クリップボード・CLI・pure-Rust Mermaid 描画）（[#226](https://github.com/ibisjp/ibis-office/issues/226)）
  - Office 2007 Standard 暗号化ファイル（DOCX/XLSX/PPTX）の復号対応（[#190](https://github.com/ibisjp/ibis-office/issues/190)）
  - DOCX の段落 ID（`w14:paraId`）の編集・保存時保持（[#221](https://github.com/ibisjp/ibis-office/issues/221)）
  - 表計算の機械可読性チェック（デジタル庁基準）（[#194](https://github.com/ibisjp/ibis-office/issues/194)）
- **Web 版・モバイル・Linux デスクトップ**
  - Web 版 Wasm の分割配信（CDN サイズ制限対応）（[#152](https://github.com/ibisjp/ibis-office/issues/152)）とフォント字形のオンデマンド取得（[#157](https://github.com/ibisjp/ibis-office/issues/157)）
  - Web 版の二本指ピンチズーム対応とタッチ入力時の Wasm パニック修正（[#212](https://github.com/ibisjp/ibis-office/issues/212), [#207](https://github.com/ibisjp/ibis-office/issues/207)）
  - Android でファイル選択画面から戻る際のクラッシュ修正（[#256](https://github.com/ibisjp/ibis-office/issues/256)）と Ubuntu AppImage のドックアイコン表示修正（[#147](https://github.com/ibisjp/ibis-office/issues/147)）

## 現時点での upstream との主な差異

upstream 統合状況や詳細な機能差分は [docs/fork-feature-differences.md](fork-feature-differences.md) に記録しています。現時点で `soraneDocs` に含まれる主な独自機能・拡張は以下のとおりです。

- **ナレッジワークスペース（`SoraneWorkspace`）・Markdown / PlantUML**
  - Obsidian 風のデスクトップシェル（`crates/sorane-workspace`）：ペイン分割、タブ固定・履歴、セッション永続化、コマンドパレット、デイリーノート、Wiki リンク、タグブラウザ、Outgoing Links、Local Graph、アウトライン、Office インスペクタ、Git ドック（[#283](https://github.com/ibisjp/ibis-office/issues/283)）。
  - ブロック単位の Markdown `SourceMap` による WYSIWYG 外科的書き戻しと双方向カーソル／スクロール同期（[#286](https://github.com/ibisjp/ibis-office/issues/286)）。
  - 内蔵 Mermaid 描画資産・IR を共有したオフライン pure-Rust PlantUML SVG 描画（UML 全般・C4/JSON/YAML/EBNF/nwdiag 等・軽量テーマ対応）（[#298](https://github.com/ibisjp/ibis-office/issues/298)）。
- **追加の文書・図面・データ形式**
  - **Visio（`.vsdx`）**：閲覧・新規作成・編集・ステンシルパレット・フローチャート図形・Wasm 版対応（[#292](https://github.com/ibisjp/ibis-office/issues/292)）。
  - **一太郎（`.jtd`・`.jtt`・`.jttc`）**：縦書き・ルビ・罫線・画像・目次を復元する読み取り専用プレビューと DOCX/HTML 出力（[jtd-preview.md](jtd-preview.md)）。
  - **DocuWorks（`.xdw`・`.xbd`）・Hancom（`.hwp`・`.hwpx`）**：閲覧および編集可能な DOCX としての読み込み（[#254](https://github.com/ibisjp/ibis-office/issues/254), [#288](https://github.com/ibisjp/ibis-office/issues/288)）。
  - **EPUB 3・制作素材画像**：EPUB 3 ネイティブ読み込み、HEIC・DPX・Cineon・OpenEXR・JPEG 2000 プレビュー（[production-image-preview-ja.md](production-image-preview-ja.md)）。
  - **外部データ取り込み**：表計算への Parquet・JSON Lines・Arrow IPC・Avro・SQLite 取り込みと、文字コードを考慮した CSV 上書き保存（[#263](https://github.com/ibisjp/ibis-office/issues/263)）。
- **日本の実務・法令・医療・トラスト拡張**
  - **名簿突合**：`FIELDMATCH` 系関数、異体字辞書、日付レビュー、行政 PUP・外字の正規化（[#257](https://github.com/ibisjp/ibis-office/issues/257)）。
  - **法令・医療**：e-Gov 法令 XML／Lawtext 双方向変換と新旧対照表生成（[#229](https://github.com/ibisjp/ibis-office/issues/229)）、pure-Rust 医療文書（UKE/DPC・FHIR/CDA/HL7 v2）および非診断用 DICOM 相互運用（Non-SaMD 範囲）（[#246](https://github.com/ibisjp/ibis-office/issues/246)）。
  - **署名・資格情報・サプライチェーン**：文書電子署名検査、画像 C2PA 検証、SD-JWT・mDOC・OID4VP の表計算連携、CycloneDX SBOM/CBOM・OSV-Scanner レビュー（[#232](https://github.com/ibisjp/ibis-office/issues/232), [#279](https://github.com/ibisjp/ibis-office/issues/279), [#280](https://github.com/ibisjp/ibis-office/issues/280)）。
- **Web・PWA・モバイル・AI アシスタント**
  - **Web 版（`docs.sorane.dev`）**：オフラインローカルファイル対応の PWA、GitHub App リポジトリ（Vault）ログイン・作成・編集、HTML 書き出し（[#208](https://github.com/ibisjp/ibis-office/issues/208)）、静的日本語フォントサブセット配信。
  - **モバイル・高速化**：Android / iOS ネイティブ GPUI ビルド（[#247](https://github.com/ibisjp/ibis-office/issues/247)）、描画・行ジオメトリ等のホットパスキャッシュ最適化（[#299](https://github.com/ibisjp/ibis-office/issues/299), [#300](https://github.com/ibisjp/ibis-office/issues/300)）。
  - **AI アシスタント**：OpenAI 互換 API（デスクトップ）および Workers AI（Web）連携の編集支援サイドバー（[#231](https://github.com/ibisjp/ibis-office/issues/231)）。
- **ブランディング・来歴検証・配布**
  - upstream の `ibisOffice` と共存できるよう、独自の `soraneDocs` ブランドおよびアプリ識別子を使用。
  - upstream との競合・混同を避けるため、自動更新チェックと Windows の「プログラムから開く」自動登録は無効化。
  - CI で CycloneDX 1.6 SBOM/CBOM と Linux AppImage の GitHub Artifact Attestations（署名付きビルド来歴）を生成（[#285](https://github.com/ibisjp/ibis-office/issues/285)）。

## ビルドと開発

```bash
git clone --recurse-submodules https://github.com/sorane-dev/docs.git
cd docs
cargo build --release
```

- **Web 版**：`scripts/build-web.sh --release`（WebAssembly + WebGPU 版を `www/` に生成）
- **デスクトップ版**：`scripts/bundle-macos.sh --release` / `scripts/bundle-linux.sh --release` / Windows は `cargo build --release -p ibis-office-app`
- **ブランチ運用・CI**：統合ブランチは `sorane` です。テスト・CI 検証・配信手順は [docs/fork-workflow.md](fork-workflow.md) をご覧ください。

## ライセンスと帰属表示

- **Powered by [ibisOffice](https://github.com/ibisjp/ibis-office)**（[TRADEMARK.md](TRADEMARK.md) に準拠）
- ライセンスは **AGPL-3.0-only** です（[LICENSE](../LICENSE)）。
- ibisOffice 本体の著作権は株式会社アイビス（ibis inc.）に帰属します。サードパーティライセンス表記は [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) をご覧ください。
