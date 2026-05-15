mod server;
mod mcp_logger;

#[tokio::main]
async fn main() {
    mcp_logger::init();
    mcp_log_info!("[tekmerdb-mcp] starting — connecting to tekmerdb on http://localhost:3000");
    server::run().await;
}