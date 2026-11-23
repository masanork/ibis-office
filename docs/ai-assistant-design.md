# Assistant (OpenAI互換API連携) 設計メモ

## MCP と共通の文書ツール

デスクトップ版の内蔵アシスタントは、DOCX・XLSX・PPTX で共通の操作を `ibis-doc-service` のカタログから取得する。MCP と内蔵アシスタントは同じスキーマ・入力検証・編集処理を使う。内蔵側は `document_id` を差し込む接続処理を持ち、XLSX ではアクティブシートを既定値として使う。ファイルを開く・保存する・書き出す操作は MCP セッションに属する。

### 編集の確定

- **MCP** は、編集ツールを呼ぶたびにセッション内の文書へ適用する。呼び出し一回が Undo 一回で、ファイルへの保存は `app_save_document` で行う。
- **内蔵アシスタント** は、開いている文書から変更案を作り、後続のツールもその変更案を読み取る。適用時は元の文書の状態を再確認し、編集を一回の Undo にまとめる。停止・通信失敗・破棄では変更案を捨てる。
- **Markdown・Mermaid のソース編集** は、対象と元のソースを保持して内蔵側でレビューする。適用時に対象とソースが一致することを確認する。

ウィンドウや選択状態に関わる操作はシェルで扱う。法制執務やスライド複製などの内蔵固有ツールを今後 MCP に公開する場合は、対象を明示できる引数にしてカタログへ加える。リンク付きグラフ作成は `xlsx_create_chart` として共通化済みで、元データと配置先を個別に指定できる。

## フォルダーを参照する

デスクトップ版の「フォルダーを開く」で、左側に配下の資料が表示される。資料を開いて編集すると、そのファイルに保存する。「更新」で一覧に新規作成や削除を反映できる。ルートに `index.md` があれば一覧の先頭に出し、アシスタントにも参照の入口として知らせる。

内蔵アシスタントは、同じ一覧に対して `folder_list`、`folder_search`、`folder_read` を使う。Office 文書は読み取り専用の文書サービスを介して参照し、アクティブ文書の編集中の内容は既存の文書ツールで参照する。検索はローカルで行い、ツールの返したパス・抜粋は設定されたモデルに送信する。参照したファイルは会話内から開ける。取得できた位置情報があれば、段落・シート範囲・スライドも表示する。

開いたフォルダーが Git 作業ツリー内にある場合は、`git_status` で未コミットの変更を、`git_log` でファイルまたはフォルダーの履歴を参照できる。履歴は最初の親を辿り、`git_diff` は指定した２つのコミット間でファイル１件を比較する。Office 文書は Markdown に抽出した内容を比較する。返すパスは開いたフォルダーからの相対パスで、履歴と差分にはコミット SHA を付ける。

## ローカルの coding agent から GUI をデバッグする

macOS と Linux では、実行ファイルを子プロセスとして直接起動する。macOS のインストール済みアプリなら実行ファイルは
`/Applications/Docs.app/Contents/MacOS/ibisOffice`。標準入力と標準出力をパイプでつなぎ、標準エラーはログとして扱う。

```sh
ibisOffice --assistant-stdio sample.xlsx
```

専用の GUI ウィンドウで Assistant が動く。標準出力の `ready`（`version: 1`）で接続完了。
`loading: true` の場合、最初のファイルが開くまで `document_opened` を待つ。読み取りエラーは `document_error` で通知する。
`document_blocked`（`reason: "password"`）が届いたら、GUI でパスワードを入力する。
ファイルを省略して起動した場合は、対象ウィンドウで文書を開いてから送信する。

標準入力は1行1 JSON。各要求に `id` を付ける。1行は改行込みで最大256 KiB。上限を超えると `protocol_error` を返して入力を終了する。

デモでは次の3行を順に送る。各要求の成功応答を待ち、入力欄の内容を見せたい場合は `set_input` と `send` の間を空ける。

```jsonl
{"id":1,"op":"open"}
{"id":2,"op":"set_input","text":"Data シートの A1 に確認済みと入力して"}
{"id":3,"op":"send"}
```

`open` で Assistant サイドバーを開く。`set_input` は表示中の入力欄の全文を置き換える。空文字で消去できる。
入力後は GUI からも編集・送信できる。`send` は送信時点で入力欄にあるテキストを GUI の送信ボタンと同じ処理で送る。
編集案が出ている間や応答中も入力欄に次の指示を準備できる。送信はターンの終了と編集案の解決を待つ。
入力欄を経由せずに送信する場合は `{"id":4,"op":"prompt","text":"Data シートの範囲を教えて"}` を使う。

- 要求への応答には同じ `id` と `ok` が入る。`open` と `set_input` の成功応答はこの2項目。実行ターンを扱う操作の成功応答には `run_id` が入り、失敗応答には `error` が入る。同期イベントが応答より先に届くこともある。
- 停止・適用・破棄は `{"id":5,"op":"stop","run_id":0}` のように送る。`op` は `stop`、`apply`、`discard` のいずれか。`run_id` は `send` または `prompt` の成功応答、あるいは GUI 送信時の `started` イベントから取得する。
- `started`、`delta`、`message`、`tool_call`、`tool`、`review` で、送信内容、応答断片、確定した応答テキスト、ツールの実行前の引数、実行結果、GUI と同じ編集プレビューを参照できる。各イベントに `run_id` が入る。
- `completed`、`stopped`、`error` でターンの終了を通知する。`completed` の `review: true` は適用または破棄を待っている状態。GUI と標準入力のどちらから操作しても `review_resolved` の `outcome` に `applied`、`discarded`、`error` のいずれかが入る。
- Assistant の履歴を消すと `reset` が届く。接続はそのウィンドウに属する。標準入力を閉じても、GUI から操作を続けられる。

