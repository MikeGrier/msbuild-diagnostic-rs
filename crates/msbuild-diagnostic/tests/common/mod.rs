// Copyright (c) 2026 Mike Grier
//
//! Shared helpers for integration tests.
//!
//! Each test binary that includes this module via `mod common;` will use a
//! subset of the helpers. Allow dead code so adding a helper for one test
//! does not warn from another test binary.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use munin_msbuild::jsonlog::{
    ArchiveB64, JsonlogEvent, JsonlogEventBody, JsonlogFile, JsonlogHeader,
};
use munin_msbuild::BinlogIndex;

/// Produce a valid `.binlog` byte stream containing zero events.
///
/// Use when the test only needs the binlog to parse cleanly — i.e. the
/// scenario does not assert anything about the binlog's contents.
pub fn synthesize_empty_binlog() -> Vec<u8> {
    write_jsonlog(JsonlogFile {
        munin_jsonlog_version: 1,
        header: JsonlogHeader {
            file_format_version: 18,
            min_reader_version: 14,
        },
        strings: vec![],
        name_value_lists: vec![],
        archives: vec![],
        events: vec![],
    })
}

/// Produce a valid `.binlog` byte stream with one `ProjectStarted` event
/// per `project_files` entry and one embedded import zip containing the
/// `(path, contents)` entries in `imports`.
pub fn synthesize_binlog(project_files: &[&str], imports: &[(&str, &str)]) -> Vec<u8> {
    // Build the embedded "ProjectImportArchive" payload: a real zip with
    // the supplied (path, contents) entries.
    let archive_bytes = build_import_archive_zip(imports);

    // One ProjectStarted event per project_file. The munin writer
    // auto-interns strings, so the strings table can stay empty.
    let events: Vec<JsonlogEvent> = project_files
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let ev = munin_msbuild::events::ProjectStartedEvent {
                project_file: Some((*p).into()),
                ..munin_msbuild::events::ProjectStartedEvent::default()
            };
            JsonlogEvent {
                kind: "ProjectStarted".to_string(),
                byte_offset: i as u64,
                body: JsonlogEventBody::Decoded(serde_json::to_value(&ev).expect("event json")),
            }
        })
        .collect();

    let archives = if archive_bytes.is_empty() {
        vec![]
    } else {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        vec![ArchiveB64 {
            data_b64: BASE64.encode(&archive_bytes),
        }]
    };

    write_jsonlog(JsonlogFile {
        munin_jsonlog_version: 1,
        header: JsonlogHeader {
            file_format_version: 18,
            min_reader_version: 14,
        },
        strings: vec![],
        name_value_lists: vec![],
        archives,
        events,
    })
}

fn write_jsonlog(file: JsonlogFile) -> Vec<u8> {
    let index = BinlogIndex::from_jsonlog(file).expect("from_jsonlog");
    let mut out = Vec::new();
    index.write_binlog(&mut out).expect("write_binlog");
    out
}

/// Synthesize a binlog containing one `ProjectStarted` event plus, for
/// each `(target, input, output)` triple, a `TargetStarted` (with
/// `project_file` set to `project_file`) followed by a `BuildMessage`
/// reading `Input file "<input>" is newer than output file "<output>".`.
/// AR-16 uses this to drive the AR-15 correlator without requiring a
/// real MSBuild invocation (per the D-15 deviation pattern).
pub fn synthesize_correlation_binlog(
    project_file: &str,
    newer_than: &[(&str, &str, &str)],
) -> Vec<u8> {
    let mut events: Vec<JsonlogEvent> = Vec::new();
    let proj_ev = munin_msbuild::events::ProjectStartedEvent {
        project_file: Some(project_file.into()),
        ..munin_msbuild::events::ProjectStartedEvent::default()
    };
    events.push(JsonlogEvent {
        kind: "ProjectStarted".to_string(),
        byte_offset: 0,
        body: JsonlogEventBody::Decoded(serde_json::to_value(&proj_ev).expect("event json")),
    });

    let mut offset: u64 = 1;
    for (target, input, output) in newer_than {
        let ts = munin_msbuild::events::TargetStartedEvent {
            target_name: Some((*target).into()),
            project_file: Some(project_file.into()),
            ..munin_msbuild::events::TargetStartedEvent::default()
        };
        events.push(JsonlogEvent {
            kind: "TargetStarted".to_string(),
            byte_offset: offset,
            body: JsonlogEventBody::Decoded(serde_json::to_value(&ts).expect("event json")),
        });
        offset += 1;

        let mut msg = munin_msbuild::events::BuildMessageEvent::default();
        msg.fields.message = Some(format!(
            "Input file \"{input}\" is newer than output file \"{output}\"."
        ));
        // Field-flag bitmask must include MESSAGE so write_binlog
        // serializes the message string (munin's binlog writer is
        // flags-driven).
        msg.fields.flags = munin_msbuild::BuildEventArgsFieldFlags::from_raw(0x0004);
        events.push(JsonlogEvent {
            kind: "Message".to_string(),
            byte_offset: offset,
            body: JsonlogEventBody::Decoded(serde_json::to_value(&msg).expect("event json")),
        });
        offset += 1;
    }

    write_jsonlog(JsonlogFile {
        munin_jsonlog_version: 1,
        header: JsonlogHeader {
            file_format_version: 18,
            min_reader_version: 14,
        },
        strings: vec![],
        name_value_lists: vec![],
        archives: vec![],
        events,
    })
}

fn build_import_archive_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    if entries.is_empty() {
        return Vec::new();
    }
    // Dedupe by path so the test inputs match what munin will surface.
    let mut by_path: BTreeMap<&str, &str> = BTreeMap::new();
    for (p, c) in entries {
        by_path.insert(*p, *c);
    }

    let mut buf = Cursor::new(Vec::<u8>::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (path, contents) in by_path {
            zw.start_file(path, opts).expect("start_file");
            zw.write_all(contents.as_bytes()).expect("write");
        }
        zw.finish().expect("finish");
    }
    buf.into_inner()
}
