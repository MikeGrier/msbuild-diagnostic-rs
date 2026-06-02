// Copyright (c) 2026 Mike Grier

//! Archive sanitizer (AR-18 / D-9).
//!
//! Reads a capture archive produced by the `archive` subcommand and
//! emits a sanitized counterpart suitable for sharing in a public
//! GitHub issue, plus a sibling **local-only** pseudonym map that
//! records the original values that were rewritten. The map file is
//! never included in the sanitized zip (AR-18 explicit constraint).
//!
//! The transforms are driven by the rule registry in [`super::rules`]
//! (D-9, D-10). Artifacts whose classification is unknown are dropped
//! and recorded in the sanitization report (AR-19); see
//! [`classify_artifact_kind`] for the artifact-name → registry-key
//! mapping.

use std::collections::BTreeMap;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::archive::{IMPORTS_DIR_NAME, TLOGS_DIR_NAME, TREE_JSON_NAME};
use crate::manifest::MANIFEST_NAME;
use crate::sanitize::pseudonym::Pseudonymizer;
use crate::sanitize::rules::{classify, Classification, RedactRule};

/// Schema version for `<stem>-pseudonym-map.local.json`.
pub const PSEUDONYM_MAP_SCHEMA_VERSION: u32 = 1;

/// Schema version for the `sanitization-report.json` artifact added
/// to every sanitized archive (AR-19).
pub const SANITIZATION_REPORT_SCHEMA_VERSION: u32 = 1;

/// Canonical artifact name for the sanitization report inside the
/// sanitized zip (AR-19).
pub const SANITIZATION_REPORT_NAME: &str = "sanitization-report.json";

/// Placeholder substituted for any binlog property value the sanitizer
/// can identify (D-9 `BinlogPropertyValue`).
pub const REDACTED_PROPERTY_PLACEHOLDER: &str = "<REDACTED-PROPERTY>";

/// On-disk shape of `<stem>-pseudonym-map.local.json`. The file lives
/// **outside** the sanitized zip and carries the original values that
/// were rewritten so the operator can correlate sanitized output back
/// to local sources during triage. Treat the file as sensitive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PseudonymMap {
    pub schema_version: u32,
    /// User-profile prefix substituted as `<USER>`. `None` when no
    /// user-profile prefix was discovered in the input.
    pub user_profile: Option<String>,
    /// Machine name substituted as `<MACHINE>`. `None` when the
    /// archive did not record a machine name.
    pub machine: Option<String>,
    /// Source archive the map was produced for (filename only).
    pub source_archive: String,
}

/// Per-entry record inside [`SanitizationReport`] (AR-19).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SanitizationEntry {
    /// Archive-relative entry name (e.g. `tree.json`, `imports/foo.props`).
    pub path: String,
    /// Disposition: `verbatim`, `redacted`, or `dropped`.
    pub disposition: Disposition,
    /// Human-readable description (e.g. the rule ID) of the transform
    /// that was applied. `None` for verbatim entries.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rule: Option<String>,
    /// Reason text for `dropped` entries (e.g. "unknown artifact",
    /// "absolute import path").
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
}

/// AR-19 disposition tag for each archive entry processed by the
/// sanitizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Verbatim,
    Redacted,
    Dropped,
}

/// Top-level `sanitization-report.json` payload (AR-19). Listed inside
/// the sanitized zip alongside the entries it describes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SanitizationReport {
    pub schema_version: u32,
    pub source_archive: String,
    pub entries: Vec<SanitizationEntry>,
    /// Artifact names the rule registry had no decision for. Each is
    /// dropped per D-9 deny-by-default; the names are surfaced here so
    /// gaps in the registry are immediately visible.
    pub unknown_artifacts: Vec<String>,
}

