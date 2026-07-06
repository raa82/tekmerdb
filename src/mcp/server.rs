use crate::{mcp_log_info, mcp_log_error};
// TekmerDB MCP server
// Two transport modes:
//   stdio — for Claude Desktop and subprocess agents (default)
//   SSE   — for network agents via HTTP (--sse flag)

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use axum::{
    Router,
    routing::get,
    response::sse::{Event, Sse},
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use futures::stream::{self, Stream, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;

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

#[derive(Debug, Serialize, Clone)]
struct RpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Serialize, Clone)]
struct RpcError {
    code: i32,
    message: String,
}

impl RpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        RpcResponse { jsonrpc: "2.0".to_string(), id, result: Some(result), error: None }
    }
    fn err(id: Value, code: i32, message: String) -> Self {
        RpcResponse { jsonrpc: "2.0".to_string(), id, result: None, error: Some(RpcError { code, message }) }
    }
}

// ── Tool definitions (shared) ──────────────────────────────────────────────────

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "insert_pfo",
                "description": "Insert a new Probabilistic Fact Object (PFO) into the database. The engine resolves the source name to a UUID automatically, registering it if unknown. Returns the PFO ID, confidence, and resolved source_id.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "claim_text": { "type": "string", "description": "The factual claim to store" },
                        "confidence":  { "type": "number", "description": "Initial confidence 0.0–1.0" },
                        "source":      { "type": "string", "description": "Source name — engine resolves to UUID." },
                        "domain":      { "type": "string", "description": "EU AI Act domain.", "default": "CriticalInfrastructure" }
                    },
                    "required": ["claim_text", "confidence", "source"]
                }
            },
            {
                "name": "get_pfo",
                "description": "Retrieve a PFO by its UUID.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "id": { "type": "string", "description": "PFO UUID" } },
                    "required": ["id"]
                }
            },
            {
                "name": "search",
                "description": "Semantic search for PFOs by natural language query.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Natural language search query" },
                        "k":     { "type": "integer", "description": "Number of results (default 5)", "default": 5 }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "get_source",
                "description": "Look up a source by name. Returns UUID, effective weight, corroboration count, conflict count.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "name": { "type": "string", "description": "Source name" } },
                    "required": ["name"]
                }
            },
            {
                "name": "register_source",
                "description": "Register a new source by name. Idempotent — returns existing if already registered.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name":   { "type": "string", "description": "Source name" },
                        "domain": { "type": "string", "description": "Optional domain hint" }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "update_confidence",
                "description": "Manually adjust a PFO confidence. Requires a documented reason for EU AI Act audit trail.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id":         { "type": "string", "description": "PFO UUID" },
                        "confidence": { "type": "number", "description": "New confidence 0.0–1.0" },
                        "reason":     { "type": "string", "description": "Documented reason (required for audit)" }
                    },
                    "required": ["id", "confidence", "reason"]
                }
            }
        ]
    })
}

// ── HTTP helpers (shared) ──────────────────────────────────────────────────────

async fn http_get(client: &reqwest::Client, path: &str) -> anyhow::Result<Value> {
    let url = format!("{}{}", ENGINE_URL, path);
    mcp_log_info!("[tekmerdb-mcp] GET {}", url);
    let resp = client.get(&url).send().await?;
    let status = resp.status();
    let body: Value = resp.json().await?;
    if !status.is_success() { anyhow::bail!("engine returned {}: {}", status, body); }
    Ok(body)
}

async fn http_post(client: &reqwest::Client, path: &str, body: &Value) -> anyhow::Result<Value> {
    let url = format!("{}{}", ENGINE_URL, path);
    mcp_log_info!("[tekmerdb-mcp] POST {}", url);
    let resp = client.post(&url).json(body).send().await?;
    let status = resp.status();
    let resp_body: Value = resp.json().await?;
    if !status.is_success() { anyhow::bail!("engine returned {}: {}", status, resp_body); }
    Ok(resp_body)
}

async fn http_patch(client: &reqwest::Client, path: &str, body: &Value) -> anyhow::Result<Value> {
    let url = format!("{}{}", ENGINE_URL, path);
    mcp_log_info!("[tekmerdb-mcp] PATCH {}", url);
    let resp = client.patch(&url).json(body).send().await?;
    let status = resp.status();
    let resp_body: Value = resp.json().await?;
    if !status.is_success() { anyhow::bail!("engine returned {}: {}", status, resp_body); }
    Ok(resp_body)
}

