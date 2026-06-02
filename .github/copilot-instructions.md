# Copilot Instructions

Operating rules for Copilot in this repository. The rules below mix
general engineering conventions the author applies across projects with
repository-specific policies (MSBuild binlogs, mono-repo handling,
`.scratch/` usage, etc.). Finer-grained, component-scoped rules belong
in a separate file alongside the code they govern (e.g. `AGENTS.md`,
`COMPONENT.md`, or `DESIGN-INSTRUCTIONS.md`).

## Interaction style

- Be concise. Skip filler, restatement, and decorative formatting.
- No emojis unless explicitly requested.
- When a tool prompt requires a secret (password, token, API key), tell the
  user to type it directly into the terminal. Never collect secrets through
  ask-question UIs or model-visible channels.

## Terminal and Git rules — hang prevention

These rules prevent terminal hangs that freeze the session.

- Every `git` subcommand that can produce paged output **must** be run with
  `git --no-pager <subcommand>`: `diff`, `show`, `log`, `blame`, `reflog`,
  `stash list`, `branch -v`, `shortlog`, `tag -n`, `whatchanged`, `grep`,
  and any other. **If unsure whether a `git` subcommand may page, use
  `--no-pager`.**
- Never run `git commit` without `-m "…"`. Commit messages passed via `-m`
  must be a **single line**. For longer messages, write the message to a
  file under `.scratch/` and use `git commit -F .scratch/<file>`.
- Never use PowerShell here-strings (`@"…"@`) or embedded newlines inside a
  `-m` argument — PowerShell either hangs waiting for the terminator or
  passes `\n` literally.
- Never run `git pull` or `git merge` without `--no-edit`.
- Never run interactive commands: `git rebase -i`, `git add -p`, etc.
- Do not use `less`, `more`, or any other interactive pager.

## Scratch directory for temporary files

When capturing command output, test results, debug logs, build warnings, or
other temporary diagnostic data to disk, **always write under the
`.scratch/` directory** at the repository root. This directory is
git-ignored.

- Create `.scratch/` if it does not exist.
- Use descriptive filenames (e.g. `.scratch/test_parser_output.txt`,
  `.scratch/build_warnings.txt`).
- Never write scratch or debug files to the repository root or any source
  directory.

## File I/O — use `tpu_*` MCP tools when available

<!-- tpu-mcp:setup:begin -->
If your client has the **tpu-mcp** MCP server installed and registered,
prefer the `tpu_*` tools over PowerShell or shell file commands.
Availability is per-user (driven by the client's installed extensions /
MCP registrations), not per-repository — check the available tool list at
the start of the session rather than assuming. Plain
`Get-Content` / `Set-Content` / `Out-File` / `>` / `cat` / `sed` round-trip
files through the active code page and silently corrupt UTF-8, UTF-16,
smart quotes, em-dashes, and box-drawing characters.

| MCP tool | Use it for |
|---|---|
| `tpu_read_file` | reading text files (UTF-8, UTF-16, Windows-1252, Shift-JIS, …) |
| `tpu_read_head` / `tpu_read_tail` | first/last N lines or bytes |
| `tpu_read_file_binary` | inspecting raw bytes of binary files |
| `tpu_read_file_escaped` | reading text as a single 7-bit-clean escaped line |
| `tpu_write_file` | replacing a text file's full contents |
| `tpu_append_file` | appending text to an existing file |
| `tpu_replace_in_file` | regex / fixed-string substitution (use `fixed_strings: true` for literal targets) |
| `tpu_edit_file` | targeted insert/delete/splice at known line numbers |
| `tpu_validate_file` | pre-flight assertion that a file is in the expected state |
| `tpu_count_file` | line / word / char / byte / pattern counts |
| `tpu_find` | encoding-aware grep across files and globs |
| `tpu_copy_file` | copy a file or recursively copy a tree |
| `tpu_render_file` | populate a file from a `{{TOKEN}}` template |
| `tpu_stat_file` | verify a write actually persisted (mtime / size) |

- Prefer `tpu_replace_in_file` with `fixed_strings: true` over
  `tpu_edit_file` when the target text is unique — line numbers can shift
  between reads.