`apply` は編集中の文書に反映する。保存は通常の GUI 操作で行う。実モデルとローカルのモック API のどちらにも、下記の環境変数で接続できる。標準出力にはプロンプトやツールが読んだ文書内容が含まれるため、agent 側でログを保存する際は取り扱いに注意する。

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

#### デスクトップ版の環境変数

アプリの実行ファイルを、環境変数を設定したシェルから起動する。環境変数のキーは設定画面に表示せず、設定ファイルにも保存しない。

| 接続先 | `IBIS_AI_PROVIDER` | 認証 | モデルとリージョン |
| --- | --- | --- | --- |
| Claude API | `anthropic`（`claude` も可） | `ANTHROPIC_API_KEY` | `ANTHROPIC_MODEL` |
| Amazon Bedrock の Claude | `bedrock` | `AWS_BEARER_TOKEN_BEDROCK` または AWS CLI プロファイル | `ANTHROPIC_MODEL` にモデル ID または推論プロファイル ID。リージョンは `AWS_REGION` または `AWS_DEFAULT_REGION` |
| OpenAI 互換 API | `openai` | GUI のキー、`IBIS_AI_API_KEY`、`OPENAI_API_KEY` の順 | `IBIS_AI_BASE_URL` と `IBIS_AI_MODEL` が GUI の値に優先 |

`IBIS_AI_PROVIDER` を指定すると、その接続先を選ぶ。省略時に `CLAUDE_CODE_USE_BEDROCK=1` があれば Bedrock を選ぶ。両方とも省略した場合は保存済みの GUI 設定または OpenAI 互換用のキーがあれば OpenAI 互換 API を選ぶ。それらが空の場合は、Claude API と Bedrock のキーのうち一方だけが設定されていれば、その接続先を選ぶ。両方設定した場合は `IBIS_AI_PROVIDER` を指定する。API キーがなく `IBIS_BEDROCK_MODEL_ID` がある場合は Bedrock の AWS プロファイル認証を選ぶ。

Bedrock は [Bedrock Runtime の Anthropic Messages API](https://docs.aws.amazon.com/bedrock/latest/userguide/inference-messages-api.html) に API キーまたは AWS CLI プロファイルで接続する。GUI の保存項目は OpenAI 互換 API 用。ブラウザー版の接続は同一オリジンの Worker が担当する。

Claude API と Bedrock のモデル指定は `ANTHROPIC_MODEL`、`IBIS_AI_MODEL`、`IBIS_BEDROCK_MODEL_ID`（Bedrock のみ）、`ANTHROPIC_DEFAULT_MODEL` の順で参照する。Bedrock では利用するモデル ID、推論プロファイル ID、または ARN を指定する。

OpenAI 公式エンドポイントで GPT-6.2 系を使う場合、Assistant はツール呼び出しに対応する Responses API を選択する。\`/responses\` まで含めたURLも設定できる。会話の継続に必要な出力項目はアプリ側で再送し、Responses API へのリクエストには \`store: false\` を指定する。

#### Bedrock に AWS IAM Identity Center (SSO) で接続

AWS CLI v2 でプロファイルを作成し、ログインしてから同じシェルでアプリを起動する。

```sh
aws configure sso --profile bedrock-sso   # プロファイル作成時のみ
aws sso login --profile bedrock-sso
export AWS_PROFILE=bedrock-sso
export CLAUDE_CODE_USE_BEDROCK=1
export ANTHROPIC_MODEL='利用できる Claude のモデル ID または推論プロファイル ID'
unset AWS_BEARER_TOKEN_BEDROCK
/Applications/Docs.app/Contents/MacOS/ibisOffice
```

アプリは AWS CLI の認証情報からリクエストごとに短期間有効な Bedrock トークンを作り、ストリーミングに使う。AWS のキーや生成したトークンはアプリの設定に保存しない。推論リージョンは `AWS_REGION`、`AWS_DEFAULT_REGION`、AWS プロファイルの `region` の順で参照する。SSO 用の `sso_region` と推論リージョンは別々に設定できる。既存の Bedrock API キーが環境変数にある場合は、そのキーを優先する。

### 3. `crates/ibis-office-shell`（LLM クライアント & サイドバー UI）

* OpenAI 互換 HTTP / SSE クライアント
  * GPUI の `http_client` と `cx.spawn` を用いて非同期 POST・SSE チャンクの逐次パースを行う
  * `choices[0].delta.content` をチャット欄にリアルタイム反映
  * `choices[0].delta.tool_calls` を `index` ごとに結合し、`finish_reason == "tool_calls"` でツールを実行して `role: "tool"` として再送する
  * デスクトップ版はモデルがツールを要求する間、結果を返して応答を継続する。ユーザーは「停止」で中断でき、通信エラーや中断時は未適用の変更案を破棄する
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
