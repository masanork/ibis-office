# `SORANE.*` 汎用 JSON 展開関数 ＆ 名簿テンプレート設計書

> **ステータス**: レビュー待ち（設計フェーズ・2026-11-26）  
> **関連ドキュメント**: [Spot Relay プロトコル設計書](spot-relay-protocol.md) / [シート正規化・表記ゆれ突合の設計](sheet-normalization-sources.md) / [フォーム入力支援 × Vault × VC 統合設計書](vault-form-vc-architecture.md)

---

## 1. 背景と設計原則

[Spot Relay](spot-relay-protocol.md) などの外部フォーム・JSONL・API から回収した構造化データを `soraneDocs` のスプレッドシートへ反映し、名簿整理・名寄せ・集計を行う仕組みを設計する。

本設計では以下の 3 原則を厳守する：

1. **表計算基盤へ業務ロジックを持ち込まない**
   - `ibis-calc` / `ibis-office-xlsx` に「避難所」「安否」「Spot」などの業務固有機能や専用ダイアログを組み込まない。
   - エンジンが提供するのは **「JSON をセル／配列へ展開する汎用関数」** と **「既存の表記ゆれ・異体字照合関数」** のみに留め、業務上の抽出・分類・集計ルールはすべて **テンプレート側の数式** に閉じ込める。
2. **`SORANE.*` プレフィックスによる名前空間の統一**
   - `soraneDocs` 由来の独自ワークシート関数は `SORANE.` プレフィックス（XLSX 保存時は `_xlfn.SORANE.*`）に統一する。
   - 既設の `IBIS.FIELDMATCH` / `IBIS.RECORDMATCH` も `SORANE.FIELDMATCH` / `SORANE.RECORDMATCH` へ改名し（`IBIS.*` は当面互換エイリアスとして維持）、本設計で追加する汎用関数も同じ名前空間に揃える。
3. **取り込み元との疎結合**
   - シート側が扱うのは「プレーンな JSON（または JSONL / CSV）」のみとし、Spot 復号出力・VC クレーム検証結果・AI 評価 JSONL・外部 Webhook 出力のいずれであっても同じテンプレート基盤で処理できる構造にする。

---

## 2. `SORANE.*` 汎用データ展開関数の仕様

### 2.1 関数一覧

| 関数名 | 最小/最大引数 | 揮発性 | 概要 |
| :--- | :--- | :--- | :--- |
| `SORANE.JSONVALUE` | 2〜3 | `false` | 単一 JSON 文字列から指定パスの値をスカラーとして取得する |
| `SORANE.JSONTABLE` | 2〜3 | `false` | JSON 配列または 1 セル 1 JSON の範囲を 2 次元スピル配列へ展開する |
| `SORANE.JSONKEYS` | 1〜2 | `false` | JSON オブジェクト（群）のキーまたはリーフパス一覧を縦配列で返す |

### 2.2 パス記法（Path Syntax）
複雑な JSONPath 全体ではなく、数式内で読み書きしやすい **軽量ドット＋添字パス** を採用する。

- **オブジェクト参照**: `"name"`, `"answers.name"`, `"meta.received_at"`
- **配列添字**: `"items[0].title"`, `"[0].id"`
- **ルート自身**: `""` または `"$"`（必要に応じて先頭 `"$."` は許容して読み飛ばす）
- **キー名に `.` や `[` を含む場合**: `["key.with.dot"]` のブラケットクォート形式のみ許容

### 2.3 `SORANE.JSONVALUE(json_text, path, [default])`
- **第 1 引数 `json_text`**: JSON 文字列。セル範囲が渡された場合は動的配列ブロードキャスト（または配列数式評価）に従い要素ごとに評価する。
- **第 2 引数 `path`**: 抽出パス文字列。
- **第 3 引数 `default`（省略可）**: パスが存在しない、または JSON が `null` の場合の戻り値。省略時は `""`（空文字）を返す（名簿数式で `#N/A` 連鎖を防ぐため）。
- **型変換ルール**:
  - JSON `string` → `CellValue::Text`
  - JSON `number`（有限値）→ `CellValue::Number`
  - JSON `boolean` → `CellValue::Bool`
  - JSON `null` / 未存在 → `default`（省略時は `""`）
  - JSON `array` / `object` → コンパクト JSON 文字列（`CellValue::Text`）
