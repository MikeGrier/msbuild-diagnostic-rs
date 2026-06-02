// Copyright (c) 2026 Mike Grier

//! Binlog parsing — typed inventory of what the binlog tells us.
//!
//! This is the only place in the crate that calls into `munin_msbuild` to
//! decode the binlog. Per D-12 the actual algorithms (root discovery,
//! tlog discovery, sanitization) operate on the [`BinlogProjectInventory`]
//! and [`BinlogImport`] typed values produced here — they never re-parse
//! the binlog themselves.
//!
//! Per D-7 the specification is ours; `munin_msbuild` is the
//! implementation choice. The `ProjectStarted.project_file` field and the
//! `extract_archives()` API are the precise affordances our spec requires.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use munin_msbuild::events::ProjectStartedEvent;
use munin_msbuild::{ArchiveEntry, BinlogEvent, BinlogIndex};

/// One project file MSBuild built, as recorded in a `ProjectStarted` event.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InventoryProject {
    pub project_file: PathBuf,
}

/// What the binlog tells us about the build at a structural level. The
/// only required field today is the set of project files; later
/// milestones extend this with target / task / property views.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BinlogProjectInventory {
    /// Project files referenced by `ProjectStarted` events, deduplicated
    /// and sorted lexicographically for determinism.
    pub projects: Vec<InventoryProject>,
}

/// One entry from the binlog's embedded `ProjectImportArchive` records.
///
/// `path` is the in-archive relative path as MSBuild emitted it.
/// `contents` is the text payload. munin currently only surfaces UTF-8
/// payloads (binary entries are silently dropped at the munin layer);
/// for project imports these are XML files so the constraint is fine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinlogImport {
    pub path: String,
    pub contents: String,
}

impl From<ArchiveEntry> for BinlogImport {
    fn from(e: ArchiveEntry) -> Self {
        Self {
            path: e.path,
            contents: e.contents,
        }
    }
}

/// Pure: reduce a slice of `ProjectStarted` events to a deduplicated,
/// lexicographically sorted [`BinlogProjectInventory`]. Events with no
/// `project_file` are skipped.
pub fn inventory_from_project_starts(events: &[ProjectStartedEvent]) -> BinlogProjectInventory {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    for ev in events {
        if let Some(p) = &ev.project_file {
            seen.insert(PathBuf::from(p));
        }
    }
    BinlogProjectInventory {
        projects: seen
            .into_iter()
            .map(|project_file| InventoryProject { project_file })
            .collect(),
    }
}

/// Pure: reduce munin's `Vec<ArchiveEntry>` to our [`BinlogImport`] model,
/// deduplicated by path (later entries with the same path win — matches
/// MSBuild's "last writer" semantics for repeated imports).
pub fn imports_from_archive_entries(entries: Vec<ArchiveEntry>) -> Vec<BinlogImport> {
    let mut by_path: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for e in entries {
        by_path.insert(e.path, e.contents);
    }
    by_path
        .into_iter()
        .map(|(path, contents)| BinlogImport { path, contents })
        .collect()
}

/// Read `path` and produce the [`BinlogProjectInventory`] plus the
/// extracted imports. This is the thin FS + parse wrapper; algorithms
/// downstream operate on the returned typed values.
pub fn read_binlog(path: &Path) -> io::Result<(BinlogProjectInventory, Vec<BinlogImport>)> {
    let f = File::open(path)?;
    let index = BinlogIndex::open(BufReader::new(f)).map_err(munin_to_io)?;
    let events = index.get_all().map_err(munin_to_io)?;
    let starts: Vec<ProjectStartedEvent> = events
        .into_iter()
        .filter_map(|e| match e {
            BinlogEvent::ProjectStarted(p) => Some(p),
            _ => None,
        })
        .collect();
    let inventory = inventory_from_project_starts(&starts);
    let archives = index.extract_archives().map_err(munin_to_io)?;
    let imports = imports_from_archive_entries(archives);
    Ok((inventory, imports))
}

fn munin_to_io(e: munin_msbuild::MuninError) -> io::Error {
    io::Error::other(format!("binlog parse: {e}"))
}

/// Read `path` and return the raw event stream. Thin FS + parse wrapper
/// used by AR-15 correlation (the pure model builder consumes a slice
/// of [`BinlogEvent`]; this function is the only place that opens the
/// file).
pub fn read_binlog_events(path: &Path) -> io::Result<Vec<BinlogEvent>> {
    let f = File::open(path)?;
    let index = BinlogIndex::open(BufReader::new(f)).map_err(munin_to_io)?;
    index.get_all().map_err(munin_to_io)
}

#[cfg(test)]
mod tests {
    //! Hermetic unit tests (D-14). All inputs are constructed in memory.
    use super::*;
    use munin_msbuild::ArchiveEntry;

    fn proj(path: &str) -> ProjectStartedEvent {
        ProjectStartedEvent {
            project_file: Some(path.into()),
            ..ProjectStartedEvent::default()
        }
    }

    #[test]
    fn inventory_dedupes_repeated_projects() {
        let inv = inventory_from_project_starts(&[
            proj("a/A.csproj"),
            proj("b/B.csproj"),
            proj("a/A.csproj"),
        ]);
        assert_eq!(inv.projects.len(), 2);
        assert_eq!(inv.projects[0].project_file, PathBuf::from("a/A.csproj"));
        assert_eq!(inv.projects[1].project_file, PathBuf::from("b/B.csproj"));
    }

    #[test]
    fn inventory_skips_events_without_project_file() {
        let inv =
            inventory_from_project_starts(&[ProjectStartedEvent::default(), proj("only.csproj")]);
        assert_eq!(inv.projects.len(), 1);
        assert_eq!(inv.projects[0].project_file, PathBuf::from("only.csproj"));
    }

    #[test]
    fn inventory_sorts_lexicographically() {
        let inv =
            inventory_from_project_starts(&[proj("z.csproj"), proj("a.csproj"), proj("m.csproj")]);
        let names: Vec<_> = inv
            .projects
            .iter()
            .map(|p| p.project_file.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.csproj", "m.csproj", "z.csproj"]);
    }

    #[test]
    fn imports_dedupe_by_path_last_writer_wins() {
        let entries = vec![
            ArchiveEntry {
                path: "Directory.Build.props".into(),
                contents: "<Project>first</Project>".into(),
            },
            ArchiveEntry {
                path: "Sdk.props".into(),
                contents: "<Project>sdk</Project>".into(),
            },
            ArchiveEntry {
                path: "Directory.Build.props".into(),
                contents: "<Project>second</Project>".into(),
            },
        ];
        let imports = imports_from_archive_entries(entries);
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].path, "Directory.Build.props");
        assert_eq!(imports[0].contents, "<Project>second</Project>");
        assert_eq!(imports[1].path, "Sdk.props");
    }

    #[test]
    fn imports_empty_input_yields_empty_output() {
        assert!(imports_from_archive_entries(Vec::new()).is_empty());
    }
}
