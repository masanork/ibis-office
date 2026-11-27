# Phase 4 設計書：Web 版（GitHub Vault）・モバイル連携・制度ユースケース展開

> **ステータス**: Phase 4A / 4B / 4C 実装完了（2026-11-26）  
> **親設計書**: [soraneDocs フォーム入力支援 × Obsidian 風 Vault × Verifiable Credentials 統合設計書](vault-form-vc-architecture.md)  
> **関連ドキュメント**: [Spot 中継プロトコル](spot-relay-protocol.md) / [就労証明書の XLSX 受け渡しと項目対応](../employment-certificate-xlsx-ja.md)

---

## 1. 目的と前提

Phase 0〜3 でデスクトップ版（`SoraneWorkspace` + ローカル Vault + OS キーチェーン / `0600` 鍵保管 + `curl` `direct_post`）における **フォーム入力支援（XLSX / DOCX / Markdown / PDF）→ 選択的開示提出 → 提出レシート → 一括検証 BI** の一連の体験が完成した。

Phase 4 では、このモデルを **`ibis.sorane.dev`（Web / PWA 版）**、**モバイル端末（Android / スマホ Wallet）**、および **標準業務・制度様式レシピ** へ安全に拡張する。

---

## 2. Web 版（`ibis.sorane.dev` × GitHub Vault）の成立性と境界設計

### 2.1 現状の土台

- [worker/github-vault.ts](../../worker/github-vault.ts) と [www/sorane-github-vault.js](../../www/sorane-github-vault.js) が GitHub App OAuth ログイン・リポジトリツリー取得（`/api/github/tree`）・ファイル取得/更新（`/api/github/file`）を実装済み。
- [crates/sorane-workspace/src/workspace.rs](../../crates/sorane-workspace/src/workspace.rs) の `web_platform::Folder` が `ibisOfficeHydrateWebVaultFile` 経由でインメモリ Vault インデックスを保持している。

### 2.2 信頼境界（何を GitHub に置き、何を置かないか）

| データ分類 | 保存先（Web 版） | 理由・セキュリティ条件 |
| :--- | :--- | :--- |
| `profiles/*.md`（基本プロフィール） | GitHub Private Repo（任意） | ユーザー管理下のプライベートリポジトリに保存。Git 履歴で変更追跡可能。 |
| `forms/*.form-map.md`（様式対応表） | GitHub Repo（公開/非公開） | 様式定義は機密を含まないためチーム・コミュニティ共有にも適する。 |
| `requests/*.req.md`（提出先定義） | GitHub Repo | Verifier の `client_id` / `response_uri` / 要求クレーム定義。 |
| `credentials/*.sd-jwt`, `*.mdoc` | GitHub Private Repo **または** IndexedDB（ローカル専用） | 個人番号や要配慮情報を含む VC はリポジトリに上げずブラウザ内 IndexedDB のみに留める選択も可能にする。 |
| **秘密鍵（Holder / Issuer ES256）** | **ブラウザ内 WebCrypto (`extractable: false`) + IndexedDB のみ** | **絶対に GitHub リポジトリや Worker 側へ保存・送信しない**。`SubtleCrypto.generateKey({ name: "ECDSA", namedCurve: "P-256" }, false, ["sign", "verify"])` で秘密鍵バイト列を JS/WASM から取り出せない形で保持する。 |
| `submissions/*.md`, `verifications/*.sheet.md` | GitHub Private Repo / ローカルダウンロード | 提出記録と一括検証 BI シート。 |

### 2.3 Web 版特有の技術課題と解決策

1. **Phase 4A（今回の MVP）— GitHub Vault × Form Assist 入力・検証 BI**
   - `www/sorane-github-vault.js` のハイドレーション対象を `.md` / `.markdown` に加えて `.sd-jwt` / `.mdoc`（サイズ上限 256 KiB）および `profiles/`・`forms/`・`requests/` 優先へ拡張する。
   - `web_platform::Folder::rebuild_index` で `ibis_formats::form_assist` のパーサ（`parse_profile_markdown`, `parse_form_mapping_markdown`, `parse_presentation_request_markdown`, `parse_credential_bytes`）を直接呼び出し、デスクトップ版と同一の `FormAssist`（入力支援＋一括検証サマリー）を Web 上で有効化する。
