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
- 表計算の `assistant_analysis_context` は、通常クリックで選んだ棒の系列、カテゴリ、参照セルと、各SQLレポートの適用済みパラメーターを返す。セルの数値は含めない。
- `assistant_chart_drill_down` は、棒を通常クリックして選んだ状態で、元の費目・値セル、⌘クリックの移動先、移動先から直接リンクされた集計根拠を返す。金額は数値と表示文字列の両方で返し、数式付きセルには数式も含める。現在適用済みのブックを参照する読み取り専用操作で、画面の選択は変わらない。移動先が単一の内部リンクならそこから右下の最大100行・32列、各範囲で最大100個の非空セル、根拠リンクは最大4件を返す。各範囲の応答は256 KiB、セル内の文字列は4,096文字まで。棒の元セルや取得済みの範囲に戻るリンクは除く。
- `assistant_preview_analysis` に `{"parameter_values":{"year":"r9"}}` のように変更する値だけを渡す。利用者が「SQLレポートの更新」を開いて元ファイルを割り当て、対象レポートを選択した状態で使う。モデルを呼び出さず、既存の更新プレビューを開始する。完了後は `assistant_analysis_context` の `report_preview` で変わるグラフを確認できる。適用はGUIの「更新」で行い、ブックがプレビューから変更されていれば再プレビューが必要。
- SQLレポートのプレビューが完了したら、読み取り専用の `assistant_analysis_preview_diff` を呼べる。引数なしでは、変わるグラフごとに絶対差額の上位3カテゴリを返す。カテゴリに複数系列がある場合は最大の差を示す系列を採用する。同じ系列ラベルとカテゴリラベルで対応づけ、ラベルが重複する点や数値のない点は順位から除く。応答のグラフは最大24件で、同じ上位3件が更新画面にも表示される。
- 応答の `shape_id`、`series`、`category` を指定すると、適用済みと候補それぞれの値、内訳、集計根拠を返す。0は数値として保持し、空欄、点の不在、ラベルの重複は状態で区別する。増減率は差を変更前の絶対値で割った百分率で、変更前が0の場合は空欄となる。数値は元セルから読み、元ファイルの再読込やSQLの再実行なしに、適用予定のスナップショットを参照する。
- 同じグラフ点の比較対象を分解するには、数値セルに内部リンクを置き、スクリーンヒントを `ibis:cohort` にする。リンク先を、列 `key`（文字列）と `amount`（数値）を持つ更新対象のテーブルの左上に設定し、比較対象だけを1キー1行で出力する。金額はグラフの元セルと同じ単位・同じ集計条件を使う。適用済みと候補の両方で合計が棒の値に一致した場合、継続・追加・除外の件数と金額寄与を更新画面とMCPの `cohort` に表示する。ゼロ金額の対象も数え、重複キー・空欄・合計不一致は状態で返す。
- `cohort` の `increases` と `decreases` は各5件まで返し、同額ではキーの昇順に並べる。`remainder` は残りの件数と金額差を含む。各行には適用済み・候補のシート、キーセル、金額セルを含め、片側の集計対象に含まれない場合はその側が `null` になる。プレビューの詳細取得に `contributor_key` を渡すと、順位外のキーも指定できる。適用後は棒を選択して `assistant_chart_drill_down` に同じ `contributor_key` を渡すと、現在の根拠行を取得できる。
- グラフを作らないレポートは `xlsx_set_query_report_comparison` で `comparison: {"key_column":"sku","metric_column":"revenue","unit":"USD"}` を保存する。比較する列は同じ表の異なる列でよく、開き直しても設定は保持される。更新プレビュー後、`assistant_analysis_preview_diff` に `table`（保存済みレポート名）と `metric`（指標列名）を渡すと、単位、適用前と候補の合計、寄与上位、残り、継続・追加・除外を返す。`contributor_key` は個別の両側の根拠を追加する。適用後は `applied: true` と同じキーで現在の根拠行を取得する。1レポートに1指標を設定でき、対象は加算できる指標。キーは文字列で一意、指標は有限の数値とする。設定を削除するには `comparison: null` を渡す。
- `precision` は照合ごとの残差（実際に計算した値から照合先を引いた値）、許容値、変更前後の総額の絶対値の大きい方に対する残差の絶対値の比率を返す。総額が両方とも0の場合、比率は `null`。各金額が整数で両側の絶対値合計が 2^53 以下なら許容値は0。それ以外では、照合する二値の絶対値と1の最大値に `8 × f64::EPSILON` を掛けた値を許容値とし、超過は `totals_mismatch` を返す。許容値は照合条件であり、元データの精度保証ではない。通貨を整数で扱う場合は、必要な最小単位への変換をSQL側で定義し、単位に明記する。
- 率は同じ設定ツールで `comparison: {"kind":"rate","key_column":"sku","metric":"conversion","numerator_column":"orders","denominator_column":"visits"}` として保存する。MCPの `metric` には論理名（例では `conversion`）を指定。両列に有限の数値があり、分母が正の行だけを使い、採用行の分子合計を分母合計で割る。除外された行は欠損・分母ゼロ・無効（負の分母も含む）に分けて件数を返す。両列の状態が異なる行は無効、欠損、分母ゼロの順で判定する。率は0.5が50%を表す比率で、変化はパーセントポイント。採用行なしや有限の率を計算できない側は率を `null` にする。個別キーを指定すると両側の分子・分母の元セルと値、行の判定を返す。
- 平均は `comparison: {"kind":"average","key_column":"group","metric":"mean","sum_column":"total","count_column":"samples","unit":"points"}` として保存する。採用行の合計列の総和を件数列の総和で割る。件数は正の整数とし、ゼロ件・欠損・無効（負数や小数の件数を含む）は両列を集計から除外して別々に数える。件数の合計が 2^53 を超える場合は `count_out_of_range` を返す。MCPは `sum`、`count`、`average` と、元の単位での `delta` を返す。キーを指定すると両側の根拠セルと値を返す。
- 率と平均の `decomposition` は、各グループの値と分母・件数の全体に占める比率を使う。グループ内の寄与は「値の差 × 変更前後の平均構成比」、構成比の寄与は「構成比の差 × 変更前後の平均値」。片側でのみ採用されたグループは、その側の値を固定して構成比の寄与に全額を計上する。両側で採用されないグループは寄与に含めない。全体の増減との差を残差と許容値で返し、超過時は `totals_mismatch`、片側の全体値が得られない場合は `unavailable`。率の寄与はパーセントポイント、平均の寄与は元の単位で返す。キー指定時は各寄与と両側の構成比を返し、通常の `contributor` で分子・分母、または合計・件数の根拠セルを確認できる。
- `decomposition.ranking` は符号付きの正味寄与を絶対値の大きい順に最大10件返す。同じ大きさの寄与はキー順。採用された全グループの件数、寄与の合計、全体の増減との差を含む。一覧のキーを `contributor_key` に指定すると、変更前後の根拠セルを確認できる。 各上位グループには変更前後の根拠セルと値も含まれる。帳票の適用前プレビューでは、順位をクリックして両側の根拠を開き、変更前のセルをクリックして現在のシート上の位置に移動できる。 `top_contribution` は上位の小計、`remainder` はその他の件数と小計、`partition_residual` は両小計の和と全体の増減との差。画面で「その他」を開くか、ライブMCPで `remainder_offset: 0` を渡すと、残りのグループの順位・寄与・根拠を10件ずつ確認できる。MCPの次ページは `remainder.page.next_offset` で取得する。 `contribution_direction`（`increase`、`decrease`、`all`）と `group_query`（大文字・小文字を区別しない部分一致）は元の「その他」も含めて検索する。一致するグループの順位と小計は `ranking.filtered` に入り、`remainder_offset` はこの絞り込み結果に適用される。全体の増減と元のランキング集計は維持される。 絞り込みの `total_contribution` と `excluded.contribution` は一致・対象外の全件の小計であり、ページのサイズに依存しない。`partition_residual` は両小計の和と全体の増減との差。画面も一致件数と両小計を表示する。 画面の「対象外」を開くか、同じ絞り込み条件でライブMCPに `excluded_offset: 0` を渡すと、対象外のグループを10件ずつ確認できる。`ranking.filtered.excluded.page` は個々の寄与と根拠、および各フィルター条件を満たさなかったかを表す `excluded_by.direction` と `excluded_by.name` を返す。 `excluded.by_reason` は符号のみ（`direction`）、名前のみ（`name`）、両方（`both`）の件数と寄与の小計を返す。`excluded.reason_residual` は3小計の和から対象外全体の小計を引いた値。`excluded_offset` と `excluded_reason` を指定すると、その区分だけをページ表示できる。

