<!-- Copyright (c) {{license_year}} {{author_name}} -->
# {{project_name}}

[![CI](https://github.com/{{github_owner}}/{{project_name}}/actions/workflows/ci.yml/badge.svg)](https://github.com/{{github_owner}}/{{project_name}}/actions/workflows/ci.yml)
{% if include_release_please %}[![release-please](https://github.com/{{github_owner}}/{{project_name}}/actions/workflows/release-please.yml/badge.svg)](https://github.com/{{github_owner}}/{{project_name}}/actions/workflows/release-please.yml)
{% endif %}[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

{{project_description}}

## Crates

| Crate | What it is |
|---|---|
| [`{{core_crate_name}}`](crates/{{core_crate_name}}) | Core library crate. |
{% if include_mcp %}| [`{{mcp_crate_name}}`](crates/{{mcp_crate_name}}) | MCP (Model Context Protocol) server that exposes tools to AI agents like GitHub Copilot. |
{% endif %}{% if include_vscode_extension %}| [`{{vscode_extension_display_name}}` VS Code extension](crates/{{mcp_crate_name}}/extension) | Bundles the MCP server binary and registers it with VS Code automatically. |
{% endif %}

## Build

Requires a recent Rust toolchain (MSRV: see `[workspace.package].rust-version`
in [Cargo.toml](Cargo.toml)).

```powershell
cargo build --workspace --release
cargo test --workspace
{% if include_mcp %}cargo run --release -p {{mcp_crate_name}}
{% endif %}
```

{% if include_release_please %}## Release pipeline

Versioning, tagging, and publishing are automated:

1. Land commits on `main` using
   [Conventional Commits](https://www.conventionalcommits.org/)
   (`fix:`, `feat:`, `feat!:`).
2. [`release-please`](.github/workflows/release-please.yml) opens or updates
   a Release PR that bumps the workspace version{% if include_vscode_extension %},
   the extension's `package.json`,{% endif %} and the changelog.
3. Merging the Release PR creates a `v<version>` tag.
{% if include_vscode_extension %}4. [`publish-extension`](.github/workflows/publish-extension.yml) builds
   per-platform VSIXes, then — gated behind a required-reviewer environment —
   publishes them to the VS Code Marketplace and attaches them to a GitHub
   Release.
{% endif %}

Crates.io publishing is currently manual.
{% endif %}

## License

MIT — see [LICENSE](LICENSE).
