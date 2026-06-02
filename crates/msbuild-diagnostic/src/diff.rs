// Copyright (c) 2026 Mike Grier

//! Diff algorithm for two [`TreeSnapshot`] values (AR-13).
//!
//! Pure function per D-12. The CLI shell (in `cli.rs`) handles the I/O
//! of reading the two archives, deserializing `tree.json` and
//! `manifest.json`, calling [`diff_snapshots`], and writing the
//! resulting [`DiffReport`] back out.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::snapshot::{EntryKind, TimestampNs, TreeEntry, TreeSnapshot};

/// Current `diff-report.json` schema version.
pub const DIFF_REPORT_SCHEMA_VERSION: u32 = 1;

/// Top-level diff report. Serializable as `diff-report.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffReport {
    pub schema_version: u32,
    /// Per-root diff. Roots present in only one side are still recorded
    /// (with all entries reported as added or removed accordingly).
    pub roots: Vec<RootDiff>,
}

/// Diff for one root path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootDiff {
    pub root: PathBuf,
    /// Files present in T2 but not T1.
    pub added: Vec<DiffEntry>,
    /// Files present in T1 but not T2.
    pub removed: Vec<DiffEntry>,
    /// Files present on both sides with at least one observable change.
    pub changed: Vec<ChangedEntry>,
    /// Files present on both sides with no observable change (same
    /// size, mtime, sha256, and kind). Listed so the report is
    /// self-checking — total = added + removed + changed + unchanged
    /// equals union(T1, T2) per root.
    pub unchanged: Vec<DiffEntry>,
}

/// A single-sided entry (added, removed, or unchanged).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffEntry {
    pub relpath: PathBuf,
    pub size: u64,
    pub mtime_unix_nanos: TimestampNs,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sha256: Option<String>,
    pub entry_kind: EntryKind,
}

/// An entry present on both sides; carries before/after plus a typed
/// set of which fields differ. The flags exist so consumers
/// (correlation engine, Markdown renderer) can branch without
/// re-comparing the structs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangedEntry {
    pub relpath: PathBuf,
    pub before: DiffEntry,
    pub after: DiffEntry,
    pub changes: ChangeFlags,
}

/// Which fields of a pair of entries differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeFlags {
    pub size: bool,
    pub mtime: bool,
    pub sha256: bool,
    pub kind: bool,
}

impl ChangeFlags {
    /// True iff at least one field differs.
    pub fn any(&self) -> bool {
        self.size || self.mtime || self.sha256 || self.kind
    }

    /// True iff only mtime differs and content (size + sha256 + kind)
    /// is identical. This is the "touched but content-identical" case
    /// AR-16 cares about.
    pub fn is_mtime_only(&self) -> bool {
        self.mtime && !self.size && !self.sha256 && !self.kind
    }
}

/// Pure diff over two [`TreeSnapshot`] values.
///
/// Per-root pairing is by literal root path. Within a root, entries are
/// paired by `relpath`. The output is fully sorted by `(root, relpath)`
/// for deterministic round-tripping.
pub fn diff_snapshots(t1: &TreeSnapshot, t2: &TreeSnapshot) -> DiffReport {
    // Pair roots by literal path. Both sides retain insertion order via
    // sorted iteration so callers get a stable layout.
    type RootPair<'a> = (Option<&'a [TreeEntry]>, Option<&'a [TreeEntry]>);
    let mut by_root: BTreeMap<&PathBuf, RootPair<'_>> = BTreeMap::new();
    for r in &t1.roots {
        by_root.entry(&r.root).or_default().0 = Some(&r.entries);
    }
    for r in &t2.roots {
        by_root.entry(&r.root).or_default().1 = Some(&r.entries);
    }

    let mut roots = Vec::with_capacity(by_root.len());
    for (root, (t1_entries, t2_entries)) in by_root {
        roots.push(diff_root(
            root.clone(),
            t1_entries.unwrap_or(&[]),
            t2_entries.unwrap_or(&[]),
        ));
    }

    DiffReport {
        schema_version: DIFF_REPORT_SCHEMA_VERSION,
        roots,
    }
}

