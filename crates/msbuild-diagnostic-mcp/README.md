<!-- Copyright (c) 2026 Mike Grier -->
# msbuild-diagnostic-mcp

MCP (Model Context Protocol) server for `msbuild-diagnostic-rs`.
Communicates over JSON-RPC 2.0 on stdio.

## Tools

| Tool | Description |
|---|---|
| `hello` | Example tool that returns a greeting. Replace with your own. |

## Build

```powershell
cargo build --release -p msbuild-diagnostic-mcp
```

The binary is produced at `target/release/msbuild-diagnostic-mcp.exe`.

## VS Code configuration

Add to `.vscode/mcp.json`:

```json
{
    "servers": {
        "msbuild-diagnostic-mcp": {
            "type": "stdio",
            "command": "${workspaceFolder}/target/release/msbuild-diagnostic-mcp.exe",
            "args": []
        }
    }
}
```

The easier path for end users is the bundled VS Code extension under
[`extension/`](extension), which auto-registers this server with no
`mcp.json` editing required.