// ── Tool dispatch (shared) ─────────────────────────────────────────────────────

async fn dispatch_tool(client: &reqwest::Client, tool_name: &str, args: &Value) -> anyhow::Result<Value> {
    match tool_name {
        "insert_pfo" => {
            let body = json!({
                "claim_text": args["claim_text"].as_str().ok_or_else(|| anyhow::anyhow!("claim_text required"))?,
                "confidence": args["confidence"].as_f64().ok_or_else(|| anyhow::anyhow!("confidence required"))?,
                "source":     args["source"].as_str().ok_or_else(|| anyhow::anyhow!("source required"))?,
                "domain":     args["domain"].as_str().unwrap_or("CriticalInfrastructure")
            });
            http_post(client, "/pfo", &body).await
        }
        "get_pfo" => {
            let id = args["id"].as_str().ok_or_else(|| anyhow::anyhow!("id required"))?;
            http_get(client, &format!("/pfo/{}", id)).await
        }
        "search" => {
            let query = args["query"].as_str().ok_or_else(|| anyhow::anyhow!("query required"))?;
            let k = args["k"].as_u64().unwrap_or(5);
            http_get(client, &format!("/search?q={}&k={}", urlencoding_encode(query), k)).await
        }
        "get_source" => {
            let name = args["name"].as_str().ok_or_else(|| anyhow::anyhow!("name required"))?;
            http_get(client, &format!("/source?name={}", urlencoding_encode(name))).await
        }
        "register_source" => {
            let name = args["name"].as_str().ok_or_else(|| anyhow::anyhow!("name required"))?;
            let mut body = json!({ "name": name });
            if let Some(domain) = args["domain"].as_str() { body["domain"] = json!(domain); }
            http_post(client, "/source", &body).await
        }
        "update_confidence" => {
            let id = args["id"].as_str().ok_or_else(|| anyhow::anyhow!("id required"))?;
            let body = json!({
                "confidence": args["confidence"].as_f64().ok_or_else(|| anyhow::anyhow!("confidence required"))?,
                "reason":     args["reason"].as_str().ok_or_else(|| anyhow::anyhow!("reason required"))?
            });
            http_patch(client, &format!("/pfo/{}/confidence", id), &body).await
        }
        _ => anyhow::bail!("unknown tool: {}", tool_name),
    }
}

fn urlencoding_encode(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        ' ' => "%20".to_string(),
        _ => format!("%{:02X}", c as u32),
    }).collect()
}

// ── Request handler (shared) ───────────────────────────────────────────────────

async fn handle_request(client: &reqwest::Client, req: RpcRequest) -> RpcResponse {
    let id = req.id.clone().unwrap_or(Value::Null);

    match req.method.as_str() {
        "initialize" => RpcResponse::ok(id, json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "tekmerdb-mcp", "version": "0.1.0" }
        })),
        "notifications/initialized" => RpcResponse::ok(id, json!({})),
        "tools/list" => RpcResponse::ok(id, tools_list()),
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
            mcp_log_info!("[tekmerdb-mcp] tool call: {} args: {}", tool_name, args);
            match dispatch_tool(client, tool_name, &args).await {
                Ok(result) => {
                    mcp_log_info!("[tekmerdb-mcp] tool result: {}", result);
                    RpcResponse::ok(id, json!({
                        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()) }]
                    }))
                }
                Err(e) => {
                    mcp_log_error!("[tekmerdb-mcp] tool error: {}", e);
                    RpcResponse::err(id, -32000, e.to_string())
                }
            }
        }
        other => {
            mcp_log_info!("[tekmerdb-mcp] unknown method: {}", other);
            RpcResponse::err(id, -32601, format!("method not found: {}", other))
        }
    }
}

// ── stdio transport ────────────────────────────────────────────────────────────

pub async fn run_stdio() {
    let client = reqwest::Client::new();
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();

    mcp_log_info!("[tekmerdb-mcp] stdio ready — reading from stdin");

    while let Ok(Some(line)) = reader.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() { continue; }

        mcp_log_info!("[tekmerdb-mcp] received: {}", line);

        let req: RpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                mcp_log_error!("[tekmerdb-mcp] parse error: {}", e);
                let err = RpcResponse::err(Value::Null, -32700, format!("parse error: {}", e));
                let mut out = serde_json::to_string(&err).unwrap();
                out.push('\n');
                stdout.write_all(out.as_bytes()).await.unwrap();
                stdout.flush().await.unwrap();
                continue;
            }
        };

        if req.method.starts_with("notifications/") {
            mcp_log_info!("[tekmerdb-mcp] notification: {}", req.method);
            continue;
        }

        let resp = handle_request(&client, req).await;
        let mut out = serde_json::to_string(&resp).unwrap();
        out.push('\n');
        mcp_log_info!("[tekmerdb-mcp] sending: {}", out.trim());
        stdout.write_all(out.as_bytes()).await.unwrap();
        stdout.flush().await.unwrap();
    }

    mcp_log_info!("[tekmerdb-mcp] stdin closed — exiting");
}

