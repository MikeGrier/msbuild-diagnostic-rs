// Copyright (c) 2026 Mike Grier

//! Command-line interface for `msbuild-diagnostic`.
//!
//! The CLI is intentionally thin: it parses arguments and delegates to
//! library entry points (D-1, D-12). Subcommands currently stub out their
//! work; behavior is filled in by later checklist items.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
        Command::Archive(args) => archive_stub(&args, out),
    }
}

fn archive_stub<W: Write>(args: &ArchiveArgs, out: &mut W) -> std::io::Result<()> {
    writeln!(out, "archive (stub)")?;
    writeln!(out, "  binlog: {}", args.binlog.display())?;
    writeln!(out, "  kind: {}", args.kind)?;
    if let Some(id) = &args.pair_id {
        writeln!(out, "  pair-id: {id}")?;
    }
    writeln!(out, "  out: {}", args.out.display())?;
    writeln!(
        out,
        "  small-file-hash-threshold: {}",
        args.small_file_hash_threshold
    )?;
    if args.roots.is_empty() {
        writeln!(out, "  roots: (auto-discover)")?;
    } else {
        writeln!(out, "  roots:")?;
        for r in &args.roots {
            writeln!(out, "    - {}", r.display())?;
        }
    }
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
    fn run_archive_writes_parsed_summary() {
        let cli = parse(&["msbuild-diagnostic", "archive", "--binlog", "b.binlog"]);
        let mut buf = Vec::new();
        run(cli, &mut buf).expect("run");
        let s = String::from_utf8(buf).expect("utf8");
        assert!(s.contains("archive (stub)"));
        assert!(s.contains("b.binlog"));
        assert!(s.contains("(auto-discover)"));
    }
}
