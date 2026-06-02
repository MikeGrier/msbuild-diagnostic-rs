// Copyright (c) 2026 Mike Grier

//! Cross-reference target-run reasons against the captured tree
//! snapshots (AR-15).
//!
//! Per D-7 our specification is: for every MSBuild BuildMessage of the
//! form `... "<input>" is newer than ... "<output>" ...` (the canonical
//! "input newer than output" diagnostic), look up both file paths in
//! the T1 and T2 [`TreeSnapshot`] values and report the observable
//! mtime/sha256 deltas. The correlator is a pure function over
//! `(BinlogModel, TreeSnapshot, TreeSnapshot) -> CorrelationReport`
//! (D-12) and never touches the filesystem.
//!
//! A finding whose `input.content_identical == true` is the
//! "touched-but-content-identical" signal AR-16 asserts on: the input's
//! mtime moved forward but its sha256 (and size, kind) are unchanged,
//! so MSBuild rebuilt for nothing.
//!
//! Markdown rendering uses the [`std::io::Write`] trait as its output
//! abstraction so the same code paths drive an in-memory buffer (tests)
//! and a file (CLI).

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use munin_msbuild::BinlogEvent;
use serde::{Deserialize, Serialize};

use crate::diff::DiffEntry;
use crate::snapshot::{EntryKind, TimestampNs, TreeEntry, TreeSnapshot};
use crate::targets::TargetRun;

/// Schema version of the JSON shape of [`CorrelationReport`].
pub const CORRELATION_REPORT_SCHEMA_VERSION: u32 = 1;

/// A single `"<input>" is newer than ... "<output>"` BuildMessage tied
/// to the nearest preceding non-skipped [`TargetRun`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewerThanEvent {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub target_name: Option<String>,
    pub project_file: Option<String>,
}

/// The typed binlog projection AR-15 consumes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinlogModel {
    pub target_runs: Vec<TargetRun>,
    pub newer_than_events: Vec<NewerThanEvent>,
}

/// One file's before/after view, derived from the two snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryChange {
    pub before: Option<DiffEntry>,
    pub after: Option<DiffEntry>,
    pub mtime_changed: bool,
    pub sha256_changed: bool,
    /// `true` when the file is present in both snapshots, mtime moved,
    /// and content (size + sha256 + kind) is identical. This is the
    /// "touched-but-content-identical" case that defeats incremental
    /// build — the file MSBuild thinks is dirty actually is not.
    pub content_identical: bool,
}

/// One correlated finding: a "newer than" message plus the looked-up
/// state of both files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrelationFinding {
    pub target_name: Option<String>,
    pub project_file: Option<String>,
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    /// `None` when the input path could not be found in either
    /// snapshot (e.g. lies outside any captured root).
    pub input: Option<EntryChange>,
    pub output: Option<EntryChange>,
}

/// Top-level correlation report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrelationReport {
    pub schema_version: u32,
    pub findings: Vec<CorrelationFinding>,
}

// ---------------------------------------------------------------------------
// Binlog → typed model
// ---------------------------------------------------------------------------

