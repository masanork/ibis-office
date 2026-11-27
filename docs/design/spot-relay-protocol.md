# Spot Relay: サーバー疎結合・マルチテナント・オフライン対応フォーム回収プロトコル設計（PoC）

> **ステータス**: PoC 設計・プロトタイプ実装中（`feat/spot-relay-poc` / 2026-11-26）  
> **位置づけ**: [フォーム入力支援 × Vault × VC 統合設計書](vault-form-vc-architecture.md) とは独立したトランスポート・回収プロトコル検証。将来的に VC（SD-JWT / mDOC）を暗号化封筒のペイロード属性として同梱することは可能だが、本 PoC は **アプリ未インストールの PWA ブラウザ単体＋汎用リレー** で完結する。

---

## 1. 課題と設計原則

### 1.1 解決したい課題
1. **静的ホスト＋サーバー疎結合**: フォームの配信・入力を `docs.sorane.dev`（または `spot.docs.sorane.dev`）上の静的 PWA に置き、帳票や自治体（テナント）が増えても個別サーバーの構築・デプロイを不要にする。
2. **アプリインストール不要（PWA 完結）**: 避難時・オフライン環境でネイティブアプリを新規ダウンロードさせるのは非現実的。標準ブラウザ（iOS Safari / Android Chrome）だけで入力・暗号化・保存・提出が完結すること。
3. **オフライン・密集環境でのスケール（多対 1）**: 携帯回線や Wi-Fi が不安定・不通でも、**端末内キュー（IndexedDB）＋ 近接提出（QR / ローカル Wi-Fi AP HTTP）** にフォールバックでき、重複排除（冪等性）により二重送信しても破綻しないこと。
4. **マルチテナント・ゼロ知識リレー**: Cloudflare 上に汎用サーバー機能（Relay）を置く場合でも、サーバーは帳票スキーマや個人情報を一切解読せず、暗号化済み封筒をテナント別に冪等保管・引き渡しするだけに留める。

### 1.2 配置ドメインの整理（`spot.sorane.dev` 独立構成）
- **`spot.sorane.dev`（3 階層・専用 Worker）**:
  - Cloudflare の標準 Universal SSL（`*.sorane.dev`）でカバーされ、避難所向け超軽量 PWA として **Service Worker スコープ・IndexedDB オリジン・キャッシュ容量（数十 KB）を本体 WASM アプリ（`docs.sorane.dev` / `ibis.sorane.dev`）から完全分離** する。
  - 将来の汎用フォーム（`form.sorane.dev` 等）とは別用途・別ライフサイクルとして切り出しつつ、クライアントコードは相対パス基調でオリジン非依存に保つ。
  - リポジトリ内では `wrangler.spot.jsonc` と `worker/spot-entry.ts`（静的アセット `./www/spot`）で本体側 `wrangler.jsonc` を汚さず単独デプロイ・ローカル検証できる構成とする。

---

## 2. 全体アーキテクチャ（3 層分離）

```text
[避難者ブラウザ (汎用 1-page PWA)]
  │ 1. Form Manifest (JSON) を取得（SW でオフラインキャッシュ）
  │ 2. 入力値を圧縮 (deflate-raw) → 受付公開鍵で COSE_Encrypt0 暗号化
  │ 3. envelope_id = SHA-256(cose_bytes) を算出し IndexedDB にキュー保存
  │
  ├─ (Online)  ──POST /api/relay/v1/submit──▶ [Cloudflare 汎用リレー]
  ├─ (Offline) ──QR コード表示──────────────▶ [受付端末カメラ読取] ──(後刻同期)──▶ [Cloudflare 汎用リレー]
  └─ (LocalAP) ──POST http://192.168.x.x────▶ [受付ローカル HTTP] ──(後刻同期)──▶ [Cloudflare 汎用リレー]
                                                                                          │
                                                                                          ▼
                                                                           [受付・集計ビュー (秘密鍵で復号)]
```

---

## 3. データ・プロトコル仕様（PoC v1）

### 3.1 Form Manifest（静的 JSON / 自己完結 URL）
テナント（自治体・避難所・組織）と帳票の種類が増えても、この JSON を追加・URL 指定（`?manifest=<url>`、非圧縮 `?m=<base64url>`、または `deflate-raw` 圧縮 `?mz=<base64url>`）するだけで汎用 PWA がフォームを描画する。

- 標準サンプルは **個人単位（1人＝1行）** モデルとし、`claim_path`（将来の Wallet / Vault Form Assist 自動入力向けマッピング）、`input_helper`（代理入力者）、`group_id`（同行者引き継ぎ）を予約する。

