// Copyright (c) 2026 Mike Grier

//! `report` subcommand (AR-20 + AR-21).
//!
//! Wraps the sanitizer with a "ready-to-share" packaging step: takes
//! one or two capture archives plus the operator's expected/actual
//! description, sanitizes every input, packs the sanitized output(s)
//! into `submission.zip`, extracts a `submission-preview/` directory
//! for human review, writes a structured `ISSUE.md`, and composes a
//! prefilled GitHub issue URL (AR-21 — printed only, never opened).
//!
//! Per AR-23, every value interpolated into `ISSUE.md` must come from
//! a sanitized artifact. The pure [`build_issue_md`] helper takes a
//! [`SanitizedEnvironment`] value that is constructed exclusively
//! from a *sanitized* manifest; `generate_report` enforces this by
//! reading the manifest back out of the sanitized zip rather than the
//! input zip.

use std::collections::BTreeSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::MANIFEST_NAME;
use crate::sanitize::pipeline::{
    default_output_paths, sanitize_archive, Disposition, SanitizationReport, SanitizeInputs,
    SANITIZATION_REPORT_NAME,
};
use crate::sanitize::pseudonym::Pseudonymizer;

/// Default base URL for the prefilled GitHub issue link (AR-21).
pub const DEFAULT_ISSUE_BASE_URL: &str =
    "https://github.com/MikeGrier/msbuild-diagnostic-rs/issues/new";

/// Filename of the operator-facing report rendered into the submission
/// preview directory.
pub const ISSUE_MD_NAME: &str = "ISSUE.md";

/// Filename of the packaged submission archive.
pub const SUBMISSION_ZIP_NAME: &str = "submission.zip";

/// Name of the directory the sanitized zip(s) are extracted into so
/// the operator can review what they are about to share.
pub const SUBMISSION_PREVIEW_DIR: &str = "submission-preview";

/// Sanitized capture environment used to fill in the `ISSUE.md`
/// template. Every field here must originate from a sanitized
/// artifact (AR-23). Stored as plain owned strings so the type cannot
/// inadvertently carry serde-decorated references back to original
/// data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SanitizedEnvironment {
    pub machine: String,
    pub os: String,
    pub arch: String,
    pub roots: Vec<String>,
    pub kind: String,
}

impl SanitizedEnvironment {
    /// Parse a [`SanitizedEnvironment`] out of the JSON text of a
    /// **sanitized** `manifest.json`. The caller is responsible for
    /// ensuring the JSON came from a sanitized artifact; this
    /// function does not re-run any redaction.
    pub fn from_sanitized_manifest_json(raw: &str) -> io::Result<Self> {
        let v: serde_json::Value = serde_json::from_str(raw).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("sanitized manifest.json: {e}"),
            )
        })?;
        let get_str =
            |k: &str| -> String { v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string() };
        let roots = v
            .get("roots")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(Self {
            machine: get_str("machine"),
            os: get_str("os"),
            arch: get_str("arch"),
            roots,
            kind: get_str("kind"),
        })
    }
}

/// Inputs to [`generate_report`].
pub struct ReportInputs<'a> {
    /// One or two capture archives. Order is preserved (T1 then T2).
    pub inputs: &'a [PathBuf],
    pub expected: &'a str,
    pub actual: &'a str,
    /// Directory into which `submission.zip`, `submission-preview/`,
    /// and `ISSUE.md` will be written. Created if absent.
    pub out_dir: &'a Path,
    /// Pseudonymizer applied during sanitization.
    pub pseudonymizer: &'a Pseudonymizer,
    /// Base URL for the prefilled issue link (AR-21). Tests override
    /// this to keep assertions stable.
    pub issue_base_url: &'a str,
}

/// Paths produced by [`generate_report`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportArtifacts {
    pub submission_zip: PathBuf,
    pub preview_dir: PathBuf,
    pub issue_md: PathBuf,
    /// Prefilled GitHub issue URL (AR-21). The CLI prints this; nothing
    /// opens it automatically.
    pub issue_url: String,
}

