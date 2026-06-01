// Copyright (c) {{license_year}} {{author_name}}

//! `{{mcp_crate_name}}` — minimal MCP server skeleton.
//!
//! Speaks JSON-RPC 2.0 over stdio with newline-delimited messages. Supports
//! the four methods every MCP client expects on startup: `initialize`,
//! `notifications/initialized`, `tools/list`, and `tools/call`.
//!
//! Replace the tool in `tools.rs` with your own and extend this dispatcher
//! as needed.

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use {{mcp_crate_name | replace: "-", "_"}}::tools;

#[derive(Deserialize)]
struct Message {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: Value,
    #[serde(flatten)]
    body: ResponseBody,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ResponseBody {
    Ok { result: Value },
    Err { error: RpcError },
}

#[derive(Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

mod code {
    pub const PARSE_ERROR: i32 = -32700;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
}

const PROTOCOL_VERSION: &str = "2024-11-05";

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = handle_line(&line);
        if let Some(resp) = response {
            writeln!(out, "{}", serde_json::to_string(&resp).unwrap())?;
            out.flush()?;
        }
    }
    Ok(())
}

fn handle_line(line: &str) -> Option<Response> {
    let msg: Message = match serde_json::from_str(line) {
        Ok(m) => m,
        Err(e) => {
            return Some(Response {
                jsonrpc: "2.0",
                id: Value::Null,
                body: ResponseBody::Err {
                    error: RpcError {
                        code: code::PARSE_ERROR,
                        message: format!("parse error: {e}"),
                    },
                },
            });
        }
    };

    // Notifications have no id and expect no response.
    let id = msg.id.clone()?;

    let result = match msg.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "{{mcp_crate_name}}",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        "tools/list" => Ok(tools::list()),
        "tools/call" => {
            let name = msg.params.get("name").and_then(Value::as_str);
            let args = msg.params.get("arguments").cloned().unwrap_or(json!({}));
            match name {
                Some(n) => tools::call(n, &args).map_err(|e| RpcError {
                    code: code::INVALID_PARAMS,
                    message: e,
                }),
                None => Err(RpcError {
                    code: code::INVALID_PARAMS,
                    message: "missing 'name' in tools/call params".into(),
                }),
            }
        }
        other => Err(RpcError {
            code: code::METHOD_NOT_FOUND,
            message: format!("unknown method: {other}"),
        }),
    };

    Some(Response {
        jsonrpc: "2.0",
        id,
        body: match result {
            Ok(result) => ResponseBody::Ok { result },
            Err(error) => ResponseBody::Err { error },
        },
    })
}
