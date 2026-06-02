// Copyright (c) 2026 Mike Grier

//! Archive writer.
//!
//! Produces the on-disk `.zip` payload described in D-3. The writer is
//! intentionally parameterized on readers (not paths) for the binlog so
//! unit tests can exercise it hermetically per D-14.

use std::io::{Read, Seek, Write};

use zip::write::{SimpleFileOptions, ZipWriter};
use zip::CompressionMethod;

use crate::manifest::{Manifest, MANIFEST_NAME};
use crate::snapshot::TreeSnapshot;

/// Canonical name for the file-tree snapshot inside the archive.
pub const TREE_JSON_NAME: &str = "tree.json";

/// Canonical name for the imports subdirectory inside the archive (D-3).
pub const IMPORTS_DIR_NAME: &str = "imports/";

/// Inputs to a single archive write. Additional fields (tlogs, extracted
/// imports) will land here in subsequent checklist items.
pub struct ArchiveInputs<'a> {
    /// Name to store the binlog under inside the archive (typically the
    /// binlog's original filename).
    pub binlog_name: &'a str,
    /// File-tree snapshot to serialize as `tree.json`.
    pub tree: &'a TreeSnapshot,
    /// Capture manifest to serialize as `manifest.json`.
    pub manifest: &'a Manifest,
}

/// Write the archive to `out`. `out` must be seekable (the central
/// directory is written after each entry's local header). `binlog` is
/// streamed verbatim into the archive under `inputs.binlog_name`.
pub fn write_archive<W: Write + Seek, R: Read>(
    inputs: &ArchiveInputs<'_>,
    mut binlog: R,
    out: W,
) -> std::io::Result<()> {
    let mut zw = ZipWriter::new(out);
    let file_opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let dir_opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o755);

    zw.add_directory(IMPORTS_DIR_NAME, dir_opts)
        .map_err(zip_to_io)?;

    zw.start_file(inputs.binlog_name, file_opts)
        .map_err(zip_to_io)?;
    std::io::copy(&mut binlog, &mut zw)?;

    zw.start_file(TREE_JSON_NAME, file_opts)
        .map_err(zip_to_io)?;
    serde_json::to_writer_pretty(&mut zw, inputs.tree)?;

    zw.start_file(MANIFEST_NAME, file_opts).map_err(zip_to_io)?;
    serde_json::to_writer_pretty(&mut zw, inputs.manifest)?;

    zw.finish().map_err(zip_to_io)?;
    Ok(())
}

fn zip_to_io(e: zip::result::ZipError) -> std::io::Error {
    std::io::Error::other(e)
}

#[cfg(test)]
mod tests {
    //! Hermetic unit tests for the zip writer (D-14). The output is a
    //! `Cursor<Vec<u8>>` and the verification reader is also in-memory; no
    //! filesystem access.
    use super::*;
    use crate::manifest::{build_manifest, CaptureEnvironment, ManifestInputs};
    use crate::snapshot::{EntryKind, RootSnapshot, TimestampNs, TreeEntry, TREE_SCHEMA_VERSION};
    use std::io::Cursor;
    use std::path::PathBuf;

    fn sample_tree() -> TreeSnapshot {
        TreeSnapshot {
            schema_version: TREE_SCHEMA_VERSION,
            small_file_hash_threshold: 1024,
            roots: vec![RootSnapshot {
                root: PathBuf::from("src"),
                entries: vec![TreeEntry {
                    relpath: PathBuf::from("a.txt"),
                    size: 3,
                    mtime_unix_nanos: TimestampNs(42),
                    sha256: Some("deadbeef".into()),
                    entry_kind: EntryKind::File,
                }],
            }],
        }
    }

    fn sample_manifest() -> Manifest {
        let env = CaptureEnvironment {
            machine: "HOST".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
        };
        build_manifest(ManifestInputs {
            captured_at: TimestampNs(1_700_000_000_000_000_000),
            env: &env,
            roots: &[PathBuf::from("src")],
            binlog_archive_name: "build.binlog",
            kind: "T1",
            pair_id: None,
        })
    }

    fn build(binlog_bytes: &[u8]) -> Vec<u8> {
        let tree = sample_tree();
        let manifest = sample_manifest();
        let inputs = ArchiveInputs {
            binlog_name: "build.binlog",
            tree: &tree,
            manifest: &manifest,
        };
        let mut buf = Cursor::new(Vec::<u8>::new());
        write_archive(&inputs, Cursor::new(binlog_bytes), &mut buf).expect("write");
        buf.into_inner()
    }

    fn open(zip_bytes: Vec<u8>) -> zip::ZipArchive<Cursor<Vec<u8>>> {
        zip::ZipArchive::new(Cursor::new(zip_bytes)).expect("zip open")
    }

    #[test]
    fn archive_contains_binlog_tree_and_imports_dir() {
        let zip_bytes = build(b"BINLOG-PAYLOAD-BYTES");
        let archive = open(zip_bytes);
        let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
        assert!(names.iter().any(|n| n == "build.binlog"));
        assert!(names.iter().any(|n| n == TREE_JSON_NAME));
        assert!(names.iter().any(|n| n == IMPORTS_DIR_NAME));
    }

    #[test]
    fn binlog_bytes_are_stored_verbatim() {
        let payload = b"\x00\x01\x02BINLOG\xff\xfe";
        let zip_bytes = build(payload);
        let mut archive = open(zip_bytes);
        let mut f = archive.by_name("build.binlog").expect("binlog entry");
        let mut got = Vec::new();
        f.read_to_end(&mut got).expect("read");
        assert_eq!(got, payload);
    }

    #[test]
    fn tree_json_round_trips_through_archive() {
        let zip_bytes = build(b"x");
        let mut archive = open(zip_bytes);
        let mut f = archive.by_name(TREE_JSON_NAME).expect("tree.json");
        let mut buf = String::new();
        f.read_to_string(&mut buf).expect("read");
        let back: TreeSnapshot = serde_json::from_str(&buf).expect("parse");
        assert_eq!(back, sample_tree());
    }

    #[test]
    fn manifest_round_trips_through_archive() {
        let zip_bytes = build(b"x");
        let mut archive = open(zip_bytes);
        let mut f = archive.by_name(MANIFEST_NAME).expect("manifest.json");
        let mut buf = String::new();
        f.read_to_string(&mut buf).expect("read");
        let back: Manifest = serde_json::from_str(&buf).expect("parse");
        assert_eq!(back, sample_manifest());
    }

    #[test]
    fn imports_directory_is_present_and_empty() {
        let zip_bytes = build(b"x");
        let archive = open(zip_bytes);
        let imports_children: Vec<String> = archive
            .file_names()
            .filter(|n| n.starts_with(IMPORTS_DIR_NAME) && *n != IMPORTS_DIR_NAME)
            .map(str::to_owned)
            .collect();
        assert!(
            imports_children.is_empty(),
            "imports/ should be empty in M1: {imports_children:?}"
        );
    }
}