/// Run the end-to-end report flow (AR-20 + AR-21).
pub fn generate_report(args: &ReportInputs<'_>) -> io::Result<ReportArtifacts> {
    if args.inputs.is_empty() || args.inputs.len() > 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "report requires 1 or 2 capture archives",
        ));
    }

    std::fs::create_dir_all(args.out_dir)?;

    // Sanitize every input into out_dir/staging/. The pseudonym map
    // also lands in staging — it is **never** rolled into
    // submission.zip (AR-18 constraint, re-applied here).
    let staging = args.out_dir.join("staging");
    std::fs::create_dir_all(&staging)?;

    let mut sanitized_zips: Vec<PathBuf> = Vec::new();
    let mut reports: Vec<SanitizationReport> = Vec::new();

    for input in args.inputs {
        let (default_out, default_map) = default_output_paths(input);
        let out_zip = staging.join(default_out.file_name().unwrap());
        let map_path = staging.join(default_map.file_name().unwrap());
        let report = sanitize_archive(&SanitizeInputs {
            input,
            output: &out_zip,
            map: &map_path,
            pseudonymizer: args.pseudonymizer,
        })?;
        sanitized_zips.push(out_zip);
        reports.push(report);
    }

    // Build submission.zip — contains the sanitized zip(s) only.
    let submission_zip = args.out_dir.join(SUBMISSION_ZIP_NAME);
    {
        let f = std::fs::File::create(&submission_zip)?;
        let mut zout = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for z in &sanitized_zips {
            let name = z.file_name().unwrap().to_string_lossy().into_owned();
            zout.start_file(&name, opts).map_err(zip_to_io)?;
            let mut src = std::fs::File::open(z)?;
            io::copy(&mut src, &mut zout)?;
        }
        zout.finish().map_err(zip_to_io)?;
    }

    // Extract sanitized zip(s) into submission-preview/.
    let preview_dir = args.out_dir.join(SUBMISSION_PREVIEW_DIR);
    if preview_dir.exists() {
        std::fs::remove_dir_all(&preview_dir)?;
    }
    std::fs::create_dir_all(&preview_dir)?;
    for z in &sanitized_zips {
        let stem = z
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "archive".into());
        let dest = preview_dir.join(&stem);
        std::fs::create_dir_all(&dest)?;
        let f = std::fs::File::open(z)?;
        let mut zin = zip::ZipArchive::new(f).map_err(zip_to_io)?;
        for i in 0..zin.len() {
            let mut entry = zin.by_index(i).map_err(zip_to_io)?;
            let rel = match entry.enclosed_name() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };
            let target = dest.join(rel);
            if entry.is_dir() {
                std::fs::create_dir_all(&target)?;
            } else {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut outf = std::fs::File::create(&target)?;
                io::copy(&mut entry, &mut outf)?;
            }
        }
    }

    // Read the sanitized manifest back out of the first sanitized
    // zip so every interpolated field is provably sourced from
    // sanitized data (AR-23 invariant).
    let env = {
        let first = sanitized_zips.first().expect("at least one input");
        let f = std::fs::File::open(first)?;
        let mut zin = zip::ZipArchive::new(f).map_err(zip_to_io)?;
        let mut manifest_text = String::new();
        zin.by_name(MANIFEST_NAME)
            .map_err(zip_to_io)?
            .read_to_string(&mut manifest_text)?;
        SanitizedEnvironment::from_sanitized_manifest_json(&manifest_text)?
    };

    let redactions = summarize_redactions(&reports);
    let issue_md_text = build_issue_md(&env, args.expected, args.actual, &redactions);
    let issue_md = args.out_dir.join(ISSUE_MD_NAME);
    std::fs::write(&issue_md, &issue_md_text)?;

    let issue_url = build_issue_url(
        args.issue_base_url,
        &default_issue_title(&env),
        &issue_md_text,
    );

    Ok(ReportArtifacts {
        submission_zip,
        preview_dir,
        issue_md,
        issue_url,
    })
}

