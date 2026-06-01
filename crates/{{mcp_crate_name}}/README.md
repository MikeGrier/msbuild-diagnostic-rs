<!-- Copyright (c) {{license_year}} {{author_name}} -->
# {{mcp_crate_name}}

MCP (Model Context Protocol) server for `{{project_name}}`.
Communicates over JSON-RPC 2.0 on stdio.

## Tools

| Tool | Description |
|---|---|
| `hello` | Example tool that returns a greeting. Replace with your own. |

## Build

```powershell
cargo build --release -p {{mcp_crate_name}}
```

The binary is produced at `target/release/{{mcp_crate_name}}.exe`.

## VS Code configuration

Add to `.vscode/mcp.json`:

```json
{
    "servers": {
        "{{mcp_crate_name}}": {
            "type": "stdio",
            "command": "${workspaceFolder}/target/release/{{mcp_crate_name}}.exe",
            "args": []
        }
    }
}
```
{% if include_vscode_extension %}
The easier path for end users is the bundled VS Code extension under
[`extension/`](extension), which auto-registers this server with no
`mcp.json` editing required.
{% endif %}