fn diff_root(root: PathBuf, t1: &[TreeEntry], t2: &[TreeEntry]) -> RootDiff {
    // Index by relpath. BTreeMap gives sorted iteration.
    type EntryPair<'a> = (Option<&'a TreeEntry>, Option<&'a TreeEntry>);
    let mut by_path: BTreeMap<&PathBuf, EntryPair<'_>> = BTreeMap::new();
    for e in t1 {
        by_path.entry(&e.relpath).or_default().0 = Some(e);
    }
    for e in t2 {
        by_path.entry(&e.relpath).or_default().1 = Some(e);
    }

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();

    for (_, (a, b)) in by_path {
        match (a, b) {
            (None, Some(b)) => added.push(to_diff_entry(b)),
            (Some(a), None) => removed.push(to_diff_entry(a)),
            (Some(a), Some(b)) => {
                let flags = compare(a, b);
                if flags.any() {
                    changed.push(ChangedEntry {
                        relpath: a.relpath.clone(),
                        before: to_diff_entry(a),
                        after: to_diff_entry(b),
                        changes: flags,
                    });
                } else {
                    unchanged.push(to_diff_entry(a));
                }
            }
            (None, None) => unreachable!("BTreeMap entry must have at least one side"),
        }
    }

    RootDiff {
        root,
        added,
        removed,
        changed,
        unchanged,
    }
}

fn to_diff_entry(e: &TreeEntry) -> DiffEntry {
    DiffEntry {
        relpath: e.relpath.clone(),
        size: e.size,
        mtime_unix_nanos: e.mtime_unix_nanos,
        sha256: e.sha256.clone(),
        entry_kind: e.entry_kind.clone(),
    }
}

