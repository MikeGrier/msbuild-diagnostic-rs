// Copyright (c) 2026 Mike Grier

//! Default-root discovery (D-5).
//!
//! When `--root` is not supplied, default roots are derived from:
//!
//! 1. The directory containing the `.binlog`.
//! 2. The parent directory of every project file in the binlog inventory.
//! 3. The git repository root (walk parents looking for `.git`), if any.
//!
//! [`discover_default_roots`] is a pure function over typed inputs (D-12);
//! the FS-touching pieces (canonicalization, `.git` walk) live in
//! [`canonicalize_existing`] / [`find_git_root`] and are tested at the
//! integration tier (D-14).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::binlog::BinlogProjectInventory;

/// Inputs to [`discover_default_roots`]. All paths must already be
/// lexically normalized by the caller — see [`canonicalize_existing`] for
/// the recommended pre-pass.
pub struct RootDiscoveryInputs<'a> {
    /// Path to the `.binlog` file itself; its parent contributes a root.
    pub binlog_path: &'a Path,
    /// Inventory derived from the binlog (AR-7).
    pub inventory: &'a BinlogProjectInventory,
    /// Git repository root, or `None` if the binlog is not in a git checkout.
    pub git_root: Option<&'a Path>,
}

/// Compute the default root set per D-5.
///
/// Returns roots in deterministic order: deduplicated by literal path and
/// sorted lexicographically. Overlapping paths (one a prefix of another)
/// are coalesced — only the shortest ancestor in each chain survives, so
/// the resulting roots tile the union without overlap. This keeps
/// `tree.json` from recording the same file under multiple roots.
pub fn discover_default_roots(inputs: RootDiscoveryInputs<'_>) -> Vec<PathBuf> {
    let mut raw: BTreeSet<PathBuf> = BTreeSet::new();

    if let Some(parent) = inputs.binlog_path.parent() {
        if !parent.as_os_str().is_empty() {
            raw.insert(parent.to_path_buf());
        }
    }

    for p in &inputs.inventory.projects {
        if let Some(parent) = p.project_file.parent() {
            if !parent.as_os_str().is_empty() {
                raw.insert(parent.to_path_buf());
            }
        }
    }

    if let Some(g) = inputs.git_root {
        raw.insert(g.to_path_buf());
    }

    coalesce_overlapping(raw.into_iter().collect())
}

/// Drop any path that has another path in the set as a proper ancestor.
/// Sorted lexicographically, which puts every ancestor before its
/// descendants — so a single linear scan with a "current keeper" works.
fn coalesce_overlapping(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort();
    let mut out: Vec<PathBuf> = Vec::with_capacity(paths.len());
    for p in paths {
        if let Some(last) = out.last() {
            if p.starts_with(last) {
                continue;
            }
        }
        out.push(p);
    }
    out
}

/// Lexically resolve `path` to an absolute path and canonicalize via the
/// filesystem (resolves symlinks, normalizes case on Windows). Falls back
/// to the absolute path if canonicalization fails (e.g. the path does not
/// exist on disk yet).
pub fn canonicalize_existing(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    std::fs::canonicalize(&abs).unwrap_or(abs)
}

/// Walk parents of `start` looking for a directory containing `.git`.
/// Returns the directory containing `.git`, or `None` if no such ancestor
/// exists.
pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = Some(start);
    while let Some(p) = cur {
        if p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binlog::{BinlogProjectInventory, InventoryProject};

    fn inv(paths: &[&str]) -> BinlogProjectInventory {
        BinlogProjectInventory {
            projects: paths
                .iter()
                .map(|p| InventoryProject {
                    project_file: PathBuf::from(p),
                })
                .collect(),
        }
    }

    #[test]
    fn includes_binlog_parent_and_project_parents_and_git_root() {
        let binlog = PathBuf::from("/repo/out/build.binlog");
        let inventory = inv(&["/repo/src/A/A.csproj", "/repo/src/B/B.csproj"]);
        let git = PathBuf::from("/repo");
        let roots = discover_default_roots(RootDiscoveryInputs {
            binlog_path: &binlog,
            inventory: &inventory,
            git_root: Some(&git),
        });
        // Git root coalesces everything else away.
        assert_eq!(roots, vec![PathBuf::from("/repo")]);
    }

    #[test]
    fn coalesces_overlapping_project_dirs_without_git_root() {
        let binlog = PathBuf::from("/work/build.binlog");
        let inventory = inv(&[
            "/work/src/A/A.csproj",
            "/work/src/A/sub/Nested.csproj",
            "/work/src/B/B.csproj",
        ]);
        let roots = discover_default_roots(RootDiscoveryInputs {
            binlog_path: &binlog,
            inventory: &inventory,
            git_root: None,
        });
        // /work is binlog parent and ancestor of every project dir.
        assert_eq!(roots, vec![PathBuf::from("/work")]);
    }

    #[test]
    fn keeps_non_overlapping_roots() {
        let binlog = PathBuf::from("/a/build.binlog");
        let inventory = inv(&["/b/proj/P.csproj"]);
        let roots = discover_default_roots(RootDiscoveryInputs {
            binlog_path: &binlog,
            inventory: &inventory,
            git_root: None,
        });
        assert_eq!(roots, vec![PathBuf::from("/a"), PathBuf::from("/b/proj")]);
    }

    #[test]
    fn dedupes_repeated_inputs() {
        let binlog = PathBuf::from("/r/build.binlog");
        let inventory = inv(&["/r/X.csproj", "/r/X.csproj"]);
        let roots = discover_default_roots(RootDiscoveryInputs {
            binlog_path: &binlog,
            inventory: &inventory,
            git_root: Some(Path::new("/r")),
        });
        assert_eq!(roots, vec![PathBuf::from("/r")]);
    }

    #[test]
    fn ignores_empty_parents() {
        // Bare filename -> parent is "" which we drop.
        let binlog = PathBuf::from("standalone.binlog");
        let inventory = BinlogProjectInventory::default();
        let roots = discover_default_roots(RootDiscoveryInputs {
            binlog_path: &binlog,
            inventory: &inventory,
            git_root: None,
        });
        assert!(roots.is_empty());
    }

    #[test]
    fn sorts_output_lexicographically() {
        let binlog = PathBuf::from("/z/build.binlog");
        let inventory = inv(&["/a/A.csproj", "/m/M.csproj"]);
        let roots = discover_default_roots(RootDiscoveryInputs {
            binlog_path: &binlog,
            inventory: &inventory,
            git_root: None,
        });
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/m"),
                PathBuf::from("/z"),
            ]
        );
    }
}
