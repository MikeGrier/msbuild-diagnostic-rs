// Copyright (c) 2026 Mike Grier

//! Command-line interface for `msbuild-diagnostic`.
//!
//! The CLI is intentionally thin: it parses arguments and delegates to
//! library entry points (D-1, D-12). Subcommands currently stub out their
//! work; behavior is filled in by later checklist items.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::archive::{write_archive, ArchiveInputs, TREE_JSON_NAME};
use crate::binlog::read_binlog;
use crate::correlate::{build_binlog_model, correlate, write_markdown_report};
use crate::diff::diff_snapshots;
use crate::manifest::{
    build_manifest, compose_archive_filename, CaptureEnvironment, ManifestInputs,
};
use crate::roots::{
    canonicalize_existing, discover_default_roots, find_git_root, RootDiscoveryInputs,
};
use crate::snapshot::{snapshot_roots, TimestampNs, TreeSnapshot};
use crate::tlogs::collect_tlogs;

/// Default SHA-256 size threshold for `tree.json` entries, in bytes (D-4).
pub const DEFAULT_SMALL_FILE_HASH_THRESHOLD: u64 = 1_048_576;

/// Default label for an archive's `kind` field when not explicitly set.
pub const DEFAULT_ARCHIVE_KIND: &str = "T1";

#[derive(Debug, Parser)]
#[command(name = "msbuild-diagnostic", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Capture a snapshot archive for incremental-build diagnosis.
    Archive(ArchiveArgs),
    /// Diff two snapshot archives' `tree.json` payloads (AR-13).
    Diff(DiffArgs),
    /// Sanitize a capture archive into a shareable counterpart (AR-18).
    Sanitize(SanitizeArgs),
    /// Package one or two capture archives into a submission with a
    /// prefilled GitHub issue URL (AR-20 / AR-21).
    Report(ReportArgs),
}

#[derive(Debug, clap::Args)]
pub struct ArchiveArgs {
    /// Path to the `.binlog` to archive alongside the captured tree state.
    #[arg(long)]
    pub binlog: PathBuf,

    /// Roots whose file trees should be enumerated into `tree.json`.
    ///
    /// Repeatable. When empty, default roots are auto-discovered (AR-8).
    #[arg(long = "root")]
    pub roots: Vec<PathBuf>,

    /// Label distinguishing this capture from its pair (e.g. `T1`, `T2`).
    #[arg(long, default_value = DEFAULT_ARCHIVE_KIND)]
    pub kind: String,

    /// Optional identifier linking a T1 / T2 pair together.
    #[arg(long = "pair-id")]
    pub pair_id: Option<String>,

    /// Directory to write the resulting archive into.
    #[arg(long, default_value = ".")]
    pub out: PathBuf,

    /// Maximum size in bytes for which SHA-256 is captured in `tree.json`.
    #[arg(long, default_value_t = DEFAULT_SMALL_FILE_HASH_THRESHOLD)]
    pub small_file_hash_threshold: u64,
}

/// Default name for the JSON diff report written by [`diff_run`].
pub const DEFAULT_DIFF_REPORT_NAME: &str = "diff-report.json";

/// Default name for the Markdown correlation report written by
/// [`diff_run`] when `--binlog` is supplied without an explicit
/// `--markdown` path.
pub const DEFAULT_CORRELATION_REPORT_NAME: &str = "correlation-report.md";

