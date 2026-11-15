# Excel フォームからの SD-JWT 生成と OID4VP 提示の設計計画

## 目的

Excel（`.xlsx`）のフォームコントロールおよび定義された名前（`DefinedName`）と JSON クレームを命名規約ベースでバインディングし、スプレッドシートへのフォーム入力から **SD-JWT（RFC 9901 / SD-JWT VC）** の生成、および **OID4VP（OpenID for Verifiable Presentations）** による選択的開示・提示までを一気通貫で行えるようにする。

独自の OOXML パートを追加せず標準の `<definedNames>`（セル名）と `<control name="...">`（フォームコントロール名）でバインディングを表現することで、Microsoft Excel 等で開いて編集・保存してもバインディングが失われない互換性を維持する。

## 1. フォーム設計と JSON バインディング規約

### 命名規約

Excel の `DefinedName` で使用可能な記号（`_`、`.`、`\`）に合わせ、セル名および `FormControl.name` の双方で共通のドット区切りプレフィックス規約を採用する。

| プレフィックス | 役割 | 指定対象の例 | SD-JWT 上の扱い |
| --- | --- | --- | --- |
| `claim.<path>` | 常時開示クレーム（平文） | セル名 `claim.vct`, `claim.iss`, `claim.address.country` | JWT ペイロードに平文クレームとして配置 |
| `sd.<path>` | 選択的開示クレーム（`_sd` 対象） | セル名 `sd.family_name`, `sd.birthdate`<br>Drop コントロール名 `sd.prefecture` | Salt 付き Disclosure を生成し、ペイロード側は `_sd` ダイジェスト配列へ格納 |
| `disclose.<path>` | 開示トグル（提示時の選択） | CheckBox コントロール名 `disclose.family_name`（またはセル内チェックボックス） | ON のときのみ該当 Disclosure を `~` 連結で出力（未定義時は既定で開示） |

- `<path>` が `.` を含む場合（例: `sd.address.locality`）は、ネストした JSON オブジェクト（`{"address": {"_sd": [...]}}`）として構築する。
- **Excel の2次元グリッドに自然に馴染む3階層構造モデル（ISO/IEC 18013-5 mDL / EUDI PID 準拠）**:
  1. **基本（フラット属性）**: 単一セルまたは単一コントロール（`sd.family_name`, `sd.birth_date`, `sd.document_number` 等）。
  2. **1階層グループ**: `sd.address.region` や `sd.place_of_birth.country` などドット記法による1階層オブジェクト。
  3. **配列・テーブル属性（複数セル範囲バインディング）**:
     - **1列または1行の範囲**（例: `sd.nationalities` = `B12:B13`）→ 空でないセルを集めたプリミティブ配列 `["JP", "US"]`。
     - **複数行×複数列の表範囲**（例: ISO 18013-5 の `sd.driving_privileges` = `A14:C16`）→ 先頭行を見出し（キー名）、2行目以降をオブジェクトレコードとして `[{"vehicle_category_code": "普通", "issue_date": "2018-04-01", "expiry_date": "2029-05-15"}]` のオブジェクト配列へ自動変換。

### 値と型の解決規則

1. **フォームコントロール（`FormControl.name`）に一致する場合**
   - `Drop` / `List`: 内部の選択番号（`value: 1, 2, ...`）ではなく、入力範囲（`fmlaRange`）から解決した `items[value - 1]` の表示文字列を JSON 文字列とする（未選択 `0` または範囲外は省略）。
   - `CheckBox`: `checked == 1` を JSON の `bool` とする。
   - `Radio`: 同一グループ（`first_button` 区切り）内で `checked == 1` のボタンのキャプション文字列（`text`）を JSON 文字列とする。
   - `Spin` / `Scroll`: `value` を JSON の数値とする。
2. **定義された名前（`DefinedName.name`）のセルまたは範囲に一致する場合**
   - そのセルを `fmlaLink`（リンクするセル）として参照する `Drop` / `List` コントロールが同一シート上に存在する場合は、セル内のインデックス番号ではなくコントロール側の表示文字列 `items[value - 1]` を優先して解決する。
   - セル内チェックボックス（`CellStyle::checkbox`）または `CellValue::Bool` は JSON の `bool` とする。
   - セルの表示形式が `@`（文字列）の場合は、先頭ゼロを含む免許証番号（`"001234567890"`）や郵便番号が数値化されないよう常に JSON 文字列として解決する。
   - `CellValue::Number` でセルの表示形式が日付書式の場合は ISO 8601 日付文字列（`"YYYY-MM-DD"`）へ変換する（ただし `iat`、`exp`、`nbf` クレームは NumericDate 整数とする）。
   - 上記以外の数値・文字列はそのまま JSON の数値・文字列とし、空セルはクレームを省略する。
   - 1次元範囲は非空要素の配列 `[...]`、2次元範囲は1行目を見出しキーとするオブジェクト配列 `[{...}]` として解決する。

### フォーム編集 UI の拡張

- `ibis-office-xlsx` の「コントロールの書式設定」パネル（`crates/ibis-office-xlsx/src/control_insert.rs`）にコントロール名（`FormControl.name`）の入力・編集欄を追加し、UI 上から `sd.*` や `disclose.*` の名前を設定できるようにする。
- このコントロール名編集 UI 自体は純粋な Excel 互換機能の改善であるため、単体で upstream（`ibisjp/ibis-office`）へ PR 還元可能な形で切り出す。

## 2. フォーム入力からの SD-JWT 生成

- バインディング解決結果から SD-JWT ペイロードおよび Disclosures（`[salt, claim_name, value]` の Base64url エンコード）を生成し、`_sd_alg`（既定 `"sha-256"`）によるダイジェスト配列 `_sd` を構成する。
- `claim.iss`、`claim.vct` 等のメタデータクレームがシート上に定義されていればそれを使用し、未定義の場合は生成ダイアログの設定値で補完する。
- **署名鍵の扱い**:
  - ワークブック（`.xlsx`）内には秘密鍵を保存しない。
  - デモ・検証用途としてアプリ内でテスト用のエフェメラル鍵ペア（Issuer 用 ES256 鍵、および Holder Key Binding `cnf.jwk` 用 ES256 鍵）を自動生成して署名できるモードと、外部 JWK を読み込んで署名するモードの双方を用意する。
- シート上の `disclose.<path>` チェックボックスの状態を反映した SD-JWT（`<Issuer-signed JWT>~<Disclosure 1>~...~`）およびデコード済みペイロード・Disclosures 一覧をプレビュー・コピー・エクスポートできるようにする。

## 3. OID4VP によるクレデンシャル提示

- **Authorization Request の受け取り**:
  - Verifier からの `openid4vp://...` URI または Authorization Request JSON（DCQL `dcql_query` / Presentation Exchange `presentation_definition`）を入力・解析する。
