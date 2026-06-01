# {{vscode_extension_display_name}}

VS Code extension that bundles the `{{mcp_crate_name}}` MCP server and
registers it with VS Code automatically. No `mcp.json` editing required.

> Pre-built binaries ship for **Windows x64 and arm64**. Linux/macOS users
> should
> [build from source](https://github.com/{{github_owner}}/{{project_name}}#build).

## Commands

- **{{mcp_crate_name}}: Copy bundled server binary path** — copies the bundled
  binary path to the clipboard.
- **{{mcp_crate_name}}: Show bundled server version** — displays the bundled
  server version.

## Settings

| Setting | Default | Description |
|---|---|---|
| `{{mcp_crate_name}}.binaryPath` | _(bundled)_ | Override the path to the `{{mcp_crate_name}}` binary. |
| `{{mcp_crate_name}}.extraArgs` | `[]` | Extra command-line arguments passed to the server. |

## Local development

```powershell
cd crates/{{mcp_crate_name}}/extension
npm install
npm run compile
```

The release workflow builds the platform-appropriate Rust binary, stages it
into `bin/`, then packages a VSIX per target.

## License

MIT
