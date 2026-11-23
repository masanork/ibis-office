---
name: ibis-office
description: Operate ibisOffice through MCP. Use for reading, editing, or checking Word, Excel, or PowerPoint files with ibisOffice, analyzing datasets for a workbook, or directing the Assistant in an open ibisOffice window.
---

# Operate ibisOffice

Choose the connection based on what the user wants to change.

- **A file on disk.** Use the file tools. Discover the installed schemas with `tools/list`, open the file with `app_open_document`, and use the returned `document_id` in the same MCP process. Inspect the relevant paragraphs, cells, or slides before editing. File edits change the in-memory document. Save to the requested path with `app_save_document` and reopen or read back the result to verify it. For a document template, save to a new output path.
- **A document already open in the GUI.** Use the live Assistant connection. Check `assistant_status` and confirm the active document before sending `assistant_prompt`. Use the returned `run_id` to poll `assistant_result`, passing its `cursor` as `after` for subsequent polls. A response in `awaiting_review` describes a proposed edit. When the user explicitly asks to apply that proposal, call `assistant_apply` with its exact `run_id` if the connection offers it. This updates the open document. Saving remains a separate operation.

If the server is unavailable, read [references/mcp-setup.md](references/mcp-setup.md) for connection examples and platform differences. If an advertised tool differs from an example, use the installed schema.

For recurring workbook checks, use [references/monthly-report.md](references/monthly-report.md). It includes a small synthetic input and an expected result for checking the workflow. For ad hoc dataset analysis, describe the datasets before writing SQL, use bounded result windows, and report the exact query, source, and any truncation. Respect the user's choice to preview, edit, or save.
