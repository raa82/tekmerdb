mod server;
mod mcp_logger;

#[tokio::main]
async fn main() {
    mcp_logger::init();

    // detect --sse flag
    let sse_mode = std::env::args().any(|a| a == "--sse");

    if sse_mode {
        // read mcp_host and mcp_port from config file
        let (host, port) = load_mcp_config();
        mcp_log_info!("[tekmerdb-mcp] SSE mode — listening on {}:{}", host, port);
        server::run_sse(host, port).await;
    } else {
        mcp_log_info!("[tekmerdb-mcp] stdio mode — connecting to tekmerdb on http://localhost:3000");
        server::run_stdio().await;
    }
}

fn load_mcp_config() -> (String, u16) {
    let default_host = "0.0.0.0".to_string();
    let default_port: u16 = 3001;

    let content = match std::fs::read_to_string("tekmerdb-server.conf") {
        Ok(c) => c,
        Err(_) => return (default_host, default_port),
    };

    let table: toml::Table = match toml::from_str(&content) {
        Ok(t) => t,
        Err(_) => return (default_host, default_port),
    };

    let host = table.get("mcp_host")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0.0")
        .to_string();

    let port = table.get("mcp_port")
        .and_then(|v| v.as_integer())
        .unwrap_or(3001) as u16;

    (host, port)
}