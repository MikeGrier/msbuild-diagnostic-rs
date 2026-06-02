// Copyright (c) 2026 Mike Grier

//! Path pseudonymization for report rendering (AR-17, D-9).
//!
//! Sanitization rules in [`super::rules`] declare *that* a field needs
//! pseudonymization; this module supplies the actual substitution.
//! Today it implements the user-profile rewrite that AR-17 verifies:
//! any occurrence of the user's home directory (Windows `USERPROFILE`,
//! Unix `HOME`) inside a string is replaced with the literal token
//! `<USER>`. Forward and backward slashes are both recognized so a
//! single rewrite handles both Windows-shape and Unix-shape paths.
//!
//! The pseudonymizer is constructed from explicit values for tests
//! (hermetic per D-14) and from the live environment for the CLI.

/// Replaces well-known sensitive substrings with stable pseudonyms.
///
/// Created via [`Pseudonymizer::from_explicit`] (hermetic) or
/// [`Pseudonymizer::from_environment`] (CLI). [`Pseudonymizer::noop`]
/// returns a pseudonymizer that performs no substitutions; existing
/// callers that have not yet adopted pseudonymization pass it.
#[derive(Debug, Clone, Default)]
pub struct Pseudonymizer {
    user_profile: Option<String>,
}

/// Replacement token for the user's profile directory.
pub const USER_TOKEN: &str = "<USER>";

impl Pseudonymizer {
    /// Pseudonymizer that performs no substitutions.
    pub fn noop() -> Self {
        Self::default()
    }

    /// Build a pseudonymizer from explicit values. Pass `None` to
    /// disable a given substitution.
    pub fn from_explicit(user_profile: Option<String>) -> Self {
        Self {
            user_profile: user_profile.filter(|s| !s.is_empty()),
        }
    }

    /// Build a pseudonymizer from the current process environment.
    /// Reads `USERPROFILE` then falls back to `HOME`.
    pub fn from_environment() -> Self {
        let user_profile = std::env::var("USERPROFILE")
            .ok()
            .or_else(|| std::env::var("HOME").ok())
            .filter(|s| !s.is_empty());
        Self { user_profile }
    }

    /// Rewrite the user-profile prefix in `text` to `<USER>` (covers
    /// both forward and backward slashes that may follow).
    pub fn rewrite(&self, text: &str) -> String {
        let Some(profile) = self.user_profile.as_deref() else {
            return text.to_string();
        };
        // Match the profile prefix as written, plus a fallback that
        // normalizes its slashes to match either flavor in the input.
        let candidates = [
            profile.to_string(),
            profile.replace('\\', "/"),
            profile.replace('/', "\\"),
        ];
        let mut out = text.to_string();
        for c in &candidates {
            if c.is_empty() {
                continue;
            }
            out = out.replace(c.as_str(), USER_TOKEN);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_pseudonymizer_returns_input_verbatim() {
        let p = Pseudonymizer::noop();
        assert_eq!(
            p.rewrite("C:\\Users\\alice\\src\\a.cs"),
            "C:\\Users\\alice\\src\\a.cs"
        );
    }

    #[test]
    fn rewrites_windows_user_profile_prefix() {
        let p = Pseudonymizer::from_explicit(Some("C:\\Users\\alice".into()));
        assert_eq!(
            p.rewrite("C:\\Users\\alice\\src\\a.cs"),
            "<USER>\\src\\a.cs"
        );
    }

    #[test]
    fn rewrites_unix_home_prefix() {
        let p = Pseudonymizer::from_explicit(Some("/home/alice".into()));
        assert_eq!(p.rewrite("/home/alice/src/a.cs"), "<USER>/src/a.cs");
    }

    #[test]
    fn rewrites_when_slash_flavor_differs_from_profile_value() {
        // Profile recorded with backslashes; input has forward slashes.
        let p = Pseudonymizer::from_explicit(Some("C:\\Users\\alice".into()));
        assert_eq!(p.rewrite("C:/Users/alice/src/a.cs"), "<USER>/src/a.cs");
    }

    #[test]
    fn empty_profile_value_is_ignored() {
        let p = Pseudonymizer::from_explicit(Some(String::new()));
        assert_eq!(p.rewrite("anything"), "anything");
    }

    #[test]
    fn multiple_occurrences_are_all_rewritten() {
        let p = Pseudonymizer::from_explicit(Some("/h/u".into()));
        assert_eq!(p.rewrite("/h/u/a and /h/u/b"), "<USER>/a and <USER>/b");
    }
}
