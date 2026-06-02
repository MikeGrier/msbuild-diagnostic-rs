# msbuild-diagnostic — Design Notes

Current canonical design decisions for the `msbuild-diagnostic` crate. Each
decision answers "what is the decision?" and "what constraint forced it?".

## Decision Index

- [D-1: Tool surface — CLI binary in this crate, MCP wrapper later](#d-1-tool-surface)
- [D-2: Incremental-build diagnosis archive — symmetric T1/T2 payload](#d-2-incremental-build-diagnosis-archive)
- [D-3: Archive payload contents](#d-3-archive-payload-contents)
- [D-4: File-tree snapshot record schema](#d-4-file-tree-snapshot-record-schema)
- [D-5: Default root discovery](#d-5-default-root-discovery)
- [D-6: Archive container format and naming](#d-6-archive-container-format-and-naming)
- [D-7: Specified behavior is owned, munin-msbuild is the implementation choice](#d-7-specified-behavior-is-owned-munin-msbuild-is-the-implementation-choice)

---

### D-1: Tool surface

The primary surface is a CLI binary `msbuild-diagnostic` (added to this
crate as a `[[bin]]`) with subcommands. The MCP integration in
`msbuild-diagnostic-mcp` becomes a thin wrapper that calls the same
library entry points so both surfaces are guaranteed to behave
identically.

Constraint: the developer's first use is reproducing a known-bad
incremental build at the command line in a neighboring repo; an
interactive MCP-only path would block that workflow.

### D-2: Incremental-build diagnosis archive

The tool captures a **symmetric** payload at both T1 (clean or
baseline build) and T2 (incremental rebuild). The two archives can
then be diffed to answer "why did MSBuild decide target X had to run
in T2?".

Constraint: the binlog already records *why* MSBuild ran each target
("building because input A is newer than output B"). Verifying that
reason after the fact requires preserved evidence about A and B at
both points in time. Capturing the same shape at both times keeps the
diff trivially comparable and removes the need to predict in advance
which timestamps will matter.

### D-3: Archive payload contents

Each archive (`.zip`) contains:

1. The `.binlog` file, verbatim.
2. All `*.tlog` files reachable from each project's `obj/` directory.
3. `tree.json` — file-tree snapshot of the configured root(s) (see
   D-4 and D-5).
4. `manifest.json` — capture metadata (see D-6).
5. `imports/` — files extracted from the binlog's embedded
   `ProjectImportArchive` records (so the import graph survives even
   if source files later move/disappear).

Constraint: `.tlog` files are MSBuild's own input/output tracking
truth for many task families; without them the diagnosis is missing
one of two primary "why" sources. `tree.json` provides the
ground-truth timestamps that the binlog's reasoning refers to.

### D-4: File-tree snapshot record schema

Each entry in `tree.json` is `{ "relpath": String, "size": u64,
"mtime_unix_nanos": i128, "sha256": Option<String> }`. SHA-256 is
computed only when `size <= small_file_hash_threshold` (default 1
MiB).

Constraint: distinguishing "file was re-stamped with identical
content" from "file actually changed" requires content identity. For
large binary outputs the cost outweighs the benefit; thresholding
keeps the snapshot bounded while still catching the common
"`Copy` task touched an unchanged file" failure mode. Threshold is
configurable so users with pathological trees can adjust.

### D-5: Default root discovery

Default roots when `--root` is not passed:

1. Directory containing the `.binlog`.
2. Every project file directory referenced by the binlog (parsed via
   `munin_msbuild::BinlogIndex` ProjectStarted events).
3. The git repository root (walk parents looking for `.git`), if any.

User may override with one or more `--root <path>` flags.

Constraint: the binlog knows exactly which projects were built;
auto-discovery from it is more reliable than asking the user. Adding
the git root catches `Directory.Build.props` and other up-the-tree
imports.

### D-6: Archive container format and naming

Container is a single `.zip` file. Name:
`<binlog-stem>-<UTC-YYYYMMDDTHHMMSSZ>-<kind>.zip` where `kind` is
`T1` or `T2` (free-form labels permitted; `T1`/`T2` is the
convention). `manifest.json` includes: ISO-8601 timestamp, machine
name, OS, MSBuild and dotnet versions (parsed from the binlog when
available), configured roots, binlog path inside the archive, the
`kind` label, and an optional `pair_id` linking T1 and T2.

Constraint: `.zip` is universally readable, supports the modest sizes
expected (low-MB to low-GB), and lets users extract individual files
without bespoke tooling.

### D-7: Specified behavior is owned, munin-msbuild is the implementation choice

We **specify** that the archive records the project files MSBuild
built, the embedded `ProjectImportArchive` contents, and (for the
diff command) the skip/run reason for each target. We **use**
`munin_msbuild` to parse the binlog because its public surface
(`BinlogIndex::open`, `indices_by_kind`, `ProjectStarted.project_file`,
`BinlogReader::extract_archives`) already exposes everything our
specification needs.

Constraint: design autonomy requires our behavior to be defined
independently of any dependency. If `munin_msbuild` ever stops
exposing one of these fields, the dependency is wrong, not our
specification — we'd wrap, fork, or replace it.
