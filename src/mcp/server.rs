// pfodb MCP server
// Transport: stdio JSON-RPC (Model Context Protocol standard)
// All tool calls forward to the pfodb HTTP API on localhost:3000
//
// MCP message flow:
//   agent → stdin  → this server → HTTP → pfodb engine
//   agent ← stdout ← this server ← HTTP ← pfodb engine
//
// stderr is used for all diagnostic logging — never stdout
// stdout is reserved exclusively for MCP JSON-RPC messages

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const ENGINE_URL: &str = "http://localhost:3000";

// ── MCP JSON-RPC types ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

impl RpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        RpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Value, code: i32, message: String) -> Self {
        RpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(RpcError { code, message }),
        }
    }
}

// ── Tool definitions ───────────────────────────────────────────────────────────

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "insert_pfo",
                "description": "Insert a new Probabilistic Fact Object (PFO) into the database. The engine resolves the source name to a UUID automatically, registering it if unknown. Returns the PFO ID, confidence, and resolved source_id.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "claim_text": {
                            "type": "string",
                            "description": "The factual claim to store"
                        },
                        "confidence": {
                            "type": "number",
                            "description": "Initial confidence 0.0–1.0"
                        },
                        "source": {
                            "type": "string",
                            "description": "Source name as a string (e.g. 'Reuters Energy Desk'). Engine resolves to UUID."
                        },
                        "domain": {
                            "type": "string",
                            "description": "EU AI Act domain (e.g. CriticalInfrastructure). Engine overrides if wrong.",
                            "default": "CriticalInfrastructure"
                        }
                    },
                    "required": ["claim_text", "confidence", "source"]
                }
            },
            {
                "name": "get_pfo",
                "description": "Retrieve a PFO by its UUID. Returns the full fact including confidence, source, conflict refs, and corroboration count.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "PFO UUID"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "search",
                "description": "Semantic search for PFOs by natural language query. Returns top-k results ranked by vector similarity, each with confidence and conflict status.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Natural language search query"
                        },
                        "k": {
                            "type": "integer",
                            "description": "Number of results to return (default 5)",
                            "default": 5
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "get_source",
                "description": "Look up a source by name. Returns the source UUID, current effective weight, corroboration count, and conflict trigger count. Use this before inserting a PFO to check source reliability.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Source name (e.g. 'Reuters Energy Desk')"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "register_source",
                "description": "Register a new source by name. Returns the assigned UUID and initial effective weight (0.5). Idempotent — returns existing source if name already registered.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Source name"
                        },
                        "domain": {
                            "type": "string",
                            "description": "Optional domain hint"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "update_confidence",
                "description": "Manually adjust a PFO's confidence. Requires a documented reason for EU AI Act audit trail. Returns before/after values.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "PFO UUID"
                        },
                        "confidence": {
                            "type": "number",
                            "description": "New confidence value 0.0–1.0"
                        },
                        "reason": {
                            "type": "string",
                            "description": "Documented reason for adjustment (required for audit trail)"
                        }
                    },
                    "required": ["id", "confidence", "reason"]
                }
            }
        ]
    })
}

// ── HTTP helpers ───────────────────────────────────────────────────────────────

async fn http_get(client: &reqwest::Client, path: &str) -> anyhow::Result<Value> {
    let url = format!("{}{}", ENGINE_URL, path);
    eprintln!("[pfodb-mcp] GET {}", url);
    let resp = client.get(&url).send().await?;
    let status = resp.status();
    let body: Value = resp.json().await?;
    if !status.is_success() {
        anyhow::bail!("engine returned {}: {}", status, body);
    }
    Ok(body)
}

async fn http_post(client: &reqwest::Client, path: &str, body: &Value) -> anyhow::Result<Value> {
    let url = format!("{}{}", ENGINE_URL, path);
    eprintln!("[pfodb-mcp] POST {} {}", url, body);
    let resp = client.post(&url).json(body).send().await?;
    let status = resp.status();
    let resp_body: Value = resp.json().await?;
    if !status.is_success() {
        anyhow::bail!("engine returned {}: {}", status, resp_body);
    }
    Ok(resp_body)
}

async fn http_patch(client: &reqwest::Client, path: &str, body: &Value) -> anyhow::Result<Value> {
    let url = format!("{}{}", ENGINE_URL, path);
    eprintln!("[pfodb-mcp] PATCH {} {}", url, body);
    let resp = client.patch(&url).json(body).send().await?;
    let status = resp.status();
    let resp_body: Value = resp.json().await?;
    if !status.is_success() {
        anyhow::bail!("engine returned {}: {}", status, resp_body);
    }
    Ok(resp_body)
}

// ── Tool dispatch ──────────────────────────────────────────────────────────────