/// Inputs to [`sanitize_archive`].
pub struct SanitizeInputs<'a> {
    /// Input capture archive produced by the `archive` subcommand.
    pub input: &'a Path,
    /// Output path for the sanitized zip (`<stem>-sanitized.zip`).
    pub output: &'a Path,
    /// Output path for the local-only pseudonym map JSON
    /// (`<stem>-pseudonym-map.local.json`). Never written inside the
    /// sanitized zip.
    pub map: &'a Path,
    /// Pseudonymizer to apply. The CLI passes one built from the
    /// process environment; tests pass an explicit one.
    pub pseudonymizer: &'a Pseudonymizer,
}

/// Run the sanitizer end-to-end. Returns the [`SanitizationReport`]
/// that was written into the sanitized zip (AR-19) so callers can
/// inspect dispositions without re-reading the output.
pub fn sanitize_archive(inputs: &SanitizeInputs<'_>) -> io::Result<SanitizationReport> {
    let in_file = std::fs::File::open(inputs.input)?;
    let mut zin = zip::ZipArchive::new(in_file).map_err(zip_to_io)?;

    let source_archive = inputs
        .input
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Two passes: pass 1 collects the manifest so we can populate the
    // map (machine name) before we serialize any artifact that quotes
    // it; pass 2 transforms each entry in zip order.
    let original_machine: Option<String> = read_manifest_machine(&mut zin)?;

    let map = PseudonymMap {
        schema_version: PSEUDONYM_MAP_SCHEMA_VERSION,
        user_profile: pseudonymizer_profile(inputs.pseudonymizer),
        machine: original_machine.clone(),
        source_archive: source_archive.clone(),
    };

    if let Some(parent) = inputs.output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let out_file = std::fs::File::create(inputs.output)?;
    let mut zout = zip::write::ZipWriter::new(out_file);
    let file_opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let dir_opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);

    let mut entries: Vec<SanitizationEntry> = Vec::new();
    let mut unknown_artifacts: Vec<String> = Vec::new();

    // Track which top-level dirs we have already written so the
    // imports/ and tlogs/ entries stay present even when empty.
    let mut wrote_imports_dir = false;
    let mut wrote_tlogs_dir = false;

    let names: Vec<String> = (0..zin.len())
        .map(|i| zin.by_index(i).map(|e| e.name().to_string()))
        .collect::<Result<_, _>>()
        .map_err(zip_to_io)?;

    for name in &names {
        if name.ends_with('/') {
            // Directory entry. Reproduce known top-level ones so the
            // sanitized zip's shape matches the input's.
            if name == IMPORTS_DIR_NAME {
                zout.add_directory(IMPORTS_DIR_NAME, dir_opts)
                    .map_err(zip_to_io)?;
                wrote_imports_dir = true;
            } else if name == TLOGS_DIR_NAME {
                zout.add_directory(TLOGS_DIR_NAME, dir_opts)
                    .map_err(zip_to_io)?;
                wrote_tlogs_dir = true;
            }
            continue;
        }

        let kind = classify_artifact_kind(name);
        let mut entry_reader = zin.by_name(name).map_err(zip_to_io)?;

        match kind {
            ArtifactKind::Manifest => {
                let mut s = String::new();
                entry_reader.read_to_string(&mut s)?;
                let sanitized =
                    sanitize_manifest(&s, inputs.pseudonymizer, original_machine.as_deref())?;
                zout.start_file(name, file_opts).map_err(zip_to_io)?;
                zout.write_all(sanitized.as_bytes())?;
                entries.push(SanitizationEntry {
                    path: name.clone(),
                    disposition: Disposition::Redacted,
                    rule: Some("manifest:machine+roots".into()),
                    reason: None,
                });
            }
            ArtifactKind::Tree => {
                let mut s = String::new();
                entry_reader.read_to_string(&mut s)?;
                let sanitized = sanitize_tree(&s, inputs.pseudonymizer)?;
                zout.start_file(name, file_opts).map_err(zip_to_io)?;
                zout.write_all(sanitized.as_bytes())?;
                entries.push(SanitizationEntry {
                    path: name.clone(),
                    disposition: Disposition::Redacted,
                    rule: Some("tree:roots+relpath".into()),
                    reason: None,
                });
            }
            ArtifactKind::Import => {
                let mut s = String::new();
                entry_reader.read_to_string(&mut s)?;
                let sanitized = sanitize_import_text(&s, inputs.pseudonymizer);
                zout.start_file(name, file_opts).map_err(zip_to_io)?;
                zout.write_all(sanitized.as_bytes())?;
                entries.push(SanitizationEntry {
                    path: name.clone(),
                    disposition: Disposition::Redacted,
                    rule: Some(redact_rule_id(RedactRule::BinlogPropertyValue)),
                    reason: None,
                });
            }
            ArtifactKind::Tlog => {
                let mut s = String::new();
                entry_reader.read_to_string(&mut s)?;
                let sanitized = inputs.pseudonymizer.rewrite(&s);
                zout.start_file(name, file_opts).map_err(zip_to_io)?;
                zout.write_all(sanitized.as_bytes())?;
                entries.push(SanitizationEntry {
                    path: name.clone(),
                    disposition: Disposition::Redacted,
                    rule: Some(redact_rule_id(RedactRule::PathPseudonym)),
                    reason: None,
                });
            }
            ArtifactKind::Binlog => {
                // The binlog binary stream carries property values and
                // environment variables (D-9). The sanitizer does not
                // yet rewrite binary binlog records; the safer
                // disposition is to drop the binlog from the sanitized
                // zip and record the gap explicitly. The diff/correlate
                // reports a user attaches to an issue already convey
                // the information the binlog would.
                entries.push(SanitizationEntry {
                    path: name.clone(),
                    disposition: Disposition::Dropped,
                    rule: Some("binlog:not-yet-implemented".into()),
                    reason: Some(
                        "binlog binary records carry property values and environment variables; \
                         binary-stream redaction is not yet implemented"
                            .into(),
                    ),
                });
            }
            ArtifactKind::Unknown => {
                unknown_artifacts.push(name.clone());
                entries.push(SanitizationEntry {
                    path: name.clone(),
                    disposition: Disposition::Dropped,
                    rule: Some("unknown-artifact".into()),
                    reason: Some("unknown artifact (deny-by-default per D-9)".into()),
                });
            }
        }
    }

    // Preserve the empty-directory marker even when no member files
    // were copied — keeps the sanitized zip shape consistent with the
    // capture's (D-3).
    if !wrote_imports_dir {
        zout.add_directory(IMPORTS_DIR_NAME, dir_opts)
            .map_err(zip_to_io)?;
    }
    if !wrote_tlogs_dir {
        zout.add_directory(TLOGS_DIR_NAME, dir_opts)
            .map_err(zip_to_io)?;
    }

    let report = SanitizationReport {
        schema_version: SANITIZATION_REPORT_SCHEMA_VERSION,
        source_archive,
        entries,
        unknown_artifacts,
    };

    zout.start_file(SANITIZATION_REPORT_NAME, file_opts)
        .map_err(zip_to_io)?;
    serde_json::to_writer_pretty(&mut zout, &report)?;

    zout.finish().map_err(zip_to_io)?;

    // Map file is sibling-only. Never inside the zip (AR-18 constraint).
    if let Some(parent) = inputs.map.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let map_file = std::fs::File::create(inputs.map)?;
    serde_json::to_writer_pretty(map_file, &map)?;

    Ok(report)
}