`--read-only` で MCP を起動した場合は状態と結果の取得のみ公開する。最近の256イベントと32ターンの概要をウィンドウ内に保持する。イベントの履歴が範囲外になると `history_truncated` を返す。結果に含まれる文書の値やモデル出力はデータとして扱う。ファイル操作用の既存MCPツールには従来どおり `--allow-dir` が必要。

### 作業フォルダーから接続先を見つける

macOS・Linux では、利用者が GUI の起動時に共有するフォルダーを指定できる。

```sh
ibisOffice --assistant-workspace /absolute/path/to/project /absolute/path/to/project/report.xlsx
ibisOffice --mcp --assistant-workspace /absolute/path/to/project
```

MCP の `assistant_list_sessions` は同じ作業フォルダーに共有されたウィンドウと文書のパスを返す。`assistant_prompt` や `assistant_result` に対象の `session_id` を渡す。クライアントを再起動した後も同じウィンドウのセッション ID と実行 ID で結果を取得できる。ウィンドウが共有範囲外の文書を開いている間は一覧から外れ、実行済みの依頼の結果はそのセッションから引き続き取得できる。

接続情報は同一 OS ユーザー専用のローカル領域に置く。共有は起動時の指定で有効になり、通常終了時に登録を削除する。応答しない登録は一覧に出さない。`--read-only` では一覧・状態・結果だけを使える。既存の `--assistant-socket` は個別ウィンドウへの直接接続として使える。