- Guard writes with `validate: [{ "selector": "line-contains:N", "value":
  "..." }]` when the file is expected to be in a particular state.
- When `tpu_*` tools are unavailable and you must fall back to PowerShell,
  do not round-trip non-ASCII files through `Get-Content` / `Set-Content`
  — read and write via `[System.IO.File]::ReadAllBytes` /
  `WriteAllBytes`.
- When `tpu_*` tools are unavailable on a non-Windows host (Linux/macOS
  bash, zsh, etc.), ensure a UTF-8 locale before running text tools
  (`LC_ALL=C.UTF-8` or `LC_ALL=en_US.UTF-8`), and never pipe binary
  files through text utilities (`cat`, `sed`, `awk`, `tr`) — operate on
  them with byte-safe tools (`dd`, `xxd`, `od`, language-native byte
  APIs) instead.
- Text content sent to `tpu_*` tools is auto-normalized to LF before
  processing; the file's existing line-ending convention is preserved on
  disk.
<!-- tpu-mcp:setup:end -->

## Cargo commands — use `cargo_*` MCP tools when available

When your client has the **cargo-mcp** MCP server installed and
registered, **always** use the `cargo_*` tools instead of running `cargo`
in a terminal. This applies even inside a larger workflow — do not switch
to the terminal for cargo just because a previous step used the terminal.
Availability is per-user (driven by the client's installed extensions /
MCP registrations), not per-repository — check the available tool list at
the start of the session rather than assuming.

| MCP tool | Replaces |
|---|---|
| `cargo_metadata` | `cargo metadata` |
| `cargo_check` | `cargo check` |
| `cargo_build` | `cargo build` |
| `cargo_test` | `cargo test` |
| `cargo_clippy` | `cargo clippy` |
| `cargo_fmt_check` | `cargo fmt --check` |
| `cargo_fmt` | `cargo fmt` |
| `cargo_tree` | `cargo tree` |
| `cargo_doc` | `cargo doc` |
| `cargo_clean` | `cargo clean` |
| `cargo_update` | `cargo update` |
| `cargo_fix` | `cargo fix` |
| `cargo_add` | `cargo add` |
| `cargo_remove` | `cargo remove` |
| `cargo_publish` | `cargo publish` |
| `cargo_diagnostic` | *(no terminal equivalent)* |

Operational rules:

- Run `cargo_fmt` and `cargo_clippy` before every commit; fix all
  formatting issues and clippy warnings before pushing.
- Use `cargo_clean` before a clean rebuild — not the terminal.
- Use `cargo_add` / `cargo_remove` / `cargo_update` for dependency
  management. Do not hand-edit `Cargo.toml` for dependency version
  changes when these tools are available.
- Use `cargo_fix` to apply machine-applicable fixes in bulk after
  `cargo_check` or `cargo_clippy`.
- `cargo_publish` — always run with `dry_run: true` first; only publish
  for real when the dry-run succeeds.

## C# / MSBuild builds — always emit a binlog

Building with an MSBuild binary log (`.binlog`) costs almost nothing and
captures a complete, structured record of the build that can be inspected
afterward.