/// Compose default sibling paths for `<stem>-sanitized.zip` and
/// `<stem>-pseudonym-map.local.json` given an input archive path.
pub fn default_output_paths(input: &Path) -> (PathBuf, PathBuf) {
    let parent = input.parent().unwrap_or(Path::new("."));
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".into());
    (
        parent.join(format!("{stem}-sanitized.zip")),
        parent.join(format!("{stem}-pseudonym-map.local.json")),
    )
}

/// Stable rule identifier used in the sanitization report. Stable
/// strings let downstream tooling track rule application without
/// depending on the enum's debug shape.
pub fn redact_rule_id(rule: RedactRule) -> String {
    match rule {
        RedactRule::PathPseudonym => "path-pseudonym".into(),
        RedactRule::MachineName => "machine-name".into(),
        RedactRule::BinlogPropertyValue => "binlog-property-value".into(),
        RedactRule::EnvironmentVariable => "environment-variable".into(),
    }
}

/// Artifact categories the sanitizer knows how to transform. Unknown
/// artifacts are dropped per D-9 deny-by-default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactKind {
    Manifest,
    Tree,
    Import,
    Tlog,
    Binlog,
    Unknown,
}

/// Map an archive entry name to its [`ArtifactKind`]. Treats anything
/// ending in `.binlog` at the top level as the captured binlog.
fn classify_artifact_kind(name: &str) -> ArtifactKind {
    if name == MANIFEST_NAME {
        return ArtifactKind::Manifest;
    }
    if name == TREE_JSON_NAME {
        return ArtifactKind::Tree;
    }
    if let Some(rest) = name.strip_prefix(IMPORTS_DIR_NAME) {
        if !rest.is_empty() {
            return ArtifactKind::Import;
        }
    }
    if let Some(rest) = name.strip_prefix(TLOGS_DIR_NAME) {
        if !rest.is_empty() {
            return ArtifactKind::Tlog;
        }
    }
    if name.ends_with(".binlog") && !name.contains('/') {
        return ArtifactKind::Binlog;
    }
    // Defer to the rule registry for any other artifact name —
    // currently nothing else is classified, so this falls through to
    // Unknown. Touching the registry keeps the call site honest about
    // what would happen if a rule were added later.
    let _ = classify(name, "*");
    ArtifactKind::Unknown
}

