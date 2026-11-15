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
- 初期スコープは「1シート＝1件のクレデンシャル（単票フォーム）」およびネストオブジェクトを対象とする。

### 値と型の解決規則

1. **フォームコントロール（`FormControl.name`）に一致する場合**
   - `Drop` / `List`: 内部の選択番号（`value: 1, 2, ...`）ではなく、入力範囲（`fmlaRange`）から解決した `items[value - 1]` の表示文字列を JSON 文字列とする（未選択 `0` または範囲外は省略）。
   - `CheckBox`: `checked == 1` を JSON の `bool` とする。
   - `Radio`: 同一グループ（`first_button` 区切り）内で `checked == 1` のボタンのキャプション文字列（`text`）を JSON 文字列とする。
   - `Spin` / `Scroll`: `value` を JSON の数値とする。
2. **定義された名前（`DefinedName.name`）の単一セルに一致する場合**
   - そのセルを `fmlaLink`（リンクするセル）として参照する `Drop` / `List` コントロールが同一シート上に存在する場合は、セル内のインデックス番号ではなくコントロール側の表示文字列 `items[value - 1]` を優先して解決する。
   - セル内チェックボックス（`CellStyle::checkbox`）または `CellValue::Bool` は JSON の `bool` とする。
   - `CellValue::Number` でセルの表示形式が日付書式の場合は ISO 8601 日付文字列（`"YYYY-MM-DD"`）へ変換する（ただし `iat`、`exp`、`nbf` クレームは NumericDate 整数とする）。
   - 上記以外の数値・文字列はそのまま JSON の数値・文字列とし、空セルはクレームを省略する。

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