**Rule:** any command that builds a C# project or solution — `dotnet
build`, `dotnet msbuild`, `MSBuild.exe`, or anything that builds as a side
effect (`dotnet test`, `dotnet publish`, `dotnet pack`, wrapping scripts)
— **must** emit a binlog next to what is being built.

How to satisfy the rule:

- Pass `-bl:<path>.binlog` (a.k.a. `/bl` or `--binaryLogger`) to whichever
  tool drives the build. Every entry point above accepts it.
- Place the binlog next to the project or solution being built —
  `<project-dir>/msbuild.binlog` for a `.csproj`, or
  `<sln-dir>/<sln-name>.binlog` for a `.sln`. Do **not** write binlogs
  into `bin/` or `obj/` (they get cleaned).
- Update existing build tasks rather than adding parallel ones. If a
  `.vscode/tasks.json`, `Makefile`, PowerShell script, or CI workflow
  already builds a project, add `-bl:` to that existing command. Do not
  add a second "build with binlog" task that duplicates it.
- Leave the C# Dev Kit's auto-generated build/run actions alone. They do
  not emit binlogs and they are not under our control. If a binlog from a
  Dev Kit-driven build is needed, run the corresponding explicit task
  instead.

Example task:

```jsonc
{
  "label": "build <project>",
  "type": "process",
  "command": "dotnet",
  "args": [
    "build",
    "${workspaceFolder}/path/to/<project>/<project>.csproj",
    "-bl:${workspaceFolder}/path/to/<project>/msbuild.binlog"
  ],
  "problemMatcher": "$msCompile",
  "group": "build"
}
```

### Covering Dev Kit-driven builds — ask the user first

The per-task `-bl:` flag does not cover builds that the C# Dev Kit
triggers itself (Solution Explorer build/run, design-time builds, test
discovery). A `Directory.Build.rsp` file at the repo root containing
`-bl:msbuild.binlog` covers those uniformly because MSBuild auto-prepends
it to every command line.

This is a repo-wide change that affects every contributor. **Do not add
it silently.** When setting up or modifying C# build configuration in a
repo that does not already have a `Directory.Build.rsp` covering binlogs,
surface the choice first:

> This repo doesn't capture binlogs for Dev Kit-driven builds. How would
> you like to handle that?
>
> 1. Add a repo-wide `Directory.Build.rsp` so every MSBuild invocation
>    emits a binlog (covers Dev Kit, CLI, and CI uniformly).
> 2. Only add `-bl:` to the specific task being set up now.
> 3. Skip — don't capture binlogs from Dev Kit.

If option 1 is chosen, create `Directory.Build.rsp` at the repo root
containing the single line `-bl:msbuild.binlog` and note that the path is
relative to the build's working directory and concurrent builds will
overwrite the same file.

## Vendored source attribution

Any source code checked in from another project ("vendored") must include
clear attribution:

- A comment at the top of the file with the original repository URL, the
  original license, and a note of any modifications made.
- The commit message must mention the source and the modifications.
- A copy of the original license text where the license requires it.
- If the original source is not licensed for redistribution, do not check
  it in.

## Design Autonomy — behavior is owned, never inherited from dependencies

We **define** our behavior. We **choose** dependencies that can satisfy our
definition.

It is never acceptable to describe behavior as "whatever crate X does" or
"we delegate to library Y." That surrenders our autonomy and makes it
impossible to reason about correctness, versioning risk, or future
migration.

The correct framing:

1. State **what our specified behavior is** (inputs accepted, outputs
   produced, errors raised).
2. Note **which dependency is used to achieve it** and that the dependency
   was chosen because its behavior matches the specification.
3. If a dependency's actual behavior diverges from our specification, the
   dependency is wrong, not our specification. Constrain it, wrap it, or
   replace it.

We may align our specification with a dependency's behavior when that
behavior is sensible — but the specification must still be written down
explicitly and owned by us. When a dependency is upgraded or replaced, our
specification does not change; only the implementation does.

Applies to file formats, parse rules, error messages, wire protocols,
encoding choices, and any other observable behavior.

## Mono-repo bug policy — fix the layer, don't work around it

In a mono-repo, when work in one component reveals a bug or deficiency in
an underlying component, **fix it at the source** rather than working
around it in the consumer. The whole point of the mono-repo is that every
layer is owned and can be changed together.

If the fix would require significant refactoring that derails the current
task, raise the issue to the engineer driving forward progress so the
decision to fix-now or defer is explicit. The default is always: fix the
bug where it lives.

## Defect-class harvesting — ask "what class of learning is this?"

Every defect, surprise, or "huh, that's weird" moment is an instance of
a **class** of similar problems we will see again. Before fixing it
in-place and moving on, pause for a one-line judgement call:

> Is this an isolated typo / one-off mistake, or is it an instance of a
> recurring class — something where the next instance will look
> *different in detail but identical in shape*?

If it's a class, do **not** just fix the instance. Take the small extra
step that converts the one fix into ongoing leverage. Concretely:

- A bug that a check could have caught → add the check (lint, test,
  schema validation, CI guard), not just the fix.
- A surprise the docs / design notes didn't predict → add a
  `DESIGN-NOTES.md` entry (or update one) capturing the invariant, so
  the next reader is not surprised.
- A workflow step that was easy to forget → add it to a checklist
  template or to `copilot-instructions.md`.
- A diagnostic that took multiple round-trips to understand → improve
  the tool's output (or file a checklist item to), so the next
  occurrence is self-explanatory.
- A class of user data we hadn't classified → add the classification
  rule (e.g. sanitization-rule registry) in the same commit as the fix.

The rule is **not** "always take the long path". Small one-offs stay
small. The rule is: never skip the 30-second judgement call. The cost
of asking is trivial; the cost of repeatedly re-learning the same
class of lesson compounds.

When in doubt, default to capturing the lesson in writing (a
`DESIGN-NOTES.md` entry, a memory note, or a new check). The cheapest
form of harvest is a single-line note that a future search can find.

## Coding conventions

### No manifest numeric constants in source code

Never write bare integer or byte literals as discriminants or protocol
tags inline in logic code. Use either a named `#[repr(u8)]` enum or a
`mod` of typed `const` values, and use those names everywhere — in match
arms, `vec![]` pushes, assertions, and doc tables. Both approaches are
acceptable; consistency within a file or module matters.

