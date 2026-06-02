//! Sanitization rule registry (D-9, D-10).
//!
//! This module is intentionally minimal at M1: a typed enumeration of the
//! data classes the sanitizer can produce, plus the per-field decisions
//! for every field introduced by Milestone 1. Later milestones add their
//! own field entries here in the same commit that introduces the data
//! (D-10).
//!
//! The sanitizer itself is **deny-by-default**: any artifact / field not
//! present in [`M1_FIELD_RULES`] (or a later-milestone equivalent) is
//! [`Classification::Drop`]. See [`classify`].

/// How the sanitizer treats a single field, per D-9.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// Field is reproduced byte-for-byte in the sanitized output.
    Verbatim,
    /// Field is reproduced after a documented transform (see [`RedactRule`]).
    Redact(RedactRule),
    /// Field is omitted entirely and recorded in `sanitization-report.json`.
    Drop,
}

/// The transforms the sanitizer knows how to apply (D-9). Each variant
/// corresponds to a single, documented rewrite that downstream
/// `sanitize` code must implement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactRule {
    /// Rewrite paths through the per-archive pseudonymization map
    /// (`<USER>`, `<MACHINE>`, `<DRIVE0>`, `<REPO>`).
    PathPseudonym,
    /// Replace machine identifier with `<MACHINE>`.
    MachineName,
    /// Replace a binlog property value with `<REDACTED-PROPERTY>`.
    /// MSBuild properties routinely carry tokens, connection strings,
    /// signing keys, and other credentials — the value is never safe to
    /// reproduce verbatim. The property *name* is retained.
    BinlogPropertyValue,
    /// Replace an environment variable's value with `<REDACTED-ENV>`.
    /// Variable name is retained for diagnostic purposes.
    EnvironmentVariable,
}

/// A single field-level decision. `artifact` is the in-archive filename
/// the field appears in (e.g. `"tree.json"`); `field` is a dot-separated
/// JSON path inside that artifact (e.g. `"roots.entries.relpath"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldRule {
    pub artifact: &'static str,
    pub field: &'static str,
    pub classification: Classification,
}

