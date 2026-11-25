# PlantUML 基本シーケンス図から Mermaid への機械変換

追加 crate を使わず、既存の PlantUML 基本シーケンス図を Mermaid `sequenceDiagram` へ機械変換し、内蔵の Mermaid 描画器（`ibis_formats::mermaid`）で SVG 確認するための変換範囲と非対応境界をまとめます（関連: #188）。

- 実装: `crates/ibis-formats/src/mermaid/plantuml.rs`（`convert_plantuml_sequence` / `render_plantuml_sequence`）
- 検証用 CLI: `cargo run --locked -p ibis-formats --example plantuml_sequence_convert -- <input.puml> [--svg <out.svg>]`
- 変換結果（`PlantUmlConversion`）は常に **原本（`original`）をそのまま保持**し、対応外または一部情報が落ちる構文を **1 始まりの行番号・種別（`kind`）・原文抜粋・理由**付きで返します。

## 機械変換できる範囲（Mermaid `sequenceDiagram` に対応）

| 領域 | PlantUML 構文 | Mermaid 変換先 | 備考 |
| --- | --- | --- | --- |
| 外枠・注釈 | `@startuml` / `@enduml`、`'` 行コメント、`/' ... '/` ブロックコメント | `sequenceDiagram`、`%% ...` | `@enduml` 以降は無視 |
| メタ情報 | `title ...`、複数行 `title ... end title`、`autonumber` | `title ...`、`autonumber` | 複数行タイトルは ` / ` で連結 |
| 参加者 | `participant`、`actor`、`boundary`、`control`、`entity`、`database`、`collections`、`queue` | `participant` / `actor` / `@{ "type": ... }` | `"表示名" as 別名`・`別名 as "表示名"`・引用符付き名・ハイフン/空白/日本語識別子に対応 |
| 参加者の並び順・リンク・枠 | トップレベルの `order N`、`url of X is [[url title]]`、`box "Title" [#Color] ... end box` | 宣言順の安定ソート、`link X: title @ url`、`box ... end` | `order` の既定値は `0`。`box` 併用時や `create` 上の `order` は並び替えず `layout` 診断 |
| ライフサイクル | `create` / `destroy` / `activate` / `deactivate`、矢印略記 `++` / `--` / `--++` / `++--` / `**` / `!!`、`return [label]` | `create participant` / `destroy` / `activate` / `deactivate` / `+` / `-` / `B-->>-A: ...` | `return` は直前の `++` またはメッセージ直後の `activate` と対にして変換 |
| メッセージ矢印 | `->` / `-->` / `->>` / `-->>`、`<-` / `<--` / `<<-` / `<<--`、`->x` / `-->x` / `x<-` / `x<--`、`<->` / `<-->` / `<<->>` / `<<-->>`、`->o` / `o->` / `o->o` / `<-o` / `o<-` / `o<-o`（破線含む）、片矢印（`-\` / `-/` / `--\` / `--/` 等の左右）、`-[dashed]->` / `-[dotted]->` / `-[plain]->` | `->>` / `-->>` / `->` / `-->` / `-x` / `--x` / `<<->>` / `<<-->>` / `()` 端点 / `-)` / `--)` | 逆向き矢印は送受信を入れ替えて変換。ラベル内 `\n` は ` / ` に置換 |
| 注記 | `note left of` / `right of` / `over`（1〜複数参加者）、複数行 `note ... end note`、暗黙の `note left` / `note right` | `Note left of` / `right of` / `over A,B` | 3 者以上の `note over A, B, C` は両端 `Note over A,C` に変換 |
| 基本的な分岐・反復 | `alt` / `else`、`opt`、`loop`、`par`（内部の `else` は `and` へ）、`break`、`critical`（`option` / `else`）、`end` | 同名の Mermaid フラグメント | 空ラベルのフラグメントは `-` で補完 |

## Mermaid では表現できないため対象外とするもの（諦める範囲）

Mermaid `sequenceDiagram`（および内蔵描画器）に対応する意味構造がないため、以下は **変換せず原本保持＋行番号付き診断**にとどめます。

### 変換自体を中止するもの（`mermaid: None`）

| 種別（`kind`） | 対象構文の例 | 対象外とする理由 |
| --- | --- | --- |
| `timing` | `robust`、`concise`、`clock`、`binary`、`analog`、`mode`、`@0` / `@+100` | Mermaid にタイミング図相当がなく、時間軸・状態遷移波形・制約の意味が失われるため |
| `diagram-type` | `@startgantt` / `@startmindmap` / `@startjson` / `@startsalt` 等の非 `@startuml`、`class` / `interface` / `enum` / `state` / `usecase` / `component` / `node` / `package` / `[*] -->` 等 | シーケンス図以外の UML・図種は本変換の対象外のため |

### 部分変換しつつ行番号付き診断を出すもの（`unsupported` に記録）

| 種別（`kind`） | 対象構文の例 | 振る舞いと理由 |
| --- | --- | --- |
| `macro` | `!include`、`!define`、`!procedure` / `!function` ... `!endprocedure`、`!if` / `!endif`、`!theme`、`!pragma`、`$macro(...)` | プリプロセッサ展開や外部ファイル読込は行わない（プロシージャ本体はスキップ） |
| `divider` | `== 後半 ==` | Mermaid `sequenceDiagram` にセクション区切り線がないためスキップ |
| `delay` | `...`、`... 5分後 ...` | Mermaid に時間経過（遅延）記号がないためスキップ |
| `reference` | `ref over A, B : ...`（複数行 `ref ... end ref` 含む） | 内蔵 Mermaid 描画器に相互作用参照フレーム（`ref`）がないためスキップ |
| `layout` | `\|\|\|` / `\|\|45\|\|`、`newpage`、`header` / `footer` / `legend` / `caption`、`scale`、Teoz 並列 `& ...`、`/ note ...`、`note across`、`box` 内や `create` 上の `order N` | ページ分割・余白・並列配置・横並び注記は Mermaid に直接対応がない（`/ note` と `note across` は近似変換しつつ診断を記録） |
| `fragment` | `group カスタム見出し` | Mermaid に任意ラベルの `group` 枠がないため `opt` に近似しつつ診断を記録 |
| `autonumber` | `autonumber 10 5`、`autonumber "<b>[000]"`、`autonumber stop` / `resume` / `inc` | Mermaid の `autonumber` は開始番号・増分・書式指定や途中停止に対応しないため |
| `styling` | `skinparam`、`<style> ... </style>`、`hide` / `show`、参加者や矢印・活性化バー・フラグメント・注記の `#Color` / `<<stereotype>>` / `bold` / `thickness`、`hnote` / `rnote` | 配色・線種装飾・注記形状は落とすが、図の構造（参加者・矢印・分岐・注記本文）は可能な限り変換 |
| `syntax` | 境界ゲート矢印（`[-> A` / `A ->]`）、`autoactivate`、先行活性化のない `return`、対応のない `else` / `end` / 未閉じブロック、その他未解釈行 | Mermaid に直接の表現がないか、対応関係が閉じない構文として行番号を報告 |

## 運用上の目安

- `PlantUmlConversion::is_lossless()` が `true` の図は、追加 crate なしで Mermaid `sequenceDiagram` として等価に描画・移行できます。
- `unsupported` が空でない場合は、行番号付きの診断を見て「配色だけ落ちる（`styling`）」のか「意味のある構文が落ちる（`macro` / `divider` / `reference` / `timing` 等）」のかを確認したうえで、原本を保持するか手直しして Mermaid へ移行するかを判断します。
