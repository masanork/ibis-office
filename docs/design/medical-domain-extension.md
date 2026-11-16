# 医療ドメイン拡張設計書（`masanork/ibis-office`）

本設計書は、政府・自治体向け機能（Lawtext・SD-JWT/mDOC/OID4VP・Harunobu 機械可読性・日本語データ正規化・Pure-Rust PDF/XPS/OFD）の先行実装を踏まえ、医療分野へ段階的に拡張するためのアーキテクチャ・ライセンス衛生・非 SaMD ガバナンスを定義する。

---

## 1. 基本方針と非 SaMD（プログラム医療機器非該当）境界

### 1.1 位置づけ
ibisOffice / Sorane Docs は **「クライアント完結型の構造化文書・表計算・プレゼンテーション処理基盤」** であり、疾病の診断・治療・予防を目的とするプログラム医療機器（SaMD）ではない。

### 1.2 禁止事項（コントリビューション審査基準）
1. **診断・治療推奨の禁止**: 検査値（FHIR `Observation`）、傷病名（`Condition`）、レセプト明細から疾患リスク・鑑別診断・治療方針を自動判定・推奨する機能（BYOK AI ツールのプロンプト／ツール定義を含む）は実装・受け入れしない。
2. **一次読影ビューア化の禁止**: DICOM 機能は「学会発表・症例報告・臨床研究資料向けのローカル匿名化および Office 文書埋め込みアセット化」に限定し、一次読影・診断用ビューアとしての機能・訴求を行わない。
3. **外部送信ゼロの維持**: 患者情報を含む入力ファイル（UKE / FHIR / DICOM / DOCX / XLSX）を外部サーバーへ送信しないローカル処理完結性を維持する。

---

## 2. 既存の `masanork/ibis-office` 資産との接続

| 既存モジュール | 既存機能 | 医療ドメインでの転用先 |
| :--- | :--- | :--- |
| `crates/ibis-pptx/src/uke.rs` | `RECEIPTC.UKE` 可変長 CSV 構造化展開・先頭ゼロ保持・検算サマリー | Phase 1: 医療事務・レセプト点検ビューア |
| `crates/ibis-doc-model/src/machine_readability*.rs` / `crates/ibis-office-xlsx/src/harunobu.rs` | Harunobu 機械可読性検査（1セル複数値、外字/機種依存文字、空白/結合セル等） | Phase 1: レセプト・DPC・患者台帳の提出前品質チェック |
| `crates/ibis-doc-model/src/japanese_date_review.rs` / `variant_clusters.rs` | 和暦・西暦正規化、異体字・文字クラスタレビュー | Phase 1: 生年月日・診療年月日の表記統一、患者名/傷病名の異体字揺れ検出 |
| `crates/ibis-pptx/src/lawtext.rs` | 構造化 XML ⇄ Markdown、新旧対照表、静的 lint | Phase 1-2: 診療報酬改定マスター・通知差分、HL7 FHIR XML/JSON の可読文書化 |
| `crates/ibis-doc-model/src/sd_jwt*.rs` / `crates/ibis-office-xlsx/src/sd_jwt_panel.rs` | SD-JWT / ISO 18013-5 mDOC / OID4VP 検証・提示 | Phase 2: 医師等5資格のデジタル資格者証・HPKI・患者同意 VC の検証・紐付け |
| `crates/ibis-pdf` / `crates/ibis-office-pdf` / `crates/ibis-xps` / `crates/ibis-ofd` | Pure-Rust 固定レイアウト描画・`/ByteRange` および SM2withSM3 署名検査 | Phase 2: 診療情報提供書 PDF 等の電子署名・改ざん境界検証 |

---

## 3. ライセンス衛生と外部マスターデータの厳格な分離

### 3.1 コード依存の規律（`deny.toml`）
ibisOffice は `AGPL-3.0-only` と商用ライセンスのデュアルライセンスを採用しており、ルート `deny.toml` により依存クレートは **MIT / Apache-2.0 / BSD-2-Clause / BSD-3-Clause / ISC / Zlib / CC0-1.0 / OFL-1.1 / 未改変 MPL-2.0** に制限されている。新規導入クレートはすべて `cargo deny check licenses` を通過しなければならない。

### 3.2 公的マスター・標準規格データの非同梱原則
診療報酬基本マスター（医科・歯科・調剤・医薬品・特定器材・傷病名・コメント）や JP-Core 用語集（Terminology）等の公的・団体配布データは、原著作物の利用規約と AGPL-3.0 の再配布要件との衝突を避けるため、**リポジトリおよび配布バイナリに一切同梱しない**。