fn pseudonymizer_profile(p: &Pseudonymizer) -> Option<String> {
    // Round-trip the rewrite of a synthetic anchor to discover the
    // pseudonymizer's user-profile prefix without exposing private
    // state. If the anchor is unchanged, no rewrite is configured.
    const ANCHOR: &str = "<<PROFILE_ANCHOR>>";
    let probed = p.rewrite(ANCHOR);
    if probed == ANCHOR {
        // The pseudonymizer has no profile prefix configured, OR the
        // configured prefix simply doesn't appear in the anchor — in
        // which case we can't recover it from the API and report None.
        // The map's `user_profile` field is best-effort and may be
        // populated separately by the CLI in future revisions.
        None
    } else {
        Some(probed)
    }
}

fn read_manifest_machine<R: Read + Seek>(
    zin: &mut zip::ZipArchive<R>,
) -> io::Result<Option<String>> {
    let names: Vec<String> = (0..zin.len())
        .map(|i| zin.by_index(i).map(|e| e.name().to_string()))
        .collect::<Result<_, _>>()
        .map_err(zip_to_io)?;
    if !names.iter().any(|n| n == MANIFEST_NAME) {
        return Ok(None);
    }
    let mut s = String::new();
    zin.by_name(MANIFEST_NAME)
        .map_err(zip_to_io)?
        .read_to_string(&mut s)?;
    let v: serde_json::Value = serde_json::from_str(&s)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("manifest.json: {e}")))?;
    Ok(v.get("machine")
        .and_then(|m| m.as_str())
        .map(str::to_string))
}

