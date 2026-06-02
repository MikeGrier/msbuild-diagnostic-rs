// Copyright (c) 2026 Mike Grier

//! File-tree snapshot data model and snapshotter.
//!
//! Per D-12, the snapshotter is the only code in this crate permitted to
//! observe the live filesystem (`std::fs::metadata`, `read_dir`,
//! `symlink_metadata`, `read_link`). Everything downstream operates on the
//! typed [`TreeSnapshot`] value defined here.
//!
//! Per D-14, unit tests in this module are hermetic: they exercise the
//! data model (construction, JSON round-trip, timestamp serialization)
//! without touching the filesystem. End-to-end snapshotter behavior is
//! verified by integration tests.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Nanoseconds since the Unix epoch, serialized as a JSON string (D-13).
///
/// Negative values are legal (pre-1970 mtimes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimestampNs(pub i128);

impl TimestampNs {
    /// Build a `TimestampNs` from a `std::time::SystemTime`.
    pub fn from_system_time(t: std::time::SystemTime) -> Self {
        match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => Self(i128::from(d.as_secs()) * 1_000_000_000 + i128::from(d.subsec_nanos())),
            Err(e) => {
                let d = e.duration();
                Self(-(i128::from(d.as_secs()) * 1_000_000_000 + i128::from(d.subsec_nanos())))
            }
        }
    }
}

impl Serialize for TimestampNs {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for TimestampNs {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        s.parse::<i128>().map(TimestampNs).map_err(|e| {
            <D::Error as serde::de::Error>::custom(format!("invalid TimestampNs: {e}"))
        })
    }
}

/// What a [`TreeEntry`] represents on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryKind {
    /// Regular file. `sha256` may be present per the size threshold.
    File,
    /// Directory (recorded so empty directories are visible).
    Directory,
    /// Symbolic link. `target` is the raw link contents; not followed.
    Symlink { target: PathBuf },
}

/// Single file-tree entry. Forbidden fields beyond this struct: nothing
/// implicit — `serde(deny_unknown_fields)` makes unknown JSON keys an
/// error so capture-time additions are visible to the sanitization
/// rule registry (D-9, D-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeEntry {
    /// Path relative to the owning [`RootSnapshot::root`].
    pub relpath: PathBuf,
    /// Size in bytes. For directories and symlinks, `0`.
    pub size: u64,
    /// Last-modified time (D-13). For directories and symlinks, the
    /// metadata's own mtime — best-effort.
    pub mtime_unix_nanos: TimestampNs,
    /// SHA-256 hex digest, present only when this entry is a file whose
    /// size is `<= small_file_hash_threshold` at capture time (D-4).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sha256: Option<String>,
    pub entry_kind: EntryKind,
}

/// Snapshot of one configured root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootSnapshot {
    pub root: PathBuf,
    pub entries: Vec<TreeEntry>,
}

/// Top-level `tree.json` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeSnapshot {
    /// Schema version of this `tree.json` payload. Bumped on any
    /// breaking change to the on-disk shape.
    pub schema_version: u32,
    /// SHA-256 size threshold actually used at capture time, recorded so
    /// the diff engine can reason about why a given file lacks a hash.
    pub small_file_hash_threshold: u64,
    pub roots: Vec<RootSnapshot>,
}

/// Current `tree.json` schema version.
pub const TREE_SCHEMA_VERSION: u32 = 1;

/// Walk every configured root and produce a [`TreeSnapshot`].
///
/// Per D-12, this is the **only** function in this crate that observes
/// the live filesystem for snapshot purposes. Symlinks are recorded by
/// their raw target and never followed. Files with `size > threshold`
/// are recorded without a SHA-256.
pub fn snapshot_roots(roots: &[PathBuf], threshold: u64) -> io::Result<TreeSnapshot> {
    let mut out = TreeSnapshot {
        schema_version: TREE_SCHEMA_VERSION,
        small_file_hash_threshold: threshold,
        roots: Vec::with_capacity(roots.len()),
    };
    for root in roots {
        let entries = walk_root(root, threshold)?;
        out.roots.push(RootSnapshot {
            root: root.clone(),
            entries,
        });
    }
    Ok(out)
}