Bad:

```rust
v.push(4u8);   // what is 4?
vec![255u8]    // magic
assert_eq!(key, vec![0u8]);
```

Good (enum):

```rust
#[repr(u8)]
enum ValueKeyTag { DbNull = 0, Text = 4, Err = 255 }

v.push(ValueKeyTag::Text as u8);
vec![ValueKeyTag::Err as u8];
assert_eq!(key, vec![ValueKeyTag::DbNull as u8]);
```

Good (const module):

```rust
mod tags {
    pub const DBNULL: u8 = 0;
    pub const TEXT:   u8 = 4;
    pub const ERR:    u8 = 255;
}

v.push(tags::TEXT);
vec![tags::ERR];
assert_eq!(key, vec![tags::DBNULL]);
```

Applies to all binary encoding schemes, wire protocols, file format tags,
sort-key type bytes, and anywhere a numeric value carries identity
meaning. The enum or const module lives in the same file or module as the
logic that uses it, and its doc comment must note that changing any value
is a breaking change.

### Output abstraction — never print from multiple sites in a tool

Never call `stdout` / `stderr` / `print` / `eprintln` (or the language
equivalent) from more than one site in a tool. At the first occurrence,
introduce an output abstraction — a writer trait, a sink, or a formatter
— and route every subsequent output through it. The abstraction need not
be elaborate (a single trait with one `write_str` method is enough); the
requirement is that the storage target (file, channel, stdout, stderr)
and the formatting concern be separable from the call sites that produce
content.

Applies to any feature whose output may plausibly need to be retargeted:
CLI output, log output, diagnostic output, generated artifacts.

## Source-Components

A **Source-Component** is a directory hierarchy rooted at a directory
containing either a `Cargo.toml` file or a `COMPONENT.md` file. The
repository root is itself usually a source-component, with smaller
source-components nested inside.

## Planning — always plan in CHECKLIST.md

- Write a plan as a `CHECKLIST.md` at the **lowest common
  source-component** that contains the change whenever **any** of the
  following is true:
  - the change touches **more than one file**, **or**
  - the change adds/removes/modifies **more than ~20 lines** total, **or**
  - the change spans **more than one source-component**, **or**
  - the change requires **more than one commit** to land, **or**
  - the user explicitly asks for a plan.
  Single-file edits of ≤20 lines (typo fixes, one-line bug fixes,
  rename-in-place, comment tweaks) do **not** require a CHECKLIST.
- Keep the plan up to date as it executes.
- Maintain a `PLANS.md` at the repository root tracking every active
  `CHECKLIST.md` and its status. Create it if missing.
- When a `CHECKLIST.md` completes, move it to a `COMPLETED-PLANS.md`
  table in the same directory and remove it from `PLANS.md`.

`PLANS.md` table:

| Path to CHECKLIST.md | Status | Brief description | Design Notes |

`COMPLETED-PLANS.md` table:

| Path to CHECKLIST.md | Completion Date | Brief description | Design Notes |

Status values: `not started`, `in progress`, `completed`.

Design Notes column: path(s) to relevant `DESIGN-NOTES.md` file(s), or
`N/A`.

### Plan sizing

