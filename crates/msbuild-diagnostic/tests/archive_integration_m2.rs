// Copyright (c) 2026 Mike Grier
//
//! M2 integration test (AR-11).
//!
//! Spec text references `testprojects/csharp/helloworld/msbuild.binlog`,
//! but that artifact is not checked in (no dotnet SDK available on the
//! development host). This test synthesizes an equivalent binlog via
//! `common::synthesize_binlog` so the assertions remain meaningful and
//! the test stays hermetic.

mod common;

use std::fs;
use std::io::Read;
use std::path::Path;

use clap::Parser;
use msbuild_diagnostic::cli::{run, Cli};

#[test]
fn end_to_end_archive_has_inventory_imports_and_tlogs() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let work = tmp.path();

    // Lay out the "helloworld" project on disk.
    let proj_dir = work.join("helloworld");
    fs::create_dir_all(proj_dir.join("obj").join("Debug")).unwrap();
    let csproj_abs = proj_dir.join("helloworld.csproj");
    fs::write(&csproj_abs, b"<Project Sdk=\"Microsoft.NET.Sdk\"/>").unwrap();
    fs::write(proj_dir.join("Program.cs"), b"class Program{}").unwrap();
    // A tlog under obj/ that AR-10 must pick up.
    fs::write(
        proj_dir.join("obj").join("Debug").join("CL.read.1.tlog"),
        b"tlog-content",
    )
    .unwrap();

    // Synthesize a binlog whose ProjectStarted points at the csproj and
    // that embeds two imports.
    let csproj_str = csproj_abs.to_string_lossy().to_string();
    let binlog_bytes = common::synthesize_binlog(
        &[&csproj_str],
        &[
            ("Directory.Build.props", "<Project>top</Project>"),
            ("Sdk/Sdk.props", "<Project>sdk</Project>"),
        ],
    );
    let binlog_path = work.join("helloworld.binlog");
    fs::write(&binlog_path, &binlog_bytes).unwrap();

    // Run the CLI archive command.
    let out_dir = work.join("out");
    let cli = Cli::try_parse_from([
        "msbuild-diagnostic",
        "archive",
        "--binlog",
        binlog_path.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
    ])
    .expect("parse");
    let mut stdout = Vec::<u8>::new();
    run(cli, &mut stdout).expect("archive run");

    // Find the produced zip.
    let zip_path = find_one_zip(&out_dir);
    let zip_bytes = fs::read(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).expect("open zip");

    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();

    // Binlog is stored verbatim under its original filename.
    assert!(names.iter().any(|n| n == "helloworld.binlog"));

    // imports/ and tlogs/ directories are present (D-3).
    assert!(names.iter().any(|n| n == "imports/"));
    assert!(names.iter().any(|n| n == "tlogs/"));

    // Imports were extracted from the embedded ProjectImportArchive.
    assert!(names.iter().any(|n| n == "imports/Directory.Build.props"));
    assert!(names.iter().any(|n| n == "imports/Sdk/Sdk.props"));

    // Tlogs were collected from <project>/obj/.
    assert!(
        names
            .iter()
            .any(|n| n == "tlogs/helloworld/Debug/CL.read.1.tlog"),
        "expected tlog under tlogs/helloworld/Debug/; got {names:?}"
    );

    // manifest.json roots includes the project directory.
    let mut mf = archive.by_name("manifest.json").expect("manifest");
    let mut mf_str = String::new();
    mf.read_to_string(&mut mf_str).unwrap();
    let proj_dir_str = proj_dir.to_string_lossy().replace('\\', "/");
    let mf_norm = mf_str.replace("\\\\", "/").replace('\\', "/");
    assert!(
        mf_norm.contains(&proj_dir_str),
        "manifest.json did not contain project dir: manifest={mf_str}\nproj_dir={proj_dir_str}"
    );
    drop(mf);

    // tree.json contains the csproj as an entry.
    let mut t = archive.by_name("tree.json").expect("tree.json");
    let mut t_str = String::new();
    t.read_to_string(&mut t_str).unwrap();
    assert!(
        t_str.contains("helloworld.csproj"),
        "tree.json did not contain helloworld.csproj"
    );
}

fn find_one_zip(dir: &Path) -> std::path::PathBuf {
    let mut zips: Vec<_> = fs::read_dir(dir)
        .expect("read out dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("zip"))
        .collect();
    assert_eq!(zips.len(), 1, "expected one zip in {dir:?}: {zips:?}");
    zips.pop().unwrap()
}