#[derive(Debug, clap::Args)]
pub struct DiffArgs {
    /// First ("T1") snapshot archive.
    pub t1: PathBuf,
    /// Second ("T2") snapshot archive.
    pub t2: PathBuf,
    /// Path to write the JSON diff report to. Defaults to
    /// `diff-report.json` in the current directory.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Path to the T2 binlog. When supplied, the AR-15 correlator
    /// runs and writes a Markdown report alongside the JSON diff.
    #[arg(long)]
    pub binlog: Option<PathBuf>,
    /// Path to write the Markdown correlation report to. Implies
    /// `--binlog`; defaults to `correlation-report.md` next to the
    /// JSON diff when `--binlog` is supplied without it.
    #[arg(long)]
    pub markdown: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct SanitizeArgs {
    /// Capture archive to sanitize.
    pub input: PathBuf,
    /// Output path for the sanitized zip. Defaults to
    /// `<input-stem>-sanitized.zip` next to the input.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Output path for the local-only pseudonym map JSON. Defaults to
    /// `<input-stem>-pseudonym-map.local.json` next to the input.
    /// **Never** written inside the sanitized zip.
    #[arg(long = "map")]
    pub map: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct ReportArgs {
    /// Single capture archive to report on. Mutually exclusive with
    /// `--pair`.
    pub input: Option<PathBuf>,
    /// Two capture archives (T1 then T2) to report on as a pair.
    /// Mutually exclusive with the positional `input`.
    #[arg(long = "pair", num_args = 2, value_names = ["T1", "T2"])]
    pub pair: Option<Vec<PathBuf>>,
    /// Operator's expected behavior, free text.
    #[arg(long)]
    pub expected: String,
    /// Operator's actual observed behavior, free text.
    #[arg(long)]
    pub actual: String,
    /// Directory to write `submission.zip`, `submission-preview/`,
    /// and `ISSUE.md` into. Created if absent.
    #[arg(long)]
    pub out: PathBuf,
}

/// Dispatch a parsed CLI command. Writes human-readable output to `out`.
pub fn run<W: Write>(cli: Cli, out: &mut W) -> std::io::Result<()> {
    match cli.command {
        Command::Archive(args) => archive_run(&args, out),
        Command::Diff(args) => diff_run(&args, out),
        Command::Sanitize(args) => sanitize_run(&args, out),
        Command::Report(args) => report_run(&args, out),
    }
}

fn archive_run<W: Write>(args: &ArchiveArgs, out: &mut W) -> std::io::Result<()> {
    let binlog_name = args
        .binlog
        .file_name()
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "binlog path has no filename",
            )
        })?
        .to_string_lossy()
        .into_owned();
    let binlog_stem = args
        .binlog
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| binlog_name.clone());

    // Parse the binlog up front: we need the inventory for default-root
    // discovery (D-5) regardless of whether the user passed --root.
    let (inventory, imports) = read_binlog(&args.binlog)?;

    let roots: Vec<PathBuf> = if args.roots.is_empty() {
        let binlog_abs = canonicalize_existing(&args.binlog);
        let git_root = binlog_abs.parent().and_then(find_git_root);
        let discovered = discover_default_roots(RootDiscoveryInputs {
            binlog_path: &binlog_abs,
            inventory: &inventory,
            git_root: git_root.as_deref(),
        });
        if discovered.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "could not auto-discover any roots; pass --root <path>",
            ));
        }
        discovered
    } else {
        args.roots.clone()
    };

    let tree = snapshot_roots(&roots, args.small_file_hash_threshold)?;
    let tlogs = collect_tlogs(&inventory)?;

    let captured_at = TimestampNs::from_system_time(std::time::SystemTime::now());
    let env = CaptureEnvironment::from_process();
    let manifest = build_manifest(ManifestInputs {
        captured_at,
        env: &env,
        roots: &roots,
        binlog_archive_name: &binlog_name,
        kind: &args.kind,
        pair_id: args.pair_id.as_deref(),
    });

    let archive_filename = compose_archive_filename(&binlog_stem, captured_at, &args.kind);
    let archive_path = args.out.join(&archive_filename);

    std::fs::create_dir_all(&args.out)?;
    let binlog_file = std::fs::File::open(&args.binlog)?;
    let archive_file = std::fs::File::create(&archive_path)?;
    write_archive(
        &ArchiveInputs {
            binlog_name: &binlog_name,
            tree: &tree,
            manifest: &manifest,
            imports: &imports,
            tlogs: &tlogs,
        },
        binlog_file,
        archive_file,
    )?;

    writeln!(out, "wrote {}", archive_path.display())?;
    Ok(())
}

/// Read the `tree.json` entry from a snapshot zip and deserialize it.
fn read_tree_from_archive(path: &std::path::Path) -> std::io::Result<TreeSnapshot> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{}: not a valid zip: {e}", path.display()),
        )
    })?;
    let mut entry = zip.by_name(TREE_JSON_NAME).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{}: missing {TREE_JSON_NAME}: {e}", path.display()),
        )
    })?;
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut entry, &mut buf)?;
    serde_json::from_str::<TreeSnapshot>(&buf).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{}: invalid {TREE_JSON_NAME}: {e}", path.display()),
        )
    })
}

