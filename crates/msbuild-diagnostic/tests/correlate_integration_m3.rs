// Copyright (c) 2026 Mike Grier
//
//! M3 integration test (AR-16).
//!
//! Spec text references "build a real tiny fixture project with MSBuild
//! to produce a real binlog". The development host has no dotnet SDK
//! available, so this test follows the AR-11 / D-15 deviation pattern:
//! it synthesizes an equivalent binlog via
//! `common::synthesize_correlation_binlog` and drives the same diff +
//! correlation pipeline. The substantive AR-16 assertion — the
//! correlator identifies a touched-but-content-identical file — is
//! exercised end-to-end against real archive zips on disk.

mod common;

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use clap::Parser;
use msbuild_diagnostic::cli::{run, Cli};

#[test]
fn diff_with_binlog_identifies_touched_but_content_identical_input() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let work = tmp.path();

    // Lay out the fixture project: one source file and one build output.
    let proj_dir = work.join("hello");
    fs::create_dir_all(proj_dir.join("src")).unwrap();
    fs::create_dir_all(proj_dir.join("bin")).unwrap();
    let csproj = proj_dir.join("hello.csproj");
    fs::write(&csproj, b"<Project Sdk=\"Microsoft.NET.Sdk\"/>").unwrap();
    let src_file = proj_dir.join("src").join("a.cs");
    fs::write(&src_file, b"class A {}").unwrap();
    let out_file = proj_dir.join("bin").join("a.dll");
    fs::write(&out_file, b"\x4d\x5a-fake-pe").unwrap();

    // Set a known baseline mtime on the source file so the T2 touch
    // produces a deterministic forward delta.
    let baseline = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    set_mtime(&src_file, baseline);
    set_mtime(&out_file, baseline);

    // Synthesize a binlog whose Compile target carries a BuildMessage
    // saying src/a.cs is newer than bin/a.dll.
    let csproj_str = csproj.to_string_lossy().to_string();
    let binlog_bytes =
        common::synthesize_correlation_binlog(&csproj_str, &[("Compile", "src/a.cs", "bin/a.dll")]);
    let binlog_path = work.join("hello.binlog");
    fs::write(&binlog_path, &binlog_bytes).unwrap();

    // Archive T1.
    let out_dir = work.join("out");
    let cli_t1 = Cli::try_parse_from([
        "msbuild-diagnostic",
        "archive",
        "--binlog",
        binlog_path.to_str().unwrap(),
        "--root",
        proj_dir.to_str().unwrap(),
        "--kind",
        "T1",
        "--out",
        out_dir.to_str().unwrap(),
    ])
    .expect("parse t1");
    let mut stdout = Vec::<u8>::new();
    run(cli_t1, &mut stdout).expect("archive t1");
    let t1_zip = find_one_zip_with(&out_dir, "T1");

    // Touch src/a.cs: bump its mtime forward but leave the content (and
    // therefore size and sha256) untouched.
    let touched = baseline + Duration::from_secs(60);
    set_mtime(&src_file, touched);

    // Archive T2.
    let cli_t2 = Cli::try_parse_from([
        "msbuild-diagnostic",
        "archive",
        "--binlog",
        binlog_path.to_str().unwrap(),
        "--root",
        proj_dir.to_str().unwrap(),
        "--kind",
        "T2",
        "--out",
        out_dir.to_str().unwrap(),
    ])
    .expect("parse t2");
    let mut stdout = Vec::<u8>::new();
    run(cli_t2, &mut stdout).expect("archive t2");
    let t2_zip = find_one_zip_with(&out_dir, "T2");

    // Run diff with the binlog and a markdown destination.
    let diff_json = work.join("diff.json");
    let md_path = work.join("correlation.md");
    let cli_diff = Cli::try_parse_from([
        "msbuild-diagnostic",
        "diff",
        t1_zip.to_str().unwrap(),
        t2_zip.to_str().unwrap(),
        "--out",
        diff_json.to_str().unwrap(),
        "--binlog",
        binlog_path.to_str().unwrap(),
        "--markdown",
        md_path.to_str().unwrap(),
    ])
    .expect("parse diff");
    let mut stdout = Vec::<u8>::new();
    run(cli_diff, &mut stdout).expect("diff run");

    // The Markdown correlation report identifies src/a.cs as
    // touched-but-content-identical.
    let md = fs::read_to_string(&md_path).expect("read markdown");
    assert!(
        md.contains("touched-but-content-identical"),
        "markdown report missing touched-but-content-identical: {md}"
    );
    assert!(
        md.contains("a.cs"),
        "markdown report should mention a.cs: {md}"
    );
    assert!(
        md.contains("Compile"),
        "markdown report should mention Compile target: {md}"
    );
}

fn set_mtime(path: &Path, t: SystemTime) {
    let f = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open for set_mtime");
    f.set_modified(t).expect("set_modified");
}

fn find_one_zip_with(dir: &Path, suffix_kind: &str) -> std::path::PathBuf {
    let needle = format!("-{suffix_kind}.zip");
    let zips: Vec<_> = fs::read_dir(dir)
        .expect("read out dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(&needle))
        })
        .collect();
    assert_eq!(
        zips.len(),
        1,
        "expected one zip ending with '{needle}' in {dir:?}: {zips:?}"
    );
    zips.into_iter().next().unwrap()
}