- **シート上の開示チェックボックスとの連動**:
  - Verifier が要求したクレームパスとシート上の `claim.*` / `sd.*` を照合し、対応する `disclose.<path>` チェックボックスを自動的に ON（またはハイライト）にする。
  - 利用者は Excel シート上でどの項目が開示されるかを視覚的に確認・変更したうえで提示を実行できる。
- **Key Binding JWT の生成と応答送信**:
  - 選択された Disclosures から `sd_hash` を計算し、`typ: "kb+jwt"`、`aud`（Verifier の `client_id`）、`nonce`、`iat`、`sd_hash` を含む Key Binding JWT を Holder 鍵で署名して末尾に付与し、`vp_token` を組み立てる。
  - `response_mode`（`direct_post` 等）に従い `response_uri` へ送信する機能に加え、ブラウザの CORS 制約やオフライン検証向けに生成した `vp_token` / Authorization Response パラメータのコピー機能を提供する。

## 実装段階と upstream への還元方針

1. **段階 1（フォームコントロール名の編集 UI）**: `ControlPropertiesPanel` で `FormControl.name` を表示・変更可能にし、`<control name="...">` と描画シェイプ `<xdr:cNvPr name="...">` へ保存する。完成した段階で upstream に Issue / PR を提示する。
2. **段階 2（規約バインディングと JSON 抽出）**: `ibis-doc-model` に `claim.*` / `sd.*` / `disclose.*` のスキャンと JSON クレームツリー構築ロジックを実装し、合成 XLSX fixture で単体テストを整備する。
3. **段階 3（SD-JWT 生成エンジンと UI）**: RFC 9901 準拠の Disclosure 生成・SHA-256 ダイジェスト計算・ES256 署名・プレビュー UI を実装する。
4. **段階 4（OID4VP 提示フロー）**: Authorization Request パーサー、シート上の `disclose.*` チェックボックス自動連動、Key Binding JWT 生成および `direct_post` 提示 UI を実装する。
5. **段階 5（スキーマ構造強化とオーサリング UX）**:
   - 1次元範囲（プリミティブ配列）および2次元表範囲（見出し付きオブジェクト配列、例: ISO 18013-5 `driving_privileges`）のバインディング解決、および `@` 文字列書式セルの文字列型維持。
   - 本人確認 (EUDI PID 風) および運転免許証 (ISO/IEC 18013-5 mDL 風) テンプレートからのワンクリック・シート生成。
   - 選択したセル／表範囲へのワンクリック `sd.*` / `claim.*` バインド（＋右隣セルへの `disclose.*` チェックボックス自動配置、左隣ラベルからの属性名自動推論）およびパネル上での双方向開示トグル。

## 4. 今後の拡張ロードマップ（段階 6 以降）

段階 5 までの基盤の上に、初心者でも迷わず使える簡易 UI を維持しつつ、以下の拡張を段階的に実装する。

1. **段階 6（カスタム SD-JWT スキーマの定義と管理）**:
   - 既知プロファイル（PID / mDL）だけでなく、ユーザーが任意のスキーマ（`vct`、属性パス、型、必須/選択開示フラグ、ネスト・表構造）をパネル上またはスキーマシートとして定義・保存・再利用できる機能。
   - シートの構造から逆に SD-JWT スキーマ（JSON Schema / Type Metadata 互換）を自動推論・エクスポートする機能。
2. **段階 7（既存 `.sd-jwt` / ISO 18013-5 `.mdoc` ファイルのシート表示・インポート）**:
   - `.sd-jwt`（Compact Serialization）や `.mdoc`（CBOR / COSE_Sign1 ベースの ISO/IEC 18013-5 モバイルドキュメント）を読み込み、属性・値・開示可否・ネスト配列（`driving_privileges` 等）をワークシート上に展開して閲覧・検証できる機能。
   - 署名・`iss`・`vct` / `docType`・ダイジェスト整合性の検証ステータスをセルやサイドバーに表示する機能。
3. **段階 8（VC 発行と VP 提示のフルフロー統合）**:
   - ワークシートで編集・定義したデータから直接発行者署名を付与して VC（`.sd-jwt` / OID4VCI 応答）として書き出す発行モードと、手元にある `.sd-jwt` / `.mdoc` を読み込んで必要な属性だけを「開示 / 非開示」で選んで Key Binding 付き VP（`vp_token` / QR・URI・`direct_post`）として提示する提示モードの統合。
   - 初心者向けには「テンプレートまたはファイルを開く → 開示項目を選ぶ → 出力（コピー/保存/提示）」の3ステップのみを前面に出し、鍵管理やプロトコル詳細は詳細設定（Advanced）に分離する。
