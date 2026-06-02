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

/// Look up the classification for `(artifact, field)`. Deny-by-default:
/// unknown fields return [`Classification::Drop`].
pub fn classify(artifact: &str, field: &str) -> Classification {
    for rule in M1_FIELD_RULES {
        if rule.artifact == artifact && rule.field == field {
            return rule.classification;
        }
    }
    Classification::Drop
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
}