fn sanitize_manifest(raw: &str, p: &Pseudonymizer, machine: Option<&str>) -> io::Result<String> {
    // Operate on serde_json::Value to preserve unknown fields. The
    // typed Manifest enforces deny_unknown_fields, which we
    // intentionally bypass at sanitize time so we don't reject future
    // archive versions during the redact pass.
    let mut v: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("manifest.json: {e}")))?;
    if let Some(obj) = v.as_object_mut() {
        if matches!(
            classify(MANIFEST_NAME, "machine"),
            Classification::Redact(_)
        ) {
            if let Some(orig) = machine {
                obj.insert(
                    "machine".into(),
                    serde_json::Value::String(rewrite_machine(orig)),
                );
            }
        }
        if matches!(classify(MANIFEST_NAME, "roots"), Classification::Redact(_)) {
            if let Some(roots) = obj.get_mut("roots").and_then(|r| r.as_array_mut()) {
                for r in roots.iter_mut() {
                    if let Some(s) = r.as_str() {
                        *r = serde_json::Value::String(p.rewrite(s));
                    }
                }
            }
        }
    }
    serde_json::to_string_pretty(&v)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("manifest.json: {e}")))
}

fn sanitize_tree(raw: &str, p: &Pseudonymizer) -> io::Result<String> {
    let mut v: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("tree.json: {e}")))?;
    if let Some(roots) = v.get_mut("roots").and_then(|r| r.as_array_mut()) {
        for root in roots.iter_mut() {
            let Some(obj) = root.as_object_mut() else {
                continue;
            };
            if let Some(s) = obj.get("root").and_then(|r| r.as_str()) {
                let rewritten = p.rewrite(s);
                obj.insert("root".into(), serde_json::Value::String(rewritten));
            }
            if let Some(entries) = obj.get_mut("entries").and_then(|e| e.as_array_mut()) {
                for entry in entries.iter_mut() {
                    if let Some(s) = entry.get("relpath").and_then(|r| r.as_str()) {
                        let rewritten = p.rewrite(s);
                        entry
                            .as_object_mut()
                            .unwrap()
                            .insert("relpath".into(), serde_json::Value::String(rewritten));
                    }
                }
            }
        }
    }
    serde_json::to_string_pretty(&v)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("tree.json: {e}")))
}

/// Sanitize a project-import file's text content. Imports are
/// MSBuild XML carrying property values that may include credentials;
/// the rule registry classifies their contents as
/// [`RedactRule::BinlogPropertyValue`]. The transform replaces every
/// `<PropertyName>...value...</PropertyName>` body with
/// [`REDACTED_PROPERTY_PLACEHOLDER`] and then path-pseudonymizes the
/// remainder so absolute filesystem paths in the markup do not leak.
pub fn sanitize_import_text(raw: &str, p: &Pseudonymizer) -> String {
    let stripped = redact_xml_property_bodies(raw);
    p.rewrite(&stripped)
}

fn redact_xml_property_bodies(raw: &str) -> String {
    // Replace the text content of every `<PropertyName>BODY</PropertyName>`
    // pair inside an MSBuild `<PropertyGroup>`. The implementation is a
    // small forward scan that ignores XML attributes and self-closed
    // tags. It is intentionally conservative: anything ambiguous is
    // left as-is — the alternative (rewriting via a real XML parser)
    // is more dependency than this milestone justifies.
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(open_lt) = rest.find('<') {
        out.push_str(&rest[..open_lt]);
        rest = &rest[open_lt..];
        let Some(open_gt) = rest.find('>') else {
            out.push_str(rest);
            return out;
        };
        let open_tag = &rest[..=open_gt];
        out.push_str(open_tag);
        rest = &rest[open_gt + 1..];
        // Skip closing tags, self-closed tags, and comments / PIs.
        if open_tag.starts_with("</")
            || open_tag.starts_with("<!--")
            || open_tag.starts_with("<?")
            || open_tag.ends_with("/>")
        {
            continue;
        }
        // Pull the element name (first whitespace- or `>`-terminated token).
        let inner = &open_tag[1..open_tag.len() - 1];
        let name = inner
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("");
        if name.is_empty() || !is_likely_property_name(name) {
            continue;
        }
        let close = format!("</{name}>");
        let Some(close_at) = rest.find(close.as_str()) else {
            continue;
        };
        let body = &rest[..close_at];
        // Only redact non-empty, single-line bodies — multi-line bodies
        // are almost always structured markup (ItemGroup contents,
        // CDATA) and should not be flattened.
        let trimmed = body.trim();
        if !trimmed.is_empty() && !body.contains('\n') {
            out.push_str(REDACTED_PROPERTY_PLACEHOLDER);
        } else {
            out.push_str(body);
        }
        out.push_str(&close);
        rest = &rest[close_at + close.len()..];
    }
    out.push_str(rest);
    out
}

