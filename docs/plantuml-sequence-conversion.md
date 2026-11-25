# PlantUML シーケンス図の共有 IR ネイティブ描画と Mermaid 変換

追加 crate を使わず、内蔵 Mermaid 描画器（`crates/ibis-formats/src/mermaid/`）のレイアウト・SVG 描画資産を共有して PlantUML シーケンス図を扱うための 2 つの経路と対応境界をまとめます（関連: #188）。

- **共有 IR ネイティブ描画（`render_plantuml_sequence_native`）**: PlantUML シーケンス図を直接 [`SeqDiagram`](../crates/ibis-formats/src/mermaid/sequence.rs) IR へ構築し、既存の `sequence.rs` SVG 描画器で描画します。Mermaid テキスト記法に表現がない `== Divider ==`・`... Delay ...`・`ref over`・境界ゲート・`hide footbox`・`group`・`hnote`/`rnote`・`autonumber` 制御までレイアウト重複なしで描画できます。
- **Mermaid テキスト移行変換（`convert_plantuml_sequence` / `render_plantuml_sequence`）**: 外部の Mermaid 環境（GitHub / mermaid.js 等）へ持ち出すための標準 `sequenceDiagram` テキストへ変換します。
- **検証用 CLI**:
  - Mermaid 移行確認: `cargo run --locked -p ibis-formats --example plantuml_sequence_convert -- <input.puml> [--svg <out.svg>]`
  - 共有 IR ネイティブ描画: `cargo run --locked -p ibis-formats --example plantuml_sequence_convert -- <input.puml> --native --svg <out.svg>`

どちらの経路も **原本（`original`）をそのまま保持**し、対応外または一部情報が落ちる構文を **1 始まりの行番号・種別（`kind`）・原文抜粋・理由**付きで返します。

## 1. 両経路で共通に対応する基本範囲

| 領域 | PlantUML 構文 | ネイティブ IR | Mermaid テキスト移行 |
| --- | --- | --- | --- |
| 外枠・注釈・メタ情報 | `@startuml`/`@enduml`、`'`、`/' ... '/`、単行/複数行 `title`、`autonumber` | 対応 | 対応 |
| 参加者 | `participant` / `actor` / `boundary` / `control` / `entity` / `database` / `collections` / `queue`、両方向の `as`、トップレベル `order N`、`url of X is [[url title]]`、`box ... end box` | 対応（識別子変換不要） | 対応（識別子を自動サニタイズ） |
| ライフサイクル | `create` / `destroy` / `activate` / `deactivate`、`++` / `--` / `--++` / `++--` / `**` / `!!`、`return [label]`（`++` および直前メッセージ後の `activate` に対応） | 対応 | 対応 |
| メッセージ矢印 | 実線・破線・開矢印・失われたメッセージ・双方向・始点/終点/両端 `o`・左右逆向き・片矢印（`-\` / `-/` 等）・`-[dashed]->` / `-[dotted]->` / `-[plain]->` | 対応 | 対応 |
| 注記・基本分岐 | 単行/複数行 `note left/right of` / `over`（1〜3 者以上）・暗黙 `note left`/`right`、`alt`/`else`/`opt`/`loop`/`par`→`and`/`break`/`critical`/`option`/`end` | 対応 | 対応 |

## 2. Mermaid テキスト記法にないため共有 IR ネイティブ描画でのみ対応するもの

以下は Mermaid 標準のテキスト記法には存在しない（または近似になる）ため、**Mermaid テキスト移行では行番号付き診断を出し、共有 IR ネイティブ描画（`render_plantuml_sequence_native`）ではそのまま描画**します。

| 構文 | 共有 IR ネイティブ描画 | Mermaid テキスト移行 |
| --- | --- | --- |
| `hide footbox` | 対応（下部アクター枠を省略） | 非対応（`styling` 診断） |
| `== セクション ==` | 対応（二重線＋中央見出し） | 非対応（`divider` 診断） |
| `... [待機] ...` | 対応（ライフライン破断＋中央注釈） | 非対応（`delay` 診断） |
| `ref over A, B : ...`（複数行含む） | 対応（`ref` フレーム描画） | 非対応（`reference` 診断） |
| 境界ゲート `[-> A` / `A ->]` / `[<- A` / `A <-]` | 対応（左右端からの境界矢印） | 非対応（`syntax` 診断） |
| `group 任意ラベル` / フラグメント背景色 `#Color` | 対応（`group` タグ＋背景色） | `opt` 近似（`fragment` / `styling` 診断） |
| `hnote` / `rnote` および注記の `#Color` | 対応（六角・角丸なし・塗り色） | 通常 `Note` へ変換（`styling` 診断） |
| `autonumber <start> [step]` / `stop` / `resume` | 対応（番号制御を反映） | 単純 `autonumber` 化・`stop`/`resume` は診断（`autonumber`） |
| `note across` | 対応（全参加者確定後に両端へ展開） | 既知範囲で近似（`layout` 診断） |

## 3. 共有 IR ネイティブ描画でも対象外とするもの（割り切る範囲）

| 種別（`kind`） | 対象構文の例 | 方針と理由 |
| --- | --- | --- |
| `macro` | `!include`、`!define`、`!procedure` / `!function`、`!if` / `!endif`、`!theme`、`!pragma`、`$macro(...)` | テキスト展開言語・外部ファイル参照はオフライン安全性の観点からも展開しない |
| `timing` | `robust`、`concise`、`clock`、`binary`、`analog`、`mode`、`@0` / `@+100` | 既存 IR とレイアウトモデルが異なるため `output: None`＋行番号診断 |
| `diagram-type` | `class` / `state` / `usecase` / `er` / `activity` / `@startmindmap` / `@startwbs` / `@startgantt` 等 | Phase 1 のシーケンス IR では `output: None`。Phase 2・3 で既存の `class.rs` / `state.rs` / `usecase.rs` / `er.rs` / `flowchart.rs` / `mindmap.rs` / `treeview.rs` / `gantt.rs` へ段階的に共有 IR 接続する予定 |
| `styling` | `skinparam`、`<style> ... </style>`、`hide`/`show`（`hide footbox` 除く）、参加者/矢印/活性化バー/`else` の `#Color`・`<<stereotype>>`・`bold` | 構造の描画を優先し、未反映装飾を行番号で報告 |
| `layout` | `\|\|\|`、`newpage`、`header` / `footer` / `legend` / `caption`、`scale`、Teoz 並列 `& ...`、横並び `/ note ...`、`box`/`create` 上の `order N` | 単一 SVG・1 イベント 1 行モデルと構造が異なるため診断を記録 |
| `syntax` | `autoactivate`、先行活性化のない `return`、対応のない `else` / `end` / 未閉じブロック、その他未解釈行 | 行番号付きで報告 |