/// Build the [`BinlogModel`] from a binlog event stream. Pure (D-12).
pub fn build_binlog_model(events: &[BinlogEvent]) -> BinlogModel {
    use std::collections::HashSet;

    let mut skipped: HashSet<(String, String)> = HashSet::new();
    let mut current_target: Option<(Option<String>, Option<String>)> = None;
    let mut newer_than = Vec::new();
    // Re-run the AR-14 extractor for target_runs to keep that view
    // authoritative — both views are derived from the same event stream
    // here so they cannot disagree.
    let target_runs = crate::targets::extract_target_runs(events);

    for event in events {
        match event {
            BinlogEvent::TargetSkipped(s) => {
                if let (Some(p), Some(n)) = (&s.fields.project_file, &s.target_name) {
                    skipped.insert((p.clone(), n.clone()));
                }
            }
            BinlogEvent::TargetStarted(s) => {
                let project = s
                    .project_file
                    .clone()
                    .or_else(|| s.fields.project_file.clone());
                if let (Some(p), Some(n)) = (&project, &s.target_name) {
                    if skipped.contains(&(p.clone(), n.clone())) {
                        continue;
                    }
                }
                current_target = Some((s.target_name.clone(), project));
            }
            BinlogEvent::Message(m) => {
                if let Some(text) = &m.fields.message {
                    if let Some((input, output)) = parse_newer_than(text) {
                        let (tname, pfile) = current_target.clone().unwrap_or((None, None));
                        newer_than.push(NewerThanEvent {
                            input_path: input,
                            output_path: output,
                            target_name: tname,
                            project_file: pfile,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    BinlogModel {
        target_runs,
        newer_than_events: newer_than,
    }
}

/// Parse `... "<A>" ... newer than ... "<B>" ...` out of a BuildMessage
/// text. Returns the first two quoted paths when the phrase `newer
/// than` appears between them.
pub fn parse_newer_than(text: &str) -> Option<(PathBuf, PathBuf)> {
    let (first, after_first) = extract_first_quoted(text)?;
    let (second, _) = extract_first_quoted(after_first)?;
    // Confirm "newer than" appears between the two quoted segments.
    let between_start = text.find(&first)? + first.len();
    let between_end = text[between_start..].find(&second)? + between_start;
    let between = &text[between_start..between_end];
    if !between.contains("newer than") {
        return None;
    }
    Some((PathBuf::from(first), PathBuf::from(second)))
}

fn extract_first_quoted(text: &str) -> Option<(String, &str)> {
    let start = text.find('"')? + 1;
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some((rest[..end].to_string(), &rest[end + 1..]))
}

// ---------------------------------------------------------------------------
// Correlation
// ---------------------------------------------------------------------------

/// Pure: correlate `model` against the two snapshots.
pub fn correlate(model: &BinlogModel, t1: &TreeSnapshot, t2: &TreeSnapshot) -> CorrelationReport {
    let index_t1 = SnapshotIndex::build(t1);
    let index_t2 = SnapshotIndex::build(t2);

    let findings = model
        .newer_than_events
        .iter()
        .map(|ev| {
            let input = lookup_change(&ev.input_path, &index_t1, &index_t2);
            let output = lookup_change(&ev.output_path, &index_t1, &index_t2);
            CorrelationFinding {
                target_name: ev.target_name.clone(),
                project_file: ev.project_file.clone(),
                input_path: ev.input_path.clone(),
                output_path: ev.output_path.clone(),
                input,
                output,
            }
        })
        .collect();

    CorrelationReport {
        schema_version: CORRELATION_REPORT_SCHEMA_VERSION,
        findings,
    }
}

/// Path → [`TreeEntry`] index for a single snapshot. The key is the
/// joined absolute path (`root.join(entry.relpath)`); we also store the
/// owning root so callers can build [`DiffEntry`]s with consistent
/// relpaths.
struct SnapshotIndex<'a> {
    by_full_path: HashMap<PathBuf, &'a TreeEntry>,
}

impl<'a> SnapshotIndex<'a> {
    fn build(t: &'a TreeSnapshot) -> Self {
        let mut by_full_path = HashMap::new();
        for root in &t.roots {
            for entry in &root.entries {
                by_full_path.insert(root.root.join(&entry.relpath), entry);
            }
        }
        Self { by_full_path }
    }

    fn get(&self, path: &Path) -> Option<&'a TreeEntry> {
        // Exact match first. Then suffix-match: any indexed full path
        // whose tail equals the queried path. This handles binlog
        // messages that quote paths the runner sees as relative.
        if let Some(e) = self.by_full_path.get(path) {
            return Some(*e);
        }
        for (full, entry) in &self.by_full_path {
            if full.ends_with(path) {
                return Some(*entry);
            }
        }
        None
    }
}

fn to_diff_entry(e: &TreeEntry) -> DiffEntry {
    DiffEntry {
        relpath: e.relpath.clone(),
        size: e.size,
        mtime_unix_nanos: TimestampNs(e.mtime_unix_nanos.0),
        sha256: e.sha256.clone(),
        entry_kind: e.entry_kind.clone(),
    }
}

fn lookup_change(
    path: &Path,
    t1: &SnapshotIndex<'_>,
    t2: &SnapshotIndex<'_>,
) -> Option<EntryChange> {
    let a = t1.get(path);
    let b = t2.get(path);
    if a.is_none() && b.is_none() {
        return None;
    }
    let before = a.map(to_diff_entry);
    let after = b.map(to_diff_entry);
    let mtime_changed = match (a, b) {
        (Some(x), Some(y)) => x.mtime_unix_nanos != y.mtime_unix_nanos,
        _ => false,
    };
    let sha256_changed = match (a, b) {
        (Some(x), Some(y)) => x.sha256 != y.sha256,
        _ => false,
    };
    let size_kind_changed = match (a, b) {
        (Some(x), Some(y)) => x.size != y.size || !same_kind(&x.entry_kind, &y.entry_kind),
        _ => false,
    };
    let content_identical = match (a, b) {
        (Some(_), Some(_)) => mtime_changed && !sha256_changed && !size_kind_changed,
        _ => false,
    };
    Some(EntryChange {
        before,
        after,
        mtime_changed,
        sha256_changed,
        content_identical,
    })
}

fn same_kind(a: &EntryKind, b: &EntryKind) -> bool {
    matches!(
        (a, b),
        (EntryKind::File, EntryKind::File)
            | (EntryKind::Directory, EntryKind::Directory)
            | (EntryKind::Symlink { .. }, EntryKind::Symlink { .. })
    )
}

// ---------------------------------------------------------------------------
// Markdown rendering
// ---------------------------------------------------------------------------

/// Render `report` as Markdown into `w`. Uses [`Write`] as the output
/// abstraction so the same routine drives in-memory buffers (tests),
/// files (CLI), and pipes (future MCP tool). All path-bearing and
/// message fields are routed through `pseudonymizer` per AR-17 / D-9.
pub fn write_markdown_report<W: Write>(
    report: &CorrelationReport,
    w: &mut W,
    pseudonymizer: &crate::sanitize::pseudonym::Pseudonymizer,
) -> io::Result<()> {
    writeln!(w, "# Incremental build correlation report")?;
    writeln!(w)?;
    writeln!(
        w,
        "Schema version: {}. Findings: {}.",
        report.schema_version,
        report.findings.len()
    )?;
    writeln!(w)?;
    if report.findings.is_empty() {
        writeln!(w, "_No `is newer than` BuildMessages were captured._")?;
        return Ok(());
    }
    for (i, f) in report.findings.iter().enumerate() {
        writeln!(w, "## Finding {}", i + 1)?;
        if let Some(t) = &f.target_name {
            writeln!(w, "- Target: `{t}`")?;
        }
        if let Some(p) = &f.project_file {
            writeln!(w, "- Project: `{}`", pseudonymizer.rewrite(p))?;
        }
        writeln!(
            w,
            "- Input:  `{}`",
            pseudonymizer.rewrite(&f.input_path.display().to_string())
        )?;
        writeln!(
            w,
            "- Output: `{}`",
            pseudonymizer.rewrite(&f.output_path.display().to_string())
        )?;
        write_change(w, "Input", &f.input)?;
        write_change(w, "Output", &f.output)?;
        writeln!(w)?;
    }
    Ok(())
}

fn write_change<W: Write>(w: &mut W, label: &str, change: &Option<EntryChange>) -> io::Result<()> {
    match change {
        None => writeln!(w, "  - {label}: not found in either snapshot.")?,
        Some(c) => {
            let kind = if c.content_identical {
                "touched-but-content-identical"
            } else if c.mtime_changed && c.sha256_changed {
                "modified (mtime + sha256)"
            } else if c.mtime_changed {
                "mtime changed"
            } else if c.sha256_changed {
                "sha256 changed"
            } else if c.before.is_some() && c.after.is_none() {
                "removed at T2"
            } else if c.before.is_none() && c.after.is_some() {
                "added at T2"
            } else {
                "unchanged"
            };
            writeln!(w, "  - {label}: {kind}.")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Hermetic unit tests (D-14). Inputs are constructed in memory.
    use super::*;
    use crate::snapshot::{EntryKind, RootSnapshot, TimestampNs, TreeEntry, TreeSnapshot};
    use munin_msbuild::events::{BuildMessageEvent, TargetStartedEvent};

    fn entry(relpath: &str, size: u64, mtime: i128, sha: Option<&str>) -> TreeEntry {
        TreeEntry {
            relpath: PathBuf::from(relpath),
            size,
            mtime_unix_nanos: TimestampNs(mtime),
            sha256: sha.map(|s| s.to_string()),
            entry_kind: EntryKind::File,
        }
    }

    fn snap(root: &str, entries: Vec<TreeEntry>) -> TreeSnapshot {
        TreeSnapshot {
            schema_version: 1,
            small_file_hash_threshold: 1024,
            roots: vec![RootSnapshot {
                root: PathBuf::from(root),
                entries,
            }],
        }
    }

    #[test]
    fn parse_newer_than_extracts_first_two_quoted_paths() {
        let text = r#"Input file "C:\src\a.cs" is newer than output file "C:\out\a.dll"."#;
        let parsed = parse_newer_than(text).expect("should parse");
        assert_eq!(parsed.0, PathBuf::from(r"C:\src\a.cs"));
        assert_eq!(parsed.1, PathBuf::from(r"C:\out\a.dll"));
    }

    #[test]
    fn parse_newer_than_requires_phrase_between_quotes() {
        // Two quoted strings but no "newer than" between them.
        assert!(parse_newer_than(r#"Compare "a" and "b" maybe."#).is_none());
    }

    #[test]
    fn parse_newer_than_requires_two_quoted_strings() {
        assert!(parse_newer_than(r#""only one" thing"#).is_none());
        assert!(parse_newer_than("no quotes here").is_none());
    }

    #[test]
    fn build_binlog_model_collects_newer_than_with_target_context() {
        let ts = TargetStartedEvent {
            target_name: Some("Compile".into()),
            project_file: Some("P.csproj".into()),
            ..TargetStartedEvent::default()
        };
        let mut msg = BuildMessageEvent::default();
        msg.fields.message = Some(r#"Input "a.cs" is newer than "a.dll"."#.into());
        let events = vec![BinlogEvent::TargetStarted(ts), BinlogEvent::Message(msg)];
        let model = build_binlog_model(&events);
        assert_eq!(model.newer_than_events.len(), 1);
        let ev = &model.newer_than_events[0];
        assert_eq!(ev.target_name.as_deref(), Some("Compile"));
        assert_eq!(ev.project_file.as_deref(), Some("P.csproj"));
        assert_eq!(ev.input_path, PathBuf::from("a.cs"));
        assert_eq!(ev.output_path, PathBuf::from("a.dll"));
    }

    #[test]
    fn correlate_detects_content_identical_touched_input() {
        let model = BinlogModel {
            target_runs: Vec::new(),
            newer_than_events: vec![NewerThanEvent {
                input_path: PathBuf::from("a.cs"),
                output_path: PathBuf::from("a.dll"),
                target_name: Some("Compile".into()),
                project_file: Some("P.csproj".into()),
            }],
        };
        let t1 = snap(
            "/repo",
            vec![
                entry("a.cs", 10, 1000, Some("AA")),
                entry("a.dll", 20, 500, Some("BB")),
            ],
        );
        let t2 = snap(
            "/repo",
            vec![
                // mtime moved forward, sha unchanged: touched-but-identical
                entry("a.cs", 10, 2000, Some("AA")),
                entry("a.dll", 20, 500, Some("BB")),
            ],
        );
        let report = correlate(&model, &t1, &t2);
        assert_eq!(report.findings.len(), 1);
        let f = &report.findings[0];
        let input = f.input.as_ref().expect("input should be found");
        assert!(input.content_identical, "input change: {input:?}");
        assert!(input.mtime_changed);
        assert!(!input.sha256_changed);
        let output = f.output.as_ref().expect("output should be found");
        assert!(!output.content_identical);
        assert!(!output.mtime_changed);
    }

    #[test]
    fn correlate_detects_genuine_content_change() {
        let model = BinlogModel {
            target_runs: Vec::new(),
            newer_than_events: vec![NewerThanEvent {
                input_path: PathBuf::from("a.cs"),
                output_path: PathBuf::from("a.dll"),
                target_name: None,
                project_file: None,
            }],
        };
        let t1 = snap("/repo", vec![entry("a.cs", 10, 1000, Some("AA"))]);
        let t2 = snap("/repo", vec![entry("a.cs", 11, 2000, Some("BB"))]);
        let report = correlate(&model, &t1, &t2);
        let input = report.findings[0].input.as_ref().unwrap();
        assert!(!input.content_identical);
        assert!(input.sha256_changed);
    }

    #[test]
    fn correlate_returns_none_for_paths_outside_any_root() {
        let model = BinlogModel {
            target_runs: Vec::new(),
            newer_than_events: vec![NewerThanEvent {
                input_path: PathBuf::from("/elsewhere/x.cs"),
                output_path: PathBuf::from("/elsewhere/x.dll"),
                target_name: None,
                project_file: None,
            }],
        };
        let t1 = snap("/repo", vec![entry("a.cs", 10, 1000, None)]);
        let t2 = snap("/repo", vec![entry("a.cs", 10, 1000, None)]);
        let report = correlate(&model, &t1, &t2);
        let f = &report.findings[0];
        assert!(f.input.is_none());
        assert!(f.output.is_none());
    }

    #[test]
    fn write_markdown_report_renders_findings() {
        let report = CorrelationReport {
            schema_version: CORRELATION_REPORT_SCHEMA_VERSION,
            findings: vec![CorrelationFinding {
                target_name: Some("Compile".into()),
                project_file: Some("P.csproj".into()),
                input_path: PathBuf::from("a.cs"),
                output_path: PathBuf::from("a.dll"),
                input: Some(EntryChange {
                    before: None,
                    after: None,
                    mtime_changed: true,
                    sha256_changed: false,
                    content_identical: true,
                }),
                output: None,
            }],
        };
        let mut buf = Vec::new();
        write_markdown_report(
            &report,
            &mut buf,
            &crate::sanitize::pseudonym::Pseudonymizer::noop(),
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("# Incremental build correlation report"));
        assert!(s.contains("Compile"));
        assert!(s.contains("touched-but-content-identical"));
        assert!(s.contains("not found in either snapshot"));
    }

    #[test]
    fn write_markdown_report_handles_empty_findings() {
        let report = CorrelationReport {
            schema_version: CORRELATION_REPORT_SCHEMA_VERSION,
            findings: Vec::new(),
        };
        let mut buf = Vec::new();
        write_markdown_report(
            &report,
            &mut buf,
            &crate::sanitize::pseudonym::Pseudonymizer::noop(),
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("No `is newer than`"));
    }

    #[test]
    fn correlation_report_json_round_trip() {
        let report = CorrelationReport {
            schema_version: CORRELATION_REPORT_SCHEMA_VERSION,
            findings: vec![CorrelationFinding {
                target_name: None,
                project_file: None,
                input_path: PathBuf::from("a"),
                output_path: PathBuf::from("b"),
                input: None,
                output: None,
            }],
        };
        let s = serde_json::to_string(&report).unwrap();
        let back: CorrelationReport = serde_json::from_str(&s).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn markdown_report_pseudonymizes_user_profile_path() {
        // AR-17 sanitization checkpoint. Construct a report whose
        // paths sit inside an explicit "user profile" prefix, render
        // it through a Pseudonymizer with that same prefix, and assert
        // that the rendered Markdown contains `<USER>` and not the
        // raw prefix.
        let profile = "/home/synthetic-user";
        let report = CorrelationReport {
            schema_version: CORRELATION_REPORT_SCHEMA_VERSION,
            findings: vec![CorrelationFinding {
                target_name: Some("Compile".into()),
                project_file: Some(format!("{profile}/proj/p.csproj")),
                input_path: PathBuf::from(format!("{profile}/proj/src/a.cs")),
                output_path: PathBuf::from(format!("{profile}/proj/bin/a.dll")),
                input: Some(EntryChange {
                    before: None,
                    after: None,
                    mtime_changed: true,
                    sha256_changed: false,
                    content_identical: true,
                }),
                output: None,
            }],
        };
        let p = crate::sanitize::pseudonym::Pseudonymizer::from_explicit(Some(profile.into()));
        let mut buf = Vec::new();
        write_markdown_report(&report, &mut buf, &p).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            !s.contains(profile),
            "raw user-profile prefix leaked into report:\n{s}"
        );
        assert!(
            s.contains("<USER>/proj/src/a.cs"),
            "missing pseudonymized input path:\n{s}"
        );
        assert!(
            s.contains("<USER>/proj/bin/a.dll"),
            "missing pseudonymized output path:\n{s}"
        );
        assert!(
            s.contains("<USER>/proj/p.csproj"),
            "missing pseudonymized project file:\n{s}"
        );
    }

    #[test]
    fn suffix_match_finds_path_quoted_as_relative() {
        let model = BinlogModel {
            target_runs: Vec::new(),
            newer_than_events: vec![NewerThanEvent {
                input_path: PathBuf::from("src/a.cs"),
                output_path: PathBuf::from("bin/a.dll"),
                target_name: None,
                project_file: None,
            }],
        };
        let t1 = snap(
            "/repo",
            vec![
                entry("src/a.cs", 10, 1000, Some("AA")),
                entry("bin/a.dll", 20, 500, Some("BB")),
            ],
        );
        let t2 = snap(
            "/repo",
            vec![
                entry("src/a.cs", 10, 2000, Some("AA")),
                entry("bin/a.dll", 20, 500, Some("BB")),
            ],
        );
        let report = correlate(&model, &t1, &t2);
        let input = report.findings[0].input.as_ref().expect("found");
        assert!(input.content_identical);
    }
}
