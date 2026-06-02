// Copyright (c) 2026 Mike Grier
//
//! Shared helpers for integration tests.

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