- **エラー処理**:
  - `json_text` が空文字または空白のみ → `default`（省略時は `""`）
  - `json_text` が不正な JSON 構文 → `#VALUE!`
  - 引数にエラー値が含まれる場合 → そのエラー値を伝播

### 2.4 `SORANE.JSONTABLE(json_or_range, paths, [include_header])`
1 本の数式で「取り込み用 Raw 列」から一覧表をスピル生成するための主力関数。

- **第 1 引数 `json_or_range`**:
  - **単一文字列の場合**: JSON 配列 `[{...}, {...}]` として各要素を 1 行とみなす（単一オブジェクト `{...}` の場合は 1 行として扱う）。
  - **セル範囲 / 配列の場合**: 各セルを 1 レコードの JSON オブジェクト文字列として縦方向に走査する（例: `Raw!C2:C1000`）。
    - **空セルの扱い**: 末尾や途中の空セル・空白のみセルに対しては **出力行も全列 `""` の空行にするか、あるいはスキップするか** が重要になる。名簿の `Raw` シートと行番号を 1:1 対応させたいケース（例: `Raw!A2` の `envelope_id` と `名簿!A2` を並べる）が多いため、**「範囲入力時は行位置を保持し、空セル行は全列 `""` を返す」** を基本とし、不要行の圧縮は `=FILTER(名簿!A2:F1000, 名簿!A2:A1000<>"")` に委ねる設計とする。
- **第 2 引数 `paths`**:
  - 抽出パスの 1 次元配列またはセル範囲（例: `{"answers.name","answers.party_size","answers.shelter","answers.needs","submitted_at"}`、または見出し行の裏に置いたパス定義セル範囲 `B1:F1`）。
- **第 3 引数 `include_header`（省略可, デフォルト `FALSE`）**:
  - `TRUE` の場合、先頭行に `paths` の各要素（パス文字列）をヘッダ行として付加してスピルする。
- **出力**: `(行数) × (paths 長)` の 2 次元 `Value::Array`（`ibis-calc` の既存動的配列スピルで自動展開）。

### 2.5 `SORANE.JSONKEYS(json_or_range, [max_depth])`
- 未知のフォームや JSONL を `Raw` シートに貼った際、どのようなパスが存在するかを調べる補助関数。
- `max_depth`（デフォルト `2`）までのリーフパス（例: `answers.name`, `answers.party_size`, `submitted_at`）を出現順・重複なしで 1 列スピルする。

### 2.6 実装・性能面の考慮（`crates/ibis-calc`）
- `ibis-calc` はすでに `serde_json` をワークスペース依存に持つか確認の上、`serde_json::Value` ベースで純粋関数として実装する（外部ネットワークや I/O 一切なし、`volatile: false`）。
- `SORANE.JSONTABLE` は 1 行あたり 1 回 JSON をパースし、同じ行の全 `paths` を 1 パスで抜き出すため、列ごとに `JSONVALUE` を並べるよりパース回数を `1 / 列数` に削減できる。

---

## 3. 名簿テンプレート設計（汎用・非業務依存モデル）

テンプレートは「外部から取り込んだ生データ（Raw）を、正規化・名寄せ・抽出・集計する」汎用ワークブックとして構成する。業務別の差異（避難所受付、イベント受付、点検報告など）は **テンプレート上の見出し・パス定義・数式** のみを変えて対応する。

### 3.1 4 シート構成

```text
[1. Raw] ──(JSONTABLE)──▶ [2. 一覧 (List)] ──(FIELDMATCH / RECORDMATCH / FILTER)──▶ [3. 突合・抽出 (Review)]
                                    │
                                    └────────(UNIQUE / COUNTIFS / SUMIFS)─────────▶ [4. 集計 (Summary)]
```

#### シート 1: `Raw`（データ取り込み口）
外部ツール（Spot 受付ビュー等）から貼り付け・インポートする不変の生データ層。