fn walk_root(root: &Path, threshold: u64) -> io::Result<Vec<TreeEntry>> {
    let mut entries = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let read = match fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for ent in read {
            let ent = ent?;
            let path = ent.path();
            let meta = fs::symlink_metadata(&path)?;
            let rel = path
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.clone());
            let mtime = meta
                .modified()
                .map(TimestampNs::from_system_time)
                .unwrap_or(TimestampNs(0));

            if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).unwrap_or_default();
                entries.push(TreeEntry {
                    relpath: rel,
                    size: 0,
                    mtime_unix_nanos: mtime,
                    sha256: None,
                    entry_kind: EntryKind::Symlink { target },
                });
            } else if meta.is_dir() {
                entries.push(TreeEntry {
                    relpath: rel,
                    size: 0,
                    mtime_unix_nanos: mtime,
                    sha256: None,
                    entry_kind: EntryKind::Directory,
                });
                stack.push(path);
            } else {
                let size = meta.len();
                let sha256 = if size <= threshold {
                    Some(hash_file(&path)?)
                } else {
                    None
                };
                entries.push(TreeEntry {
                    relpath: rel,
                    size,
                    mtime_unix_nanos: mtime,
                    sha256,
                    entry_kind: EntryKind::File,
                });
            }
        }
    }
    entries.sort_by(|a, b| a.relpath.cmp(&b.relpath));
    Ok(entries)
}

fn hash_file(path: &Path) -> io::Result<String> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    //! Hermetic unit tests for the snapshot data model (D-14). No
    //! filesystem access; that lives in integration tests.
    use super::*;

    #[test]
    fn timestamp_serializes_as_decimal_string() {
        let ts = TimestampNs(1_748_880_235_123_456_700);
        let s = serde_json::to_string(&ts).unwrap();
        assert_eq!(s, "\"1748880235123456700\"");
    }

    #[test]
    fn timestamp_round_trips_through_json() {
        let cases = [
            TimestampNs(0),
            TimestampNs(1),
            TimestampNs(-1),
            TimestampNs(i128::MAX),
            TimestampNs(i128::MIN),
        ];
        for ts in cases {
            let s = serde_json::to_string(&ts).unwrap();
            let back: TimestampNs = serde_json::from_str(&s).unwrap();
            assert_eq!(back, ts);
        }
    }

    #[test]
    fn timestamp_rejects_non_integer_strings() {
        let err = serde_json::from_str::<TimestampNs>("\"not-a-number\"").unwrap_err();
        assert!(err.to_string().contains("invalid TimestampNs"));
    }

    #[test]
    fn timestamp_rejects_json_number_to_force_string_form() {
        // D-13: timestamps MUST be JSON strings; a JSON number is a
        // capture-tool bug we want to catch at deserialization.
        let err = serde_json::from_str::<TimestampNs>("1234567890").unwrap_err();
        assert!(
            err.to_string().contains("expected a string") || err.to_string().contains("invalid")
        );
    }

    fn sample_snapshot() -> TreeSnapshot {
        TreeSnapshot {
            schema_version: TREE_SCHEMA_VERSION,
            small_file_hash_threshold: 1_048_576,
            roots: vec![RootSnapshot {
                root: PathBuf::from("src"),
                entries: vec![
                    TreeEntry {
                        relpath: PathBuf::from("dir"),
                        size: 0,
                        mtime_unix_nanos: TimestampNs(100),
                        sha256: None,
                        entry_kind: EntryKind::Directory,
                    },
                    TreeEntry {
                        relpath: PathBuf::from("dir/file.txt"),
                        size: 11,
                        mtime_unix_nanos: TimestampNs(200),
                        sha256: Some("abc123".into()),
                        entry_kind: EntryKind::File,
                    },
                    TreeEntry {
                        relpath: PathBuf::from("link"),
                        size: 0,
                        mtime_unix_nanos: TimestampNs(300),
                        sha256: None,
                        entry_kind: EntryKind::Symlink {
                            target: PathBuf::from("../elsewhere"),
                        },
                    },
                ],
            }],
        }
    }

    #[test]
    fn tree_snapshot_round_trips_through_json() {
        let snap = sample_snapshot();
        let s = serde_json::to_string(&snap).unwrap();
        let back: TreeSnapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn tree_snapshot_rejects_unknown_fields() {
        // D-10 / D-9: any new field in a future tree.json must be
        // classified before this crate will load it.
        let json = r#"{
            "schema_version": 1,
            "small_file_hash_threshold": 1024,
            "roots": [],
            "future_field": "surprise"
        }"#;
        let err = serde_json::from_str::<TreeSnapshot>(json).unwrap_err();
        assert!(err.to_string().contains("future_field"));
    }

    #[test]
    fn tree_entry_omits_sha256_when_none() {
        let entry = TreeEntry {
            relpath: PathBuf::from("big.bin"),
            size: 10_000_000,
            mtime_unix_nanos: TimestampNs(1),
            sha256: None,
            entry_kind: EntryKind::File,
        };
        let s = serde_json::to_string(&entry).unwrap();
        assert!(!s.contains("sha256"));
    }
}