2. **Phase 4B — WebCrypto 署名ブリッジ & WASM PDF フォーム**
   - `ibis-doc-model` の `Es256KeyPair::sign_es256` と対になる非同期署名トレイトまたは JS ブリッジ（`window.soraneWebCryptoSignP256(keyId, messageBytes)`）を導入し、非抽出 `CryptoKey` で `kb+jwt` 署名を行えるようにする。
   - `crates/ibis-formats/Cargo.toml` の `ibis-pdf` 依存を `[target.'cfg(not(target_family = "wasm"))']` から共通 `[dependencies]` へ移し（純 Rust 実装のため WASM 動作可能、`verapdf` 外部 CLI 呼び出しのみ `#[cfg(not(target_family = "wasm"))]` を維持）、Web 版でも PDF AcroForm 自動入力を有効化する。
3. **Phase 4C — CORS 制約を越える OID4VP `direct_post` 中継**
   - Verifier 側が `Access-Control-Allow-Origin` を返さない場合、ブラウザ `fetch` 単体では `direct_post` が CORS でブロックされる。
   - 解決策として、まずブラウザ直接 `fetch` を試み、CORS 失敗時かつユーザーが明示承認した `https://` 送信先に限り、Cloudflare Worker の `/api/vc/direct-post` へ中継する（SSRF 防止のためループバック・プライベート IP・リンクローカル・メタデータ IP を厳格拒否し、トークン内容を一切ログ保存しない）。

---

## 3. モバイル連携（スマホ Wallet ⇄ PC / Web フォーム）

すでに `www/spot/` と `worker/spot-relay.ts` に実装済みの **QR コード（`www/spot/qr.ts`）＋ Durable Objects 一時リレー ＋ COSE 暗号化封筒（`www/spot/cose-envelope.ts`）** を、クロスデバイス VC 提示プロトコルへ応用する。

1. **W3C Digital Credentials API（ブラウザネイティブ）**
   - 対応ブラウザ（Chrome / Safari の新実装）上では `navigator.credentials.get({ digital: { providers: [{ protocol: "openid4vp", request: ... }] } })` を優先利用し、OS 標準 Wallet（マイナンバーカード / EUDI Wallet 等）から直接レスポンスを受け取る。
2. **QR ＋ Spot リレー（デスクトップ版・非対応ブラウザ向けフォールバック）**
   - デスクトップ版 `FormAssist` の「スマホから証明書を取り込む」で一時セッション（HPKE / ECDH P-256 公開鍵＋ワンタイム `nonce`）の QR を表示。
   - スマホ側（Web / PWA または Android アプリ）で QR を読み取り、Wallet から受け取った `.sd-jwt` / `.mdoc` を暗号化封筒で `spot-relay` 経由返送 → PC 側が復号して `FormAssist` へ流し込む。

---

## 4. 段階的実装プラン（Phase 4A 〜 4C）

| サブフェーズ | スコープ | 主な変更対象 | 完了条件 |
| :--- | :--- | :--- | :--- |
| **Phase 4A（今回着手）** | Web Vault インデックス接続（GitHub Vault ハイドレーション拡張 ＋ `web_platform::Folder` のプロフィール／VC／対応表／要求パーサ実装） | `crates/sorane-workspace/src/workspace.rs`, `crates/sorane-workspace/src/form_assist.rs`, `www/sorane-github-vault.js` | `web_platform::Folder` 単体テストで `profiles/`・`forms/`・`requests/`・`.sd-jwt` がインデックスされ、`cargo check`（デスクトップ＋非デスクトップパス）が通ること |
| **Phase 4B** | WebCrypto 非抽出鍵署名ブリッジ ＋ WASM 対応 `ibis-pdf` AcroForm | `crates/ibis-formats/Cargo.toml`, `www/main.ts`, `crates/ibis-office-shell/src/vc_transport.rs` | ブラウザ上で `kb+jwt` 署名と PDF フォーム記入が秘密鍵エクスポートなしで動作すること |
| **Phase 4C** | Worker `direct_post` 中継 ＋ Digital Credentials API / QR リレー ＋ 業務レシピ集（請求書・就労証明書） | `worker/index.ts`, `www/spot/`, `docs/samples/` | クロスデバイス提示から Excel/PDF フォーム自動入力・提出記録まで一連動作すること |
