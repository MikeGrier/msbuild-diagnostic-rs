// Copyright (c) 2026 Mike Grier

//! Archive writer.
//!
//! Produces the on-disk `.zip` payload described in D-3. The writer is
//! intentionally parameterized on readers (not paths) for the binlog so
//! unit tests can exercise it hermetically per D-14.

use std::io::{Read, Seek, Write};

use zip::write::{SimpleFileOptions, ZipWriter};
use zip::CompressionMethod;

use crate::binlog::BinlogImport;
use crate::manifest::{Manifest, MANIFEST_NAME};
use crate::snapshot::TreeSnapshot;
use crate::tlogs::TlogFile;

/// Canonical name for the file-tree snapshot inside the archive.
pub const TREE_JSON_NAME: &str = "tree.json";

/// Canonical name for the imports subdirectory inside the archive (D-3).
pub const IMPORTS_DIR_NAME: &str = "imports/";

/// Canonical name for the tlogs subdirectory inside the archive (D-3).
pub const TLOGS_DIR_NAME: &str = "tlogs/";

/// Inputs to a single archive write.
pub struct ArchiveInputs<'a> {
    /// Name to store the binlog under inside the archive (typically the
    /// binlog's original filename).
    pub binlog_name: &'a str,
    /// File-tree snapshot to serialize as `tree.json`.
    pub tree: &'a TreeSnapshot,
    /// Capture manifest to serialize as `manifest.json`.
    pub manifest: &'a Manifest,
    /// Extracted `ProjectImportArchive` entries to write under `imports/`.
    /// May be empty — the `imports/` directory is always present (D-3)
    /// even with no entries.
    pub imports: &'a [BinlogImport],
    /// `.tlog` files collected from each project's `obj/` directory.
    /// May be empty — the `tlogs/` directory is always present (D-3).
    pub tlogs: &'a [TlogFile],
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

    for import in inputs.imports {
        // Sanitize: imports/<path>; reject absolute or parent-escape paths.
        let safe = sanitize_import_path(&import.path).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("unsafe import path: {}", import.path),
            )
        })?;
        let entry_name = format!("{IMPORTS_DIR_NAME}{safe}");
        zw.start_file(&entry_name, file_opts).map_err(zip_to_io)?;
        zw.write_all(import.contents.as_bytes())?;
    }

    zw.add_directory(TLOGS_DIR_NAME, dir_opts)
        .map_err(zip_to_io)?;
    for tlog in inputs.tlogs {
        let rel = tlog
            .archive_relpath
            .to_str()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("non-utf8 tlog path: {:?}", tlog.archive_relpath),
                )
            })?
            .replace('\\', "/");
        let safe = sanitize_import_path(&rel).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("unsafe tlog path: {rel}"),
            )
        })?;
        let entry_name = format!("{TLOGS_DIR_NAME}{safe}");
        zw.start_file(&entry_name, file_opts).map_err(zip_to_io)?;
        zw.write_all(&tlog.contents)?;
    }

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

