//! Integration tests for M4 sanitize (AR-19 + AR-22).
//!
//! Per D-14 integration-tier these tests touch the real filesystem
//! (tempdir-bound). They build a fixture capture archive by hand,
//! invoke `sanitize_archive`, and assert the AR-22 properties: paths
//! pseudonymized, property bodies redacted, unknown artifacts dropped
//! and surfaced in `sanitization-report.json`, pseudonym map present
//! **outside** the zip and absent **inside** it.

use std::io::{Read, Write};
use std::path::PathBuf;

use msbuild_diagnostic::archive::{IMPORTS_DIR_NAME, TREE_JSON_NAME};
use msbuild_diagnostic::manifest::MANIFEST_NAME;
use msbuild_diagnostic::sanitize::pipeline::{
    default_output_paths, sanitize_archive, Disposition, PseudonymMap, SanitizationReport,
    SanitizeInputs, REDACTED_PROPERTY_PLACEHOLDER, SANITIZATION_REPORT_NAME,
};
use msbuild_diagnostic::sanitize::pseudonym::Pseudonymizer;

const FAKE_USER_PROFILE: &str = "/home/synthetic-user";
const FAKE_API_KEY: &str = "AKIA-FAKE-CREDENTIAL-1234567890";

fn build_fixture_zip(path: &std::path::Path, binlog_name: &str) {
    let file = std::fs::File::create(path).expect("create fixture zip");
    let mut zip = zip::ZipWriter::new(file);
    let file_opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let dir_opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    // manifest.json — references the synthetic user profile in `roots`
    // and carries a recognizable machine name.
    let manifest = serde_json::json!({
        "schema_version": 1,
        "captured_at": "1700000000000000000",
        "machine": "HOST-SYNTHETIC-42",
        "os": "linux",
        "arch": "x86_64",
        "roots": [format!("{FAKE_USER_PROFILE}/proj")],
        "binlog_archive_name": binlog_name,
        "kind": "T1"
    });
    zip.start_file(MANIFEST_NAME, file_opts).unwrap();
    zip.write_all(serde_json::to_string_pretty(&manifest).unwrap().as_bytes())
        .unwrap();

    // tree.json — root + one entry, both under the synthetic profile.
    let tree = serde_json::json!({
        "schema_version": 1,
        "small_file_hash_threshold": 4096,
        "roots": [{
            "root": format!("{FAKE_USER_PROFILE}/proj"),
            "entries": [{
                "relpath": format!("{FAKE_USER_PROFILE}/proj/src/a.cs"),
                "size": 42,
                "mtime_unix_nanos": "1700000000000000000",
                "entry_kind": { "kind": "file" }
            }]
        }]
    });
    zip.start_file(TREE_JSON_NAME, file_opts).unwrap();
    zip.write_all(serde_json::to_string_pretty(&tree).unwrap().as_bytes())
        .unwrap();

    // imports/ directory marker, then a single import carrying a fake
    // API key in an MSBuild property body and a profile-path
    // attribute.
    zip.add_directory(IMPORTS_DIR_NAME, dir_opts).unwrap();
    let import_xml = format!(
        "<Project ToolsPath=\"{FAKE_USER_PROFILE}/sdk\"><PropertyGroup>\
         <ApiKey>{FAKE_API_KEY}</ApiKey>\
         </PropertyGroup></Project>"
    );
    zip.start_file("imports/secrets.props", file_opts).unwrap();
    zip.write_all(import_xml.as_bytes()).unwrap();

    // Unknown artifact under obj/ — must be dropped and listed.
    zip.start_file("obj/random.dat", file_opts).unwrap();
    zip.write_all(b"opaque-binary-bytes").unwrap();

    // Captured binlog — content irrelevant; sanitizer must drop and
    // record (D-17).
    zip.start_file(binlog_name, file_opts).unwrap();
    zip.write_all(b"\x1F\x8B\x08\x00FAKE-BINLOG").unwrap();

    zip.finish().unwrap();
}

fn open_zip(path: &std::path::Path) -> zip::ZipArchive<std::fs::File> {
    let f = std::fs::File::open(path).expect("open zip");
    zip::ZipArchive::new(f).expect("read zip")
}

fn read_zip_text(zin: &mut zip::ZipArchive<std::fs::File>, name: &str) -> String {
    let mut entry = zin.by_name(name).expect("entry present");
    let mut s = String::new();
    entry.read_to_string(&mut s).expect("read entry");
    s
}

fn zip_entry_names(zin: &mut zip::ZipArchive<std::fs::File>) -> Vec<String> {
    (0..zin.len())
        .map(|i| zin.by_index(i).unwrap().name().to_string())
        .collect()
}

