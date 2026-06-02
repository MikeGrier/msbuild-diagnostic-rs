// Copyright (c) 2026 Mike Grier

//! `msbuild-diagnostic` command-line entry point.

use std::process::ExitCode;

use clap::Parser;
use msbuild_diagnostic::cli::{run, Cli};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match run(cli, &mut out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("msbuild-diagnostic: {e}");
            ExitCode::FAILURE
        }
    }
}