```json
{
  "schema": "sorane.spot.form.v1",
  "tenant_id": "demo-city",
  "form_id": "evac-checkin",
  "version": "1.1.0",
  "title": "避難所受付票（個人単位・PoC）",
  "relay_url": "/api/relay/v1/submit",
  "inbox_url": "/api/relay/v1/inbox",
  "recipient_key": {
    "kty": "EC",
    "crv": "P-256",
    "kid": "demo-city-2026-01",
    "x": "...",
    "y": "..."
  },
  "fields": [
    { "id": "name", "label": "氏名", "type": "text", "required": true, "claim_path": "person.full_name" },
    { "id": "birth_date", "label": "生年月日", "type": "text", "claim_path": "person.birth_date" },
    { "id": "address", "label": "住所・居住地", "type": "text", "claim_path": "person.address" },
    { "id": "shelter", "label": "避難場所・拠点", "type": "text" },
    { "id": "needs", "label": "要配慮事項・必要な支援", "type": "textarea" },
    { "id": "input_helper", "label": "入力補助者・連絡窓口（代理入力時）", "type": "text" },
    { "id": "group_id", "label": "同行者グループID（任意）", "type": "text" }
  ]
}
```

### 3.2 暗号化封筒（COSE_Encrypt0 準拠エンベロープ）
- **暗号方式**: Web Crypto API 標準機能のみで実装可能な `ECDH-ES + HKDF-SHA256`（鍵合意）＋ `AES-256-GCM`（本文暗号化）。
- **COSE_Encrypt0（RFC 9052）構造**:
  - `protected`: CBOR マップ `{ 1 (alg): -29 (ECDH-ES + HKDF-256 / PoC 識別), 4 (kid): <recipient_kid> }`
  - `unprotected`: `{ -1 (epk): <送信側エフェメラル P-256 COSE_Key>, 5 (iv): <12-byte GCM IV> }`
  - `ciphertext`: AES-256-GCM 暗号文（16-byte tag 含む）
  - **External AAD**: `CBOR([ "sorane.spot.v1", tenant_id, form_id, version ])` を AAD にバインドし、別テナント・別帳票への差し替えを検知する。
  - **平文ペイロード**: `deflate-raw` 圧縮（または未圧縮フラグ付き）の JSON（`answers` + `verification: { method, trust_status }` + `submitted_at`）。
- **文字列表現**: `spot1:<base64url(COSE_Encrypt0_Tagged_CBOR)>`
  - テキスト 1 件あたり通常 **300〜900 文字程度** に収まり、**QR Version 1〜40（ECC L・最大 2,953 bytes）** の静止 QR コード 1 枚で表示・読み取り可能。
- **冪等 ID**: `envelope_id = hex(SHA-256(cose_bytes))`
  - どの経路（直接 POST / QR / ローカル AP）から何回届いても同一 `envelope_id` になり、サーバーおよび受付端末で自動的に重複排除される。

### 3.3 Cloudflare 汎用マルチテナントリレー API（Durable Objects SQLite）
Worker 側はテナント別 Durable Object（`SpotTenantRelay` / SQLite ストレージ）に `tenant_id` と `envelope_id` のみを保管し、**帳票スキーマや復号鍵を一切保持しない**。

| エンドポイント | 用途 | リクエスト / レスポンス |
| :--- | :--- | :--- |
| `POST /api/relay/v1/tenants` | テナント閲覧保護登録 | `{ tenant_id, read_token_hash }` → `201`（重複は `409`） |
| `POST /api/relay/v1/submit` | 単発・バッチ提出 | `{ tenant_id, items: [{ envelope_id, form_id, envelope }] }` → `200 { accepted, duplicates }` |
| `GET /api/relay/v1/inbox` | 受付側の回収 | `?tenant_id=...&since=...`（登録済みテナントは `Bearer <read_token>` 必須）→ `{ tenant_id, items: [...] }` |

---

## 4. 密集環境でのスケーラビリティの根拠
1. **電波輻輳の回避**: ブラウザ間で BLE メッシュや P2P コネクションを張り合わない。無通信時は **QR（視認・空間分割）** または **ローカル Wi-Fi AP への短時間 HTTP POST** に逃がす。
2. **ペイロード極小化**: 圧縮＋CBOR により 1 件数百バイトに抑え、QR 1 枚 / HTTP 1 リクエストで完結する。
3. **完全冪等な Store-and-Forward**: オフライン中に複数回再試行したり、QR 提出と回線回復後の自動再送が重複しても、`envelope_id` により 1 件に集約される。
