# Using ibisOffice from an AI assistant (MCP)

**English** | [日本語](mcp-ja.md)

ibisOffice can run as an MCP (Model Context Protocol) server. An AI assistant such as Claude opens Word (.docx), Excel (.xlsx) and PowerPoint (.pptx) files through it, reads and edits their text, cells, tables and slides, undoes edits, and saves or exports the result. No window is opened and nothing leaves your computer: the server runs on your machine and speaks only to the program that started it.

This page, in order:

1. [Starting the server](#starting-the-server)
2. [Allowing access to your folders](#allowing-access-to-your-folders)
3. [Tools](#tools)
4. [Connecting Claude Desktop](#connecting-claude-desktop)
5. [Connecting Claude Code](#connecting-claude-code)
6. [Connecting ChatGPT](#connecting-chatgpt)
7. [Safety](#safety)

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

## Allowing access to your folders

By default, no file can be accessed. Give each folder with `--allow-dir`, for example `--allow-dir ~/Documents --allow-dir ~/Work`. A file can be opened or written only when its real path (links followed, `..` resolved) is under one of these folders and its extension is docx, docm, dotx, xlsx, xlsm, pptx or pptm (an export writes .pdf or .png). Add `--read-only` to let the assistant read but not change anything.

## Tools

Open a file with `app_open_document`. It returns a `document_id` that goes into the other tools. Read, edit, then save with `app_save_document`. Lengths are in points. One tool call is one step of undo.

| Group | Tools |
|---|---|
| App (all formats) | `app_get_version`, `app_list_documents`, `app_open_document`, `app_get_document_info`, `app_save_document`, `app_close_document`, `app_export_document` (PDF or PNG pages), `app_undo_document`, `app_redo_document` |
| Word (.docx) | Read: `docx_get_outline`, `docx_get_text` (Markdown or plain text), `docx_list_paragraphs`, `docx_get_paragraph`, `docx_list_tables`, `docx_find_text`. Edit: `docx_insert_text`, `docx_delete_text`, `docx_replace_text`, `docx_set_run_format`, `docx_insert_table` |
| Excel (.xlsx) | Read: `xlsx_list_sheets`, `xlsx_get_used_range`, `xlsx_get_cells`, `xlsx_find_text`. Edit: `xlsx_set_cells` (a string starting with `=` is a formula), `xlsx_clear_cells`, `xlsx_set_format`, `xlsx_add_sheet`, `xlsx_delete_sheet`, `xlsx_rename_sheet`, `xlsx_calculate_workbook` |
| PowerPoint (.pptx) | Read: `pptx_list_slides`, `pptx_get_slide`, `pptx_get_shape_text`, `pptx_get_notes`, `pptx_list_layouts`. Edit: `pptx_set_shape_text`, `pptx_replace_text`, `pptx_set_run_format`, `pptx_set_notes`, `pptx_add_slide`, `pptx_delete_slide`, `pptx_move_slide`, `pptx_set_frame`, `pptx_delete_shape` |

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
return an error. Capture or HTTP transport is handled by the caller.
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