If a plan exceeds roughly 10 work items or 3 levels of nesting,
checkpoint it into a `CHECKLIST.md` file in the repository before
continuing. The goal is that the plan survives a lost session.

### Milestones and sub-step notation

Checklists longer than 2–3 items should be organized into milestones.
Milestones should typically contain about 5 work items (guideline, not a
rule) and should end with integration tests where possible. Work items
within a milestone must be self-contained and in dependency order.

When a checklist step is broken into sub-steps, **always use decimal
notation**: `RC-1.1`, `RC-1.2`, `RC-1.3` (or whatever prefix is in use).
Never use lettered sub-items (`RC-1a`, `RC-1b`) or nested bullet lists to
represent sub-steps.

### End-of-milestone steps (implicit, never in the checklist)

At the end of every milestone, the following steps are required and must
**not** be written as checklist items:

1. Clean compile of the entire repo, with **zero warnings**, in both
   debug and release. "Clean" means: discard prior build artifacts first
   so all warnings re-emit (incremental caching can suppress them). Fix
   all warnings in the repo, even those unrelated to the milestone's
   changes.
2. Test only the in-scope crate / source-component, not the entire repo.
3. Sync with origin: `git fetch`, then merge or rebase the current branch
   on top of the updated upstream tip (`--no-edit`), resolve any
   conflicts, then push. Pushing is permitted at milestone boundaries
   without further confirmation; outside milestone boundaries, follow the
   standard "ask before pushing" rule.

## Checklist execution discipline

**Per-item ritual — run this exact sequence after finishing each item, in
order, before touching the next item:**

1. Run the appropriate test command for the in-scope source-component.
2. If tests fail: fix failures caused by this item. Pre-existing,
   unrelated failures do not block the commit but **must** be recorded in
   `UNRESOLVED-TEST-FAILURES.md` before proceeding.
3. Check the item off in `CHECKLIST.md` (`- [ ]` → `- [x]`).
4. If this was the **last** item in the checklist, update its entry in
   `PLANS.md` to `completed` **in the same commit**.
5. Commit with message: `Completed item: <item-id>: <full item text>`.
6. `git fetch` and rebase/merge the current branch on the updated
   upstream tip (`--no-edit`).
7. Push **only** if at a milestone boundary or explicitly prompted.

The rules below explain *why* each step exists and define the invariants
that must hold across items.

- **One item at a time.** Implement exactly one checklist item, then
  **stop and commit**, then move on. "Stop" means: do not begin reading,
  planning, or editing for the next item until the current one's commit
  has succeeded.
- **Hard prohibition on batching.** Do not implement, edit, or stage
  changes for item N+1 until item N is committed. If code is touched
  that belongs to a later item, **revert that touch** and finish the
  current item first. Convenience, similarity, or shared context is
  **not** an exception. Once code for two items is intermingled in the
  working tree, the per-item commit rule has already been violated; the
  only correct response is a single combined commit referencing both
  item IDs (this is a defect to acknowledge, not a workflow to repeat).
- **Commit immediately after each item** — before moving to the next.
- **Commit message format:** `Completed item: <item-id>: <full item
  text>`.
- **Check the item off** in `CHECKLIST.md` (`- [ ]` → `- [x]`) in the
  same commit.
- After the commit, pull / rebase from origin. Push only at milestone
  boundaries unless prompted to do so.
- **Tests must pass** before committing. Run the appropriate test
  command after each item and fix failures before committing.
  Pre-existing failures unrelated to the current item do not block the
  commit, but must be recorded in `UNRESOLVED-TEST-FAILURES.md` first.
- **When the last item in a CHECKLIST file is completed**, update its
  `PLANS.md` entry to `completed` in the same commit.

## CHECKLIST file hygiene

CHECKLIST files are **action-only**. They contain pending, in-progress,
and recently completed (`[x]`) items awaiting migration to
`COMPLETED-CHECKLIST.md`. Never leave historical records, prose
summaries, rationale, or context in a CHECKLIST file.

When a group of related items is fully complete:

1. Move the completed group to `COMPLETED-CHECKLIST.md` in the same
   directory.
2. Prefix the moved block with a heading: `## Moved YYYY-MM-DD —
   <brief description of what was done>`.
3. `COMPLETED-CHECKLIST.md` is **append-only**; always add new groups
   at the bottom.