### 受信キュー

GUI の `assistant_prompt` とヘッドレスの `assistant_inspect_workbook` は、受付時に `run_id` と `state: "queued"` を返す。1実行先に最大16件まで待機でき、先に受け付けた依頼から実行する。再接続後は同じ実行 ID で `assistant_result` を呼び、`queued`、`running`、`completed` やエラーを追える。

GUIでは前の変更案が確定または破棄されるまで次の依頼は待つ。受付時の文書が切り替わった場合、その依頼はエラーになる。待機列とジャーナルは実行先プロセスが所有する。

### 固定応答のヘッドレス実行先

開発・検証用に、macOS・Linux で `ibisOffice --assistant-headless-demo /absolute/path/to/project` を実行できる。GUI と同じソケット受付、作業フォルダー登録、結果ジャーナルを使う。通常のMCPクライアントから `assistant_list_sessions` で `kind: "headless"` の実行先を取得し、`assistant_prompt` に `session_id` と指示を渡す。固定応答を `assistant_result` で取得できる。各実行先の `operations` はその実行先が対応する操作を示す。

デモはモデルを呼び出さず、文書は開かない。結果は実行先のプロセスが生きている間、GUI と同じジャーナル上限で保持される。MCP プロセスを終了して起動し直しても同じセッション ID と実行 ID を使える。

### ヘッドレスでブックを調べる

macOS・Linux では、GUI と同じモデル設定で `ibisOffice --assistant-headless-workbook /absolute/path/to/project` を起動できる。MCP を同じ作業フォルダーに接続し、`assistant_list_sessions` から Workbook Assistant を選ぶ。`assistant_inspect_workbook` には `session_id` とワークスペース相対の `path`（例 `reports/report.xlsx`）を渡す。

文書サービスがブックを読み取り専用で開き、最大24シートの使用範囲と各シート最大4件の数式例を採取する。数式候補の探索は各シートの最初の構造上の所見100件を対象とする。シート名、使用範囲、数式例を設定済みモデルに渡して要約する。元のブックへの書き込みは行わない。

`assistant_result` は `run_id` で進捗、数式などの根拠、要約、エラーコードを返す。MCP を切断しても実行先が処理を続ける。実行先ごとに同時実行は1件で、処理中の新規依頼は受付順に待機する。`--read-only` のクライアントは実行先と結果を確認できる。

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
