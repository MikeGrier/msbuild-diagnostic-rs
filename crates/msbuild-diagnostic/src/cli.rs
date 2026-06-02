// Copyright (c) 2026 Mike Grier

//! Command-line interface for `msbuild-diagnostic`.
//!
//! The CLI is intentionally thin: it parses arguments and delegates to
//! library entry points (D-1, D-12). Subcommands currently stub out their
//! work; behavior is filled in by later checklist items.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::archive::{write_archive, ArchiveInputs};
use crate::binlog::read_binlog;
use crate::manifest::{
    build_manifest, compose_archive_filename, CaptureEnvironment, ManifestInputs,
};
use crate::roots::{
    canonicalize_existing, discover_default_roots, find_git_root, RootDiscoveryInputs,
};
use crate::snapshot::{snapshot_roots, TimestampNs};
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

/// Dispatch a parsed CLI command. Writes human-readable output to `out`.
pub fn run<W: Write>(cli: Cli, out: &mut W) -> std::io::Result<()> {
    match cli.command {
        Command::Archive(args) => archive_run(&args, out),
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(argv: &[&str]) -> Cli {
        Cli::try_parse_from(argv).expect("parse")
    }

    #[test]
    fn archive_requires_binlog() {
        let err = Cli::try_parse_from(["msbuild-diagnostic", "archive"]).unwrap_err();
        assert!(err.to_string().contains("--binlog"));
    }

    #[test]
    fn archive_parses_minimal_args() {
        let cli = parse(&["msbuild-diagnostic", "archive", "--binlog", "build.binlog"]);
        let Command::Archive(args) = cli.command;
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
        let Command::Archive(args) = cli.command;
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
}
