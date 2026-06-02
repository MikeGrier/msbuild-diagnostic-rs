# msbuild-diagnostic — CHECKLIST

Incremental-build diagnosis archive tooling. See
[DESIGN-NOTES.md](DESIGN-NOTES.md) for the canonical design.

Item IDs use the `AR-` prefix (Archive). Items within a milestone are in
dependency order. Every milestone ends with an integration test and
(where new data is introduced) a sanitization checkpoint per D-10.

## Milestone 1 — Snapshot core (no binlog parsing)

- [x] **AR-1**: Add `[[bin]]` `msbuild-diagnostic` to this crate. Wire `clap` (derive) with a top-level `archive` subcommand taking `--binlog <path>`, repeatable `--root <path>`, `--kind <label>` (default `T1`), `--pair-id <id>`, `--out <dir>` (default cwd), `--small-file-hash-threshold <bytes>` (default 1048576). Subcommand currently prints the parsed args and exits 0.
- [x] **AR-2**: Define typed `TreeSnapshot` value (and entry struct: `{ relpath, size, mtime_unix_nanos: TimestampNs, sha256: Option<String> }`) with `serde` derive. `TimestampNs` is a newtype around `i128` serialized as a JSON string (D-13). Implement the snapshotter that walks the configured roots and produces a `TreeSnapshot`. SHA-256 computed iff `size <= threshold`. Symlinks recorded by target path, not followed. The snapshotter is the **only** code in the crate permitted to call `std::fs::metadata` / `read_dir` (D-12). Hermetic unit tests cover the `TreeSnapshot` JSON round-trip and the timestamp newtype's serialization (D-14).
- [x] **AR-3**: Implement the zip writer. Stage the `.binlog` (verbatim), `tree.json`, and a placeholder empty `imports/` directory into a temp dir, then zip to `<out>/<archive-name>.zip`. Deflate compression.
- [x] **AR-4**: Implement `manifest.json` (timestamp, machine name, OS, configured roots, binlog path inside archive, kind, optional pair-id). Add it to the archive. Archive filename: `<binlog-stem>-<UTC-YYYYMMDDTHHMMSSZ>-<kind>.zip`.
- [x] **AR-5**: **Integration test** — synthesize a tempdir tree with ~1000 files of varied sizes (some > threshold), invoke the binary, unzip the result, assert: manifest.json round-trips, tree.json contains every file with correct size/mtime, sha256 present iff under threshold, archive filename matches the naming convention. (Integration-tier per D-14; uses the real filesystem.)
- [x] **AR-6**: **Sanitization checkpoint (M1)** — enumerate every field introduced by M1 (`tree.json` relpath / size / mtime / sha256, `manifest.json` machine name / OS / roots / binlog path). Classify each under D-9 (verbatim / redact / drop). Write `crates/msbuild-diagnostic/src/sanitize/rules.rs` skeleton with the per-field decisions encoded as data; add a unit test asserting unknown fields are dropped by default.

## Milestone 2 — Binlog-informed enumeration

- [x] **AR-7**: Parse the binlog with `munin_msbuild::BinlogIndex::open`. Extract project file paths from `ProjectStarted` events; expose via a typed `BinlogProjectInventory` struct.
- [x] **AR-8**: Implement default root discovery (when `--root` not supplied): binlog dir + each project file's parent dir + git root. Dedupe overlapping paths.
- [x] **AR-9**: Extract embedded `ProjectImportArchive` entries via `BinlogReader::extract_archives` into `imports/` in the zip, preserving relative paths.
- [x] **AR-10**: For each project dir, locate `obj/` and copy all `*.tlog` files into the zip under `tlogs/<project-relpath>/`. Files outside any project's `obj/` are not collected.
- [ ] **AR-11**: **Integration test** using `testprojects/csharp/helloworld/msbuild.binlog`. Assert: project file in the inventory, at least one `imports/` entry, `tlogs/` directory present.
- [ ] **AR-12**: **Sanitization checkpoint (M2)** — for each new artifact (binlog records exposed via parsing, `ProjectImportArchive` payloads, `*.tlog` files), classify under D-9. Add property values and environment-variable records to a redact list; verify `*.tlog` schemas in scope are allow-listed and unknown extensions under `obj/` get dropped. Add a fixture containing a fake credential string in a binlog property; test asserts it is redacted.

## Milestone 3 — Diff command