/// Compose the body of `ISSUE.md` from a [`SanitizedEnvironment`] and
/// the operator's expected/actual prose. The implementation is pure
/// (D-12) so AR-23 can assert on its output without running the full
/// pipeline.
pub fn build_issue_md(
    env: &SanitizedEnvironment,
    expected: &str,
    actual: &str,
    redactions: &[RedactionSummary],
) -> String {
    let mut out = String::new();
    out.push_str("# Incremental-build diagnostic report\n\n");
    out.push_str("## Expected\n\n");
    out.push_str(expected.trim_end());
    out.push_str("\n\n## Actual\n\n");
    out.push_str(actual.trim_end());
    out.push_str("\n\n## Environment (sanitized)\n\n");
    out.push_str(&format!("- Machine: `{}`\n", env.machine));
    out.push_str(&format!("- OS: `{}`\n", env.os));
    out.push_str(&format!("- Arch: `{}`\n", env.arch));
    out.push_str(&format!("- Capture kind: `{}`\n", env.kind));
    out.push_str("- Roots:\n");
    for r in &env.roots {
        out.push_str(&format!("  - `{r}`\n"));
    }
    out.push_str("\n## Redactions applied\n\n");
    if redactions.is_empty() {
        out.push_str("_None._\n");
    } else {
        out.push_str("| Disposition | Rule | Count |\n");
        out.push_str("|---|---|---|\n");
        for r in redactions {
            out.push_str(&format!(
                "| {} | `{}` | {} |\n",
                r.disposition_label(),
                r.rule,
                r.count
            ));
        }
    }
    out.push_str(
        "\n## Attachment\n\n\
         Attach `submission.zip` (next to this file) manually before submitting.\n",
    );
    out
}

/// Build a prefilled GitHub `issues/new?title=...&body=...` URL (AR-21).
/// `body` is interpreted as plain text and percent-encoded per the
/// URL component charset (`unreserved`); newlines become `%0A`.
pub fn build_issue_url(base_url: &str, title: &str, body: &str) -> String {
    format!(
        "{base_url}?title={}&body={}",
        percent_encode(title),
        percent_encode(body)
    )
}

/// Default issue title derived from the sanitized environment.
pub fn default_issue_title(env: &SanitizedEnvironment) -> String {
    format!("Incremental-build report ({} / {})", env.os, env.arch)
}

/// Per-rule redaction count rolled up across every sanitization report
/// in the submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionSummary {
    pub disposition: Disposition,
    pub rule: String,
    pub count: usize,
}

impl RedactionSummary {
    fn disposition_label(&self) -> &'static str {
        match self.disposition {
            Disposition::Verbatim => "verbatim",
            Disposition::Redacted => "redacted",
            Disposition::Dropped => "dropped",
        }
    }
}

/// Aggregate the per-entry dispositions across one or more
/// [`SanitizationReport`]s into a stable, sorted summary suitable for
/// rendering in `ISSUE.md`.
pub fn summarize_redactions(reports: &[SanitizationReport]) -> Vec<RedactionSummary> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for report in reports {
        for entry in &report.entries {
            let disposition_key = match entry.disposition {
                Disposition::Verbatim => "verbatim",
                Disposition::Redacted => "redacted",
                Disposition::Dropped => "dropped",
            }
            .to_string();
            let rule = entry.rule.clone().unwrap_or_else(|| "<none>".to_string());
            *counts.entry((disposition_key, rule)).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .map(|((dk, rule), count)| RedactionSummary {
            disposition: match dk.as_str() {
                "verbatim" => Disposition::Verbatim,
                "redacted" => Disposition::Redacted,
                _ => Disposition::Dropped,
            },
            rule,
            count,
        })
        .collect()
}

/// Percent-encode a byte string per RFC 3986 unreserved chars
/// (`A-Z a-z 0-9 - _ . ~`). Every other byte becomes `%XX`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        let c = *b;
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~') {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

/// Convenience for the CLI: list the embedded `sanitization-report.json`
/// names that appear inside a sanitized zip. Returned as a sorted
/// [`BTreeSet`] so callers can format without re-sorting.
pub fn embedded_report_names_in(preview_dir: &Path) -> io::Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(preview_dir)? {
        let entry = entry?;
        let p = entry.path().join(SANITIZATION_REPORT_NAME);
        if p.exists() {
            out.insert(p.to_string_lossy().into_owned());
        }
    }
    Ok(out)
}