- 実装方式: `MedicalMasterIndex::from_csv_bytes` のように、利用者が自身のローカル環境または公式配布元から取得した CSV/TSV を実行時に読み込んでローカルキャッシュする方式を採る。
- テスト・プレイグラウンド: リポジトリ内の自動テストや Web プレイグラウンドには、完全に架空のダミーデータ（例: `tests/fixtures/synthetic_receiptc.uke`）のみを用いる。

### 3.3 医師・医療事務コミュニティ向け宣言的定義レイヤー
医療従事者に Rust / GPUI の記述を求めないため、以下のドメイン知識はコアバイナリから分離した外部アセット／宣言的定義（JSON / YAML / Markdown / DOCX・PPTX テンプレート）として受け入れる構造とする：
- 診療科別の診療情報提供書・退院時サマリーテンプレート（DOCX / Markdown）
- レセプト・DPC 点検チェックプロファイル（YAML / JSON）
- 学会・倫理審査基準に応じた DICOM 匿名化タグプロファイル（JSON）

---

## 4. フェーズ別実装計画

### Phase 1: 医療事務・診療報酬データ処理（先行実装済み PoC ＋ 拡張）
- **実装済み（PoC）**: `crates/ibis-pptx/src/uke.rs` による `RECEIPTC.UKE` 自動判別・Shift_JIS/UTF-8 デコード・先頭ゼロ保持・2シート構成（`レセプト明細` / `レセプトサマリー`）・`HO` / `GO` 点数検算・任意外部マスター解決（`MedicalMasterIndex`）。
- **次期拡張**:
  - DPC 関連ファイル（様式1・E/F ファイル）の固定長・タブ区切り安全インポート。
  - `harunobu` / `japanese_date_review` / `variant_clusters` を束ねた「医療事務データ品質チェック」ワンクリック実行プロファイル。
  - 改定前後マスター比較および施設基準様式（DOCX/XLSX/JTD）への影響箇所ハイライト。

### Phase 2: HL7 FHIR（3文書6情報）＆ デジタル資格・署名ハブ
- **パーサ層**: `deny.toml` 適合クレート（`hl7kit`, `fhirkit-rs` 等）および `roxmltree` / `serde_json` を用いた JP-Core 3文書6情報（診療情報提供書・退院時サマリー・健診結果等）のローカルパース。
- **出力ブリッジ**: パース済みリソースを `ibis-doc-model` の `FlowDocument`（DOCX / Markdown）および `ibis-pdf` へ忠実にレンダリング。
- **真正性検証**: 既存の `sd_jwt*.rs` / `ibis-pdf::signature` を接続し、HPKI / デジタル資格者証による作成者属性と非改ざん性を文書単位で表示。

### Phase 3: 非診断 DICOM アセット統合
- **匿名化・変換**: `dicom-rs`（`dicom-object`, `dicom-pixeldata`: Apache-2.0 OR MIT）をメモリ内処理で呼び出し、個人識別タグ（`(0010,0010)` PatientName、`(0010,0020)` PatientID、`(0010,0030)` PatientBirthDate 等）を除去・マスクした上で、PNG/RGBA 画像および非個人撮像パラメータ表を生成し `.pptx` / `.docx` へ直接挿入する。
- **Wasm 制約の安全フォールバック**: `dicom-transfer-syntax-registry` 上、JPEG 2000（`openjpeg-sys`）や JPEG-LS（`charls`）等の C 依存転送構文は Wasm で動作しないため、Wasm ビルドでは `default-features = false` とし、`rayon` や C バインディングを除外した構成を採る。非対応構文を含むファイル投入時はクラッシュさせずデスクトップ版利用を案内する。
- **スパイク検証で確認されたライセンス注意点（`cargo deny check licenses`）**:
  - `fhirkit v0.1.0`・`hl7kit v0.2.0`・`dicom-object v0.10.1`（`deflate`）は `wasm32-unknown-unknown` でのコンパイルおよび現行 `deny.toml` の両方をそのまま通過する。
  - `dicom-pixeldata v0.10.1` で `jpeg` 機能を有効化すると、`dicom-transfer-syntax-registry` 経由で `jpeg-encoder v0.7.1`（ライセンス: `(MIT OR Apache-2.0) AND IJG`）が入り、現行 `deny.toml` では `IJG` が未許可エラーとなる。商用利用条件を確認して `deny.toml` に `"IJG"` を明示許可するか、エンコード機能を外してデコード専用構成にする設計判断が必要。