/// True if `name` looks like an MSBuild property element (PascalCase,
/// no namespace prefix, no reserved well-known element name). The
/// allow-list deliberately excludes structural elements (`Project`,
/// `PropertyGroup`, `ItemGroup`, `Target`, etc.) so their bodies are
/// not flattened.
fn is_likely_property_name(name: &str) -> bool {
    const STRUCTURAL: &[&str] = &[
        "Project",
        "PropertyGroup",
        "ItemGroup",
        "Target",
        "ImportGroup",
        "Import",
        "Choose",
        "When",
        "Otherwise",
        "UsingTask",
        "ItemDefinitionGroup",
    ];
    if STRUCTURAL.contains(&name) {
        return false;
    }
    if name.contains(':') {
        return false;
    }
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Substitute a machine name with `<MACHINE>`. Kept separate so the
/// pseudonym map's `machine` field can also record the original.
pub fn rewrite_machine(_orig: &str) -> String {
    "<MACHINE>".to_string()
}

/// Convenience for tests / scripts: collapse a [`SanitizationReport`]
/// into a path → disposition map.
pub fn report_disposition_map(report: &SanitizationReport) -> BTreeMap<String, Disposition> {
    report
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.disposition))
        .collect()
}

fn zip_to_io(e: zip::result::ZipError) -> io::Error {
    io::Error::other(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_rule_id_strings_are_stable() {
        assert_eq!(redact_rule_id(RedactRule::PathPseudonym), "path-pseudonym");
        assert_eq!(redact_rule_id(RedactRule::MachineName), "machine-name");
        assert_eq!(
            redact_rule_id(RedactRule::BinlogPropertyValue),
            "binlog-property-value"
        );
        assert_eq!(
            redact_rule_id(RedactRule::EnvironmentVariable),
            "environment-variable"
        );
    }

    #[test]
    fn classify_artifact_kind_maps_known_names() {
        assert_eq!(
            classify_artifact_kind(MANIFEST_NAME),
            ArtifactKind::Manifest
        );
        assert_eq!(classify_artifact_kind(TREE_JSON_NAME), ArtifactKind::Tree);
        assert_eq!(
            classify_artifact_kind("imports/foo.props"),
            ArtifactKind::Import
        );
        assert_eq!(
            classify_artifact_kind("tlogs/p/build.tlog"),
            ArtifactKind::Tlog
        );
        assert_eq!(classify_artifact_kind("build.binlog"), ArtifactKind::Binlog);
    }

    #[test]
    fn classify_artifact_kind_unknown_for_unclassified_names() {
        assert_eq!(classify_artifact_kind("random.txt"), ArtifactKind::Unknown);
        assert_eq!(classify_artifact_kind("imports/"), ArtifactKind::Unknown);
        assert_eq!(
            classify_artifact_kind("nested/build.binlog"),
            ArtifactKind::Unknown
        );
    }

    #[test]
    fn default_output_paths_compose_sibling_names() {
        let (zip, map) = default_output_paths(Path::new("/some/dir/build-T1.zip"));
        assert_eq!(zip, PathBuf::from("/some/dir/build-T1-sanitized.zip"));
        assert_eq!(
            map,
            PathBuf::from("/some/dir/build-T1-pseudonym-map.local.json")
        );
    }

    #[test]
    fn sanitize_import_text_redacts_property_body_and_pseudonymizes_paths() {
        let p = Pseudonymizer::from_explicit(Some("/home/alice".into()));
        // The path lives in an XML attribute (outside any property
        // body) so the pseudonymizer rewrite is observable. The
        // ApiKey body is inside a property, which gets flattened to
        // the REDACTED placeholder before the pseudonymizer runs.
        let xml = "<Project ToolsPath=\"/home/alice/sdk\"><PropertyGroup>\
                   <ApiKey>AKIA-FAKE-CREDENTIAL-1234567890</ApiKey>\
                   </PropertyGroup></Project>";
        let out = sanitize_import_text(xml, &p);
        assert!(
            !out.contains("AKIA-FAKE-CREDENTIAL"),
            "API key leaked: {out}"
        );
        assert!(out.contains(REDACTED_PROPERTY_PLACEHOLDER));
        assert!(
            out.contains("<USER>/sdk"),
            "pseudonymizer did not rewrite: {out}"
        );
        assert!(!out.contains("/home/alice"));
    }

    #[test]
    fn sanitize_import_text_leaves_structural_elements_alone() {
        let p = Pseudonymizer::from_explicit(Some("/home/alice".into()));
        let xml = "<Project><ItemGroup><Compile Include=\"a.cs\"/></ItemGroup></Project>";
        let out = sanitize_import_text(xml, &p);
        // Structural element bodies must survive.
        assert!(out.contains("<ItemGroup>"));
        assert!(out.contains("Include=\"a.cs\""));
        assert!(!out.contains(REDACTED_PROPERTY_PLACEHOLDER));
    }

    #[test]
    fn redact_xml_property_bodies_skips_multiline_bodies() {
        // Multi-line property bodies (rare, but legal — Conditioned
        // properties etc.) are left alone to avoid collapsing real
        // markup nesting.
        let xml =
            "<Project><PropertyGroup><Big>\n  line1\n  line2\n</Big></PropertyGroup></Project>";
        let out = redact_xml_property_bodies(xml);
        assert!(out.contains("line1"));
        assert!(out.contains("line2"));
    }

    #[test]
    fn pseudonym_map_json_round_trip() {
        let m = PseudonymMap {
            schema_version: PSEUDONYM_MAP_SCHEMA_VERSION,
            user_profile: Some("/home/alice".into()),
            machine: Some("HOST-42".into()),
            source_archive: "build-T1.zip".into(),
        };
        let s = serde_json::to_string(&m).unwrap();
        let back: PseudonymMap = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn sanitization_report_json_round_trip() {
        let r = SanitizationReport {
            schema_version: SANITIZATION_REPORT_SCHEMA_VERSION,
            source_archive: "build-T1.zip".into(),
            entries: vec![SanitizationEntry {
                path: "tree.json".into(),
                disposition: Disposition::Redacted,
                rule: Some("tree:roots+relpath".into()),
                reason: None,
            }],
            unknown_artifacts: vec!["random.txt".into()],
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: SanitizationReport = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn is_likely_property_name_rejects_structural_elements() {
        assert!(!is_likely_property_name("Project"));
        assert!(!is_likely_property_name("PropertyGroup"));
        assert!(!is_likely_property_name("ItemGroup"));
        assert!(!is_likely_property_name("Import"));
    }

    #[test]
    fn is_likely_property_name_accepts_pascal_case_names() {
        assert!(is_likely_property_name("ApiKey"));
        assert!(is_likely_property_name("IntermediateOutputPath"));
    }

    #[test]
    fn is_likely_property_name_rejects_namespaced_names() {
        assert!(!is_likely_property_name("xsi:type"));
        assert!(!is_likely_property_name("lowercase"));
    }

    #[test]
    fn pseudonymizer_profile_round_trips_when_anchor_is_unrewritten() {
        let p = Pseudonymizer::noop();
        assert_eq!(pseudonymizer_profile(&p), None);
    }
}