| 列 | 見出し | 内容 |
| :--- | :--- | :--- |
| `A` | `record_id` | 重複排除キー（Spot の場合は `envelope_id`） |
| `B` | `source_ts` | 受信・記録時刻（ISO 8601） |
| `C` | `payload_json` | 1 行 1 件の JSON オブジェクト（例: `{"answers":{"name":"山田 太郎","party_size":3,...},"submitted_at":"..."}`） |

※ CSV で直接取り込む場合は、`Raw` シートにそのまま列展開し、`一覧` シート側が `Raw!C2:G1000` を直接参照する構成も取れる。

#### シート 2: `一覧`（JSON 展開後の正規化ビュー）
1 行目に「人間向け見出し」、2 行目（非表示または参照行）に「JSON パス」を置き、3 行目からスピル展開する。

- `A3`: `=FILTER(Raw!A2:A1000, Raw!A2:A1000<>"")`（ID 列）
- `B3`: `=SORANE.JSONTABLE(FILTER(Raw!C2:C1000, Raw!C2:C1000<>""), B2:F2)`
  - `B2:F2` に `answers.name`, `answers.party_size`, `answers.shelter`, `answers.needs`, `submitted_at` を配置しておくことで、**式の書き換えなしに列定義を変更可能**。

#### シート 3: `突合・抽出`（名寄せ・要確認・条件抽出）
既存の `SORANE.FIELDMATCH` / `SORANE.RECORDMATCH` と標準 `FILTER` を組み合わせる。

- **既存台帳や過去提出との表記ゆれ・異体字チェック**:
  - `=SORANE.FIELDMATCH(一覧!B3, 台帳!$A$2:$A$1000, "name")`
  - 複数項目（氏名・生年月日・住所など）を束ねる場合: `=SORANE.RECORDMATCH(F3:H3)`
  - `1`（要確認候補）の行だけ抽出: `=FILTER(一覧!A3:F1000, IFNA(一致判定列=1, FALSE), "候補なし")`
- **条件抽出（例: 備考・要配慮事項の記入がある行のみ）**:
  - `=FILTER(一覧!A3:F1000, 一覧!E3:E1000<>"", "該当なし")`

#### シート 4: `集計`（サマリ）
すべて Excel 標準の動的配列・集計関数だけで記述する。

- 拠点・グループ一覧: `=UNIQUE(FILTER(一覧!D3:D1000, 一覧!D3:D1000<>""))`
- 拠点別件数: `=COUNTIF(一覧!D3:D1000, A2#)`
- 拠点別合計人数: `=SUMIF(一覧!D3:D1000, A2#, 一覧!C3:C1000)`

---

## 4. `spot.sorane.dev` との連携フォーマット

`www/spot/` の受付ビュー（復号済み一覧）は、`soraneDocs` 側の `Raw` シートへそのまま渡せるよう以下の 2 形式をエクスポートできるようにする：

1. **JSONL / TSV クリップボードコピー（推奨・最速）**
   - `record_id<TAB>source_ts<TAB>payload_json` の TSV 形式でコピーし、`Raw` シートの `A2` へ 1 回の貼り付けで複数件を一括投入できる。
2. **UTF-8 BOM 付き CSV ダウンロード**
   - 展開済みのフラット表として即座に Excel / `soraneDocs` で開ける互換形式。

---

## 5. 段階的実装ロードマップ

| フェーズ | 内容 | 主な対象 |
| :--- | :--- | :--- |
| **Phase 1** | `SORANE.JSONVALUE` / `SORANE.JSONTABLE` / `SORANE.JSONKEYS` の実装と単体テスト・XLSX ラウンドトリップ確認 | `crates/ibis-calc`, `crates/ibis-office-xlsx`, `crates/ibis-formats` |
| **Phase 2** | `www/spot/` 受付ビューへの TSV（`Raw` 貼り付け用）コピー ＆ BOM 付き CSV ダウンロード追加 | `www/spot/`（worktree `feat/spot-relay-poc`） |
| **Phase 3** | 汎用名簿テンプレート（`.sheet.md` およびサンプル `.xlsx`）の作成と実データ投入検証 | `docs/samples/` / `www/samples/` |
