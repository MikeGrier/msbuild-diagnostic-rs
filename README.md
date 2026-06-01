# Rust workspace template

A GitHub repository template for kicking off a new Rust workspace with the
plumbing already wired up:

- One **core library crate** (always).
- An optional **MCP (Model Context Protocol) server crate** that depends on
  the core lib and exposes tools over JSON-RPC stdio.
- An optional **VS Code extension** that bundles the MCP binary so users
  install one extension and Copilot Chat discovers the server automatically.
- A complete **GitHub Actions** suite: CI (build/test/clippy/fmt/MSRV),
  per-platform extension packaging, CodeQL, Dependabot, and (optional)
  `release-please` + Marketplace auto-publish.

## Two ways to use this template

### Option A — `cargo generate` (interactive, recommended)

Install [cargo-generate](https://github.com/cargo-generate/cargo-generate) if
you don't already have it:

```powershell
cargo install cargo-generate
```

Then generate a new project from this template:

```powershell
cargo generate --git https://github.com/<owner>/<this-repo>
```

You'll be prompted for the project name, GitHub owner, which optional pieces
to include, etc. Folders and files are renamed and substituted automatically.

### Option B — GitHub "Use this template" button

Click **Use this template → Create a new repository** on the GitHub page.
You'll get a wholesale copy of this repository with the literal `{{...}}`
placeholders intact. You then need to:

1. Find/replace every placeholder in file contents (see the list below).
2. Rename the two crate folders `crates/{{core_crate_name}}/` and
   `crates/{{mcp_crate_name}}/` to the real names.
3. Delete `cargo-generate.toml` and this `README.md` (replace with
   `README.template.md`).
4. Delete pieces you don't want (e.g. the MCP crate or the VS Code extension).

The interactive `cargo generate` route is strictly easier; this button is
for people who don't want the extra tool.

## Placeholders

| Placeholder | Meaning |
|---|---|
| `{{project_name}}` | Workspace name; usually the GitHub repo name. |
| `{{project_description}}` | One-line description used in `Cargo.toml` and READMEs. |
| `{{author_name}}` | Author name in `[workspace.package].authors` and copyright headers. |
| `{{github_owner}}` | GitHub user/org; used in repo URL. |
| `{{license_year}}` | Year in the `LICENSE` file. |
| `{{core_crate_name}}` | Name of the core library crate (folder + Cargo `name`). |
| `{{mcp_crate_name}}` | Name of the MCP server crate (only if `include_mcp`). |
| `{{vscode_publisher}}` | VS Code Marketplace publisher id (only if `include_vscode_extension`). |
| `{{vscode_extension_display_name}}` | Display name shown in the Marketplace. |

Booleans `include_mcp`, `include_vscode_extension`, and
`include_release_please` control whether matching files are kept or removed
during generation (see `cargo-generate.toml`).

## What the generated project looks like

```
crates/
  {{core_crate_name}}/       # hello-world library crate, ready to extend
  {{mcp_crate_name}}/        # optional MCP server (lib + bin)
    extension/               # optional VS Code extension that bundles the bin
.github/
  workflows/                 # ci, codeql, build-extension, release-please, publish-extension
  actions/workspace-version/ # composite action: read workspace version from Cargo.toml
tools/
  check-encoding.ps1         # CI guard against text-file encoding corruption
Cargo.toml                   # workspace root
release-please-config.json
.release-please-manifest.json
```

The core crate ships with a single `hello()` function and one passing test.
The MCP crate (if included) ships with a minimal `initialize`/`tools/list`
JSON-RPC scaffold and one example tool, so the workspace builds and tests
green out of the box.

## After generation, do these things

1. **Flip the `Settings → Template repository` switch off** on the generated
   repo (if you got here via the GitHub button — `cargo generate` doesn't
   set this flag).
2. **Set the `RELEASE_PLEASE_TOKEN` secret** if you kept release-please —
   it must be a PAT with `repo` scope, not `GITHUB_TOKEN`. See
   `.github/workflows/release-please.yml` for why.
3. **Create the `marketplace` environment** with a required reviewer and
   the `VSCE_PAT` secret if you kept the VS Code extension publish workflow.
4. **Update `crates/<core>/src/lib.rs`** with your actual code.
5. **Push an initial `feat:` commit** so release-please opens the first
   Release PR.

## License

This template repository itself is MIT. Generated projects inherit MIT by
default; change the `LICENSE` file and `[workspace.package].license` in
`Cargo.toml` if you want something else.