fn zip_to_io(e: zip::result::ZipError) -> io::Error {
    io::Error::other(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_env() -> SanitizedEnvironment {
        SanitizedEnvironment {
            machine: "<MACHINE>".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            roots: vec!["<USER>/proj".into()],
            kind: "T1".into(),
        }
    }

    #[test]
    fn build_issue_md_includes_sanitized_fields_and_redactions() {
        let env = fake_env();
        let r = vec![
            RedactionSummary {
                disposition: Disposition::Redacted,
                rule: "path-pseudonym".into(),
                count: 3,
            },
            RedactionSummary {
                disposition: Disposition::Dropped,
                rule: "unknown-artifact".into(),
                count: 1,
            },
        ];
        let md = build_issue_md(&env, "incremental rebuild does nothing", "rebuilds all", &r);
        assert!(md.contains("# Incremental-build diagnostic report"));
        assert!(md.contains("incremental rebuild does nothing"));
        assert!(md.contains("rebuilds all"));
        assert!(md.contains("`<MACHINE>`"));
        assert!(md.contains("`<USER>/proj`"));
        assert!(md.contains("`path-pseudonym`"));
        assert!(md.contains("| dropped | `unknown-artifact` | 1 |"));
    }

    #[test]
    fn build_issue_md_contains_no_user_profile_path() {
        // AR-23 invariant: the only path data that enters ISSUE.md is
        // the sanitized environment's roots. Constructing
        // SanitizedEnvironment from a sanitized manifest is what
        // enforces this in the real pipeline; here we sanity-check
        // the renderer doesn't leak anything from the prose inputs
        // beyond what the operator wrote.
        let env = fake_env();
        let md = build_issue_md(&env, "build is wrong", "build is broken", &[]);
        assert!(!md.contains("/home/"));
        assert!(!md.contains("C:\\Users\\"));
    }

    #[test]
    fn build_issue_url_percent_encodes_body_and_title() {
        let url = build_issue_url("https://example/issues/new", "Hello world!", "a b\nc");
        assert!(url.starts_with("https://example/issues/new?title="));
        assert!(url.contains("title=Hello%20world%21"));
        assert!(url.contains("body=a%20b%0Ac"));
    }

    #[test]
    fn percent_encode_preserves_unreserved_chars() {
        assert_eq!(percent_encode("AZaz09-_.~"), "AZaz09-_.~");
        assert_eq!(percent_encode("/?&=#%"), "%2F%3F%26%3D%23%25");
    }

    #[test]
    fn summarize_redactions_aggregates_across_reports() {
        let r1 = SanitizationReport {
            schema_version: 1,
            source_archive: "a.zip".into(),
            entries: vec![
                crate::sanitize::pipeline::SanitizationEntry {
                    path: "tree.json".into(),
                    disposition: Disposition::Redacted,
                    rule: Some("tree:roots+relpath".into()),
                    reason: None,
                },
                crate::sanitize::pipeline::SanitizationEntry {
                    path: "obj/x".into(),
                    disposition: Disposition::Dropped,
                    rule: Some("unknown-artifact".into()),
                    reason: Some("u".into()),
                },
            ],
            unknown_artifacts: vec!["obj/x".into()],
        };
        let r2 = SanitizationReport {
            schema_version: 1,
            source_archive: "b.zip".into(),
            entries: vec![crate::sanitize::pipeline::SanitizationEntry {
                path: "tree.json".into(),
                disposition: Disposition::Redacted,
                rule: Some("tree:roots+relpath".into()),
                reason: None,
            }],
            unknown_artifacts: vec![],
        };
        let summary = summarize_redactions(&[r1, r2]);
        let tree = summary
            .iter()
            .find(|s| s.rule == "tree:roots+relpath")
            .unwrap();
        assert_eq!(tree.count, 2);
        let dropped = summary
            .iter()
            .find(|s| s.rule == "unknown-artifact")
            .unwrap();
        assert_eq!(dropped.count, 1);
    }

    #[test]
    fn sanitized_environment_parses_from_manifest_json() {
        let json = r#"{
            "schema_version": 1,
            "captured_at": "1",
            "machine": "<MACHINE>",
            "os": "linux",
            "arch": "x86_64",
            "roots": ["<USER>/proj"],
            "binlog_archive_name": "b.binlog",
            "kind": "T1"
        }"#;
        let env = SanitizedEnvironment::from_sanitized_manifest_json(json).unwrap();
        assert_eq!(env.machine, "<MACHINE>");
        assert_eq!(env.os, "linux");
        assert_eq!(env.arch, "x86_64");
        assert_eq!(env.roots, vec!["<USER>/proj".to_string()]);
        assert_eq!(env.kind, "T1");
    }

    #[test]
    fn default_issue_title_uses_sanitized_environment() {
        let env = fake_env();
        let t = default_issue_title(&env);
        assert_eq!(t, "Incremental-build report (linux / x86_64)");
    }

    #[test]
    fn default_issue_base_url_matches_ar21_spec() {
        // AR-21 specifies this exact URL prefix. If the repo ever
        // moves, the spec and this constant must both be updated.
        assert_eq!(
            DEFAULT_ISSUE_BASE_URL,
            "https://github.com/MikeGrier/msbuild-diagnostic-rs/issues/new"
        );
    }
}