fn compare(a: &TreeEntry, b: &TreeEntry) -> ChangeFlags {
    ChangeFlags {
        size: a.size != b.size,
        mtime: a.mtime_unix_nanos != b.mtime_unix_nanos,
        sha256: a.sha256 != b.sha256,
        kind: a.entry_kind != b.entry_kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{RootSnapshot, TREE_SCHEMA_VERSION};

    fn snap(roots: Vec<RootSnapshot>) -> TreeSnapshot {
        TreeSnapshot {
            schema_version: TREE_SCHEMA_VERSION,
            small_file_hash_threshold: 1024,
            roots,
        }
    }

    fn file(relpath: &str, size: u64, mtime: i128, sha: Option<&str>) -> TreeEntry {
        TreeEntry {
            relpath: PathBuf::from(relpath),
            size,
            mtime_unix_nanos: TimestampNs(mtime),
            sha256: sha.map(str::to_owned),
            entry_kind: EntryKind::File,
        }
    }

    fn root(path: &str, entries: Vec<TreeEntry>) -> RootSnapshot {
        RootSnapshot {
            root: PathBuf::from(path),
            entries,
        }
    }

    #[test]
    fn identical_snapshots_have_only_unchanged_entries() {
        let s = snap(vec![root(
            "src",
            vec![
                file("a.txt", 10, 100, Some("a")),
                file("b.txt", 20, 200, Some("b")),
            ],
        )]);
        let report = diff_snapshots(&s, &s);
        assert_eq!(report.roots.len(), 1);
        let r = &report.roots[0];
        assert!(r.added.is_empty());
        assert!(r.removed.is_empty());
        assert!(r.changed.is_empty());
        assert_eq!(r.unchanged.len(), 2);
    }

    #[test]
    fn added_files_appear_only_in_t2() {
        let t1 = snap(vec![root("src", vec![file("a.txt", 10, 100, Some("a"))])]);
        let t2 = snap(vec![root(
            "src",
            vec![
                file("a.txt", 10, 100, Some("a")),
                file("b.txt", 20, 200, Some("b")),
            ],
        )]);
        let r = &diff_snapshots(&t1, &t2).roots[0];
        assert_eq!(r.added.len(), 1);
        assert_eq!(r.added[0].relpath, PathBuf::from("b.txt"));
        assert!(r.removed.is_empty());
    }

    #[test]
    fn removed_files_appear_only_in_t1() {
        let t1 = snap(vec![root(
            "src",
            vec![
                file("a.txt", 10, 100, Some("a")),
                file("b.txt", 20, 200, Some("b")),
            ],
        )]);
        let t2 = snap(vec![root("src", vec![file("a.txt", 10, 100, Some("a"))])]);
        let r = &diff_snapshots(&t1, &t2).roots[0];
        assert_eq!(r.removed.len(), 1);
        assert_eq!(r.removed[0].relpath, PathBuf::from("b.txt"));
        assert!(r.added.is_empty());
    }

    #[test]
    fn mtime_only_change_sets_is_mtime_only_flag() {
        let t1 = snap(vec![root("src", vec![file("a.txt", 10, 100, Some("a"))])]);
        let t2 = snap(vec![root("src", vec![file("a.txt", 10, 200, Some("a"))])]);
        let r = &diff_snapshots(&t1, &t2).roots[0];
        assert_eq!(r.changed.len(), 1);
        let c = &r.changed[0];
        assert!(c.changes.mtime);
        assert!(!c.changes.size);
        assert!(!c.changes.sha256);
        assert!(!c.changes.kind);
        assert!(c.changes.is_mtime_only());
    }

    #[test]
    fn size_and_sha_changes_are_reported() {
        let t1 = snap(vec![root("src", vec![file("a.txt", 10, 100, Some("a"))])]);
        let t2 = snap(vec![root("src", vec![file("a.txt", 11, 200, Some("b"))])]);
        let c = &diff_snapshots(&t1, &t2).roots[0].changed[0];
        assert!(c.changes.size);
        assert!(c.changes.mtime);
        assert!(c.changes.sha256);
        assert!(!c.changes.is_mtime_only());
    }

    #[test]
    fn kind_change_is_reported() {
        let t1 = snap(vec![root("src", vec![file("a.txt", 10, 100, Some("a"))])]);
        let mut sym = file("a.txt", 0, 100, None);
        sym.entry_kind = EntryKind::Symlink {
            target: PathBuf::from("/other"),
        };
        let t2 = snap(vec![root("src", vec![sym])]);
        let c = &diff_snapshots(&t1, &t2).roots[0].changed[0];
        assert!(c.changes.kind);
    }

    #[test]
    fn roots_present_on_only_one_side_are_preserved() {
        let t1 = snap(vec![root("src", vec![file("a.txt", 10, 100, Some("a"))])]);
        let t2 = snap(vec![root(
            "docs",
            vec![file("README.md", 5, 50, Some("z"))],
        )]);
        let report = diff_snapshots(&t1, &t2);
        assert_eq!(report.roots.len(), 2);
        // BTreeMap iteration is lex-sorted: "docs" < "src".
        assert_eq!(report.roots[0].root, PathBuf::from("docs"));
        assert_eq!(report.roots[0].added.len(), 1);
        assert_eq!(report.roots[0].removed.len(), 0);
        assert_eq!(report.roots[1].root, PathBuf::from("src"));
        assert_eq!(report.roots[1].added.len(), 0);
        assert_eq!(report.roots[1].removed.len(), 1);
    }

    #[test]
    fn report_is_deterministic_under_input_reorder() {
        let entries_a = vec![
            file("b.txt", 20, 200, Some("b")),
            file("a.txt", 10, 100, Some("a")),
        ];
        let entries_b = vec![
            file("a.txt", 10, 100, Some("a")),
            file("b.txt", 20, 200, Some("b")),
        ];
        let t1 = snap(vec![root("src", entries_a)]);
        let t2 = snap(vec![root("src", entries_b)]);
        let r = &diff_snapshots(&t1, &t2).roots[0];
        // Both reordered inputs yield zero diffs and the same sorted
        // unchanged list.
        let names: Vec<_> = r.unchanged.iter().map(|e| e.relpath.clone()).collect();
        assert_eq!(names, vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]);
    }

    #[test]
    fn diff_report_round_trips_through_json() {
        let t1 = snap(vec![root(
            "src",
            vec![file("a.txt", 10, 100, Some("a")), file("c.txt", 1, 1, None)],
        )]);
        let t2 = snap(vec![root(
            "src",
            vec![
                file("a.txt", 11, 200, Some("b")),
                file("d.txt", 5, 50, None),
            ],
        )]);
        let report = diff_snapshots(&t1, &t2);
        let s = serde_json::to_string(&report).expect("ser");
        let back: DiffReport = serde_json::from_str(&s).expect("de");
        assert_eq!(back, report);
    }

    #[test]
    fn missing_root_pairings_produce_added_or_removed_entries() {
        let t1 = snap(vec![]);
        let t2 = snap(vec![root("only-t2", vec![file("x", 1, 1, Some("a"))])]);
        let r = &diff_snapshots(&t1, &t2).roots[0];
        assert_eq!(r.added.len(), 1);
        assert!(r.removed.is_empty());
    }
}
