<!-- Copyright (c) 2026 Mike Grier -->
# msbuild-diagnostic-rs

[![CI](https://github.com/MikeGrier/msbuild-diagnostic-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/MikeGrier/msbuild-diagnostic-rs/actions/workflows/ci.yml)
[![release-please](https://github.com/MikeGrier/msbuild-diagnostic-rs/actions/workflows/release-please.yml/badge.svg)](https://github.com/MikeGrier/msbuild-diagnostic-rs/actions/workflows/release-please.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Tools to help work with msbuild based project builds to diagnose and correct bad behaviors

## Crates

| Crate | What it is |
|---|---|
| [`msbuild-diagnostic`](crates/msbuild-diagnostic) | Core library crate. |
| [`msbuild-diagnostic-mcp`](crates/msbuild-diagnostic-mcp) | MCP (Model Context Protocol) server that exposes tools to AI agents like GitHub Copilot. |
| [`MSBuild Diagnostic MCP` VS Code extension](crates/msbuild-diagnostic-mcp/extension) | Bundles the MCP server binary and registers it with VS Code automatically. |


## Build

Requires a recent Rust toolchain (MSRV: see `[workspace.package].rust-version`
in [Cargo.toml](Cargo.toml)).

```powershell
cargo build --workspace --release
cargo test --workspace
cargo run --release -p msbuild-diagnostic-mcp

```

## Release pipeline

Versioning, tagging, and publishing are automated:

1. Land commits on `main` using
   [Conventional Commits](https://www.conventionalcommits.org/)
   (`fix:`, `feat:`, `feat!:`).
2. [`release-please`](.github/workflows/release-please.yml) opens or updates
   a Release PR that bumps the workspace version,
   the extension's `package.json`, and the changelog.
3. Merging the Release PR creates a `v<version>` tag.
4. [`publish-extension`](.github/workflows/publish-extension.yml) builds
   per-platform VSIXes, then — gated behind a required-reviewer environment —
   publishes them to the VS Code Marketplace and attaches them to a GitHub
   Release.


Crates.io publishing is currently manual.


## License

MIT — see [LICENSE](LICENSE).
