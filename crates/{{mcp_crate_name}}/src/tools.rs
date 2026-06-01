// Copyright (c) {{license_year}} {{author_name}}

//! Minimal MCP (Model Context Protocol) tool module.
//!
//! Replace `hello` with your own tools as the project grows.

use serde_json::{json, Value};

/// Dispatch a single MCP tool call. Returns the JSON value the server will
/// serialize as the `content` of an `tools/call` response.
pub fn call(name: &str, _arguments: &Value) -> Result<Value, String> {
    match name {
        "hello" => Ok(json!({
            "content": [
                { "type": "text", "text": "Hello from {{mcp_crate_name}}!" }
            ]
        })),
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Return the static list advertised in `tools/list`.
pub fn list() -> Value {
    json!({
        "tools": [
            {
                "name": "hello",
                "description": "Returns a greeting. Replace with your own tool.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_contains_hello() {
        let l = list();
        let tools = l["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "hello"));
    }

    #[test]
    fn hello_returns_text_content() {
        let v = call("hello", &json!({})).unwrap();
        assert_eq!(v["content"][0]["type"], "text");
    }

    #[test]
    fn unknown_tool_is_error() {
        assert!(call("does-not-exist", &json!({})).is_err());
    }
}