fn diff_run<W: Write>(args: &DiffArgs, out: &mut W) -> std::io::Result<()> {
    let t1 = read_tree_from_archive(&args.t1)?;
    let t2 = read_tree_from_archive(&args.t2)?;
    let report = diff_snapshots(&t1, &t2);

    let out_path = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DIFF_REPORT_NAME));
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let f = std::fs::File::create(&out_path)?;
    serde_json::to_writer_pretty(f, &report)?;

    let (added, removed, changed, unchanged) =
        report
            .roots
            .iter()
            .fold((0usize, 0usize, 0usize, 0usize), |(a, r, c, u), root| {
                (
                    a + root.added.len(),
                    r + root.removed.len(),
                    c + root.changed.len(),
                    u + root.unchanged.len(),
                )
            });
    writeln!(
        out,
        "wrote {} (added={added}, removed={removed}, changed={changed}, unchanged={unchanged})",
        out_path.display()
    )?;

    let binlog_path = args.binlog.as_ref().or_else(|| {
        // `--markdown` implies `--binlog`; the binlog path is required
        // either way when correlation is requested.
        args.markdown.as_ref().and(args.binlog.as_ref())
    });
    if args.markdown.is_some() && args.binlog.is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "--markdown requires --binlog",
        ));
    }
    if let Some(binlog) = binlog_path {
        let events = crate::binlog::read_binlog_events(binlog)?;
        let model = build_binlog_model(&events);
        let report = correlate(&model, &t1, &t2);
        let md_path = args.markdown.clone().unwrap_or_else(|| {
            out_path
                .parent()
                .map(|p| p.join(DEFAULT_CORRELATION_REPORT_NAME))
                .unwrap_or_else(|| PathBuf::from(DEFAULT_CORRELATION_REPORT_NAME))
        });
        if let Some(parent) = md_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut md = std::fs::File::create(&md_path)?;
        let pseudonymizer = crate::sanitize::pseudonym::Pseudonymizer::from_environment();
        write_markdown_report(&report, &mut md, &pseudonymizer)?;
        writeln!(
            out,
            "wrote {} (findings={})",
            md_path.display(),
            report.findings.len()
        )?;
    }

    Ok(())
}

fn sanitize_run<W: Write>(args: &SanitizeArgs, out: &mut W) -> std::io::Result<()> {
    let (default_zip, default_map) = crate::sanitize::pipeline::default_output_paths(&args.input);
    let output = args.out.clone().unwrap_or(default_zip);
    let map = args.map.clone().unwrap_or(default_map);
    let pseudonymizer = crate::sanitize::pseudonym::Pseudonymizer::from_environment();
    let report =
        crate::sanitize::pipeline::sanitize_archive(&crate::sanitize::pipeline::SanitizeInputs {
            input: &args.input,
            output: &output,
            map: &map,
            pseudonymizer: &pseudonymizer,
        })?;
    let unknown = report.unknown_artifacts.len();
    writeln!(
        out,
        "wrote {} (entries={}, unknown_artifacts={})",
        output.display(),
        report.entries.len(),
        unknown
    )?;
    writeln!(out, "wrote {} (pseudonym map; KEEP LOCAL)", map.display())?;
    Ok(())
}