- [ ] **AR-13**: Add `diff <T1.zip> <T2.zip>` subcommand. The diff algorithm itself is a pure function `(TreeSnapshot, TreeSnapshot) -> DiffReport` (D-12) covered by hermetic unit tests with inline / fixture JSON inputs (D-14). The subcommand only handles I/O: unzip, deserialize, call the pure function, serialize the result.
- [ ] **AR-14**: From the T2 binlog, enumerate every `TargetStarted` not preceded by `TargetSkipped`. Surface the "Building target X because Y" BuildMessage text.
- [ ] **AR-15**: Cross-reference: for each "ran because input newer than output" message, look up both files in T1 and T2 `TreeSnapshot` values and report the actual mtime+sha256 delta. The correlator is a pure function over `(BinlogModel, TreeSnapshot, TreeSnapshot) -> CorrelationReport` (D-12), unit-tested with JSON fixtures (D-14). Output a Markdown report through the output abstraction.
- [ ] **AR-16**: **Integration test** — build a real tiny fixture project with MSBuild to produce a real binlog, then use a `touch_with_mtime` helper (`std::fs::File::set_modified`) to drive a file's mtime to a known value, archive at T1 and T2, assert the diff identifies a touched-but-content-identical file as the cause. (Integration-tier per D-14.)
- [ ] **AR-17**: **Sanitization checkpoint (M3)** — the diff Markdown report quotes file paths and binlog messages. Confirm path pseudonymization (D-9) is applied to the report output, and that quoted binlog message text is run through the same property-redaction pass as M2. Add a test with a path containing the current user's profile dir; assert it appears as `<USER>/...` in the report.

## Milestone 4 — Feedback loop: sanitize + report

- [ ] **AR-18**: Implement `sanitize <archive.zip> [--out <path>]` subcommand. Reads the archive, applies D-9 rules using the rule registry built across M1–M3, emits `<stem>-sanitized.zip` plus a sibling `<stem>-pseudonym-map.local.json` containing the per-archive path/name mapping. Map file is **never** included in the sanitized zip.
- [ ] **AR-19**: Add `sanitization-report.json` to every sanitized archive: list of files included verbatim, files included after redaction (with rule ID), files dropped (with reason and rule ID). Add a top-level `unknown_artifacts` array — anything the rule registry had no decision for, dropped by default.
- [ ] **AR-20**: Implement `report` subcommand. Takes one archive or a `--pair T1.zip T2.zip`, plus `--expected <text>` and `--actual <text>`. Runs `sanitize` on each input, writes `submission.zip` and an extracted `submission-preview/` directory, and writes `ISSUE.md` (structured template: expected vs actual diagnosis, environment summary derived from sanitized manifest, list of redactions applied).
- [ ] **AR-21**: `report` prints (does not open) a prefilled `https://github.com/MikeGrier/msbuild-diagnostic-rs/issues/new?title=...&body=...` URL and explicit instructions to attach `submission.zip` manually. No HTTP calls in this command.
- [ ] **AR-22**: **Integration test** — sanitize a fixture archive that contains (a) a path under a synthetic user profile, (b) a binlog property whose value is a fake API key, (c) an unknown file under `obj/`. Assert: path pseudonymized in sanitized output, property redacted, unknown file absent from sanitized zip and listed in `sanitization-report.json` under `unknown_artifacts`, pseudonym map file present *outside* the zip and absent *inside* it.
- [ ] **AR-23**: **Sanitization checkpoint (M4)** — review the `ISSUE.md` template itself for leakage (the body is what the user pastes into GitHub). Confirm every field interpolated into it is sourced from the sanitized manifest, never the original. Add a test asserting the rendered `ISSUE.md` contains no path with the real user profile dir.

## Milestone 5 — MCP integration and field validation

- [ ] **AR-24**: Add `archive`, `archive_diff`, `archive_sanitize`, and `archive_report` tools to `msbuild-diagnostic-mcp` that call the same library entry points. Update the extension README tool table.
- [ ] **AR-25**: MCP `archive_report` returns the prefilled GitHub issue URL plus a list of files in `submission-preview/`; it does not transmit `submission.zip` over the MCP channel. Test asserts the response payload contains no file contents.
- [ ] **AR-26**: Run against the user's neighboring msbuild tree (the known-bad incremental case). For every gap encountered, file an issue via the `report` flow as dogfooding. Record findings in `DESIGN-NOTES.md` (new D-N entries) and add follow-up items to a new milestone here if needed.
