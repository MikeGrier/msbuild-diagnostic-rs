//! Sanitization (D-9, D-10). See [`rules`] for the per-field classification
//! registry. Higher-level sanitize/redact entry points land in later
//! milestones; M1 ships only the rule skeleton.

pub mod rules;
