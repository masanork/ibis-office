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

### フォルダーに同梱した Agent Skills

開いたフォルダーの `.agents/skills/<名前>/SKILL.md` を探索する。フォルダー名と YAML の `name` を揃え、`description` を付ける。OKF バンドルでは、`type` のない `SKILL.md` をコンセプトと同じバンドルで管理できる。配置や更新はファイルまたは Git で行い、フォルダーの「更新」で一覧に反映する。

Assistant のスキル一覧で名前と説明を確認できる。選択すると本文が表示され、次に送信するターンにだけ読み込まれる。本文からの相対リンクは、許可されたフォルダー内の資料と選択したスキルの `references/` に解決できる。スキルは既存の文書ツールを使い、編集は既存の確認フローを通る。`scripts/` の実行や `allowed-tools` による権限付与は扱わない。

形式と配置の慣例は [Agent Skills 仕様](https://agentskills.io/specification) と [実装ガイド](https://agentskills.io/client-implementation/adding-skills-support) を参照。

### 作業フォルダーの指示とメモリ

Assistant のフォルダー欄で `AGENTS.md` と `MEMORY.md` を個別に有効にできる。フォルダーを開いた時点ではどちらも無効。選択はフォルダーを閉じるか、別のフォルダーを開くまで保持され、GUI と MCP のターンで共通に使う。MCP の `assistant_status` は有効状態を返す。

`AGENTS.md` は開いたルートと、アクティブ文書がその配下にある場合は文書の親ディレクトリまで順番に読み込む。より文書に近い指示が優先され、現在のユーザー指示と既存の権限が上位にある。ルート外の文書には開いたフォルダーのルート指示だけを参照コンテキストとして渡す。指示の合計上限は24 KiB。最も近いファイルから予算を割り当てる。配置と優先順位は [AGENTS.md のガイド](https://agents.md/) に従う。

`MEMORY.md` はフォルダールートに置く任意のメモで、決定事項、継続タスク、プロジェクト内の用語などを利用者がファイルとして編集・共有できる。上限は16 KiB。明示的に有効にしたファイルの内容は設定済みモデルに送信する。Assistant は会話をメモリに自動保存せず、どちらのファイルも有効化によって書き込み権限を追加しない。

デスクトップ版は、会話とツール結果の原本をセッション内に保持する。モデルに送る入力が予算に近づいたら、先にツール結果の出力を古いものから順に縮小する。それでも大きい場合は、古いターンを要約に置き換え、直近のターンと未処理のユーザー指示を送る。以降の圧縮では前回の要約を更新する。元の会話は「クリア」まで保持され、要約に失敗しても残る。圧縮が行われるとツールログと MCP イベントで通知する。

入力予算は、モデルごとに設定ファイルの `ai.context_tokens.<モデルID>` でコンテキスト窓を指定できる。`IBIS_AI_CONTEXT_TOKENS` を指定するとそれを優先する。不明な場合は65,536トークンを仮定し、25%の余裕を残して、メッセージとツール定義のサイズを概算する。1ターンのツール呼び出しは16ラウンド・合計64件を上限とし、超過時は変更案を破棄する。

## ローカルの coding agent から起動中の Assistant に接続する

macOS と Linux では、利用者だけがアクセスできるディレクトリにソケットを作り、GUI を起動する。

```sh
session_dir=$(mktemp -d /tmp/ibis-assistant.XXXXXX)
ibisOffice --assistant-socket "$session_dir/assistant.sock" sample.xlsx
```

agent の MCP サーバー設定に `ibisOffice --mcp --assistant-socket <絶対パス>` を指定する。
MCP クライアントは終了・再起動しても同じ GUI ウィンドウにつなぎ直せる。
ソケットは起動時に明示したウィンドウに属し、ウィンドウを閉じるまで有効。

- `assistant_status` は開いている文書、実行中のターン、確定待ちの変更案、利用可能なスキルの名前と説明を返す。`document_ready: true` になってから指示を送る。
- `assistant_prompt` に `{"text":"Data シートの A1 を確認して"}` を渡すと `run_id` が返る。利用者がスキルを選んだときは、その名前を任意の `skill` 引数に渡す。
- `assistant_result` に `{"run_id":0}` を渡すとターンの状態とイベントが返る。次の呼び出しでは、受け取った `cursor` を `after` に渡せば続きから読める。適用または破棄すると、状態に `applied` または `discarded` が記録される。
- ユーザーが適用を明示した場合、`assistant_apply` に確定待ちの `run_id` を渡すと変更案を適用できる。対象は GUI の「元に戻す」で取り消せる文書内の編集。ソースの変更とファイルの保存は対象外。GUI からも変更案の適用・破棄ができる。
- `assistant_reset` は利用者が会話のクリアを依頼した場合に使う。実行中や変更案の確定待ちには拒否する。次のターンの会話履歴は空になり、フォルダー設定と取得済みの実行結果の履歴は保持する。

`--read-only` で MCP を起動した場合は状態と結果の取得のみ公開する。最近の256イベントと32ターンの概要をウィンドウ内に保持する。イベントの履歴が範囲外になると `history_truncated` を返す。結果に含まれる文書の値やモデル出力はデータとして扱う。ファイル操作用の既存MCPツールには従来どおり `--allow-dir` が必要。

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

#### サイドバーから接続先を選ぶ

「設定」のプルダウンで接続先を選び、「接続」を押して反映する。設定が不足している候補には「未設定」と表示し、接続ボタンを押すと必要な設定を案内する。反映後はサービス名、接続先ホスト、モデルがサイドバーに表示される。AWS プロファイルを使う場合は接続時に AWS CLI の認証情報を確認する。モデルの推論はメッセージを送ったときに実行する。

OpenAI 互換 API では、GUI または `IBIS_AI_BASE_URL` に接続先が指定されていない場合、`OPENAI_BASE_URL` と `OPENAI_API_KEY` を使える。Google Gemini はプルダウンで選び、`GEMINI_API_KEY` と `GEMINI_MODEL` を設定する。Google の OpenAI 互換エンドポイントに接続する。

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

アプリは AWS CLI の認証情報を使って Bedrock Runtime のストリーミングリクエストを SigV4 で署名する。AWS の認証情報はアプリの設定に保存しない。推論リージョンは `AWS_REGION`、`AWS_DEFAULT_REGION`、AWS プロファイルの `region` の順で参照する。SSO 用の `sso_region` と推論リージョンは別々に設定できる。既存の Bedrock API キーが環境変数にある場合は、そのキーを優先する。

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
