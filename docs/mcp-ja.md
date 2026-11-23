# AI アシスタントから ibisOffice を使う（MCP）

[English](mcp.md) | **日本語**

ibisOffice は、MCP（Model Context Protocol）のサーバーとして動かせます。Claude などの AI アシスタントが、これを通して Word（.docx）・Excel（.xlsx）・PowerPoint（.pptx）のファイルを開き、文章・セル・表・スライドを読んで書き換え、元に戻し、保存や書き出しをできます。窓は開かず、データはコンピューターの外へ出ません。サーバーはお使いのコンピューターで動き、起動したプログラムとだけ話します。

このページの順序：

1. [サーバーを起動する](#サーバーを起動する)
2. [Coding agent と設定を共有する](#coding-agent-と設定を共有する)
3. [アクセスするフォルダーを許可する](#アクセスするフォルダーを許可する)
4. [ツール](#ツール)
5. [Claude Desktop と接続する](#claude-desktop-と接続する)
6. [Claude Code と接続する](#claude-code-と接続する)
7. [ChatGPT と接続する](#chatgpt-と接続する)
8. [安全のために](#安全のために)

## サーバーを起動する

| 環境 | コマンド |
|---|---|
| Windows | `ibisOffice-mcp.exe [オプション]`（`ibisOffice.exe` と同じフォルダー。`ibisOffice.exe` は標準入出力を持たない GUI のプログラムです） |
| macOS・Linux | `ibisOffice --mcp [オプション]` |

サーバーは標準入出力で MCP を話します（1 行に JSON のメッセージを 1 つ。ログは標準エラー出力）。プロトコルの版は 2026-07-28 で、`initialize` から始めるクライアントには 2025-11-25 で答えます。HTTP のポートは持ちません。MCP のクライアントが起動する、手元のプログラムです。`--help` でオプションを表示します。

| オプション | |
|---|---|
| `--allow-dir <フォルダー>` | ファイルを開いて保存してよいフォルダー。何度でも指定できます。**1 つも無ければ、どのファイルにもアクセスできません。** |
| `--read-only` | 読み取りツールだけが利用できます。 |
| `--max-documents N` | 同時に開く文書の数（既定は 8）。 |
| `--max-file-size MB` | 開くファイルの最大の大きさ（単位は MB、既定は 200MB）。zip は大きさの 20 倍まで展開されることがあります。 |

設定する環境変数はありません。

## Coding agent と設定を共有する

リポジトリの [ibis-office スキル](../.agents/skills/ibis-office/SKILL.md) は、ディスク上の文書と起動中の GUI を操作する手順をまとめています。`.agents/skills/ibis-office/` をフォルダーごとクライアントのスキル探索先に置くと、接続例とサンプルも一緒に使えます。 [Codex・OpenCode・Claude Code の接続例](../.agents/skills/ibis-office/references/mcp-setup.md)

プロジェクトで共通の指示は `AGENTS.md`、繰り返す手順は `SKILL.md`、判定基準などの資料は `references/`、配布するひな形や入力データは `assets/` に置けます。OKF を使う場合、指標定義などのコンセプトは通常の Markdown として共有できます。クライアント設定は使うクライアントの形式で管理します。

[月次帳票の点検例](../.agents/skills/ibis-office/references/monthly-report.md) では、同梱の合成 CSV を既存の読み取り専用 MCP ツールで検査します。定型依頼を MCP Prompts、テンプレートや資料を MCP Resources として公開する機能は、今後追加する際も同じ手順・ファイルを参照できる構成です。

## アクセスするフォルダーを許可する

既定では、ファイルにアクセスできません。フォルダーごとに `--allow-dir` で指定します（例：`--allow-dir ~/Documents --allow-dir ~/Work`）。ファイルを開く・書くことができるのは、その実際のパス（リンクをたどり、`..` を解いたもの）が許可したフォルダーの下にあり、拡張子が docx・docm・dotx・xlsx・xlsm・pptx・pptm のときだけです（書き出しは .pdf と .png を書きます）。`--read-only` を付けると、読めるだけで何も変えられません。

## ツール

`app_open_document` でファイルを開くと `document_id` が返り、ほかのツールに渡します。読んで、編集して、`app_save_document` で保存します。長さの単位はポイントです。ツールの 1 回の呼び出しが、元に戻す 1 回分です。

| 区分 | ツール |
|---|---|
| アプリ（全形式） | `app_get_version`、`app_list_documents`、`app_open_document`、`app_get_document_info`、`app_save_document`、`app_close_document`、`app_export_document`（PDF またはページごとの PNG）、`app_undo_document`、`app_redo_document` |
| Word（.docx） | 読む：`docx_get_outline`、`docx_get_text`（Markdown またはプレーンテキスト）、`docx_list_paragraphs`、`docx_get_paragraph`、`docx_list_tables`、`docx_find_text`。編集：`docx_insert_text`、`docx_delete_text`、`docx_replace_text`、`docx_set_paragraph_text`、`docx_set_paragraph_format`、`docx_set_run_format`、`docx_insert_table` |
| Excel（.xlsx） | 読む：`xlsx_list_sheets`、`xlsx_get_used_range`、`xlsx_get_cells`、`xlsx_find_text`、`xlsx_check_machine_readability`、`xlsx_trace_formula`、`xlsx_list_sheet_rules`、`xlsx_list_pivots`、`xlsx_list_query_reports`。編集：`xlsx_set_cells`（`=` に続けて数式を指定）、`xlsx_clear_cells`、`xlsx_set_format`、`xlsx_add_sheet`、`xlsx_delete_sheet`、`xlsx_rename_sheet`、`xlsx_calculate_workbook`、`xlsx_create_chart`、`xlsx_set_validation`、`xlsx_clear_validation`、`xlsx_create_pivot`、`xlsx_refresh_pivot`、`xlsx_set_query_report_checks` |
| SQLite（.sqlite、.sqlite3、.db） | 読む：`sqlite_list_tables`、`sqlite_describe_table`、`sqlite_describe_datasets`、`sqlite_count_rows`、`sqlite_query`。ブック編集：`sqlite_query_to_table`、`sqlite_refresh_query_table`、`sqlite_refresh_query_reports` |
| PowerPoint（.pptx） | 読む：`pptx_list_slides`、`pptx_get_slide`、`pptx_get_shape_text`、`pptx_get_notes`、`pptx_list_layouts`。編集：`pptx_set_shape_text`、`pptx_replace_text`、`pptx_set_run_format`、`pptx_set_notes`、`pptx_add_slide`、`pptx_delete_slide`、`pptx_move_slide`、`pptx_set_frame`、`pptx_delete_shape` |

`xlsx_create_chart` は、元データの `sheet` と、配置先の `destination_sheet`・`anchor`（例 `F2`）を指定します。見出しを含む `range` または明示的な `series` を渡せます。作成したグラフは元のセルを参照します。

`xlsx_check_machine_readability` は、機械可読性の確認候補をシート名・種類・A1範囲で返します。`sheet` を省略すると非表示設定のない全シートを調べます。続きは `next_offset` を `offset` に渡して取得します。

`xlsx_trace_formula` は対象の `sheet` と1セルの `cell`（A1形式）を受け取り、`direction` に `precedents`（参照元）または `dependents`（参照先）を指定します。表示されるシート・数式を対象に、静的に解決できる直接参照をシート名と範囲で返します。`INDIRECT` などで実行時に決まる参照や外部ブック参照は対象外です。続きは `next_offset` を `offset` に渡します。

`xlsx_list_sheet_rules` はシートの入力規則と条件付き書式を一覧にします。`range` に A1 形式の範囲を指定すると、重なるルールを絞り込めます。各ルールは適用範囲をすべて返します。`kind` には `validation` または `conditional_formatting` を指定でき、続きは `next_offset` を `offset` に渡します。条件付き書式は条件と評価優先順位を返します。カラースケールやデータバーなどの表示ルールは種類を確認できます。

`xlsx_set_validation` は A1 形式の `range` 内の入力規則を置き換えます。`type` が `list` の場合、`formula1` に `"未着手,完了"` のような引用符付きの選択肢、同じシートの範囲参照、または定義名を指定します。`whole`（整数）と `decimal`（小数）では `formula1` と `formula2` に下限と上限を指定し、両端を含みます。`custom` では `formula1` に条件式を指定します。相対参照の起点は範囲の左上セルです。`xlsx_clear_validation` はセルの内容を維持したまま、指定範囲の入力規則を解除します。

`sqlite_list_tables` は、許可フォルダ内の単独で開ける SQLite ファイルを `path` に指定してテーブルを一覧にします。取得したテーブル名を `table` に渡すと、`sqlite_describe_table` で列の宣言型や主キーを確認でき、`sqlite_count_rows` で件数を集計できます。`group_by` には列名を指定でき、NULL も一つのグループになります。`limit` は返すグループ数の上限です（既定25、最大100）。すべてのグループが返ったかは `truncated` で確認できます。デスクトップの表計算アシスタントでは、パネルで SQLite ファイルを選ぶと同じ操作を会話から使えます。

`sqlite_count_rows` の `filters` には最大16個の条件を指定でき、すべてに一致する行を集計します。各条件に `column` と `operator` を指定します。`eq`、`ne`、`lt`、`le`、`gt`、`ge` では文字列または数値の `value` を渡します。`is_null` と `is_not_null` では `value` を省略します。比較には SQLite の型と NULL の規則が適用され、結果には使用した条件が含まれます。

`sqlite_query` は単一の `SELECT` または `WITH … SELECT` と、任意の位置パラメータ `params` を受け付けます。SQL、パラメータ、列名、行、打ち切りフラグ `truncated` を返します。プレビューは既定50行、最大1,000行で、応答サイズにも上限があります。クエリ実行の上限は3秒です。

CSV・Parquet・Arrow IPC・JSON Lines・Avro・SQLite のテーブルを一緒に調べるには、`sqlite_query` または `sqlite_query_to_table` に `datasets` を渡します。各要素にファイルの `source` とSQLで使うテーブル名 `alias` を指定し、SQLiteの場合は元の `table` 名も渡します。この形式では `path` を省略します。例えばSQLiteの表を `facts`、CSVを `labels` として登録し、`SELECT * FROM facts JOIN labels USING (code)` と書けます。内蔵アシスタントでは複数ファイルをまとめて選び、`source` に選択順のファイル番号を使います。

SQLを書く前に、同じ `datasets` を `sqlite_describe_datasets` に渡すと、別名・形式・SQLから見える列名と格納型を確認できます。列順は読み込み時の列選択と一致します。CSV列は `TEXT`、SQLite列は行ごとに格納型が変わり得るため `DYNAMIC` と返します。SQLiteの宣言型とキーは `sqlite_describe_table` で確認できます。JSON Linesは最大32 MiBを走査して型を推定します。データ行は返しません。

Parquet では `columns` に読み込む列の正確な名前を順番に指定できます（例 `["code", "amount"]`）。選ばなかった列は、入れ子列も含め、入力セル数の上限を適用する前に読み飛ばします。省略すると全列を読み込みます。列の選択はクエリ帳票に保存し、更新時にも使います。

Arrow IPC のファイル・ストリーム（`.arrow`、`.ipc`、`.feather`）も `columns` による列選択と帳票への保存に対応します。SQL データセットでは UTF-8 文字列配列を JSON テキストとして取り込み、空配列や配列内の null を保持します。`json_each` で展開できます。辞書エンコードは未対応です。デコード量と選択結果のセル数・テキスト量には上限があり、Turso は結果をメモリ上に読み込みます。

JSON Lines（`.jsonl`、`.ndjson`）は先にフィールドと型を調べ、その後に行を順次読み込みます。欠損は NULL、配列・オブジェクトは JSON テキストです。Avro（`.avro`）はオブジェクトコンテナのレコードをブロック単位で読み込み、表計算の取り込みと同じスカラー型と NULL 許容型に対応します。どちらも `columns` で列を選べ、帳票に保存します。

SQL のデータセットでは、Parquet の標準形式で記録された UTF-8 文字列の配列を JSON テキストとして扱います。SQL の NULL、空配列、配列内の null、重複、要素順を保持します。`json_each` で配列を展開できます。元の行単位で金額を集計する場合は、`EXISTS (SELECT 1 FROM json_each(f.tags) j WHERE j.value = ?)` で元の行を絞り込みます。配列を条件に使う場合は SQL 側で指定します。JSON テキストはデータセットのテキスト量の上限に数えます。

Parquet の `where` には、最大16列の条件を指定できます。値を直接指定すると完全一致になります。すべての条件に一致する行を取り込みます（例 `{"year": "2026", "submitted": true}`）。JSON の null は欠損値に一致します。文字列を数値へ変換せず、列の型に従って照合します。条件に使う列は `columns` から省けます。必要列を走査し、条件に一致した出力のセル数とテキスト量だけを入力上限に数えます。条件はクエリ帳票に保存され、更新時にも使われます。

範囲指定は `{"amount": {"gte": 100, "lt": 200}}` のように記述します。演算子は `gt`（より大きい）、`gte`（以上）、`lt`（未満）、`lte`（以下）です。複数の境界はすべて満たす必要があります。数値は数値順、文字列は辞書順で比較し、欠損値は範囲条件に一致しません。

符号付き整数や文字列について、並び順が一致する正確な最小値・最大値の統計があり、条件に合う行が存在しないと判断できる場合は、その行グループを読み飛ばします。確定した欠損件数も利用します。利用できる統計がないグループは走査し、読み込んだ行には引き続き条件を適用します。

CSVはカンマ区切り、先頭行が見出しで、値を文字列として扱うため先頭ゼロと空文字が残ります。ParquetとSQLiteのNULLは保持します。既存リーダーの精度を超える整数はエラーになります。最大8件、合計10万セルまでを、呼び出しごとにメモリ上のデータベースに取り込みます。

同じSQLとパラメータを `sqlite_query_to_table` に渡し、出力先の `document_id`、`destination_sheet`、左上セル `anchor` を指定すると、空いている場所に名前付きのExcelテーブルができます。出力上限は10,000行、100,000セル、4 MiBです。結果全体が収まる場合に表を作ります。空欄や重複する列名にはSQLで別名を付け、大きな整数は必要に応じて文字列に変換してください。

`xlsx_create_pivot` では元の `sheet` と返された `table` 名、または見出し付きの `range` を指定できます。配置先は `destination_sheet` と `anchor`、行・列フィールドは `rows` と `columns` で指定します。`values` には `{"field":"業務ID","aggregate":"count"}` のようにフィールド名と集計方法を渡します。レコード件数を数えるには空欄のないID列を使います。内蔵アシスタントでは表出力とピボット作成を確認用の下書きに積みます。出力した表には実行時点の結果が入ります。

`xlsx_list_pivots` は `sheet` 上のピボットの配置範囲、参照元、フィールド構成を返します。続きは `next_offset` を使います。`xlsx_refresh_pivot` に `sheet` と `pivot` 名を指定すると、ブック内の元データから再集計し、連動するピボットグラフも更新します。更新後の範囲が既存のデータに重なる場合は処理を中止します。

`sqlite_query_to_table` で作成した表には、SQL・パラメータ・元ファイルの名前と形式をブック内に記録します。`xlsx_list_query_reports` で更新できる表と元ファイルの順序を調べられます。`table` を指定すると保存済みの定義を確認できます。`sqlite_refresh_query_table` には `document_id`、`table`、保存時の順序に合わせた `sources` を渡します。MCPでは許可フォルダ内のファイル、内蔵アシスタントでは選択したファイルの番号を指定します。更新には毎回、元ファイルの明示的な指定が必要です。

更新は表のデータをその場で入れ替え、テーブル名を参照するピボットと連動グラフを更新します。すべてまとめて取り消せます。正常に0件になった場合は、見出しを残して前回の明細を消去します。見出しが変わった場合や更新範囲に別のデータがある場合は中止します。内蔵アシスタントでは下書きで確認できます。

`sqlite_refresh_query_reports` は、複数のレポートを一括で確認・更新します。`bindings` で保存時のファイル名に今回のファイルを割り当てます。例えばMCPでは `{"monthly.csv":"/data/next.csv"}`、内蔵アシスタントではパスの代わりに選択したファイル番号を指定します。同じ保存名を参照するレポートは割り当てを共有します。`tables` を省略すると全件が対象になり、1回に更新できるのは12件までです。`preview: true` でブックを変更せずに更新前後の行数と配置の衝突を確認できます。適用時にクエリを再実行し、すべてを一度に取り消せる形で反映します。途中で失敗した場合は対象の表名を返し、ブックへの反映を中止します。

帳票にはSQLが使った入力の列名と型を保存します。個別更新と一括更新の結果に `schema_changes`（代表20件まで）と `schema_change_count`（全件数）が出る場合があります。列の追加・削除・型変更は警告として扱い、既存のSQLと検証チェックが通れば更新できます。比較基準は適用が成功したときだけ更新され、既存帳票では最初に適用できた入力が基準になります。スタンドアロンSQLiteでは通常テーブルの宣言型、登録式のデータセットではSQLへ渡す列と型を比較します。帳票定義の保存上限に収まらない場合は、更新時に `input_schema_status: "unavailable"` を返します。

一括更新では、参照元と列指定が同じ帳票が DB を共有します。Arrow IPC・JSON Lines・Avro のファイル合計が 8 MiB を超える場合やメモリ取り込みの上限に達した場合は、一時ディスク DB へ順次転写します。同じグループの他形式も、従来の入力上限内で結合できます。小さい入力はメモリ DB を共有します。`arrow_disk: true` で同一の Arrow 入力を明示的にディスクへ転写することもできます。準備後に一時 DB は削除します。プレビューした内容を元ファイルの再読み込みなしで適用するには、同じ接続で `snapshot_id` と `document_id` だけを指定します。ID は一度だけ使えます。ブックの変更や新たな一括更新の要求があれば、プレビューをやり直します。

`metrics` に `{"table":"QueryResult1","column":"件数"}` のように比較する表と数値列を指定すると、合計の前回値 `previous`、今回値 `current`、差 `delta`、増減率 `percent_change` が返ります。空セルやNULLは合計に加えず、文字列などの非数値がある列はエラーになります。増減率の分母は前回値の絶対値で、前回値が0の場合はNULLになります。

`xlsx_set_query_report_checks` に保存済みレポートの `table` と `checks` を渡すと、レポートごとに最大4件の検証を登録できます。各検証に `name` と読み取り専用の `sql` を指定し、必要に応じて `params` と許容上限 `max` を渡します。SQLは0以上の整数を1つ返す必要があります。左結合でマスタ未登録の件数を数えたり、重複するキーの件数を数えたりできます。検証と本集計は同じ入力を使います。一括プレビューは検出件数を返し、上限を超えたときは `blocked` を返します。適用時に上限を超えると、個別更新も一括更新全体も中止します。`max` を省略すると件数の確認だけを行い、空の `checks` を渡すと検証を解除できます。

明細 `facts` とマスタ `labels` の結合漏れで更新を止めるには、次のSQLをチェックに登録し、`max` を `0` にします。本集計と同じ結合条件で、マスタに対応する行がない明細を数えます。

```sql
SELECT count(*)
FROM facts AS f
WHERE NOT EXISTS (SELECT 1 FROM labels AS l WHERE l.code = f.code)
```

チェックで更新が止まったら、`sqlite_query` に次のSQLと `limit: 20` を渡します。`datasets` には更新時と同じファイルを指定し、別名・列指定・絞り込みは保存済み帳票にそろえます。未一致の明細をキーとSQLiteの格納型ごとに集計できます。

```sql
SELECT f.code AS unmatched_key, typeof(f.code) AS key_type, count(*) AS occurrences
FROM facts AS f
WHERE NOT EXISTS (SELECT 1 FROM labels AS l WHERE l.code = f.code)
GROUP BY f.code, typeof(f.code)
ORDER BY occurrences DESC, key_type, unmatched_key
LIMIT 20
```

同じSQLをチェックの `diagnostic: {"sql":"…","params":[]}` に保存すると、一括プレビューの `checks[].diagnostic` に例を表示できます。チェックと同じSQLセッションで実行し、上限を超えたチェック、または件数が正の情報表示用チェックが対象です。最大20行かつ2 KiBの結果を返します。診断が失敗しても、更新可否はチェック件数と上限で決まります。

結果をマスタ側のキーと照合します。例えば未一致が整数の `1` で、マスタ側が文字列の `001` なら、型や表記の違いを確認できます。診断は元ファイルを読み直すため、失敗時の状態を再現するにはファイルを編集・差し替えする前に実行します。

デスクトップ版の **データ → クエリと接続 → SQLレポートの更新** でも、更新するレポート、元ファイル、比較する数値列を指定できます。プレビュー前に、登録済みチェックの名前・許容上限・診断の有無を確認できます。**チェックのSQLを表示**を押すと、件数チェックと診断のSQLを確認できます。更新時はプレビューで確認した内容を反映します。

ツールはメニューのコマンドと 1 対 1 で対応してはいません。読み取りと、よく使う編集をそろえたもので、形式が違っても同じツール名です。`--read-only` のときは、読み取りツールだけが利用できます。

## Claude Desktop と接続する

Claude Desktop が、サーバーを自分で起動します。設定ファイルは Claude Desktop から開くか（設定、開発者、設定を編集）、直接編集します。

- Windows：`%APPDATA%\Claude\claude_desktop_config.json`
- macOS：`~/Library/Application Support/Claude/claude_desktop_config.json`

Windows（ibisOffice を入れたフォルダーに読み替えてください。`--mcp` は要りません）：

```json
{
  "mcpServers": {
    "ibisoffice": {
      "command": "C:\\Program Files\\ibisOffice\\ibisOffice-mcp.exe",
      "args": ["--allow-dir", "C:\\Users\\me\\Documents"]
    }
  }
}
```

macOS：

```json
{
  "mcpServers": {
    "ibisoffice": {
      "command": "/Applications/ibisOffice.app/Contents/MacOS/ibisOffice",
      "args": ["--mcp", "--allow-dir", "/Users/me/Documents"]
    }
  }
}
```

ファイルに別のサーバーが既にあるときは、`mcpServers` の中にその隣へ `ibisoffice` を足します。パスは絶対パスで書きます。Claude Desktop を完全に終了して、起動し直してください。サーバーは、入力欄のコネクターに出ます。Claude Desktop の MCP のログ（`mcp.log`、`mcp-server-ibisoffice.log`）は、Windows では `%APPDATA%\Claude\logs`、macOS では `~/Library/Logs/Claude` にあります。

出典：[Connect to local MCP servers](https://modelcontextprotocol.io/docs/develop/connect-local-servers)（英語）。

## Claude Code と接続する

Windows：

```
claude mcp add ibisoffice -- "C:\Program Files\ibisOffice\ibisOffice-mcp.exe" --allow-dir "C:\Users\me\Documents"
```

macOS：

```
claude mcp add ibisoffice -- /Applications/ibisOffice.app/Contents/MacOS/ibisOffice --mcp --allow-dir ~/Documents
```

`--` の後ろが、Claude Code が起動するコマンドです。`claude mcp list` で、接続されているかを確かめられます。

## ChatGPT と接続する

現在は、標準入出力（stdio）での対話ですが、近日中に SSE またはストリーミング HTTP に対応する予定です。

## 安全のために

- アクセスできるのは、許可したフォルダーの下のファイルだけです（実際のパスで判断します）。
- ファイルを書くのは `app_save_document` と `app_export_document` だけです。**既定は新しい名前**で、既存のファイルを置き換えるには `overwrite: true` が要ります。開いた後に別のプログラムが変えたファイルは、`overwrite: "force"` のときだけ置き換えます。保存はファイルの隣に一時ファイルを書いて名前を替えるので、失敗しても元のファイルはそのままです。
- マクロは実行せず、ネットワークからは何も取りに行きません（リンクや画像の URL は値であり、要求ではありません）。
- 文書の文章はデータです。それを含む結果にはその旨が付き、アシスタントは中にある指示に従うべきではありません。
- デジタル署名のあるファイルは、編集を保存すると署名が外れます（結果にその旨が出ます）。
