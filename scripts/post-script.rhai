// cargo-generate post-script.
//
// Runs after Liquid substitution + conditional file removal. Used to:
//   1. Replace the template-only README.md (which documents the placeholder
//      syntax for human readers) with README.template.md (the README that
//      ships in the generated project).
//   2. Delete this script itself from the generated output.
//
// Reference: https://cargo-generate.github.io/cargo-generate/templates/scripting.html

// Remove the template instructions README and promote the project README.
file::delete("README.md");
file::rename("README.template.md", "README.md");

// Remove the scripts/ directory if it only contained this hook.
file::delete("scripts/post-script.rhai");