/// Normalize a binlog-supplied import path to a safe zip-relative path.
///
/// Returns `None` if the path is absolute, has a drive letter, or contains
/// any `..` component — these would let a malicious import escape the
/// `imports/` subtree on extraction. Backslashes are normalized to `/`.
fn sanitize_import_path(raw: &str) -> Option<String> {
    let normalized = raw.replace('\\', "/");
    // Reject leading-slash (absolute POSIX) and drive-letter (absolute Windows).
    if normalized.starts_with('/') {
        return None;
    }
    if normalized.len() >= 2 {
        let mut chars = normalized.chars();
        let first = chars.next().unwrap();
        let second = chars.next().unwrap();
        if first.is_ascii_alphabetic() && second == ':' {
            return None;
        }
    }
    if normalized.is_empty() {
        return None;
    }
    for segment in normalized.split('/') {
        if segment == ".." || segment.is_empty() {
            return None;
        }
    }
    Some(normalized)
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
        build_with_imports(binlog_bytes, &[])
    }

    fn build_with_imports(binlog_bytes: &[u8], imports: &[BinlogImport]) -> Vec<u8> {
        build_full(binlog_bytes, imports, &[])
    }

    fn build_full(
        binlog_bytes: &[u8],
        imports: &[BinlogImport],
        tlogs: &[crate::tlogs::TlogFile],
    ) -> Vec<u8> {
        let tree = sample_tree();
        let manifest = sample_manifest();
        let inputs = ArchiveInputs {
            binlog_name: "build.binlog",
            tree: &tree,
            manifest: &manifest,
            imports,
            tlogs,
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
            "imports/ should be empty with no inputs: {imports_children:?}"
        );
    }

    #[test]
    fn imports_are_written_under_imports_prefix() {
        let imports = vec![
            BinlogImport {
                path: "Sdk.props".into(),
                contents: "<Project>sdk</Project>".into(),
            },
            BinlogImport {
                path: "sub/Nested.targets".into(),
                contents: "<Project>nested</Project>".into(),
            },
        ];
        let zip_bytes = build_with_imports(b"x", &imports);
        let mut archive = open(zip_bytes);

        let mut a = archive.by_name("imports/Sdk.props").expect("Sdk.props");
        let mut s = String::new();
        a.read_to_string(&mut s).unwrap();
        assert_eq!(s, "<Project>sdk</Project>");
        drop(a);

        let mut b = archive
            .by_name("imports/sub/Nested.targets")
            .expect("nested");
        let mut s = String::new();
        b.read_to_string(&mut s).unwrap();
        assert_eq!(s, "<Project>nested</Project>");
    }

    #[test]
    fn imports_with_backslashes_are_normalized_to_forward_slash() {
        let imports = vec![BinlogImport {
            path: r"sub\Nested.props".into(),
            contents: "x".into(),
        }];
        let zip_bytes = build_with_imports(b"x", &imports);
        let mut archive = open(zip_bytes);
        archive
            .by_name("imports/sub/Nested.props")
            .expect("normalized");
    }

    #[test]
    fn imports_reject_absolute_paths() {
        let imports = vec![BinlogImport {
            path: "/etc/passwd".into(),
            contents: "x".into(),
        }];
        let tree = sample_tree();
        let manifest = sample_manifest();
        let inputs = ArchiveInputs {
            binlog_name: "b.binlog",
            tree: &tree,
            manifest: &manifest,
            imports: &imports,
            tlogs: &[],
        };
        let mut buf = Cursor::new(Vec::<u8>::new());
        let err = write_archive(&inputs, Cursor::new(b"x".as_slice()), &mut buf)
            .expect_err("absolute import path must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn imports_reject_parent_escape() {
        let imports = vec![BinlogImport {
            path: "sub/../../escape.props".into(),
            contents: "x".into(),
        }];
        let tree = sample_tree();
        let manifest = sample_manifest();
        let inputs = ArchiveInputs {
            binlog_name: "b.binlog",
            tree: &tree,
            manifest: &manifest,
            imports: &imports,
            tlogs: &[],
        };
        let mut buf = Cursor::new(Vec::<u8>::new());
        let err = write_archive(&inputs, Cursor::new(b"x".as_slice()), &mut buf)
            .expect_err("`..` segments must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn tlogs_directory_is_always_present() {
        let zip_bytes = build(b"x");
        let archive = open(zip_bytes);
        assert!(archive.file_names().any(|n| n == TLOGS_DIR_NAME));
    }

    #[test]
    fn tlogs_are_written_under_tlogs_prefix_with_raw_bytes() {
        let tlogs = vec![
            crate::tlogs::TlogFile {
                archive_relpath: std::path::PathBuf::from("Hello/CL.read.1.tlog"),
                contents: vec![0xFF, 0xFE, b'a', 0, b'b', 0],
            },
            crate::tlogs::TlogFile {
                archive_relpath: std::path::PathBuf::from("Hello/sub/Link.write.1.tlog"),
                contents: b"linked".to_vec(),
            },
        ];
        let zip_bytes = build_full(b"x", &[], &tlogs);
        let mut archive = open(zip_bytes);

        let mut a = archive
            .by_name("tlogs/Hello/CL.read.1.tlog")
            .expect("first tlog");
        let mut got = Vec::new();
        a.read_to_end(&mut got).unwrap();
        assert_eq!(got, vec![0xFF, 0xFE, b'a', 0, b'b', 0]);
        drop(a);

        let mut b = archive
            .by_name("tlogs/Hello/sub/Link.write.1.tlog")
            .expect("nested tlog");
        let mut got = Vec::new();
        b.read_to_end(&mut got).unwrap();
        assert_eq!(got, b"linked");
    }
}