/// Per-field decisions for every field introduced by M1. New milestones
/// **append** to a parallel constant (e.g. `M2_FIELD_RULES`) in the same
/// commit that introduces the data, and the public [`classify`] lookup
/// walks all known tables.
pub const M1_FIELD_RULES: &[FieldRule] = &[
    // tree.json — top-level
    field("tree.json", "schema_version", Classification::Verbatim),
    field(
        "tree.json",
        "small_file_hash_threshold",
        Classification::Verbatim,
    ),
    // tree.json — per-root + per-entry
    field(
        "tree.json",
        "roots.root",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "tree.json",
        "roots.entries.relpath",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field("tree.json", "roots.entries.size", Classification::Verbatim),
    field(
        "tree.json",
        "roots.entries.mtime_unix_nanos",
        Classification::Verbatim,
    ),
    field(
        "tree.json",
        "roots.entries.sha256",
        Classification::Verbatim,
    ),
    field("tree.json", "roots.entries.kind", Classification::Verbatim),
    field(
        "tree.json",
        "roots.entries.target",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    // manifest.json
    field("manifest.json", "schema_version", Classification::Verbatim),
    field("manifest.json", "captured_at", Classification::Verbatim),
    field(
        "manifest.json",
        "machine",
        Classification::Redact(RedactRule::MachineName),
    ),
    field("manifest.json", "os", Classification::Verbatim),
    field("manifest.json", "arch", Classification::Verbatim),
    field(
        "manifest.json",
        "roots",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "manifest.json",
        "binlog_archive_name",
        Classification::Verbatim,
    ),
    field("manifest.json", "kind", Classification::Verbatim),
    field("manifest.json", "pair_id", Classification::Verbatim),
];

const fn field(artifact: &'static str, field: &'static str, c: Classification) -> FieldRule {
    FieldRule {
        artifact,
        field,
        classification: c,
    }
}

/// Per-field decisions for every field introduced by M2 (D-3 archive
/// shape: `imports/`, `tlogs/`, and per-event binlog records). Walked
/// alongside [`M1_FIELD_RULES`] by [`classify`].
pub const M2_FIELD_RULES: &[FieldRule] = &[
    // imports/* — paths inside the archive are always relative to
    // `imports/`, which is itself a pseudonym-rewritten location, so the
    // path is safe to reproduce verbatim. Contents may carry property
    // values harvested from MSBuild evaluation and must be treated as
    // credential-bearing.
    field("imports/*", "path", Classification::Verbatim),
    field(
        "imports/*",
        "contents",
        Classification::Redact(RedactRule::BinlogPropertyValue),
    ),
    // tlogs/* — tlog files routinely contain absolute paths and so must
    // be path-pseudonymized. Unknown extensions are filtered out at
    // collection time (see `tlogs::collect_tlogs`); the deny-by-default
    // classifier reinforces this for any field name not explicitly
    // listed below.
    field(
        "tlogs/*",
        "path",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "tlogs/*",
        "contents",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    // Binlog event records exposed via parsing. `ProjectStarted`
    // exposes property and environment dictionaries that frequently
    // carry credentials. Only the records actually consumed by this
    // crate need entries here; unknown fields drop.
    field(
        "binlog.events",
        "ProjectStarted.project_file",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "binlog.events",
        "ProjectStarted.properties.name",
        Classification::Verbatim,
    ),
    field(
        "binlog.events",
        "ProjectStarted.properties.value",
        Classification::Redact(RedactRule::BinlogPropertyValue),
    ),
    field(
        "binlog.events",
        "ProjectStarted.environment.name",
        Classification::Verbatim,
    ),
    field(
        "binlog.events",
        "ProjectStarted.environment.value",
        Classification::Redact(RedactRule::EnvironmentVariable),
    ),
];

/// Per-field decisions for every field introduced by M3 (diff +
/// correlation reports). Walked alongside [`M1_FIELD_RULES`] and
/// [`M2_FIELD_RULES`] by [`classify`].
pub const M3_FIELD_RULES: &[FieldRule] = &[
    // diff-report.json — describes a tree-snapshot pair. Path-bearing
    // fields use the same pseudonym rule as `tree.json`; per-entry
    // value fields (size / mtime / sha256) are inherently safe.
    field(
        "diff-report.json",
        "schema_version",
        Classification::Verbatim,
    ),
    field(
        "diff-report.json",
        "roots.root",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "diff-report.json",
        "roots.added.relpath",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "diff-report.json",
        "roots.removed.relpath",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "diff-report.json",
        "roots.changed.relpath",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "diff-report.json",
        "roots.unchanged.relpath",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    // correlation-report.md — quotes paths and binlog message text
    // directly. Paths use the pseudonym map; message text is treated
    // with the same property-value redaction as M2's
    // ProjectStarted.properties.value, because the message body may
    // include property substitutions captured from the build.
    field(
        "correlation-report.md",
        "finding.input_path",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "correlation-report.md",
        "finding.output_path",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "correlation-report.md",
        "finding.project_file",
        Classification::Redact(RedactRule::PathPseudonym),
    ),
    field(
        "correlation-report.md",
        "finding.target_name",
        Classification::Verbatim,
    ),
    field(
        "correlation-report.md",
        "finding.reason_message",
        Classification::Redact(RedactRule::BinlogPropertyValue),
    ),
];

/// Look up the classification for `(artifact, field)`. Deny-by-default:
/// unknown fields return [`Classification::Drop`].
pub fn classify(artifact: &str, field: &str) -> Classification {
    for rule in M1_FIELD_RULES
        .iter()
        .chain(M2_FIELD_RULES.iter())
        .chain(M3_FIELD_RULES.iter())
    {
        if rule.artifact == artifact && rule.field == field {
            return rule.classification;
        }
    }
    Classification::Drop
}

/// Returns true iff a tlog-like file with `extension` (lowercased,
/// no leading dot) is in scope for capture under `obj/`. Anything else
/// must be dropped — see AR-12. The collector (`tlogs::collect_tlogs`)
/// is the primary enforcement point; this predicate is the rule that
/// downstream sanitizer verification can cite.
pub fn obj_extension_is_in_scope(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("tlog")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_field_drops_by_default() {
        assert_eq!(
            classify("tree.json", "totally_made_up_field"),
            Classification::Drop
        );
        assert_eq!(
            classify("unknown_artifact.json", "schema_version"),
            Classification::Drop
        );
    }

    #[test]
    fn known_verbatim_fields_classify_verbatim() {
        assert_eq!(
            classify("tree.json", "schema_version"),
            Classification::Verbatim
        );
        assert_eq!(classify("manifest.json", "os"), Classification::Verbatim);
        assert_eq!(
            classify("tree.json", "roots.entries.sha256"),
            Classification::Verbatim
        );
    }

    #[test]
    fn paths_redact_via_pseudonym_map() {
        assert_eq!(
            classify("tree.json", "roots.entries.relpath"),
            Classification::Redact(RedactRule::PathPseudonym)
        );
        assert_eq!(
            classify("manifest.json", "roots"),
            Classification::Redact(RedactRule::PathPseudonym)
        );
    }

    #[test]
    fn machine_name_redacts() {
        assert_eq!(
            classify("manifest.json", "machine"),
            Classification::Redact(RedactRule::MachineName)
        );
    }

    #[test]
    fn every_m1_artifact_field_pair_is_unique() {
        for (i, a) in M1_FIELD_RULES.iter().enumerate() {
            for b in &M1_FIELD_RULES[i + 1..] {
                assert!(
                    !(a.artifact == b.artifact && a.field == b.field),
                    "duplicate rule for {}.{}",
                    a.artifact,
                    a.field
                );
            }
        }
    }

    #[test]
    fn every_rule_pair_is_unique_across_all_milestones() {
        let all: Vec<&FieldRule> = M1_FIELD_RULES
            .iter()
            .chain(M2_FIELD_RULES.iter())
            .chain(M3_FIELD_RULES.iter())
            .collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(
                    !(a.artifact == b.artifact && a.field == b.field),
                    "duplicate rule for {}.{}",
                    a.artifact,
                    a.field
                );
            }
        }
    }

    #[test]
    fn m2_property_values_redact_with_binlog_property_value_rule() {
        assert_eq!(
            classify("binlog.events", "ProjectStarted.properties.value"),
            Classification::Redact(RedactRule::BinlogPropertyValue)
        );
        // Property *names* stay verbatim.
        assert_eq!(
            classify("binlog.events", "ProjectStarted.properties.name"),
            Classification::Verbatim
        );
    }

    #[test]
    fn m2_environment_values_redact_with_environment_variable_rule() {
        assert_eq!(
            classify("binlog.events", "ProjectStarted.environment.value"),
            Classification::Redact(RedactRule::EnvironmentVariable)
        );
    }

    #[test]
    fn m2_imports_contents_redact_property_values() {
        assert_eq!(
            classify("imports/*", "contents"),
            Classification::Redact(RedactRule::BinlogPropertyValue)
        );
    }

    #[test]
    fn m2_tlogs_paths_are_pseudonymized() {
        assert_eq!(
            classify("tlogs/*", "path"),
            Classification::Redact(RedactRule::PathPseudonym)
        );
        assert_eq!(
            classify("tlogs/*", "contents"),
            Classification::Redact(RedactRule::PathPseudonym)
        );
    }

    #[test]
    fn only_tlog_extension_is_in_scope_under_obj() {
        assert!(obj_extension_is_in_scope("tlog"));
        assert!(obj_extension_is_in_scope("TLOG"));
        assert!(!obj_extension_is_in_scope("txt"));
        assert!(!obj_extension_is_in_scope("dll"));
        assert!(!obj_extension_is_in_scope("pdb"));
        assert!(!obj_extension_is_in_scope(""));
    }

    #[test]
    fn fake_credential_in_binlog_property_is_classified_for_redaction() {
        // Fixture: a binlog ProjectStarted property whose value contains
        // a clearly fake credential string. The sanitizer rule for that
        // field must be Redact(BinlogPropertyValue); the actual
        // substring is never inspected by classify (it operates on
        // (artifact, field) only) — the test asserts that the *policy*
        // is in place for this code path.
        const FAKE_CRED: &str = "AKIA-FAKE-CREDENTIAL-1234567890";
        let property_value_in_binlog = format!("Token={FAKE_CRED}");

        let cls = classify("binlog.events", "ProjectStarted.properties.value");
        assert_eq!(
            cls,
            Classification::Redact(RedactRule::BinlogPropertyValue),
            "binlog property values must be redacted; fixture value was {property_value_in_binlog:?}"
        );

        // Unknown property-bearing artifact must Drop, not Verbatim.
        assert_eq!(
            classify("binlog.events", "TaskStarted.properties.value"),
            Classification::Drop
        );
    }

    #[test]
    fn m3_correlation_report_paths_are_pseudonymized() {
        assert_eq!(
            classify("correlation-report.md", "finding.input_path"),
            Classification::Redact(RedactRule::PathPseudonym)
        );
        assert_eq!(
            classify("correlation-report.md", "finding.output_path"),
            Classification::Redact(RedactRule::PathPseudonym)
        );
        assert_eq!(
            classify("correlation-report.md", "finding.project_file"),
            Classification::Redact(RedactRule::PathPseudonym)
        );
    }

    #[test]
    fn m3_correlation_report_message_redacts_with_binlog_property_value_rule() {
        assert_eq!(
            classify("correlation-report.md", "finding.reason_message"),
            Classification::Redact(RedactRule::BinlogPropertyValue)
        );
        assert_eq!(
            classify("correlation-report.md", "finding.target_name"),
            Classification::Verbatim
        );
    }

    #[test]
    fn m3_diff_report_per_entry_paths_are_pseudonymized() {
        for sect in ["added", "removed", "changed", "unchanged"] {
            let field = format!("roots.{sect}.relpath");
            assert_eq!(
                classify("diff-report.json", &field),
                Classification::Redact(RedactRule::PathPseudonym),
                "diff-report.json {field} must be pseudonymized"
            );
        }
        assert_eq!(
            classify("diff-report.json", "roots.root"),
            Classification::Redact(RedactRule::PathPseudonym)
        );
    }
}
