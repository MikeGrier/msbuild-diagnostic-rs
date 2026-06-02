// Copyright (c) 2026 Mike Grier

//! Capture manifest — `manifest.json` payload inside the archive (D-6).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::snapshot::TimestampNs;

/// Current `manifest.json` schema version.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Canonical name for the manifest inside the archive.
pub const MANIFEST_NAME: &str = "manifest.json";

/// On-disk shape of `manifest.json`. Captured eagerly at archive-write
/// time; all fields are owned by us (D-7) and any future addition must
/// be classified by the sanitization registry before it can land in
/// `tree.json` / `manifest.json` (D-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    /// Wall-clock capture time in nanoseconds since the Unix epoch (D-13).
    pub captured_at: TimestampNs,
    /// Hostname of the machine that produced the capture.
    pub machine: String,
    /// `std::env::consts::OS` (e.g. `windows`, `linux`).
    pub os: String,
    /// `std::env::consts::ARCH` (e.g. `x86_64`, `aarch64`).
    pub arch: String,
    /// Roots actually used for `tree.json` enumeration. Echoed here so
    /// the manifest is self-sufficient for analysis.
    pub roots: Vec<PathBuf>,
    /// Name of the binlog entry inside the archive.
    pub binlog_archive_name: String,
    /// Label distinguishing this capture from its pair (`T1`, `T2`, …).
    pub kind: String,
    /// Identifier linking a T1 / T2 pair (free-form).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pair_id: Option<String>,
}

/// Capture-tool environment values resolved from the live process (host
/// name, OS, arch). Split out so unit tests can inject a fixed value.
#[derive(Debug, Clone)]
pub struct CaptureEnvironment {
    pub machine: String,
    pub os: String,
    pub arch: String,
}

impl CaptureEnvironment {
    /// Resolve from the running process.
    pub fn from_process() -> Self {
        Self {
            machine: gethostname::gethostname().to_string_lossy().into_owned(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }
    }
}

/// Inputs to [`build_manifest`]. Pure-function shape so the manifest
/// builder is hermetically testable (D-12, D-14).
pub struct ManifestInputs<'a> {
    pub captured_at: TimestampNs,
    pub env: &'a CaptureEnvironment,
    pub roots: &'a [PathBuf],
    pub binlog_archive_name: &'a str,
    pub kind: &'a str,
    pub pair_id: Option<&'a str>,
}

/// Build a [`Manifest`] from typed inputs. Pure function (D-12).
pub fn build_manifest(inputs: ManifestInputs<'_>) -> Manifest {
    Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        captured_at: inputs.captured_at,
        machine: inputs.env.machine.clone(),
        os: inputs.env.os.clone(),
        arch: inputs.env.arch.clone(),
        roots: inputs.roots.to_vec(),
        binlog_archive_name: inputs.binlog_archive_name.to_string(),
        kind: inputs.kind.to_string(),
        pair_id: inputs.pair_id.map(str::to_owned),
    }
}

/// Compose the archive filename: `<binlog-stem>-<UTC-...>-<kind>.zip`
/// (D-6). The timestamp format is `YYYYMMDDTHHMMSSZ` and is always UTC.
///
/// Pure function over the captured timestamp; takes no environment.
pub fn compose_archive_filename(binlog_stem: &str, captured_at: TimestampNs, kind: &str) -> String {
    let ts = format_compact_utc(captured_at);
    let safe_kind = sanitize_kind(kind);
    format!("{binlog_stem}-{ts}-{safe_kind}.zip")
}

/// Format an instant as `YYYYMMDDTHHMMSSZ` (UTC, no separators).
pub fn format_compact_utc(ts: TimestampNs) -> String {
    let dt = offset_datetime_from_ns(ts);
    // Hand-formatted to avoid pulling in a format-description string.
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
    )
}

/// Format an instant as RFC 3339 in UTC (used in human-readable contexts).
pub fn format_rfc3339_utc(ts: TimestampNs) -> String {
    let dt = offset_datetime_from_ns(ts);
    dt.format(&Rfc3339).unwrap_or_else(|_| "invalid".into())
}

fn offset_datetime_from_ns(ts: TimestampNs) -> OffsetDateTime {
    // `time::OffsetDateTime::from_unix_timestamp_nanos` accepts i128 ns
    // directly. Out-of-range values fall back to the Unix epoch — the
    // recorded timestamp is preserved as-is in the manifest, so this
    // only affects filename generation.
    OffsetDateTime::from_unix_timestamp_nanos(ts.0).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn sanitize_kind(kind: &str) -> String {
    // Filenames: replace anything outside [A-Za-z0-9_.-] with '_'.
    kind.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Hermetic unit tests (D-14): no FS, no time-of-day reads.
    use super::*;

    fn env() -> CaptureEnvironment {
        CaptureEnvironment {
            machine: "HOST".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
        }
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let m = build_manifest(ManifestInputs {
            captured_at: TimestampNs(1_700_000_000_000_000_000),
            env: &env(),
            roots: &[PathBuf::from("a"), PathBuf::from("b")],
            binlog_archive_name: "build.binlog",
            kind: "T1",
            pair_id: Some("pair-7"),
        });
        let s = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn manifest_omits_pair_id_when_absent() {
        let m = build_manifest(ManifestInputs {
            captured_at: TimestampNs(0),
            env: &env(),
            roots: &[],
            binlog_archive_name: "b.binlog",
            kind: "T2",
            pair_id: None,
        });
        let s = serde_json::to_string(&m).unwrap();
        assert!(!s.contains("pair_id"));
    }

    #[test]
    fn manifest_rejects_unknown_fields() {
        let json = r#"{
            "schema_version": 1,
            "captured_at": "0",
            "machine": "h",
            "os": "linux",
            "arch": "x86_64",
            "roots": [],
            "binlog_archive_name": "b.binlog",
            "kind": "T1",
            "future_field": 7
        }"#;
        let err = serde_json::from_str::<Manifest>(json).unwrap_err();
        assert!(err.to_string().contains("future_field"));
    }

    #[test]
    fn compose_archive_filename_uses_compact_utc_and_kind() {
        // 2023-11-14T22:13:20 UTC
        let ts = TimestampNs(1_700_000_000_000_000_000);
        let name = compose_archive_filename("msbuild", ts, "T1");
        assert_eq!(name, "msbuild-20231114T221320Z-T1.zip");
    }

    #[test]
    fn compose_archive_filename_sanitizes_kind() {
        let ts = TimestampNs(0);
        let name = compose_archive_filename("b", ts, "weird/label:1");
        assert!(name.ends_with("-weird_label_1.zip"), "got {name}");
    }

    #[test]
    fn format_rfc3339_handles_epoch() {
        let s = format_rfc3339_utc(TimestampNs(0));
        assert!(s.starts_with("1970-01-01T00:00:00"));
    }
}
