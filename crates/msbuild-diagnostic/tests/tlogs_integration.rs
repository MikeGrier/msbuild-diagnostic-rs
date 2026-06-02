// Copyright (c) 2026 Mike Grier
//
//! Integration test for `tlogs::collect_tlogs` (D-14: FS-touching).

mod common;

use std::fs;
use std::path::PathBuf;

use msbuild_diagnostic::binlog::{BinlogProjectInventory, InventoryProject};
use msbuild_diagnostic::tlogs::collect_tlogs;

#[test]
fn collects_tlogs_from_each_project_obj_recursively_and_ignores_others() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path();

    // Project A with two tlogs (one nested) and a non-tlog file in obj/.
    let proj_a = root.join("ProjA");
    fs::create_dir_all(proj_a.join("obj").join("Debug")).unwrap();
    fs::write(proj_a.join("ProjA.csproj"), b"<Project/>").unwrap();
    fs::write(proj_a.join("obj").join("CL.read.1.tlog"), b"A-top").unwrap();
    fs::write(
        proj_a.join("obj").join("Debug").join("Link.write.1.TLOG"),
        b"A-nested",
    )
    .unwrap();
    fs::write(proj_a.join("obj").join("ignored.txt"), b"not a tlog").unwrap();

    // Project B with no obj/ dir.
    let proj_b = root.join("ProjB");
    fs::create_dir_all(&proj_b).unwrap();
    fs::write(proj_b.join("ProjB.csproj"), b"<Project/>").unwrap();

    // Stray tlog outside any project's obj/ should NOT be collected.
    fs::write(root.join("stray.tlog"), b"stray").unwrap();

    let inventory = BinlogProjectInventory {
        projects: vec![
            InventoryProject {
                project_file: proj_a.join("ProjA.csproj"),
            },
            InventoryProject {
                project_file: proj_b.join("ProjB.csproj"),
            },
        ],
    };

    let collected = collect_tlogs(&inventory).expect("collect");
    let names: Vec<PathBuf> = collected
        .iter()
        .map(|t| t.archive_relpath.clone())
        .collect();

    assert_eq!(
        names,
        vec![
            PathBuf::from("ProjA").join("CL.read.1.tlog"),
            PathBuf::from("ProjA")
                .join("Debug")
                .join("Link.write.1.TLOG"),
        ]
    );
    // Verify raw bytes preserved.
    assert_eq!(collected[0].contents, b"A-top");
    assert_eq!(collected[1].contents, b"A-nested");
}