#[test]
fn sanitize_fixture_satisfies_ar19_and_ar22() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let input = tmp.path().join("build-T1.zip");
    let binlog_name = "build.binlog";
    build_fixture_zip(&input, binlog_name);

    let (out_zip, map_path) = default_output_paths(&input);
    let pseudonymizer = Pseudonymizer::from_explicit(Some(FAKE_USER_PROFILE.to_string()));

    let report = sanitize_archive(&SanitizeInputs {
        input: &input,
        output: &out_zip,
        map: &map_path,
        pseudonymizer: &pseudonymizer,
    })
    .expect("sanitize");

    // ---- AR-19: report shape ----
    assert_eq!(report.unknown_artifacts, vec!["obj/random.dat".to_string()]);
    let dispositions: std::collections::BTreeMap<_, _> = report
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.disposition))
        .collect();
    assert_eq!(
        dispositions.get(MANIFEST_NAME),
        Some(&Disposition::Redacted)
    );
    assert_eq!(
        dispositions.get(TREE_JSON_NAME),
        Some(&Disposition::Redacted)
    );
    assert_eq!(
        dispositions.get("imports/secrets.props"),
        Some(&Disposition::Redacted)
    );
    assert_eq!(
        dispositions.get("obj/random.dat"),
        Some(&Disposition::Dropped)
    );
    assert_eq!(dispositions.get(binlog_name), Some(&Disposition::Dropped));
    // Every dropped entry carries both a rule id and a reason.
    for e in &report.entries {
        if matches!(e.disposition, Disposition::Dropped) {
            assert!(e.rule.is_some(), "dropped entry missing rule: {}", e.path);
            assert!(
                e.reason.is_some(),
                "dropped entry missing reason: {}",
                e.path
            );
        }
    }

    // ---- AR-22: sanitized zip contents ----
    let mut zout = open_zip(&out_zip);
    let names = zip_entry_names(&mut zout);

    // Unknown artifact absent inside.
    assert!(
        !names.iter().any(|n| n == "obj/random.dat"),
        "unknown artifact leaked into sanitized zip: {names:?}"
    );
    // Captured binlog absent (D-17).
    assert!(
        !names.iter().any(|n| n == binlog_name),
        "binlog leaked into sanitized zip: {names:?}"
    );
    // Sanitization report and known text artifacts present.
    assert!(names.iter().any(|n| n == SANITIZATION_REPORT_NAME));
    assert!(names.iter().any(|n| n == MANIFEST_NAME));
    assert!(names.iter().any(|n| n == TREE_JSON_NAME));
    assert!(names.iter().any(|n| n == "imports/secrets.props"));

    // Pseudonym map must never appear inside the zip (AR-18 constraint
    // re-asserted in AR-22).
    assert!(
        !names.iter().any(|n| n.contains("pseudonym-map")),
        "pseudonym map leaked into sanitized zip: {names:?}"
    );

    // manifest.json: machine + roots rewritten.
    let manifest_text = read_zip_text(&mut zout, MANIFEST_NAME);
    assert!(
        manifest_text.contains("<MACHINE>"),
        "machine not redacted: {manifest_text}"
    );
    assert!(
        !manifest_text.contains("HOST-SYNTHETIC-42"),
        "original machine leaked: {manifest_text}"
    );
    assert!(
        !manifest_text.contains(FAKE_USER_PROFILE),
        "original user profile leaked from manifest: {manifest_text}"
    );

    // tree.json: relpath + root rewritten.
    let tree_text = read_zip_text(&mut zout, TREE_JSON_NAME);
    assert!(
        !tree_text.contains(FAKE_USER_PROFILE),
        "original user profile leaked from tree: {tree_text}"
    );
    assert!(tree_text.contains("<USER>"));

    // imports/secrets.props: property body flattened, path attribute
    // pseudonymized.
    let import_text = read_zip_text(&mut zout, "imports/secrets.props");
    assert!(
        !import_text.contains(FAKE_API_KEY),
        "API key leaked: {import_text}"
    );
    assert!(
        import_text.contains(REDACTED_PROPERTY_PLACEHOLDER),
        "redaction placeholder missing: {import_text}"
    );
    assert!(
        !import_text.contains(FAKE_USER_PROFILE),
        "original user profile leaked from import: {import_text}"
    );
    assert!(import_text.contains("<USER>/sdk"));

    // Embedded sanitization-report.json round-trips.
    let report_text = read_zip_text(&mut zout, SANITIZATION_REPORT_NAME);
    let embedded: SanitizationReport = serde_json::from_str(&report_text).expect("report parses");
    assert_eq!(
        embedded.unknown_artifacts,
        vec!["obj/random.dat".to_string()]
    );

    // ---- AR-18 sibling map present outside, recording the originals ----
    assert!(
        map_path.exists(),
        "pseudonym map JSON missing: {map_path:?}"
    );
    let map_text = std::fs::read_to_string(&map_path).expect("read map");
    let map: PseudonymMap = serde_json::from_str(&map_text).expect("map parses");
    assert_eq!(map.machine.as_deref(), Some("HOST-SYNTHETIC-42"));
    assert_eq!(map.source_archive, "build-T1.zip");

    // Default sibling location is next to the input.
    let expected_map_path: PathBuf = input.with_file_name("build-T1-pseudonym-map.local.json");
    assert_eq!(map_path, expected_map_path);
}
