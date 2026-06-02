# msbuild-diagnostic — CHECKLIST

Incremental-build diagnosis archive tooling. See
[DESIGN-NOTES.md](DESIGN-NOTES.md) for the canonical design.

Item IDs use the `AR-` prefix (Archive). Items within a milestone are in
dependency order. The final item of each milestone is an integration test.

## Milestone 1 — Snapshot core (no binlog parsing)

- [ ] **AR-1**: Add `[[bin]]` `msbuild-diagnostic` to this crate. Wire `clap` (derive) with a top-level `archive` subcommand taking `--binlog <path>`, repeatable `--root <path>`, `--kind <label>` (default `T1`), `--pair-id <id>`, `--out <dir>` (default cwd), `--small-file-hash-threshold <bytes>` (default 1048576). Subcommand currently prints the parsed args and exits 0.
- [ ] **AR-2**: Implement `tree.json` enumeration over the configured roots. Each entry: `{ relpath, size, mtime_unix_nanos, sha256: Option<String> }`. SHA-256 computed iff `size <= threshold`. Symlinks recorded by target path, not followed. Output to a writer (per the output-abstraction rule).
- [ ] **AR-3**: Implement the zip writer. Stage the `.binlog` (verbatim), `tree.json`, and a placeholder empty `imports/` directory into a temp dir, then zip to `<out>/<archive-name>.zip`. Use deflate compression.
- [ ] **AR-4**: Implement `manifest.json` (timestamp, machine name, OS, configured roots, binlog path inside archive, kind, optional pair-id). Add it to the archive. Archive filename: `<binlog-stem>-<UTC-YYYYMMDDTHHMMSSZ>-<kind>.zip`.
- [ ] **AR-5**: **Integration test** — synthesize a tempdir tree with ~1000 files of varied sizes (some > threshold), invoke the binary, unzip the result into a second tempdir, assert: manifest.json round-trips, tree.json contains every file with correct size/mtime, sha256 present iff under threshold, archive filename matches the naming convention.

## Milestone 2 — Binlog-informed enumeration

- [ ] **AR-6**: Parse the binlog with `munin_msbuild::BinlogIndex::open`. Extract the set of project file paths from `ProjectStarted` events and expose them via a typed `BinlogProjectInventory` struct.
- [ ] **AR-7**: Implement default root discovery (when `--root` not supplied): binlog dir + each project file's parent dir + git root (walk parents for `.git`, stop at filesystem root). Dedupe overlapping paths.
- [ ] **AR-8**: Extract embedded `ProjectImportArchive` entries via `BinlogReader::extract_archives` and write them under `imports/` in the zip, preserving relative paths.
- [ ] **AR-9**: For each project dir, locate `obj/` and copy all `*.tlog` files into the zip under `tlogs/<project-relpath>/`. Files outside any project's `obj/` are not collected.
- [ ] **AR-10**: **Integration test** using the workspace's `testprojects/csharp/helloworld` binlog (build it via the existing task as a fixture if needed). Assert: the project file is in the inventory, at least one `imports/` entry, `tlogs/` is present (may be empty for a hello-world project but the directory must exist).

## Milestone 3 — Diff command

- [ ] **AR-11**: Add `diff <T1.zip> <T2.zip>` subcommand. Parses both archives' `tree.json` and `manifest.json`, reports unchanged/new/deleted/modified-mtime-only/modified-content per file.
- [ ] **AR-12**: From the T2 binlog, enumerate every `TargetStarted` that was **not** preceded by a matching `TargetSkipped`. For each, surface the textual "Building target X because Y" message from the binlog (BuildMessage events emitted by MSBuild's incremental check).
- [ ] **AR-13**: Cross-reference: for each "ran because input newer than output" message, look up both files in the T1/T2 `tree.json` and report the actual mtime+sha256 delta. Output a single Markdown report through the output abstraction.
- [ ] **AR-14**: **Integration test** — author a T1 archive and a T2 archive by hand-constructing zips (controlled timestamps; fabricated minimal binlog produced by a tiny fixture project), assert the diff output identifies the touched-but-unchanged-content file as the cause.

## Milestone 4 — MCP integration and field validation

- [ ] **AR-15**: Add `archive` and `archive_diff` tools to `msbuild-diagnostic-mcp` that call the same library entry points used by the CLI. Update the extension README tool table.
- [ ] **AR-16**: Run against the user's neighboring msbuild tree (the known-bad incremental case). Record observed gaps and surprising behaviors in `DESIGN-NOTES.md` (new D-N entries) and add concrete follow-up items to a new milestone here if anything is missing.
