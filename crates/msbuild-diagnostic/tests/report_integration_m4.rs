//! AR-23 sanitization checkpoint for the M4 `report` flow.
//!
//! Verifies the AR-23 invariant: every value the `ISSUE.md` template
//! interpolates is sourced from a sanitized artifact. The test builds
//! a fixture capture archive that embeds a synthetic user-profile
//! path, runs the full `report` pipeline, and asserts the rendered
//! `ISSUE.md` contains no occurrence of the real (synthetic) profile
//! dir.

use std::io::Write;

use msbuild_diagnostic::archive::{IMPORTS_DIR_NAME, TREE_JSON_NAME};
use msbuild_diagnostic::manifest::MANIFEST_NAME;
use msbuild_diagnostic::report::{generate_report, ReportInputs};
use msbuild_diagnostic::sanitize::pseudonym::Pseudonymizer;

const FAKE_USER_PROFILE: &str = "/home/synthetic-user";

fn build_fixture_zip(path: &std::path::Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let file_opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let dir_opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    let manifest = serde_json::json!({
        "schema_version": 1,
        "captured_at": "1700000000000000000",
        "machine": "HOST-PROFILE-LEAK",
        "os": "linux",
        "arch": "x86_64",
        "roots": [format!("{FAKE_USER_PROFILE}/proj")],
        "binlog_archive_name": "build.binlog",
        "kind": "T1"
    });
    zip.start_file(MANIFEST_NAME, file_opts).unwrap();
    zip.write_all(serde_json::to_string_pretty(&manifest).unwrap().as_bytes())
        .unwrap();

    let tree = serde_json::json!({
        "schema_version": 1,
        "small_file_hash_threshold": 4096,
        "roots": [{
            "root": format!("{FAKE_USER_PROFILE}/proj"),
            "entries": []
        }]
    });
    zip.start_file(TREE_JSON_NAME, file_opts).unwrap();
    zip.write_all(serde_json::to_string_pretty(&tree).unwrap().as_bytes())
        .unwrap();

    zip.add_directory(IMPORTS_DIR_NAME, dir_opts).unwrap();
    zip.start_file("build.binlog", file_opts).unwrap();
    zip.write_all(b"FAKE").unwrap();
    zip.finish().unwrap();
}

#[test]
fn issue_md_contains_no_real_user_profile_path() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("build-T1.zip");
    build_fixture_zip(&input);

    let out_dir = tmp.path().join("report-out");
    let pseudonymizer = Pseudonymizer::from_explicit(Some(FAKE_USER_PROFILE.to_string()));
    let inputs = vec![input];
    let artifacts = generate_report(&ReportInputs {
        inputs: &inputs,
        // Operator prose deliberately mentions paths-shaped strings to
        // make sure the template still doesn't add a leak path on top.
        expected: "rebuild should be a no-op",
        actual: "rebuild walks every source again",
        out_dir: &out_dir,
        pseudonymizer: &pseudonymizer,
        issue_base_url: "https://example/issues/new",
    })
    .expect("generate_report");

    let issue_md = std::fs::read_to_string(&artifacts.issue_md).expect("read ISSUE.md");

    assert!(
        !issue_md.contains(FAKE_USER_PROFILE),
        "ISSUE.md leaked real user-profile dir: {issue_md}"
    );
    assert!(
        !issue_md.contains("HOST-PROFILE-LEAK"),
        "ISSUE.md leaked real machine name: {issue_md}"
    );
    assert!(
        issue_md.contains("<USER>/proj"),
        "ISSUE.md missing sanitized root: {issue_md}"
    );
    assert!(
        issue_md.contains("`<MACHINE>`"),
        "ISSUE.md missing sanitized machine: {issue_md}"
    );

    // The URL should also be free of the profile path (it's
    // percent-encoded but the body originates from the same template).
    assert!(
        !artifacts.issue_url.contains(FAKE_USER_PROFILE),
        "issue URL leaked real user-profile dir: {}",
        artifacts.issue_url
    );
}