// ── SSE transport ──────────────────────────────────────────────────────────────
//
// MCP's SSE transport is two endpoints working together, not one:
//   GET  /sse      — long-lived stream. First event tells the client where to
//                    POST (`endpoint`); after that, every JSON-RPC response
//                    for this client arrives here as a `message` event.
//   POST /message  — the client posts requests here. The reply is *not* the
//                    POST's response body — it's delivered over that client's
//                    own open /sse stream instead, which is why the endpoint
//                    URI carries a per-connection sessionId: it's how
//                    /message knows which open stream to push the reply onto.
//
// (The previous version returned the reply directly as the POST's body and
// closed the /sse stream after its single `endpoint` event -- so any
// spec-compliant client opened /sse and sat waiting forever for a reply that
// could never arrive there. Confirmed via a raw protocol trace: `curl /sse`
// showed the stream ending immediately, and `curl -X POST /message` returned
// the full JSON-RPC result directly instead of a bare 202.)

type SessionMap = Mutex<HashMap<String, mpsc::UnboundedSender<Event>>>;

struct SseState {
    client: reqwest::Client,
    sessions: SessionMap,
}

pub async fn run_sse(host: String, port: u16) {
    let state = Arc::new(SseState {
        client: reqwest::Client::new(),
        sessions: Mutex::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/message", axum::routing::post(message_handler))
        .with_state(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await
        .expect("failed to bind SSE listener");

    mcp_log_info!("[tekmerdb-mcp] SSE server listening on http://{}", addr);
    mcp_log_info!("[tekmerdb-mcp] SSE endpoint: http://{}/sse", addr);
    mcp_log_info!("[tekmerdb-mcp] message endpoint: http://{}/message", addr);

    axum::serve(listener, app).await
        .expect("SSE server error");
}

// GET /sse — agent connects here; the stream stays open for the session's
// lifetime, delivering this client's JSON-RPC responses as they're ready.
async fn sse_handler(
    State(state): State<Arc<SseState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let session_id = Uuid::new_v4().to_string();
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    state.sessions.lock().unwrap().insert(session_id.clone(), tx);

    mcp_log_info!("[tekmerdb-mcp] SSE client connected — session {}", session_id);

    let endpoint = stream::once(async move {
        Ok(Event::default()
            .event("endpoint")
            .data(format!("/message?sessionId={}", session_id)))
    });
    let messages = stream::poll_fn(move |cx| rx.poll_recv(cx).map(|opt| opt.map(Ok)));

    Sse::new(endpoint.chain(messages)).keep_alive(axum::response::sse::KeepAlive::default())
}

#[derive(Debug, Deserialize)]
struct MessageQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
}

// POST /message — agent posts JSON-RPC requests here. The reply is delivered
// asynchronously over that session's open /sse stream, not this response —
// this just acks receipt.
async fn message_handler(
    State(state): State<Arc<SseState>>,
    Query(q): Query<MessageQuery>,
    Json(req): Json<RpcRequest>,
) -> StatusCode {
    mcp_log_info!("[tekmerdb-mcp] SSE message received: {} (session {})", req.method, q.session_id);

    if req.method.starts_with("notifications/") {
        mcp_log_info!("[tekmerdb-mcp] notification: {}", req.method);
        return StatusCode::ACCEPTED;
    }

    let resp = handle_request(&state.client, req).await;
    mcp_log_info!("[tekmerdb-mcp] SSE response: {}", serde_json::to_string(&resp).unwrap_or_default());

    let event = Event::default()
        .event("message")
        .data(serde_json::to_string(&resp).unwrap_or_default());

    let mut sessions = state.sessions.lock().unwrap();
    let delivered = sessions.get(&q.session_id).is_some_and(|tx| tx.send(event).is_ok());
    if !delivered {
        mcp_log_error!("[tekmerdb-mcp] session {} gone — dropping response", q.session_id);
        sessions.remove(&q.session_id);
    }

    StatusCode::ACCEPTED
}