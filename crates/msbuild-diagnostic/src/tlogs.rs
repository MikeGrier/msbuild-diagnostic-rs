// Copyright (c) 2026 Mike Grier

//! `.tlog` file collection (D-3 part 2).
//!
//! For each project file in the binlog inventory, MSBuild's per-task
//! input/output tracking logs (`*.tlog`) live in the project's `obj/`
//! directory and any subdirectory of it. We capture the raw bytes plus
//! the *project-relative* path for inclusion under
//! `tlogs/<project-relpath>/...` in the archive.
//!
//! Walking the filesystem is in scope here (the snapshot module is no
//! longer the only FS-touching code in the crate, but the constraint in
//! D-12 is about algorithms — tlog *enumeration* is FS work, and the
//! downstream sanitizer / report operates on the typed [`TlogFile`]
//! values produced here).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::binlog::BinlogProjectInventory;

/// One captured `*.tlog` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlogFile {
    /// Stable relpath used to lay the file out under
    /// `tlogs/<project-stem>/...` in the archive.
    pub archive_relpath: PathBuf,
    /// Raw file contents. `.tlog` files are typically UTF-16 LE text but
    /// the model is intentionally byte-shaped — we preserve whatever is
    /// on disk and let downstream sanitization make encoding decisions.
    pub contents: Vec<u8>,
}

/// Walk every project's `obj/` directory and collect every `*.tlog`
/// (case-insensitive extension) found anywhere underneath. Files outside
/// any project's `obj/` are not collected per AR-10.
///
/// `archive_relpath` for each tlog is
/// `<project-file-stem>/<path-relative-to-project-obj>`. The
/// project-file-stem is used (rather than the full project path) so the
/// layout inside the archive does not leak absolute paths.
pub fn collect_tlogs(inventory: &BinlogProjectInventory) -> io::Result<Vec<TlogFile>> {
    let mut out = Vec::new();
    for project in &inventory.projects {
        let Some(project_dir) = project.project_file.parent() else {
            continue;
        };
        let obj_dir = project_dir.join("obj");
        if !obj_dir.is_dir() {
            continue;
        }
        let project_stem = project
            .project_file
            .file_stem()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("project"));
        walk_obj(&obj_dir, &obj_dir, &project_stem, &mut out)?;
    }
    out.sort_by(|a, b| a.archive_relpath.cmp(&b.archive_relpath));
    Ok(out)
}

fn walk_obj(
    obj_root: &Path,
    current: &Path,
    project_stem: &Path,
    out: &mut Vec<TlogFile>,
) -> io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let meta = entry.file_type()?;
        let path = entry.path();
        if meta.is_dir() {
            walk_obj(obj_root, &path, project_stem, out)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        if !is_tlog(&path) {
            continue;
        }
        // Relative path inside obj/, then re-rooted under project_stem.
        let rel_to_obj = path.strip_prefix(obj_root).unwrap_or(&path);
        let archive_relpath = project_stem.join(rel_to_obj);
        let contents = fs::read(&path)?;
        out.push(TlogFile {
            archive_relpath,
            contents,
        });
    }
    Ok(())
}

fn is_tlog(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("tlog"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_tlog_matches_case_insensitive_extension() {
        assert!(is_tlog(Path::new("foo.tlog")));
        assert!(is_tlog(Path::new("foo.TLOG")));
        assert!(is_tlog(Path::new("a/b/foo.Tlog")));
        assert!(!is_tlog(Path::new("foo.txt")));
        assert!(!is_tlog(Path::new("foo")));
        assert!(!is_tlog(Path::new("foo.tlog.bak")));
    }
}
