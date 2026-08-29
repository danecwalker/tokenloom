//! `tokenloom mcp` — Model Context Protocol stdio server (PLAN.md §8, M8).
//!
//! Minimal JSON-RPC 2.0 over newline-delimited stdio implementing the MCP
//! lifecycle (`initialize`, `tools/list`, `tools/call`) with two tools:
//! `search` and `fetch`.

use crate::commands::App;
use std::io::{BufRead, Write};
use std::sync::Arc;
use tokenloom_core::{Category, Config};

const PROTOCOL_VERSION: &str = "2024-11-05";

pub async fn run(config: &Config) -> i32 {
    let app = match App::new(config) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("tokenloom: {e}");
            return 2;
        }
    };
    let app = Arc::new(app);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    loop {
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            let _ = writeln!(
                stdout,
                r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":-32700,"message":"parse error"}}}}"#
            );
            continue;
        };

        // Notifications (no id) get no response.
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let response = match method {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "tokenloom", "version": env!("CARGO_PKG_VERSION") }
                }
            }),
            "tools/list" => {
                let tools: serde_json::Value =
                    serde_json::from_str(TOOLS).expect("embedded tool schemas are valid JSON");
                serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "tools": tools }
                })
            }
            "tools/call" => {
                let name = msg
                    .pointer("/params/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let args = msg
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or_default();
                let (is_error, text) = call_tool(&app, name, &args).await;
                serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": text }],
                        "isError": is_error
                    }
                })
            }
            "ping" => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            other => serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("method not found: {other}") }
            }),
        };

        if writeln!(stdout, "{response}").is_err() {
            break;
        }
        let _ = stdout.flush();
    }
    0
}

const TOOLS: &str = r#"[
  {
    "name": "search",
    "description": "Federated web search across SearXNG-compatible engines with RRF ranking. Supports bangs like !ddg, !arx, !news.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "query": { "type": "string", "description": "Search query (bangs allowed)" },
        "category": { "type": "string", "description": "general|images|videos|news|map|music|it|science|files|social_media" },
        "limit": { "type": "integer", "description": "Max results (default 10)" }
      },
      "required": ["query"]
    }
  },
  {
    "name": "fetch",
    "description": "Fetch a URL and return clean, sanitised Markdown. SPA pages are delegated to Jina Reader with a 20 RPM budget and an honest fallback ladder.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "url": { "type": "string", "description": "Absolute http(s) URL to fetch" }
      },
      "required": ["url"]
    }
  }
]"#;

async fn call_tool(app: &App, name: &str, args: &serde_json::Value) -> (bool, String) {
    match name {
        "search" => {
            let Some(query_text) = args.get("query").and_then(|v| v.as_str()) else {
                return (true, "missing required argument: query".into());
            };
            let category = args
                .get("category")
                .and_then(|v| v.as_str())
                .and_then(Category::from_str);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let query = match crate::commands::search::build_query(
                &app.config,
                &app.registry,
                query_text,
                category,
                limit,
            ) {
                Ok(q) => q,
                Err(e) => return (true, format!("query error: {e}")),
            };
            let federator = tokenloom_engines::Federator::new(
                app.registry.clone(),
                app.client.raw().clone(),
                app.config.engines.weights.clone().into_iter().collect(),
            );
            let response = federator.search(&query).await;
            (false, tokenloom_output::format_search_markdown(&response))
        }
        "fetch" => {
            let Some(url) = args.get("url").and_then(|v| v.as_str()) else {
                return (true, "missing required argument: url".into());
            };
            match crate::commands::fetch::fetch_page(&app.config, app, url).await {
                Ok(page) => (false, tokenloom_output::format_fetch_markdown(&page)),
                Err(e) => (true, format!("fetch failed: {e}")),
            }
        }
        other => (true, format!("unknown tool: {other}")),
    }
}