4. Leave only the remaining pending or in-progress items in the source
   `CHECKLIST.md`.

Named feature files (`CHECKLIST-<feature>.md`) should be **deleted
entirely** once all items are complete. Move their content to
`COMPLETED-CHECKLIST.md` in the same directory before deleting.

## Design notes

Any directory may contain a `DESIGN-NOTES.md` recording design decisions
about the code in that directory and its children. When a decision needs
to be recorded, write it to either the source-component's
`DESIGN-NOTES.md` (create if missing) or to an existing
`DESIGN-NOTES.md` in any ancestor directory between the changed file and
the source-component root — whichever is closer.

### What to include

Anything a future developer should or may want to know to get up to
speed or diagnose interesting / bad behaviors.

### What not to include

Obvious things, or tutorials on underlying technology. A design note
must describe intent and unique approach, not teach the reader about
the field. External links for further reading are fine.

### Three-tier design documentation

Source-components with substantial design history should separate
current decisions from historical rationale using three tiers:

- **Tier 1: `DESIGN-NOTES.md`** — current canonical decisions. Decision
  indexes and compact detail sections stating what was decided and why.
  Every paragraph must answer "what is the decision?" or "what
  constraint forced this choice?" — not "what else did we consider?"
- **Tier 2: `DESIGN-RATIONALE.md`** — historical record of how
  decisions were reached. Alternatives considered, prior art, design
  session summaries, evolutionary reasoning. Cross-referenced by
  decision ID from Tier 1. Consulted for "why" questions, not for
  forward implementation work.
- **Tier 3: `design-sessions/DESIGN-SESSION-<YYYY-MM-DD>-<topic>.md`**
  — raw design session transcripts. Reference material, not routinely
  loaded. Stored under a `design-sessions/` subdirectory of the
  source-component root.

When recording a new decision, write to both Tier 1 and Tier 2 in the
same commit. **Never treat Tier 2 or Tier 3 as authoritative for
current decisions.** If a Tier conflicts with Tier 1, Tier 1 wins.

A source-component may have a `DESIGN-INSTRUCTIONS.md` specifying
additional design rules — including how these tiers are used — for that
component and everything below it. When working in a directory, locate
and follow the nearest `DESIGN-INSTRUCTIONS.md` in that directory or
any ancestor up to the source-component root. These directives are
binding for all work under that directory.

Not all source-components need all three tiers. Small components may
have only `DESIGN-NOTES.md`.

### Design session files

When a design conversation produces extended discussion beyond what fits
in a Tier 2 rationale section, capture it as a design session file
under `design-sessions/`. Name: `DESIGN-SESSION-<YYYY-MM-DD>-<topic-slug>.md`.
Content: a faithful record of the discussion — reasoning, alternatives,
conclusions. Does not need to be polished prose, but must be readable
by a future developer. Include a brief summary at the top noting which
decisions (D-numbers) resulted from the session.

### Historical record

As features age out of a source-component, move notes that are no
longer relevant to `DESIGN-NOTES-AGED-OUT.md` in the same directory.
Include the date of the move in `YYYY/MM/DD` format.

## Quality

### Coverage expectations
Tests must cover at least 10 normal cases plus every identifiable edge
case, unless edge-case computation would be excessive on a modern
system. Unit tests for a submodule should complete in **under one
second** on a low-end modern processor (e.g. an AMD Ryzen 7 at 1.5 GHz
with 16 GB RAM). If a vital test would take longer, capture it as a
`CHECKLIST.md` item flagged for the user to decide whether to include
— and if included, author it as an **integration test**, not a unit
test.

### Unit tests

Always reproducible. Do not use random sampling at runtime without the
developer's explicit approval; if approved, record the decision in a
`DESIGN-NOTES.md`.

### Integration tests

Use larger-scale data — start in the hundreds or thousands of elements
where applicable. Data does not have to be stable; small fixtures
(<10 KB) should be checked in (a file or encoded in source), larger
ones may be generated at runtime, exhaustively or via random
techniques.

### Build / test scope during a milestone

Build and test only the source-component in scope while iterating
inside a milestone. The full repo-wide clean build runs at milestone
boundaries (see "End-of-milestone steps" above).
