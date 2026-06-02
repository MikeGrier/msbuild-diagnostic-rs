// Copyright (c) 2026 Mike Grier

//! Target-run extraction from the binlog (AR-14).
//!
//! Per D-7 our specification is: a *target run* is a [`TargetStartedEvent`]
//! emitted by MSBuild that is **not** preceded — anywhere earlier in the
//! event stream — by a [`TargetSkippedEvent`] with the same
//! `(project_file, target_name)`. For each retained target run we also
//! surface the immediately preceding `BuildMessage` whose text reads
//! `Building target "X" ...`, where `X` matches the run's target name.
//! That message carries MSBuild's human-readable reason ("completely as
//! output file ... does not exist", "partially as input ... is newer
//! than output ...", etc.) which AR-15 cross-references against the
//! captured [`TreeSnapshot`].
//!
//! This module is pure per D-12: it operates on a slice of
//! [`BinlogEvent`] values (produced by `binlog::read_binlog`) and
//! produces typed [`TargetRun`] values. The CLI / report shells consume
//! those values; this module never touches the filesystem.

use std::collections::{HashMap, HashSet};

use munin_msbuild::BinlogEvent;
use serde::{Deserialize, Serialize};

/// One target that actually ran (was not skipped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetRun {
    pub target_name: Option<String>,
    pub project_file: Option<String>,
    pub parent_target: Option<String>,
    /// MSBuild `TargetBuiltReason` raw integer (see binlog format docs).
    /// `0` is `None`; non-zero values map to specific reasons.
    pub build_reason: i32,
    /// Text of the `BuildMessage` that immediately preceded the
    /// `TargetStarted` and referenced this target by name, when one was
    /// found. Typically begins with `Building target "X"`.
    pub reason_message: Option<String>,
}

/// Extract every target run (TargetStarted not preceded by a matching
/// TargetSkipped) from `events`, in stream order.
pub fn extract_target_runs(events: &[BinlogEvent]) -> Vec<TargetRun> {
    let mut skipped: HashSet<(String, String)> = HashSet::new();
    // Most-recent pending "Building target X" message per target name.
    let mut pending_reason: HashMap<String, String> = HashMap::new();
    let mut out = Vec::new();

    for event in events {
        match event {
            BinlogEvent::Message(m) => {
                if let Some(text) = &m.fields.message {
                    if let Some(name) = extract_building_target_name(text) {
                        pending_reason.insert(name, text.clone());
                    }
                }
            }
            BinlogEvent::TargetSkipped(s) => {
                if let (Some(p), Some(n)) = (&s.fields.project_file, &s.target_name) {
                    skipped.insert((p.clone(), n.clone()));
                }
            }
            BinlogEvent::TargetStarted(s) => {
                let project = s
                    .project_file
                    .clone()
                    .or_else(|| s.fields.project_file.clone());
                let key = match (&project, &s.target_name) {
                    (Some(p), Some(n)) => Some((p.clone(), n.clone())),
                    _ => None,
                };
                if let Some(k) = &key {
                    if skipped.contains(k) {
                        continue;
                    }
                }
                let reason_message = s
                    .target_name
                    .as_ref()
                    .and_then(|n| pending_reason.remove(n));
                out.push(TargetRun {
                    target_name: s.target_name.clone(),
                    project_file: project,
                    parent_target: s.parent_target.clone(),
                    build_reason: s.build_reason,
                    reason_message,
                });
            }
            _ => {}
        }
    }

    out
}

