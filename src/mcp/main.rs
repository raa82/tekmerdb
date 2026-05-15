mod server;

#[tokio::main]
async fn main() {
    eprintln!("[pfodb-mcp] starting — connecting to pfodb on http://localhost:3000");
    server::run().await;
}