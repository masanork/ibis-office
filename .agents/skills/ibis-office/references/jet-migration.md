# Inspecting & Migrating Access, ESE, and Outlook Data Assets

Use these MCP tools to inspect legacy Microsoft database files without Windows, COM, or ODBC drivers, query them via Turso, or migrate them to Google Workspace (`.xlsx` / Sheets) and standalone `.sqlite`.

## 1. Microsoft Access (`.mdb`, `.accdb`, `.mde`, `.accde`)

1. **Inventory schema & business logic**:
   - Call `jet_inspect_schema` with `path` to get tables, columns, indexes, relationships, saved query SQL, form/report names, and VBA module names.
   - Call `jet_migration_markdown` with `path` to generate a Markdown specification with a Mermaid `erDiagram` and saved query SQL blocks.
2. **Query & join with other datasets**:
   - Call `sqlite_describe_datasets` and `sqlite_query` with `datasets` entries such as:
     ```json
     {"source": "/allowed/legacy.accdb", "table": "Orders", "alias": "orders", "columns": ["id", "code", "amount"]}
     ```
   - Access 2007+ MultiValue, Attachment, and VersionHistory columns (`ComplexType`) are expanded into JSON arrays; use `json_each(...)` in SQLite/Turso SQL to unnest them.
3. **Materialize or export**:
   - Use `jet_export_sqlite` (read-write mode) to materialize Access tables into a standalone `.sqlite` file for fast repeated querying.
   - Use `sqlite_query_to_table` to export query results into an `.xlsx` workbook (directly importable into Google Sheets).

## 2. ESE / JET Blue (`.edb`, `.sdb`, `.dat`)

- Call `ese_inspect_schema` with `path` to list tables and column types.
- Query tables via `datasets` (`{"source": "/allowed/store.edb", "table": "TableName", "alias": "t"}`) or materialize with `ese_export_sqlite`.

## 3. Outlook Offline / Personal Stores (`.ost`, `.pst`)

- Call `pst_list_folders` with `path` to inspect folder hierarchy and item counts.
- Query via `datasets` using virtual tables `"folders"` or `"messages"` (`{"source": "/allowed/archive.ost", "table": "messages", "alias": "mail"}`) or materialize with `pst_export_sqlite`.
