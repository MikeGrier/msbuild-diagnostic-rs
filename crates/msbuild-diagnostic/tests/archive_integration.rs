//! Integration test for AR-5 (M1): synthesize a tempdir tree, invoke the
//! `msbuild-diagnostic archive` binary, unzip the result, and assert the
//! manifest / tree / filename invariants hold.
//!
//! Integration-tier per D-14: this test uses the real filesystem.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;

use msbuild_diagnostic::manifest::{Manifest, MANIFEST_NAME, MANIFEST_SCHEMA_VERSION};
use msbuild_diagnostic::snapshot::{EntryKind, TreeSnapshot, TREE_SCHEMA_VERSION};

const THRESHOLD: u64 = 4096;
const FILE_COUNT: usize = 1000;

fn bin() -> PathBuf {
    // CARGO_BIN_EXE_<name> is provided by Cargo for integration tests.
    PathBuf::from(env!("CARGO_BIN_EXE_msbuild-diagnostic"))
}

fn write_file(path: &std::path::Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    let mut f = fs::File::create(path).expect("create file");
    f.write_all(bytes).expect("write");
}

#[test]
fn archive_round_trip_over_synthetic_tree() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root_dir = tmp.path().join("root");
    let out_dir = tmp.path().join("out");
    let binlog_path = tmp.path().join("build.binlog");

    // Synthesize ~1000 files of varied sizes, some above the threshold.
    // Sizes cycle through values that bracket the threshold so we exercise
    // both branches of the hash-iff-small rule.
    let sizes: [u64; 5] = [0, 128, THRESHOLD, THRESHOLD + 1, THRESHOLD * 4];
    for i in 0..FILE_COUNT {
        let size = sizes[i % sizes.len()];
        // Spread across nested subdirectories to exercise tree walk.
        let sub = i % 16;
        let path = root_dir
            .join(format!("d{sub:02}"))
            .join(format!("f{i:04}.bin"));
        // Deterministic content: byte = (i & 0xff).
        let byte = (i & 0xff) as u8;
        write_file(&path, &vec![byte; size as usize]);
    }

    // Fake binlog payload — content is opaque to the archiver in M1.
    write_file(&binlog_path, b"BINLOG-PAYLOAD-FIXTURE\n");

    let status = Command::new(bin())
        .arg("archive")
        .arg("--binlog")
        .arg(&binlog_path)
        .arg("--root")
        .arg(&root_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--small-file-hash-threshold")
        .arg(THRESHOLD.to_string())
        .arg("--kind")
        .arg("T1")
        .status()
        .expect("spawn");
    assert!(status.success(), "archive command failed: {status:?}");

    // Exactly one archive should have been produced.
    let entries: Vec<_> = fs::read_dir(&out_dir)
        .expect("read out_dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().into_string().unwrap())
        .collect();
    assert_eq!(entries.len(), 1, "expected 1 archive, got {entries:?}");
    let archive_name = &entries[0];

    // Filename: <binlog-stem>-<YYYYMMDDTHHMMSSZ>-<kind>.zip
    assert!(
        archive_name.starts_with("build-"),
        "archive name should start with binlog stem: {archive_name}"
    );
    assert!(
        archive_name.ends_with("-T1.zip"),
        "archive name should end with -<kind>.zip: {archive_name}"
    );
    // The timestamp segment is exactly 16 chars: YYYYMMDDTHHMMSSZ.
    let middle = archive_name
        .trim_start_matches("build-")
        .trim_end_matches("-T1.zip");
    assert_eq!(
        middle.len(),
        16,
        "timestamp segment must be YYYYMMDDTHHMMSSZ: {middle}"
    );
    assert!(middle.ends_with('Z'));
    assert_eq!(&middle[8..9], "T");

    // Open the archive and round-trip manifest + tree.
    let archive_path = out_dir.join(archive_name);
    let archive_file = fs::File::open(&archive_path).expect("open archive");
    let mut zip = zip::ZipArchive::new(archive_file).expect("read zip");

    let manifest: Manifest = {
        let mut f = zip.by_name(MANIFEST_NAME).expect("manifest.json present");
        let mut s = String::new();
        f.read_to_string(&mut s).expect("read manifest");
        serde_json::from_str(&s).expect("parse manifest")
    };
    assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
    assert_eq!(manifest.kind, "T1");
    assert_eq!(manifest.binlog_archive_name, "build.binlog");
    assert_eq!(manifest.roots, vec![root_dir.clone()]);

    let tree: TreeSnapshot = {
        let mut f = zip.by_name("tree.json").expect("tree.json present");
        let mut s = String::new();
        f.read_to_string(&mut s).expect("read tree");
        serde_json::from_str(&s).expect("parse tree")
    };
    assert_eq!(tree.schema_version, TREE_SCHEMA_VERSION);
    assert_eq!(tree.small_file_hash_threshold, THRESHOLD);
    assert_eq!(tree.roots.len(), 1);

    let root_snapshot = &tree.roots[0];
    assert_eq!(root_snapshot.root, root_dir);

    // Count regular files (excluding directory entries) and verify
    // sha256-iff-small invariant.
    let mut file_count = 0usize;
    for entry in &root_snapshot.entries {
        if !matches!(entry.entry_kind, EntryKind::File) {
            continue;
        }
        file_count += 1;
        if entry.size <= THRESHOLD {
            assert!(
                entry.sha256.is_some(),
                "small file must have sha256: {} (size {})",
                entry.relpath.display(),
                entry.size
            );
        } else {
            assert!(
                entry.sha256.is_none(),
                "large file must not have sha256: {} (size {})",
                entry.relpath.display(),
                entry.size
            );
        }
    }
    assert_eq!(file_count, FILE_COUNT, "every synthesized file recorded");

    // The binlog must be stored verbatim under its filename.
    let mut bin_entry = zip.by_name("build.binlog").expect("binlog present");
    let mut bin_bytes = Vec::new();
    bin_entry.read_to_end(&mut bin_bytes).expect("read binlog");
    assert_eq!(bin_bytes, b"BINLOG-PAYLOAD-FIXTURE\n");
}
