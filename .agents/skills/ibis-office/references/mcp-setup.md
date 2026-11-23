# Connect a coding agent to ibisOffice

Keep the command and arguments together when copying examples. Replace example paths with absolute paths on the machine that runs the client. The client starts the server over standard input and output.

## File tools

On macOS and Linux, use the following command for a read-only project connection.

```sh
ibisOffice --mcp --allow-dir /absolute/path/to/project --read-only
```

On Windows, use `ibisOffice-mcp.exe` with the same options and omit `--mcp`. Use the installed executable's full path if the host cannot resolve it. Use a connection without `--read-only` when the user's task requires editing or saving. Each `--allow-dir` sets a file access boundary for the server.

### Codex

```sh
codex mcp add ibis-office -- ibisOffice --mcp --allow-dir /absolute/path/to/project --read-only
```

The equivalent MCP server entry in Codex's TOML configuration is:

```toml
[mcp_servers.ibis-office]
command = "ibisOffice"
args = ["--mcp", "--allow-dir", "/absolute/path/to/project", "--read-only"]
```

### OpenCode

Merge the `mcp` entry into the project's `opencode.json` or `opencode.jsonc`.

```json
{
  "mcp": {
    "ibis-office": {
      "type": "local",
      "command": ["ibisOffice", "--mcp", "--allow-dir", "/absolute/path/to/project", "--read-only"]
    }
  }
}
```

### Claude Code

Merge the server entry into the project's `.mcp.json`.

```json
{
  "mcpServers": {
    "ibis-office": {
      "command": "ibisOffice",
      "args": ["--mcp", "--allow-dir", "/absolute/path/to/project", "--read-only"]
    }
  }
}
```

## Live Assistant on macOS or Linux

Start the GUI with a socket inside a directory only you can access. Use the same socket path in the MCP client's server command.

```sh
session_dir=$(mktemp -d)
ibisOffice --assistant-socket "$session_dir/assistant.sock" /absolute/path/to/project/report.xlsx
```

```sh
ibisOffice --mcp --assistant-socket /absolute/path/to/session/assistant.sock
```

Use the actual path printed by `printf '%s\n' "$session_dir"` in the first shell. Configure the second command as a separate MCP server when you also use file tools. Add `--read-only` for a connection that only inspects live Assistant status and results. Add `--allow-dir` if that server also needs access to files on disk. The socket belongs to the GUI window. A new client process can reconnect while the window stays open.

Use `tools/list` to confirm the connection. File operations use their own `document_id` handles. Live Assistant operations use `run_id` and work on the open GUI window, including unsaved changes.
