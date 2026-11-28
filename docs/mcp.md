# Using ibisOffice from an AI assistant (MCP)

**English** | [日本語](mcp-ja.md)

ibisOffice can run as an MCP (Model Context Protocol) server. An AI assistant such as Claude opens Word (.docx), Excel (.xlsx) and PowerPoint (.pptx) files through it, reads and edits their text, cells, tables and slides, undoes edits, and saves or exports the result. No window is opened and nothing leaves your computer: the server runs on your machine and speaks only to the program that started it.

This page, in order:

1. [Starting the server](#starting-the-server)
2. [Sharing a workflow across coding agents](#sharing-a-workflow-across-coding-agents)
3. [Allowing access to your folders](#allowing-access-to-your-folders)
4. [Tools](#tools)
5. [Connecting Claude Desktop](#connecting-claude-desktop)
6. [Connecting Claude Code](#connecting-claude-code)
7. [Connecting ChatGPT](#connecting-chatgpt)
8. [Safety](#safety)

## Starting the server

| Platform | Command |
|---|---|
| Windows | `ibisOffice-mcp.exe [options]` (next to `ibisOffice.exe`, which is a GUI program with no standard input and output) |
| macOS, Linux | `ibisOffice --mcp [options]` |

The server speaks the Model Context Protocol on standard input and output, one JSON message per line (the log goes to standard error). It uses protocol version 2026-07-28, and 2025-11-25 for a client that starts with `initialize`. It has no HTTP port: it is a local program that an MCP client starts. `--help` prints the options.

| Option | |
|---|---|
| `--allow-dir <folder>` | A folder whose files may be opened and saved. Repeatable. **With none, no file can be accessed.** |
| `--read-only` | Only the read tools are available. |
| `--max-documents N` | Documents open at once (default 8). |
| `--max-file-size MB` | The largest file that is opened, in MB (default 200 MB). A zip may expand to 20 times its size. |

There are no environment variables to set.

## Sharing a workflow across coding agents

The repository's [ibis-office skill](../.agents/skills/ibis-office/SKILL.md) covers both files on disk and the Assistant in an open GUI window. Copy the whole `.agents/skills/ibis-office/` directory to a client skill location to keep its references and sample input together. [Connection examples for Codex, OpenCode and Claude Code](../.agents/skills/ibis-office/references/mcp-setup.md)

Keep shared project instructions in `AGENTS.md` and reusable workflows in `SKILL.md`. Keep supporting documents in `references/` and deliverable templates or sample data in `assets/`. An OKF bundle can also hold domain knowledge such as metric definitions. Each client keeps its MCP connection in its own configuration format.

The [monthly report example](../.agents/skills/ibis-office/references/monthly-report.md) checks the bundled synthetic CSV with the existing read-only MCP tools. If support for MCP Prompts and Resources is added, those features can refer to the same workflows and assets.

## Allowing access to your folders

By default, no file can be accessed. Give each folder with `--allow-dir`, for example `--allow-dir ~/Documents --allow-dir ~/Work`. A file can be opened or written only when its real path (links followed, `..` resolved) is under one of these folders and its extension is docx, docm, dotx, xlsx, xlsm, pptx or pptm (an export writes .pdf or .png). Add `--read-only` to let the assistant read but not change anything.

## Tools

Open a file with `app_open_document`. It returns a `document_id` that goes into the other tools. Read, edit, then save with `app_save_document`. Lengths are in points. One tool call is one step of undo.

| Group | Tools |
|---|---|
| App (all formats) | `app_get_version`, `app_list_documents`, `app_open_document`, `app_get_document_info`, `app_save_document`, `app_close_document`, `app_export_document` (PDF or PNG pages), `app_undo_document`, `app_redo_document` |
| Word (.docx) | Read: `docx_get_outline`, `docx_get_text` (Markdown or plain text), `docx_list_paragraphs`, `docx_get_paragraph`, `docx_list_tables`, `docx_find_text`. Edit: `docx_insert_text`, `docx_delete_text`, `docx_replace_text`, `docx_set_paragraph_text`, `docx_set_paragraph_format`, `docx_set_run_format`, `docx_insert_table` |
| Excel (.xlsx) | Read: `xlsx_list_sheets`, `xlsx_get_used_range`, `xlsx_get_cells`, `xlsx_find_text`, `xlsx_check_machine_readability`, `xlsx_trace_formula`, `xlsx_list_sheet_rules`, `xlsx_list_pivots`, `xlsx_list_query_reports`. Edit: `xlsx_set_cells` (`=` followed by formula text enters a formula), `xlsx_clear_cells`, `xlsx_set_format`, `xlsx_add_sheet`, `xlsx_delete_sheet`, `xlsx_rename_sheet`, `xlsx_calculate_workbook`, `xlsx_create_chart`, `xlsx_set_validation`, `xlsx_clear_validation`, `xlsx_create_pivot`, `xlsx_refresh_pivot`, `xlsx_set_query_report_checks` |
| SQLite (.sqlite, .sqlite3, .db), Access (.mdb, .accdb), ESE (.edb, .sdb) & Outlook (.ost, .pst) | Read: `sqlite_list_tables`, `sqlite_describe_table`, `sqlite_describe_datasets`, `sqlite_count_rows`, `sqlite_query`, `jet_inspect_schema`, `jet_migration_markdown`, `jet_inspect_vba`, `jet_inspect_form`, `ese_inspect_schema`, `pst_list_folders`. Export/Workbook edit: `jet_export_sqlite`, `ese_export_sqlite`, `pst_export_sqlite`, `sqlite_query_to_table`, `sqlite_refresh_query_table`, `sqlite_refresh_query_reports` |
| PowerPoint (.pptx) | Read: `pptx_list_slides`, `pptx_get_slide`, `pptx_get_shape_text`, `pptx_get_notes`, `pptx_list_layouts`. Edit: `pptx_set_shape_text`, `pptx_replace_text`, `pptx_set_run_format`, `pptx_set_notes`, `pptx_add_slide`, `pptx_delete_slide`, `pptx_move_slide`, `pptx_set_frame`, `pptx_delete_shape` |

For `xlsx_create_chart`, give the source `sheet` and the placement `destination_sheet` and `anchor` (for example, `F2`). Supply either a `range` with headers or explicit `series`. The chart stays linked to the source cells.

`xlsx_check_machine_readability` reports candidates by worksheet, type and A1 range. Omit `sheet` to scan visible worksheets, and pass `next_offset` back as `offset` to get the next page.

`xlsx_trace_formula` takes a target `sheet` and one A1 `cell`. Set `direction` to `precedents` or `dependents`. It returns direct, statically resolvable workbook references as sheet names and ranges. Trace results cover visible sheets and visible formulas. Runtime references such as `INDIRECT` and external workbooks are outside this trace. Use `next_offset` to fetch the next page.

`xlsx_list_sheet_rules` lists data validation and conditional formatting rules on a worksheet. Supply an A1 `range` to select rules that intersect it. Each match retains all its applied ranges. Use `kind` (`validation` or `conditional_formatting`) to filter and pass `next_offset` as `offset` for more results. Conditional formatting results describe the condition and evaluation priority. Visual rules such as color scales and data bars are identified by type.

`xlsx_set_validation` replaces validation within an A1 `range`. Use `type: list` with quoted comma-separated choices such as `"Open,Closed"`, a same-sheet reference, or a defined name in `formula1`. Use `whole` or `decimal` with inclusive lower and upper bounds in `formula1` and `formula2`, or `custom` with a condition in `formula1`. Relative references start at the top-left cell. `xlsx_clear_validation` removes validation in the specified range and preserves cell contents.

`sqlite_list_tables` reads a standalone SQLite file with `path` under an allowed folder. Pass a table name as `table` to inspect its declared columns and primary key with `sqlite_describe_table` or count its rows with `sqlite_count_rows`. Set `group_by` to an exact column name to count by value, including a separate NULL group. `limit` caps the returned groups (default 25, maximum 100). Check `truncated` before using the returned groups as a complete distribution. In the desktop spreadsheet assistant, select the same database in the Assistant panel to use these operations in chat.

`sqlite_count_rows` also accepts up to 16 `filters`, all of which must match. Each filter has a `column` and `operator`. Use `eq`, `ne`, `lt`, `le`, `gt` or `ge` with a string or number `value`. Use `is_null` or `is_not_null` without a `value`. The response includes the filters used. SQLite applies its usual type and NULL comparison rules.

`sqlite_query` accepts one `SELECT` or `WITH … SELECT` and optional positional `params`. The result includes SQL, parameters, column names, rows and a `truncated` flag. The preview defaults to 50 rows and allows up to 1,000, subject to the response size limit. Queries have a three-second execution budget.

To query CSV, Parquet, Arrow IPC, JSON Lines, Avro or selected SQLite tables together, pass `datasets` to `sqlite_query` or `sqlite_query_to_table`. Each entry supplies a `source` file path and a SQL table `alias`. An SQLite entry also supplies its original `table` name. Use these aliases in SQL and omit `path`. For example, register a SQLite table as `facts` and a CSV as `labels`, then use `SELECT * FROM facts JOIN labels USING (code)`. In the desktop assistant, select the files together and use the file numbers shown to the assistant as `source`.

Before writing SQL, call `sqlite_describe_datasets` with the same `datasets` array. It returns each alias, format, and the SQL-visible column names and storage types in projection order. CSV columns are `TEXT`; SQLite columns are reported as `DYNAMIC` because their row values can vary. Use `sqlite_describe_table` for SQLite declared types and keys. JSON Lines infers types across a bounded scan (up to 32 MiB). The tool returns no rows.

For a Parquet entry, set `columns` to the exact names of the columns to load in the requested order, for example `["code", "amount"]`. Unselected columns are skipped before applying the input-cell limit, including unselected nested columns. Omit `columns` to load all columns. The selection is saved with query reports and reused on refresh.

Arrow IPC files and streams (`.arrow`, `.ipc`, `.feather`) also support ordered `columns` projection and saved bindings. SQL datasets expose UTF-8 string lists as JSON text, including null elements and empty lists. Use `json_each` to expand them. Dictionary encoding remains unsupported. The reader bounds decoded bytes, selected cells and text, and Turso loads the result into memory.

JSON Lines (`.jsonl`, `.ndjson`) discovers fields and their types in a first pass, then streams rows. Missing values are NULL and arrays and objects remain JSON text. Avro (`.avro`) streams object container record blocks and supports the scalar and nullable types accepted by the spreadsheet importer. Both accept ordered `columns` and retain these selections in saved reports.

In SQL datasets, standard Parquet lists of UTF-8 strings are exposed as JSON text. SQL NULL, empty lists, null elements, duplicates and element order are preserved. Use `json_each` to expand a list. To sum amounts at the original row grain, filter the original rows with `EXISTS (SELECT 1 FROM json_each(f.tags) j WHERE j.value = ?)`. List conditions belong in SQL. The JSON text counts toward the dataset text budget.

A Parquet entry can also specify `where`, an object of conditions on up to 16 columns, combined with AND. A scalar value is an exact match. For example, `{"year": "2026", "submitted": true}` selects only matching rows. A JSON null matches a missing value. Conditions use the column type without text-to-number conversion. Filter columns can be omitted from `columns`. The reader scans the selected and filter columns, then counts only matching output cells and their text toward the input budget. Saved reports reuse these conditions on refresh.

Use `{"amount": {"gte": 100, "lt": 200}}` for a range. The operators are `gt`, `gte`, `lt`, and `lte`. Multiple bounds are combined with AND. Ranges compare numbers or strings in the column’s type, with strings compared lexicographically. Missing values are excluded from ranges.

When exact minimum and maximum statistics with a compatible sort order are present for signed integers or text, the reader skips row groups whose bounds prove they cannot match. It also uses known null counts. Groups with missing or unsuitable statistics are scanned and all retained rows are tested against the conditions.

CSV uses a comma delimiter, a header row, and text fields, so leading zeros and empty strings are preserved. Parquet and SQLite reads preserve NULL. Sources that exceed the readers' exact numeric range are rejected. Up to eight dataset entries and 100,000 input cells total can be loaded. Every query loads its selected tables into a fresh in-memory database.

Use the same SQL and parameters with `sqlite_query_to_table`, providing a workbook `document_id`, `destination_sheet` and single-cell `anchor`. It creates a named Excel table only when the complete result fits the export budget (10,000 rows, 100,000 cells and 4 MiB). Alias duplicate or blank column headings in SQL. Large integers can be cast to text to retain their digits. Choose a blank destination.

Pass the returned table name to `xlsx_create_pivot` with its source `sheet`, or supply a source `range` with headers. Provide a `destination_sheet`, `anchor`, optional `rows` and `columns` arrays of header names, and `values` entries such as `{"field":"operation_id","aggregate":"count"}`. Use a nonblank ID to count records. The desktop assistant stages both workbook edits for review. A table is a snapshot of the SQL result.

`xlsx_list_pivots` lists the pivots on `sheet` with their current range, resolved source and field configuration. Use `next_offset` to request more results. Use `xlsx_refresh_pivot` with that `sheet` and the `pivot` name to rebuild it from its workbook source and update any linked PivotCharts. Refresh refuses to overwrite other data.

A table created by `sqlite_query_to_table` stores the SQL, parameters and ordered source descriptions in the workbook. Use `xlsx_list_query_reports` to discover reports and their source slots. Set `table` to inspect one report's definition. Use `sqlite_refresh_query_table` with `document_id`, the report `table` and a `sources` array in saved order. On MCP, pass files under an allowed directory. In the desktop assistant, select the files and use their file numbers. Workbook metadata keeps source basenames and formats. Each refresh requires explicit bindings.

Refresh replaces the data rows in place and updates PivotTables that use the table name, including their linked charts. The entire edit is undoable. Refresh requires unchanged headers and an unobstructed destination. A successful zero-row query leaves the table headers in place and clears the prior data. The assistant stages the edit for review.

`sqlite_refresh_query_reports` previews or refreshes several reports in one operation. Pass `bindings` as a map from saved filenames to current file paths, for example `{"monthly.csv":"/data/next.csv"}`. The assistant uses selected file numbers in place of paths. A binding is shared by all reports that reference that saved filename. Omit `tables` for all reports or provide up to 12 names. `preview: true` checks overlap and returns previous and new row counts without changing the workbook. Applying re-runs the queries and commits them in one undo step. A query or layout error leaves the entire batch untouched and identifies the failing report.

Saved reports capture the input column names and types from the query session. Single and batch refreshes can return `schema_changes` (up to 20 entries) and `schema_change_count` (the total). Added, removed and retyped columns are warnings. The baseline advances on successful apply, including the first apply of an older report. Standalone SQLite uses ordinary table declarations. Registered datasets use the names and storage types sent to SQL. Refresh returns `input_schema_status: "unavailable"` when the schema exceeds the saved definition budget.

Reports with matching inputs and projections share one database during a batch. Arrow IPC, JSON Lines and Avro are streamed into a private temporary database when their combined source size exceeds 8 MiB or the in-memory read hits its input budget. Other files in the same group can join these tables within their usual input limits. Smaller groups share an in-memory database. `arrow_disk: true` explicitly stages identical Arrow-only bindings on disk. The database is discarded after preparing the batch. To apply a successful preview without rereading any sources, pass only its `snapshot_id` and `document_id` to `sqlite_refresh_query_reports`. The ID is single-use, belongs to the current connection, and requires an unchanged workbook. A newer batch request replaces the prior preview.

Add `metrics` entries such as `{"table":"QueryResult1","column":"cases"}` to compare sums by report. Each selected column returns `previous`, `current`, `delta` and `percent_change`. Blank and NULL cells contribute zero. Text and other nonnumeric values are rejected. The percentage is relative to the absolute previous total and is null if that total is zero.

Use `xlsx_set_query_report_checks` with a saved report's `table` to register up to four checks. Each check has a `name`, one read-only `sql` query, and optional positional `params` and `max`. The SQL must return one nonnegative integer. For example, count unmatched keys with a left join, or count groups with duplicate keys. Checks and report queries share the same bound inputs. Batch preview reports each count and sets `blocked` when a count exceeds its maximum. Applying a single report or a batch fails atomically on that condition. Omit `max` to report a count only. An empty list removes the checks.

Use the following SQL with `max: 0` to stop a refresh when any row in `facts` has no matching key in `labels`. Use the same join condition as the report query.

```sql
SELECT count(*)
FROM facts AS f
WHERE NOT EXISTS (SELECT 1 FROM labels AS l WHERE l.code = f.code)
```

If the check blocks a refresh, run this diagnostic with `sqlite_query` and `limit: 20`. In `datasets`, use the same current files as the refresh bindings and the saved aliases, projections and filters. The diagnostic groups unmatched rows by key and SQLite storage type.

```sql
SELECT f.code AS unmatched_key, typeof(f.code) AS key_type, count(*) AS occurrences
FROM facts AS f
WHERE NOT EXISTS (SELECT 1 FROM labels AS l WHERE l.code = f.code)
GROUP BY f.code, typeof(f.code)
ORDER BY occurrences DESC, key_type, unmatched_key
LIMIT 20
```

To show examples inline, save the same SQL under the check’s `diagnostic: {"sql":"…","params":[]}`. Batch preview reports the diagnostic under `checks[].diagnostic`. It runs in the same SQL session for checks over their maximum or informational checks with a positive count. Results are capped at 20 rows and 2 KiB. Diagnostic errors leave the count and pass/fail result intact.

Compare the results with the master keys. For example, an unmatched integer `1` and a master text key `001` point to a type or representation mismatch. The separate diagnostic reads the sources again. Run it before replacing or editing those files to reproduce the failed check.

On desktop, use **Data → Queries & Connections → Refresh SQL reports** to select reports, bind files, optionally compare numeric columns and review checks. Each report lists its saved checks, maximums and diagnostic availability before preview. Use **Show check SQL** to inspect the count and diagnostic queries. Applying uses exactly the results shown in the preview.

The tools do not correspond one to one to the menu commands: they cover reading and the common edits, with the same tool names whatever the format. With `--read-only`, only the read tools are available.

## Connecting Claude Desktop

Claude Desktop starts the server itself. Open the configuration file from Claude Desktop (Settings, Developer, Edit Config), or edit it directly:

- Windows: `%APPDATA%\Claude\claude_desktop_config.json`
- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`

Windows (use the folder ibisOffice is installed in; `--mcp` is not needed):

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

macOS:

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

If the file already has other servers, add `ibisoffice` next to them under `mcpServers`. Paths must be absolute. Quit Claude Desktop completely and start it again. The server then appears under the connectors of the chat box. Claude Desktop writes its MCP log (`mcp.log` and `mcp-server-ibisoffice.log`) in `%APPDATA%\Claude\logs` on Windows and `~/Library/Logs/Claude` on macOS.

Source: [Connect to local MCP servers](https://modelcontextprotocol.io/docs/develop/connect-local-servers).

## Connecting Claude Code

Windows:

```
claude mcp add ibisoffice -- "C:\Program Files\ibisOffice\ibisOffice-mcp.exe" --allow-dir "C:\Users\me\Documents"
```

macOS:

```
claude mcp add ibisoffice -- /Applications/ibisOffice.app/Contents/MacOS/ibisOffice --mcp --allow-dir ~/Documents
```

Everything after `--` is the command that Claude Code starts. `claude mcp list` shows whether it is connected.

## Connecting ChatGPT

The server talks over standard input and output (stdio) today; SSE and streaming HTTP are planned soon.

## Safety

- Only the files under the allowed folders can be accessed, by their real paths.
- Files are written only by `app_save_document` and `app_export_document`. **A new name is the default**; replacing a file needs `overwrite: true`, and a file changed by another program since it was opened is replaced only with `overwrite: "force"`. A save writes a temporary file next to the file and renames it, so a failure leaves the old file as it was.
- Macros are not run, and nothing is fetched from the network (links and picture URLs are values, not requests).
- The text of a document is data. A result that holds it says so, and the assistant should not follow instructions that are in it.
- A digitally signed file loses its signature when an edit is saved (the result says so).

## Wallet presentation diagnostics

The read-only `presentation_compare` tool compares a single compact SD-JWT
or hex/base64url mdoc with explicit `expected_paths` or a `dcql_query`. Pass exactly one. For SD-JWT,
`iss` and `vct` are treated as metadata. For mdoc the paths follow the
worksheet importer, which qualifies repeated element identifiers with their
namespace. Pass the raw presentation string as `token`, or use
`vp_token` with `dcql_query` to compare a decoded OpenID4VP response.
For example, `"vp_token": {"age_check": ["<presentation>"]}` must
contain exactly the credential ID from the query and one presentation string.
The response reports the matched ID as `vp_token_credential_id`. Extract the
JSON value of `vp_token` from the response, or pass a captured
`application/x-www-form-urlencoded` response body as `direct_post_body`
with `dcql_query`. Pass the original request's `state` as `expected_state`
to get a separate `state_matches` result. An absent or different response
state yields `false`; without an expectation the result is `null`.
Only the `vp_token` and optional `state` fields are accepted in the capture.
Duplicate parameters, malformed percent escapes and encrypted responses
return an error. The comparison tool itself does not open a network listener.
The tool returns paths and diagnostic results without claim values.

Supply a public issuer JWK to check the issuer signature. For SD-JWT, the
report separates the holder signature, presentation hash, nonce and audience.
`null` marks a check without sufficient input. The report does not evaluate
issuer trust, time validity or mdoc device authentication.
A match on claim paths alone is not a successful verification.

The DCQL input supports one credential query with a concrete `vct_values` or
`doctype_value` and a nonempty list of claims. The supported paths are one
field name for SD-JWT and `[namespace, element]` for mdoc. The mdoc report
uses JSON Pointer paths, for example `/org.iso.18013.5.1/age_over_18`.
Optional `values` checks compare strings, booleans and signed integers
without returning any values. The `dcql` result reports type and value
mismatches separately from missing or unexpected paths. Alternatives such
as `claim_sets`, arrays, wildcard paths, and other unsupported options
return an error. Supply the request nonce and audience separately to check
SD-JWT key binding.

Run the offline synthetic Wallet through the CLI and real MCP stdio server:

```sh
cargo build --locked -p ibis-doc-service --example wallet_fixture -p ibis-mcp --example presentation_compare --bin ibisOffice-mcp
python3 scripts/test-wallet-presentation.py
target/debug/examples/wallet_fixture sd-jwt extra --vp-token | target/debug/examples/presentation_compare
```

The fixture Wallet uses public demo keys and synthetic claims. Its mdoc cases
exercise issuer-signed selective disclosure without a device-authenticated
session. Its SD-JWT cases exercise valid key binding and a signed response
with the wrong nonce. No network or external wallet service is required.

### Run the synthetic Wallet in one command

After building the examples, run:

```sh
python3 scripts/wallet-run.py sd-jwt
python3 scripts/wallet-run.py mdoc --scenario extra
```

The runner starts the local HTTP capture, generates a fresh request, invokes
the fixture Wallet and submits its response. It prints one JSON result and
returns exit code 0 when every fixture check passes, 1 on a comparison
failure, or 2 if the run cannot complete. Use `--build-dir` for a Cargo
output directory other than `target/debug`. The mdoc check covers issuer
signature, digests and claims. Device authentication is outside this fixture.

### Run your own Wallet command

An adapter can connect your Wallet CLI to the same loopback regression:

```sh
python3 scripts/wallet-run.py sd-jwt --request-settings verifier.json \
  --wallet-command ./wallet-adapter --profile test
```

Put `--wallet-command` last. The runner starts the program directly with
the supplied arguments. The adapter reads one unsigned authorization
request JSON object from stdin and writes one response JSON object with
`vp_token` and `state` to stdout. The runner sends it to the callback over
HTTP. An adapter can use stderr for logs. Its exit status and malformed
output are treated as execution errors.

In `verifier.json`, set `audience` to the verifier's client ID and supply
the `dcql_query` and public ES256 `issuer_jwk` for the test credential.
The runner supplies a fresh nonce and state. The same report and exit
codes apply. For mdoc, a passing result covers the issuer signature,
digests, attributes and response state. It does not establish device
authentication.

### Capture a local Wallet response over HTTP

After building the examples above, write the request expectations to
`request.json`. Include `format` and `dcql_query`, plus any
`expected_state`, `audience`, `nonce` or public `issuer_jwk` that you need
to compare. The synthetic Wallet can produce a starting configuration:

```sh
target/debug/examples/wallet_fixture sd-jwt normal --vp-token | jq 'del(.vp_token)' > request.json
python3 scripts/wallet-capture.py request.json
```

In another terminal, set `RESPONSE_URI` to the printed URI and send a
synthetic response:

```sh
target/debug/examples/wallet_fixture sd-jwt extra --vp-token | jq -cj '.vp_token' |
  curl --data-urlencode 'vp_token@-' "$RESPONSE_URI"
```

The receiver binds to IPv4 loopback and exits after one
decodable presentation or a timeout. It acknowledges receipt with an empty
JSON object and prints the local comparison report separately. An HTTP 200
does not indicate that the presentation passed its checks. The receiver is
for local testing with HTTP-capable clients. It does not provide HTTPS,
sign authorization requests, or handle `direct_post.jwt`.

To generate an unsigned local authorization request, remove `nonce` and
`expected_state` from the settings, set `audience` to the test verifier's
client identifier, and start the receiver with `--generate-request`. It
adds an `authorization_request` object to the first output line with the
DCQL query, callback URL, and fresh nonce and state. The same session
values are used for the diagnostic comparison. Save just the nested
object as `authorization-request.json` and give it to the synthetic Wallet:

```sh
target/debug/examples/wallet_fixture sd-jwt normal --request < authorization-request.json > wallet-response.json
curl --data-urlencode "vp_token=$(jq -c '.vp_token' wallet-response.json)" \
     --data-urlencode "state=$(jq -r '.state' wallet-response.json)" "$RESPONSE_URI"
```

The fixture accepts its synthetic age-check query and echoes the supplied
state. For SD-JWT it also binds the presentation to the supplied nonce and
client ID. This unsigned loopback flow is intended for local fixtures and
does not exercise production Wallet trust or mdoc device authentication.

### Keep a regression matrix in XLSX

Create a sheet named `Wallet Cases` with these four columns starting at A1:

| case | format | scenario | expected_failures |
|---|---|---|---|
| Age check | sd-jwt | normal | |
| Bad nonce | sd-jwt | wrong-nonce | nonce |
| mDL over-disclosure | mdoc | extra | unexpected_paths |

Use unique case names and literal cells. Leave the last cell blank for a
clean result, or list expected diagnostic failures separated by commas.
The workbook can hold up to 100 cases. After building the examples and
`ibisOffice-mcp` as shown above, run:

```sh
python3 scripts/wallet-matrix.py "wallet cases.xlsx"
```

The runner reads the workbook through the existing read-only MCP XLSX
tools and runs each row through the synthetic Wallet and loopback
verifier. The JSON report compares the exact expected and actual failure
sets. Exit code 0 means every row matched, 1 means at least one differed,
and 2 means the workbook or runner could not complete. Use `--sheet`
to select another tab. The workbook is not modified.

To run an external Wallet across the workbook, use the same layout with
`verifier` instead of `scenario` in column C. Put a verifier settings
filename such as `age-check.json` in each row. The files have the same format
as `--request-settings` and must reside directly in the supplied directory.

```sh
python3 scripts/wallet-matrix.py "wallet cases.xlsx" --verifier-dir ./verifiers \
  --wallet-command ./wallet-adapter --profile test
```

The command and its arguments apply to all rows. Cells select verifier
settings only. The report includes the verifier filename for each case.

To review the results in a spreadsheet, add
`--report-xlsx "wallet results.xlsx"` before any `--wallet-command`.
The runner saves a new copy with `actual_failures` in column E and
`result` in column F of the case sheet, aligned to the original row
numbers. A finished run writes the report even when some cases fail.
The source and any existing destination are kept intact. The JSON output
includes the saved path and any workbook save warnings.
An unstyled input workbook can also be saved as a report.