/// Parse `Building target "X" ...` out of a BuildMessage's text. Returns
/// the value of `X` when the prefix is found and the trailing quote is
/// present. Returns `None` otherwise. This matches the verbose-logger
/// format MSBuild emits for the "why did target X run" message.
pub fn extract_building_target_name(text: &str) -> Option<String> {
    const PREFIX: &str = "Building target \"";
    let start = text.find(PREFIX)? + PREFIX.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    //! Hermetic unit tests (D-14). All inputs are in-memory
    //! [`BinlogEvent`] values; no binlog parsing is performed here.
    use super::*;
    use munin_msbuild::events::{BuildMessageEvent, TargetSkippedEvent, TargetStartedEvent};

    fn started(project: &str, target: &str) -> BinlogEvent {
        BinlogEvent::TargetStarted(TargetStartedEvent {
            target_name: Some(target.into()),
            project_file: Some(project.into()),
            ..TargetStartedEvent::default()
        })
    }

    fn skipped(project: &str, target: &str) -> BinlogEvent {
        let mut s = TargetSkippedEvent {
            target_name: Some(target.into()),
            ..TargetSkippedEvent::default()
        };
        s.fields.project_file = Some(project.into());
        BinlogEvent::TargetSkipped(s)
    }

    fn message(text: &str) -> BinlogEvent {
        let mut m = BuildMessageEvent::default();
        m.fields.message = Some(text.into());
        BinlogEvent::Message(m)
    }

    #[test]
    fn extract_building_target_name_pulls_quoted_name() {
        assert_eq!(
            extract_building_target_name(
                r#"Building target "CoreCompile" completely as output file does not exist."#
            ),
            Some("CoreCompile".to_string())
        );
    }

    #[test]
    fn extract_building_target_name_returns_none_for_unrelated_text() {
        assert_eq!(extract_building_target_name("Done building project."), None);
        // Prefix present but no closing quote.
        assert_eq!(
            extract_building_target_name("Building target \"unterminated"),
            None
        );
    }

    #[test]
    fn target_started_without_preceding_skip_is_kept() {
        let events = vec![started("P.csproj", "Build")];
        let runs = extract_target_runs(&events);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].target_name.as_deref(), Some("Build"));
        assert_eq!(runs[0].project_file.as_deref(), Some("P.csproj"));
        assert!(runs[0].reason_message.is_none());
    }

    #[test]
    fn target_started_preceded_by_skip_is_dropped() {
        let events = vec![
            skipped("P.csproj", "Compile"),
            started("P.csproj", "Compile"),
        ];
        let runs = extract_target_runs(&events);
        assert!(runs.is_empty(), "got {runs:?}");
    }

    #[test]
    fn skip_in_different_project_does_not_drop_started() {
        let events = vec![
            skipped("Other.csproj", "Compile"),
            started("P.csproj", "Compile"),
        ];
        let runs = extract_target_runs(&events);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].project_file.as_deref(), Some("P.csproj"));
    }

    #[test]
    fn skip_in_different_target_does_not_drop_started() {
        let events = vec![skipped("P.csproj", "Other"), started("P.csproj", "Compile")];
        let runs = extract_target_runs(&events);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn reason_message_is_attached_when_it_precedes_target_started() {
        let events = vec![
            message(r#"Building target "Compile" completely as output file is missing."#),
            started("P.csproj", "Compile"),
        ];
        let runs = extract_target_runs(&events);
        assert_eq!(runs.len(), 1);
        assert!(runs[0]
            .reason_message
            .as_ref()
            .map(|t| t.contains("Compile"))
            .unwrap_or(false));
    }

    #[test]
    fn reason_message_is_consumed_and_not_reused_across_two_starts() {
        let events = vec![
            message(r#"Building target "Compile" because foo."#),
            started("P.csproj", "Compile"),
            started("P.csproj", "Compile"),
        ];
        let runs = extract_target_runs(&events);
        assert_eq!(runs.len(), 2);
        assert!(runs[0].reason_message.is_some());
        assert!(
            runs[1].reason_message.is_none(),
            "second start should not inherit consumed reason: {runs:?}"
        );
    }

    #[test]
    fn unrelated_messages_do_not_attach() {
        let events = vec![
            message("Some unrelated diagnostic."),
            started("P.csproj", "Compile"),
        ];
        let runs = extract_target_runs(&events);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].reason_message.is_none());
    }

    #[test]
    fn build_reason_is_propagated() {
        let mut ts = TargetStartedEvent {
            target_name: Some("Compile".into()),
            project_file: Some("P.csproj".into()),
            ..TargetStartedEvent::default()
        };
        ts.build_reason = 3;
        let events = vec![BinlogEvent::TargetStarted(ts)];
        let runs = extract_target_runs(&events);
        assert_eq!(runs[0].build_reason, 3);
    }

    #[test]
    fn target_run_round_trips_through_json() {
        let run = TargetRun {
            target_name: Some("Compile".into()),
            project_file: Some("P.csproj".into()),
            parent_target: Some("Build".into()),
            build_reason: 2,
            reason_message: Some("Building target \"Compile\" ...".into()),
        };
        let s = serde_json::to_string(&run).unwrap();
        let back: TargetRun = serde_json::from_str(&s).unwrap();
        assert_eq!(back, run);
    }
}
