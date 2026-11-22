# AI アシスタントから ibisOffice を使う（MCP）

[English](mcp.md) | **日本語**

ibisOffice は、MCP（Model Context Protocol）のサーバーとして動かせます。Claude などの AI アシスタントが、これを通して Word（.docx）・Excel（.xlsx）・PowerPoint（.pptx）のファイルを開き、文章・セル・表・スライドを読んで書き換え、元に戻し、保存や書き出しをできます。窓は開かず、データはコンピューターの外へ出ません。サーバーはお使いのコンピューターで動き、起動したプログラムとだけ話します。

このページの順序：

1. [サーバーを起動する](#サーバーを起動する)
2. [アクセスするフォルダーを許可する](#アクセスするフォルダーを許可する)
3. [ツール](#ツール)
4. [Claude Desktop と接続する](#claude-desktop-と接続する)
5. [Claude Code と接続する](#claude-code-と接続する)
6. [ChatGPT と接続する](#chatgpt-と接続する)
7. [安全のために](#安全のために)

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

## アクセスするフォルダーを許可する

既定では、ファイルにアクセスできません。フォルダーごとに `--allow-dir` で指定します（例：`--allow-dir ~/Documents --allow-dir ~/Work`）。ファイルを開く・書くことができるのは、その実際のパス（リンクをたどり、`..` を解いたもの）が許可したフォルダーの下にあり、拡張子が docx・docm・dotx・xlsx・xlsm・pptx・pptm のときだけです（書き出しは .pdf と .png を書きます）。`--read-only` を付けると、読めるだけで何も変えられません。

## ツール

`app_open_document` でファイルを開くと `document_id` が返り、ほかのツールに渡します。読んで、編集して、`app_save_document` で保存します。長さの単位はポイントです。ツールの 1 回の呼び出しが、元に戻す 1 回分です。

| 区分 | ツール |
|---|---|
| アプリ（全形式） | `app_get_version`、`app_list_documents`、`app_open_document`、`app_get_document_info`、`app_save_document`、`app_close_document`、`app_export_document`（PDF またはページごとの PNG）、`app_undo_document`、`app_redo_document` |
| Word（.docx） | 読む：`docx_get_outline`、`docx_get_text`（Markdown またはプレーンテキスト）、`docx_list_paragraphs`、`docx_get_paragraph`、`docx_list_tables`、`docx_find_text`。編集：`docx_insert_text`、`docx_delete_text`、`docx_replace_text`、`docx_set_paragraph_text`、`docx_set_paragraph_format`、`docx_set_run_format`、`docx_insert_table` |
| Excel（.xlsx） | 読む：`xlsx_list_sheets`、`xlsx_get_used_range`、`xlsx_get_cells`、`xlsx_find_text`、`xlsx_check_machine_readability`、`xlsx_trace_formula`、`xlsx_list_sheet_rules`、`xlsx_list_pivots`、`xlsx_list_query_reports`。編集：`xlsx_set_cells`（`=` に続けて数式を指定）、`xlsx_clear_cells`、`xlsx_set_format`、`xlsx_add_sheet`、`xlsx_delete_sheet`、`xlsx_rename_sheet`、`xlsx_calculate_workbook`、`xlsx_create_chart`、`xlsx_set_validation`、`xlsx_clear_validation`、`xlsx_create_pivot`、`xlsx_refresh_pivot` |
| SQLite（.sqlite、.sqlite3、.db） | 読む：`sqlite_list_tables`、`sqlite_describe_table`、`sqlite_count_rows`、`sqlite_query`。ブック編集：`sqlite_query_to_table`、`sqlite_refresh_query_table`、`sqlite_refresh_query_reports` |
| PowerPoint（.pptx） | 読む：`pptx_list_slides`、`pptx_get_slide`、`pptx_get_shape_text`、`pptx_get_notes`、`pptx_list_layouts`。編集：`pptx_set_shape_text`、`pptx_replace_text`、`pptx_set_run_format`、`pptx_set_notes`、`pptx_add_slide`、`pptx_delete_slide`、`pptx_move_slide`、`pptx_set_frame`、`pptx_delete_shape` |

`xlsx_create_chart` は、元データの `sheet` と、配置先の `destination_sheet`・`anchor`（例 `F2`）を指定します。見出しを含む `range` または明示的な `series` を渡せます。作成したグラフは元のセルを参照します。

`xlsx_check_machine_readability` は、機械可読性の確認候補をシート名・種類・A1範囲で返します。`sheet` を省略すると非表示設定のない全シートを調べます。続きは `next_offset` を `offset` に渡して取得します。

`xlsx_trace_formula` は対象の `sheet` と1セルの `cell`（A1形式）を受け取り、`direction` に `precedents`（参照元）または `dependents`（参照先）を指定します。表示されるシート・数式を対象に、静的に解決できる直接参照をシート名と範囲で返します。`INDIRECT` などで実行時に決まる参照や外部ブック参照は対象外です。続きは `next_offset` を `offset` に渡します。

`xlsx_list_sheet_rules` はシートの入力規則と条件付き書式を一覧にします。`range` に A1 形式の範囲を指定すると、重なるルールを絞り込めます。各ルールは適用範囲をすべて返します。`kind` には `validation` または `conditional_formatting` を指定でき、続きは `next_offset` を `offset` に渡します。条件付き書式は条件と評価優先順位を返します。カラースケールやデータバーなどの表示ルールは種類を確認できます。

`xlsx_set_validation` は A1 形式の `range` 内の入力規則を置き換えます。`type` が `list` の場合、`formula1` に `"未着手,完了"` のような引用符付きの選択肢、同じシートの範囲参照、または定義名を指定します。`whole`（整数）と `decimal`（小数）では `formula1` と `formula2` に下限と上限を指定し、両端を含みます。`custom` では `formula1` に条件式を指定します。相対参照の起点は範囲の左上セルです。`xlsx_clear_validation` はセルの内容を維持したまま、指定範囲の入力規則を解除します。

`sqlite_list_tables` は、許可フォルダ内の単独で開ける SQLite ファイルを `path` に指定してテーブルを一覧にします。取得したテーブル名を `table` に渡すと、`sqlite_describe_table` で列の宣言型や主キーを確認でき、`sqlite_count_rows` で件数を集計できます。`group_by` には列名を指定でき、NULL も一つのグループになります。`limit` は返すグループ数の上限です（既定25、最大100）。すべてのグループが返ったかは `truncated` で確認できます。デスクトップの表計算アシスタントでは、パネルで SQLite ファイルを選ぶと同じ操作を会話から使えます。

`sqlite_count_rows` の `filters` には最大16個の条件を指定でき、すべてに一致する行を集計します。各条件に `column` と `operator` を指定します。`eq`、`ne`、`lt`、`le`、`gt`、`ge` では文字列または数値の `value` を渡します。`is_null` と `is_not_null` では `value` を省略します。比較には SQLite の型と NULL の規則が適用され、結果には使用した条件が含まれます。

`sqlite_query` は単一の `SELECT` または `WITH … SELECT` と、任意の位置パラメータ `params` を受け付けます。SQL、パラメータ、列名、行、打ち切りフラグ `truncated` を返します。プレビューは既定50行、最大1,000行で、応答サイズにも上限があります。クエリ実行の上限は3秒です。

CSV・Parquet・SQLiteのテーブルを一緒に調べるには、`sqlite_query` または `sqlite_query_to_table` に `datasets` を渡します。各要素にファイルの `source` とSQLで使うテーブル名 `alias` を指定し、SQLiteの場合は元の `table` 名も渡します。この形式では `path` を省略します。例えばSQLiteの表を `facts`、CSVを `labels` として登録し、`SELECT * FROM facts JOIN labels USING (code)` と書けます。列名は `SELECT * FROM labels LIMIT 0` でも確認できます。内蔵アシスタントでは複数ファイルをまとめて選び、`source` に選択順のファイル番号を使います。

CSVはカンマ区切り、先頭行が見出しで、値を文字列として扱うため先頭ゼロと空文字が残ります。ParquetとSQLiteのNULLは保持します。既存リーダーの精度を超える整数はエラーになります。最大8件、合計10万セルまでを、呼び出しごとにメモリ上のデータベースに取り込みます。

同じSQLとパラメータを `sqlite_query_to_table` に渡し、出力先の `document_id`、`destination_sheet`、左上セル `anchor` を指定すると、空いている場所に名前付きのExcelテーブルができます。出力上限は10,000行、100,000セル、4 MiBです。結果全体が収まる場合に表を作ります。空欄や重複する列名にはSQLで別名を付け、大きな整数は必要に応じて文字列に変換してください。

`xlsx_create_pivot` では元の `sheet` と返された `table` 名、または見出し付きの `range` を指定できます。配置先は `destination_sheet` と `anchor`、行・列フィールドは `rows` と `columns` で指定します。`values` には `{"field":"業務ID","aggregate":"count"}` のようにフィールド名と集計方法を渡します。レコード件数を数えるには空欄のないID列を使います。内蔵アシスタントでは表出力とピボット作成を確認用の下書きに積みます。出力した表には実行時点の結果が入ります。

`xlsx_list_pivots` は `sheet` 上のピボットの配置範囲、参照元、フィールド構成を返します。続きは `next_offset` を使います。`xlsx_refresh_pivot` に `sheet` と `pivot` 名を指定すると、ブック内の元データから再集計し、連動するピボットグラフも更新します。更新後の範囲が既存のデータに重なる場合は処理を中止します。

`sqlite_query_to_table` で作成した表には、SQL・パラメータ・元ファイルの名前と形式をブック内に記録します。`xlsx_list_query_reports` で更新できる表と元ファイルの順序を調べられます。`table` を指定すると保存済みの定義を確認できます。`sqlite_refresh_query_table` には `document_id`、`table`、保存時の順序に合わせた `sources` を渡します。MCPでは許可フォルダ内のファイル、内蔵アシスタントでは選択したファイルの番号を指定します。更新には毎回、元ファイルの明示的な指定が必要です。

更新は表のデータをその場で入れ替え、テーブル名を参照するピボットと連動グラフを更新します。すべてまとめて取り消せます。正常に0件になった場合は、見出しを残して前回の明細を消去します。見出しが変わった場合や更新範囲に別のデータがある場合は中止します。内蔵アシスタントでは下書きで確認できます。

`sqlite_refresh_query_reports` は、複数のレポートを一括で確認・更新します。`bindings` で保存時のファイル名に今回のファイルを割り当てます。例えばMCPでは `{"monthly.csv":"/data/next.csv"}`、内蔵アシスタントではパスの代わりに選択したファイル番号を指定します。同じ保存名を参照するレポートは割り当てを共有します。`tables` を省略すると全件が対象になり、1回に更新できるのは12件までです。`preview: true` でブックを変更せずに更新前後の行数と配置の衝突を確認できます。適用時にクエリを再実行し、すべてを一度に取り消せる形で反映します。途中で失敗した場合は対象の表名を返し、ブックへの反映を中止します。

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
