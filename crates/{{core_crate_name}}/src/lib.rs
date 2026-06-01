// Copyright (c) {{license_year}} {{author_name}}

//! {{project_description}}
//!
//! This is the core library crate for `{{project_name}}`.

/// Returns a hello-world greeting from this crate.
pub fn hello() -> String {
    format!("Hello from {}!", "{{core_crate_name}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_greets_this_crate() {
        assert!(hello().contains("{{core_crate_name}}"));
    }
}