async fn dispatch_tool(
    client: &reqwest::Client,
    tool_name: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    match tool_name {
        "insert_pfo" => {
            let claim_text = args["claim_text"].as_str()
                .ok_or_else(|| anyhow::anyhow!("claim_text required"))?;
            let confidence = args["confidence"].as_f64()
                .ok_or_else(|| anyhow::anyhow!("confidence required"))?;
            let source = args["source"].as_str()
                .ok_or_else(|| anyhow::anyhow!("source required"))?;
            let domain = args["domain"].as_str().unwrap_or("CriticalInfrastructure");

            let body = json!({
                "claim_text": claim_text,
                "confidence": confidence,
                "source": source,
                "domain": domain
            });
            http_post(client, "/pfo", &body).await
        }

        "get_pfo" => {
            let id = args["id"].as_str()
                .ok_or_else(|| anyhow::anyhow!("id required"))?;
            http_get(client, &format!("/pfo/{}", id)).await
        }

        "search" => {
            let query = args["query"].as_str()
                .ok_or_else(|| anyhow::anyhow!("query required"))?;
            let k = args["k"].as_u64().unwrap_or(5);
            let encoded = urlencoding_encode(query);
            http_get(client, &format!("/search?q={}&k={}", encoded, k)).await
        }

        "get_source" => {
            let name = args["name"].as_str()
                .ok_or_else(|| anyhow::anyhow!("name required"))?;
            let encoded = urlencoding_encode(name);
            http_get(client, &format!("/source?name={}", encoded)).await
        }

        "register_source" => {
            let name = args["name"].as_str()
                .ok_or_else(|| anyhow::anyhow!("name required"))?;
            let mut body = json!({ "name": name });
            if let Some(domain) = args["domain"].as_str() {
                body["domain"] = json!(domain);
            }
            http_post(client, "/source", &body).await
        }

        "update_confidence" => {
            let id = args["id"].as_str()
                .ok_or_else(|| anyhow::anyhow!("id required"))?;
            let confidence = args["confidence"].as_f64()
                .ok_or_else(|| anyhow::anyhow!("confidence required"))?;
            let reason = args["reason"].as_str()
                .ok_or_else(|| anyhow::anyhow!("reason required"))?;

            let body = json!({
                "confidence": confidence,
                "reason": reason
            });
            http_patch(client, &format!("/pfo/{}/confidence", id), &body).await
        }

        _ => anyhow::bail!("unknown tool: {}", tool_name),
    }
}

// minimal URL encoding for query params — handles spaces and special chars
fn urlencoding_encode(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        ' ' => "%20".to_string(),
        _ => format!("%{:02X}", c as u32),
    }).collect()
}

// ── MCP request handler ────────────────────────────────────────────────────────

async fn handle_request(client: &reqwest::Client, req: RpcRequest) -> RpcResponse {
    let id = req.id.clone().unwrap_or(Value::Null);

    match req.method.as_str() {
        // MCP initialisation handshake
        "initialize" => {
            RpcResponse::ok(id, json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "pfodb-mcp",
                    "version": "0.1.0"
                }
            }))
        }

        // MCP notification — no response needed but we must not error
        "notifications/initialized" => {
            RpcResponse::ok(id, json!({}))
        }

        // list available tools
        "tools/list" => {
            RpcResponse::ok(id, tools_list())
        }

        // execute a tool
        "tools/call" => {
            let params = match req.params.as_ref() {
                Some(p) => p,
                None => return RpcResponse::err(id, -32602, "params required".to_string()),
            };

            let tool_name = match params["name"].as_str() {
                Some(n) => n,
                None => return RpcResponse::err(id, -32602, "tool name required".to_string()),
            };

            let args = params.get("arguments").cloned().unwrap_or(json!({}));

            eprintln!("[pfodb-mcp] tool call: {} args: {}", tool_name, args);

            match dispatch_tool(client, tool_name, &args).await {
                Ok(result) => {
                    eprintln!("[pfodb-mcp] tool result: {}", result);
                    RpcResponse::ok(id, json!({
                        "content": [
                            {
                                "type": "text",
                                "text": serde_json::to_string_pretty(&result)
                                    .unwrap_or_else(|_| result.to_string())
                            }
                        ]
                    }))
                }
                Err(e) => {
                    eprintln!("[pfodb-mcp] tool error: {}", e);
                    RpcResponse::err(id, -32000, e.to_string())
                }
            }
        }

        other => {
            eprintln!("[pfodb-mcp] unknown method: {}", other);
            RpcResponse::err(id, -32601, format!("method not found: {}", other))
        }
    }
}

// ── Main run loop ──────────────────────────────────────────────────────────────

pub async fn run() {
    let client = reqwest::Client::new();
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();

    eprintln!("[pfodb-mcp] ready — reading from stdin");

    while let Ok(Some(line)) = reader.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        eprintln!("[pfodb-mcp] received: {}", line);

        let req: RpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[pfodb-mcp] parse error: {}", e);
                let err = RpcResponse::err(
                    Value::Null,
                    -32700,
                    format!("parse error: {}", e),
                );
                let mut out = serde_json::to_string(&err).unwrap();
                out.push('\n');
                stdout.write_all(out.as_bytes()).await.unwrap();
                stdout.flush().await.unwrap();
                continue;
            }
        };

        // notifications have no id and require no response
        if req.method.starts_with("notifications/") {
            eprintln!("[pfodb-mcp] notification received: {}", req.method);
            continue;
        }

        let resp = handle_request(&client, req).await;
        let mut out = serde_json::to_string(&resp).unwrap();
        out.push('\n');
        eprintln!("[pfodb-mcp] sending: {}", out.trim());
        stdout.write_all(out.as_bytes()).await.unwrap();
        stdout.flush().await.unwrap();
    }

    eprintln!("[pfodb-mcp] stdin closed — exiting");
}