fn report_run<W: Write>(args: &ReportArgs, out: &mut W) -> std::io::Result<()> {
    let inputs: Vec<PathBuf> = match (&args.input, &args.pair) {
        (Some(_), Some(_)) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "--pair and positional input are mutually exclusive",
            ));
        }
        (None, None) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "report requires an input archive or --pair T1 T2",
            ));
        }
        (Some(p), None) => vec![p.clone()],
        (None, Some(pair)) => pair.clone(),
    };
    let pseudonymizer = crate::sanitize::pseudonym::Pseudonymizer::from_environment();
    let artifacts = crate::report::generate_report(&crate::report::ReportInputs {
        inputs: &inputs,
        expected: &args.expected,
        actual: &args.actual,
        out_dir: &args.out,
        pseudonymizer: &pseudonymizer,
        issue_base_url: crate::report::DEFAULT_ISSUE_BASE_URL,
    })?;
    writeln!(out, "wrote {}", artifacts.submission_zip.display())?;
    writeln!(
        out,
        "extracted preview at {}",
        artifacts.preview_dir.display()
    )?;
    writeln!(out, "wrote {}", artifacts.issue_md.display())?;
    writeln!(out)?;
    writeln!(
        out,
        "Open the prefilled issue URL in your browser and ATTACH submission.zip manually:"
    )?;
    writeln!(out, "{}", artifacts.issue_url)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(argv: &[&str]) -> Cli {
        Cli::try_parse_from(argv).expect("parse")
    }

    fn archive_args(cli: Cli) -> ArchiveArgs {
        match cli.command {
            Command::Archive(a) => a,
            other => panic!("expected Archive, got {other:?}"),
        }
    }

    #[test]
    fn archive_requires_binlog() {
        let err = Cli::try_parse_from(["msbuild-diagnostic", "archive"]).unwrap_err();
        assert!(err.to_string().contains("--binlog"));
    }

    #[test]
    fn archive_parses_minimal_args() {
        let cli = parse(&["msbuild-diagnostic", "archive", "--binlog", "build.binlog"]);
        let args = archive_args(cli);
        assert_eq!(args.binlog, PathBuf::from("build.binlog"));
        assert_eq!(args.kind, DEFAULT_ARCHIVE_KIND);
        assert_eq!(args.out, PathBuf::from("."));
        assert_eq!(
            args.small_file_hash_threshold,
            DEFAULT_SMALL_FILE_HASH_THRESHOLD
        );
        assert!(args.roots.is_empty());
        assert!(args.pair_id.is_none());
    }

    #[test]
    fn archive_collects_repeated_roots() {
        let cli = parse(&[
            "msbuild-diagnostic",
            "archive",
            "--binlog",
            "b.binlog",
            "--root",
            "src",
            "--root",
            "tests",
            "--kind",
            "T2",
            "--pair-id",
            "abc",
            "--out",
            "out-dir",
            "--small-file-hash-threshold",
            "4096",
        ]);
        let args = archive_args(cli);
        assert_eq!(
            args.roots,
            vec![PathBuf::from("src"), PathBuf::from("tests")]
        );
        assert_eq!(args.kind, "T2");
        assert_eq!(args.pair_id.as_deref(), Some("abc"));
        assert_eq!(args.out, PathBuf::from("out-dir"));
        assert_eq!(args.small_file_hash_threshold, 4096);
    }

    #[test]
    fn run_archive_requires_a_real_binlog() {
        // With AR-8 default-root discovery in place, the CLI no longer
        // rejects empty `--root` up front — it tries to parse the binlog.
        // A non-existent binlog path must surface as an io error.
        let cli = parse(&[
            "msbuild-diagnostic",
            "archive",
            "--binlog",
            "does-not-exist.binlog",
        ]);
        let mut buf = Vec::new();
        let err = run(cli, &mut buf).expect_err("missing binlog must error");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn diff_requires_two_positional_archives() {
        let err = Cli::try_parse_from(["msbuild-diagnostic", "diff", "only-one.zip"]).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("T2") || msg.contains("t2") || msg.contains("required"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn diff_parses_with_optional_out() {
        let cli = parse(&[
            "msbuild-diagnostic",
            "diff",
            "t1.zip",
            "t2.zip",
            "--out",
            "report.json",
        ]);
        match cli.command {
            Command::Diff(a) => {
                assert_eq!(a.t1, PathBuf::from("t1.zip"));
                assert_eq!(a.t2, PathBuf::from("t2.zip"));
                assert_eq!(a.out.as_deref(), Some(std::path::Path::new("report.json")));
            }
            other => panic!("expected Diff, got {other:?}"),
        }
    }

    #[test]
    fn run_diff_errors_when_archive_missing() {
        let cli = parse(&[
            "msbuild-diagnostic",
            "diff",
            "does-not-exist-t1.zip",
            "does-not-exist-t2.zip",
        ]);
        let mut buf = Vec::new();
        let err = run(cli, &mut buf).expect_err("missing archive must error");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
