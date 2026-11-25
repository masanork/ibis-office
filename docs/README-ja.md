<p align="center"><a href="../README.md">English</a> | <b>日本語</b></p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="../asset/sorane-docs-lockup-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="../asset/sorane-docs-lockup-light.svg">
    <img src="../asset/sorane-docs-lockup-light.svg" alt="soraneDocs — Powered by ibisOffice" width="340">
  </picture>
</p>

<p align="center">
  <b>Rust と GPUI で作られた <a href="https://github.com/ibisjp/ibis-office">ibisOffice</a> の機能拡張フォーク</b><br>
  iOS / Android 移植、Markdown・Mermaid・PlantUML ナレッジワークスペース、多形式ドキュメント/図表対応、BI/外部データ連携、AI アシスタント。
</p>

<p align="center">
  <a href="https://docs.sorane.dev">Web 版を試す (docs.sorane.dev)</a> ·
  <a href="fork-feature-differences.md">upstream との機能差分</a> ·
  <a href="fork-workflow.md">フォーク運用</a>
</p>

---

<!-- TODO: 追って soraneDocs の画面ショットを追加 -->

## soraneDocs とは

**soraneDocs** は、[ibisOffice](https://github.com/ibisjp/ibis-office) をベースに `sorane-dev/docs` で開発・運用しているフォークです。upstream の Rust + GPUI 描画エンジン、高精度な OOXML（`.docx`・`.xlsx`・`.pptx`）互換、MCP 基盤を引き継ぎつつ、次の 2 軸で開発を進めています。

- **upstream への貢献（採用済み・提案中）**：`ibisjp/ibis-office` はコラボレーター以外の Pull Request を受け付けていないため、本フォークで検証可能な PoC コミットとして実装し、GitHub Issue を通じて提案しています。多くがすでに **本家へ採用・統合** されており、残る拡張も **提案中（Open）** として共有しています。
- **拡張ワークスペースとプラットフォーム展開**：iOS / Android へのネイティブ移植、Obsidian 風 Markdown／図表ワークスペース、各種レガシー・行政/医療・図面フォーマット、BI/外部データ取り込み、AI アシスタント、PWA・GitHub Vault 連携などを `docs.sorane.dev` およびデスクトップ/モバイル向けに統合しています。

ibisOffice 本来のベンチマーク、基本機能一覧、商用ライセンスについては [upstream リポジトリ](https://github.com/ibisjp/ibis-office) をご覧ください。

## 主な寄与と開発領域

各領域について、本家に **採用済み（Adopted）** のもの、本家へ **提案中（Proposed）** のもの、**フォーク独自（Fork）** として先行実装しているものを包括的にまとめています。

### 1. モバイル移植（iOS / Android）とクロスプラットフォーム GPUI
- **iOS / Android ネイティブ移植** *（提案中: [#247](https://github.com/ibisjp/ibis-office/issues/247)）*：GPUI + `gpui-pre-wgpu` スタックを Android（`crates/gpui-android`）および iOS（`crates/gpui-ios`）へ移植し、モバイル向けスタートビュー、タッチ操作・サーフェス復帰対応、Google Play 内部トラックへの自動 AAB 配信を整備。
- **プラットフォーム修正** *（採用済み: [#256](https://github.com/ibisjp/ibis-office/issues/256), [#147](https://github.com/ibisjp/ibis-office/issues/147)）*：Android でファイル選択画面から戻る際のクラッシュ修正、Ubuntu AppImage のドックアイコン表示修正。

### 2. Markdown・Mermaid・PlantUML 統合 / ナレッジワークスペース
- **Markdown & Mermaid 基盤** *（採用済み: [#226](https://github.com/ibisjp/ibis-office/issues/226)）*：Markdown ネイティブ往復変換、クリップボード/CLI 対応、pure-Rust によるオフライン Mermaid 描画。
- **CommonMark 互換強化と双方向同期** *（提案中: [#286](https://github.com/ibisjp/ibis-office/issues/286)）*：CommonMark 0.31.2 全 652 例での検証に基づく往復互換性改善、ブロック単位の `SourceMap` による WYSIWYG 外科的書き戻しと双方向カーソル／スクロール同期。
- **PlantUML 統合** *（提案中: [#298](https://github.com/ibisjp/ibis-office/issues/298)）*：内蔵 Mermaid 描画資産・IR を共有したオフライン pure-Rust PlantUML SVG 描画（シーケンス・クラス・ステート・ユースケース・オブジェクト/マップ・タイミング・アクティビティ・コンポーネント等の UML 全般、Mindmap/WBS/Gantt、C4/JSON/YAML/EBNF/nwdiag/chronology、軽量テーマ・Creole 書式）。
- **Obsidian 風 `SoraneWorkspace`** *（提案中: [#283](https://github.com/ibisjp/ibis-office/issues/283)）*：ペイン分割、タブ固定・履歴、セッション永続化、コマンドパレット、デイリーノート、Wiki リンク、OKF v0.3 知識項目、タグブラウザ、Outgoing Links、Local Graph、アウトライン、Office インスペクタ、Git ドックを備えたデスクトップシェル（`crates/sorane-workspace`）。

### 3. ドキュメント・図面・レガシー形式の拡張
- **Pure-Rust PDF・XPS/OpenXPS・OFD** *（採用済み: [#235](https://github.com/ibisjp/ibis-office/issues/235) / 提案中: [#272](https://github.com/ibisjp/ibis-office/issues/272)）*：Pure-Rust による固定レイアウト閲覧および署名検査対応、図を保持した Markdown 書き出し。
- **一太郎（`.jtd`・`.jtt`・`.jttc`）** *（Fork）*：論理レコード・縦書き・ルビ・罫線・画像・サムネイル・目次を復元する読み取り専用プレビューと DOCX/HTML 出力（[jtd-preview.md](jtd-preview.md)）。
- **Microsoft Visio（`.vsdx`）** *（提案中: [#292](https://github.com/ibisjp/ibis-office/issues/292)）*：閲覧・新規作成・編集、ステンシルパレット、フローチャート図形、スナップハイライト、Web/Wasm 版対応。
- **Unified Office Format（UOF / `.uot`）** *（Fork）*：UOT 文書および埋め込み画像のネイティブ読み込み。
- **WordPerfect（`.wpd`）& WPG 2** *（Fork）*：WordPerfect 文書（テキスト・表）および WPG 2 ベクタ・ビットマップ・曲線・複合パス画像の読み込み。
- **DocuWorks（`.xdw`・`.xbd`）& Hancom（`.hwp`・`.hwpx`）** *（提案中: [#254](https://github.com/ibisjp/ibis-office/issues/254), [#288](https://github.com/ibisjp/ibis-office/issues/288)）*：DocuWorks 閲覧および HWP 5.0 / HWPX の編集可能な DOCX としての読み込み。
- **EPUB 3・制作素材画像** *（Fork）*：EPUB 3 ネイティブ読み込み、HEIC・DPX・Cineon・OpenEXR・JPEG 2000 プレビュー（[production-image-preview-ja.md](production-image-preview-ja.md)）。
- **OOXML / ODF 互換強化** *（採用済み: [#190](https://github.com/ibisjp/ibis-office/issues/190), [#221](https://github.com/ibisjp/ibis-office/issues/221)）*：Office 2007 Standard 暗号化復号、DOCX 段落 ID（`w14:paraId`）の編集・保存時保持、ODF の入れ子領域・繰り返し列・非表示行列読み込み拡張。

### 4. BI・データ分析・外部データ連携
- **クロスフォーマット SQL & クエリパイプライン** *（提案中: [#263](https://github.com/ibisjp/ibis-office/issues/263)）*：CSV・Parquet（列選択・複数ファイル追記・Zstandard）・Arrow IPC・JSON Lines・Avro・SQLite テーブルをインメモリ／ストリーミング SQLite（Turso）で結合クエリし、Excel テーブルへの書き出し、バインド先を指定した一括更新、スキーマ変更検知、品質チェックゲート、文字コードを考慮した CSV 上書き保存を実現。
- **データ分析・Power Query M 言語・モデリング** *（Fork / 提案中）*：依存ゼロの Power Query M 言語インタプリタ（`crates/ibis-m-lang`）、クエリエディタ／レポート管理、ピボットテーブルの集計フィールド・集計アイテム・ドリルダウン、コホート分析チャート、統計分析パネル、予測シート、ソルバー、クイック分析、プレフィル機能。
- **Harunobu 互換の機械可読性チェック & 名簿突合** *（採用済み: [#194](https://github.com/ibisjp/ibis-office/issues/194) / 提案中: [#257](https://github.com/ibisjp/ibis-office/issues/257)）*：デジタル庁の機械可読性ルール（Harunobu 0.2.3 相当）に基づくリアルタイム検査（見出し・結合セル・非表示行列・数値文字列・数式等の検出と該当セルへの移動、[machine-readability.md](machine-readability.md)）、および `FIELDMATCH` 系関数・異体字辞書・日付レビュー・行政 PUP/外字正規化。
- **セキュリティ／コンプライアンス分析** *（提案中: [#279](https://github.com/ibisjp/ibis-office/issues/279), [#280](https://github.com/ibisjp/ibis-office/issues/280)）*：CycloneDX SBOM/CBOM 棚卸し（PQC 移行管理含む）および OSV-Scanner 脆弱性結果の表計算レビュー。

### 5. AI アシスタント & MCP サーバー拡張
- **AI アシスタントサイドバー** *（提案中: [#231](https://github.com/ibisjp/ibis-office/issues/231)）*：OpenAI 互換 API（デスクトップ）および Cloudflare Workers AI（Web）と連携した DOCX/XLSX/PPTX 共通の編集支援・変更提案。
- **MCP サーバー拡張（`ibis-mcp` & `ibis-doc-service`）** *（提案中: [#159](https://github.com/ibisjp/ibis-office/issues/159), [#275](https://github.com/ibisjp/ibis-office/issues/275), [#276](https://github.com/ibisjp/ibis-office/issues/276), [#281](https://github.com/ibisjp/ibis-office/issues/281)）*：基本的な OOXML 読み書きに加え、SQLite／クロスフォーマットデータセットクエリ（`sqlite_query`, `sqlite_query_to_table`, `sqlite_refresh_query_reports`, `xlsx_set_query_report_checks`）、ピボットテーブル、チャート、構造化プレフィル、SD-JWT/OID4VP ウォレット用の MCP ツール群を実装。さらに、起動中のデスクトップ Assistant へ外部 MCP クライアントから再接続するライブブリッジ、シート保護の非表示数式尊重、入力展開前のセル数上限チェックを備えています（[mcp-ja.md](mcp-ja.md)）。

### 6. 日本語組版・行政・法令・医療・トラスト
- **日本語組版・フォント描画** *（採用済み: [#158](https://github.com/ibisjp/ibis-office/issues/158), [#160](https://github.com/ibisjp/ibis-office/issues/160), [#193](https://github.com/ibisjp/ibis-office/issues/193), [#222](https://github.com/ibisjp/ibis-office/issues/222), [#230](https://github.com/ibisjp/ibis-office/issues/230), [#248](https://github.com/ibisjp/ibis-office/issues/248), [#251](https://github.com/ibisjp/ibis-office/issues/251)）*：行政事務標準文字と異体字シーケンス（IVS）対応および macOS 字形欠落修正、非 Windows 環境での「ＭＳ 明朝」代替ルール修正、和文記号/仮名文字送り・GPOS オフセット改善、縦書き傍線位置・表セル文字方向/縦結合・両端揃え箇条書き修正。
- **法令・医療・構造化フォーム** *（提案中: [#229](https://github.com/ibisjp/ibis-office/issues/229), [#246](https://github.com/ibisjp/ibis-office/issues/246), [#270](https://github.com/ibisjp/ibis-office/issues/270)）*：e-Gov 法令 XML／Lawtext 双方向変換と新旧対照表生成、pure-Rust 医療文書（UKE/DPC・FHIR/CDA/HL7 v2）・非診断用 DICOM 相互運用（Non-SaMD 範囲）、DOCX フォーム作成と構造化データからの生成。
- **電子署名・デジタル資格情報** *（提案中: [#232](https://github.com/ibisjp/ibis-office/issues/232)）*：文書電子署名検査、画像 C2PA 検証、SD-JWT・mDOC・OID4VP の表計算連携。

### 7. Web（Wasm/WebGPU）・PWA・高速化・サプライチェーン
- **Web 配信・タッチ操作** *（採用済み: [#152](https://github.com/ibisjp/ibis-office/issues/152), [#157](https://github.com/ibisjp/ibis-office/issues/157), [#207](https://github.com/ibisjp/ibis-office/issues/207), [#212](https://github.com/ibisjp/ibis-office/issues/212)）*：CDN サイズ制限対応の Wasm 分割配信、フォント字形のオンデマンド取得、二本指ピンチズーム、タッチ入力時パニック修正。
- **PWA・GitHub Vault・HTML 出力** *（提案中: [#208](https://github.com/ibisjp/ibis-office/issues/208)）*：`https://docs.sorane.dev` でのオフラインローカルファイル対応 PWA、GitHub App リポジトリ（Vault）ログイン・作成・編集、構造化 HTML 書き出し、静的日本語フォントサブセット配信。
- **高速化・ビルド来歴** *（提案中: [#285](https://github.com/ibisjp/ibis-office/issues/285), [#299](https://github.com/ibisjp/ibis-office/issues/299), [#300](https://github.com/ibisjp/ibis-office/issues/300)）*：GPUI 描画・設定・メニューバー・DOCX/XLSX ジオメトリのホットパスキャッシュ最適化、CycloneDX 1.6 SBOM/CBOM 生成と Linux AppImage の GitHub Artifact Attestations（署名付き来歴）。

## 現時点での upstream との差異・運用方針

upstream `main` とのコミット・機能単位の対比は [docs/fork-feature-differences.md](fork-feature-differences.md) にまとめています。

上記の先行実装・提案中の機能に加え、`soraneDocs` では安全性と共存のため次の点でも upstream と差別化しています：
- **独自ブランディング**：`ibisOffice` と共存できるよう、独自の `soraneDocs` ブランドおよびアプリ識別子を使用。
- **更新・関連付けの分離**：upstream 側のリリースフィード参照や既定アプリ登録を上書きしないよう、自動更新チェックと Windows の「プログラムから開く」自動登録は無効化。

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
