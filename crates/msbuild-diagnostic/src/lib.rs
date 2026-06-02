// Copyright (c) 2026 Mike Grier

//! Tools to help work with msbuild based project builds to diagnose and correct bad behaviors
//!
//! This is the core library crate for `msbuild-diagnostic-rs`.

pub mod archive;
pub mod binlog;
pub mod cli;
pub mod correlate;
pub mod diff;
pub mod manifest;
pub mod report;
pub mod roots;
pub mod sanitize;
pub mod snapshot;
pub mod targets;
pub mod tlogs;

/// Returns a hello-world greeting from this crate.
pub fn hello() -> String {
    format!("Hello from {}!", "msbuild-diagnostic")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_greets_this_crate() {
        assert!(hello().contains("msbuild-diagnostic"));
    }
}
