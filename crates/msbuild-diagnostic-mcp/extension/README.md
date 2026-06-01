# MSBuild Diagnostic MCP

VS Code extension that bundles the `msbuild-diagnostic-mcp` MCP server and
registers it with VS Code automatically. No `mcp.json` editing required.

> Pre-built binaries ship for **Windows x64 and arm64**. Linux/macOS users
> should
> [build from source](https://github.com/MikeGrier/msbuild-diagnostic-rs#build).

## Commands

- **msbuild-diagnostic-mcp: Copy bundled server binary path** — copies the bundled
  binary path to the clipboard.
- **msbuild-diagnostic-mcp: Show bundled server version** — displays the bundled
  server version.

## Settings

| Setting | Default | Description |
|---|---|---|
| `msbuild-diagnostic-mcp.binaryPath` | _(bundled)_ | Override the path to the `msbuild-diagnostic-mcp` binary. |
| `msbuild-diagnostic-mcp.extraArgs` | `[]` | Extra command-line arguments passed to the server. |

## Local development

```powershell
cd crates/msbuild-diagnostic-mcp/extension
npm install
npm run compile
```

The release workflow builds the platform-appropriate Rust binary, stages it
into `bin/`, then packages a VSIX per target.

## License

MIT
