# Assistant (OpenAI互換API連携) 設計メモ

## MCP と共通の文書ツール

デスクトップ版の内蔵アシスタントは、DOCX・XLSX・PPTX で共通の操作を `ibis-doc-service` のカタログから取得する。MCP と内蔵アシスタントは同じスキーマ・入力検証・編集処理を使う。内蔵側は `document_id` を差し込む接続処理を持ち、XLSX ではアクティブシートを既定値として使う。ファイルを開く・保存する・書き出す操作は MCP セッションに属する。

### 編集の確定

- **MCP** は、編集ツールを呼ぶたびにセッション内の文書へ適用する。呼び出し一回が Undo 一回で、ファイルへの保存は `app_save_document` で行う。
- **内蔵アシスタント** は、開いている文書から変更案を作り、後続のツールもその変更案を読み取る。適用時は元の文書の状態を再確認し、編集を一回の Undo にまとめる。停止・通信失敗・破棄では変更案を捨てる。
- **Markdown・Mermaid のソース編集** は、対象と元のソースを保持して内蔵側でレビューする。適用時に対象とソースが一致することを確認する。

ウィンドウや選択状態に関わる操作はシェルで扱う。法制執務やスライド複製などの内蔵固有ツールを今後 MCP に公開する場合は、対象を明示できる引数にしてカタログへ加える。リンク付きグラフ作成は `xlsx_create_chart` として共通化済みで、元データと配置先を個別に指定できる。

以下は導入時の設計メモ。

## 目的

デスクトップ版 `ibis-office` に、OpenAI 互換 API（Chat Completions + SSE ストリーミング + Tool Calling）を用いた編集支援パネル（Assistant サイドバー）を追加する。
UI Automation や画面操作ではなく、既存の `ibis-doc-model` のコマンド層（`Command` / `SheetCommand`）へ直接接続することで、再計算・Undo/Redo と整合した高速かつ安定した自動化を実現する。

## スコープ（Phase 1: 最小構成）

* **対象環境**: デスクトップ版のみ（Web 版プロキシ・認証はスコープ外）
* **対象形式**: 表計算（`.xlsx`）を最初のターゲットとする
* **設定管理**: ローカル保存（`ibis-office-base` の `settings.txt` + 環境変数フォールバック）
* **応答形式**: SSE ストリーミング（`stream: true`）による逐次表示 + Tool Calling ループ

## アーキテクチャ

### 1. `crates/ibis-doc-model`（UI 非依存ロジック・単体テスト対象）

* シート状態の軽量ダンプ生成（使用範囲・選択範囲の抜粋テキスト / JSON）
* OpenAI Tool Schema（JSON Schema）の定義
* Tool 引数（JSON）から `Command::Sheet(SheetCommand::...)` への変換

### 2. `crates/ibis-office-base`（設定キー）

既存の `settings.rs`（`key=value` 形式）を利用する。

* `ai.base_url`: OpenAI 互換エンドポイント（例: `https://api.openai.com/v1`, `http://localhost:11434/v1`）
* `ai.model`: モデル名（例: `gpt-6.1-terra`）
* `ai.api_key`: API キー（ローカル LLM 等で空の場合は Bearer ヘッダーを省略。環境変数 `IBIS_AI_API_KEY` / `OPENAI_API_KEY` をフォールバックとして参照可能）

### 3. `crates/ibis-office-shell`（LLM クライアント & サイドバー UI）

* OpenAI 互換 HTTP / SSE クライアント
  * GPUI の `http_client` と `cx.spawn` を用いて非同期 POST・SSE チャンクの逐次パースを行う
  * `choices[0].delta.content` をチャット欄にリアルタイム反映
  * `choices[0].delta.tool_calls` を `index` ごとに結合し、`finish_reason == "tool_calls"` でツールを実行して `role: "tool"` として再送する
  * 無限ループ防止のため 1 ターン最大 8 ステップで打ち切り、UI から中断（停止）できるようにする
* 右サイドバー UI
  * ヘッダー: 接続中モデル名、設定ボタン
  * メッセージ履歴: ユーザー発言、AI ストリーミング応答、実行した Tool の要約表示
  * フッター: 入力欄、送信 / 停止ボタン

### 4. `crates/ibis-office-xlsx`（表計算エディタとの接続）

* `sidebars.rs` へ右サイドバー（例: `sidebar.ai-assistant`）を登録
* アクティブシート・選択範囲の取得と、生成された `Command` の適用
* 1 ターンの指示で複数回 Tool が呼ばれた場合、可能な限り `Command::Group` としてまとめて Undo 1 回（`⌘Z`）で元に戻せるようにする

## 最初の Tool セット（表計算・最小 6 個）

| Tool 名 | 概要 | 対応するモデル操作 |
|---|---|---|
| `get_workbook_overview` | シート一覧、アクティブシート、使用範囲、現在選択範囲を取得 | `Workbook` / 選択状態参照 |
| `read_range` | 指定範囲（例: `A1:D20`）の値・数式・表示テキストを取得（上限セル数あり） | `Sheet` 参照 |
| `set_cells` | 指定セルへ値または数式（`=SUM(...)` 等）を一括書き込み | `SheetCommand::SetCells` |
| `set_format` | 範囲指定で太字・文字色・背景色・数値書式等を設定 | `SheetCommand::SetFormat` |
| `clear_cells` | 指定範囲の値/数式、または書式をクリア | `SheetCommand::SetCells` (`Clear`) / `ClearFormats` |
| `select_range` | 編集箇所や注目セルへ選択枠を移動 | エディタ選択状態更新 |

## 段階的な実装ステップ

1. **Step 1**: `ibis-doc-model` に表計算 AI Tool ブリッジ（範囲パース、シート要約、Tool 引数 → `SheetCommand` 変換）と単体テストを実装
2. **Step 2**: `ibis-office-base` / `ibis-office-shell` に AI 設定キーと OpenAI 互換 SSE クライアント（SSE パーサ単体テスト付き）を実装
3. **Step 3**: `ibis-office-shell` / `ibis-office-xlsx` に右サイドバー UI と Tool 実行ループを接続し、実機動作確認
