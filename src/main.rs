mod engine;
mod storage;
mod api;

use std::sync::{Arc, Mutex};
use engine::engine::Engine;
use engine::config::EngineConfig;
use api::router;

#[tokio::main]
async fn main() {
    // initialise server logger — all output goes to log/server.log
    engine::logger::init("log", "server.log");

    log_info!("[tekmerdb] starting...");

    // load config from file — falls back to defaults if missing
    let config = EngineConfig::load("tekmerdb-server.conf");
    let bind_address = config.bind_address();

    let engine = Engine::new_with_config("data/crb.bin", config)
        .expect("failed to start engine");
    let state = Arc::new(Mutex::new(engine));

    let app = router(state);

    let listener = tokio::net::TcpListener::bind(&bind_address).await.unwrap();
    log_info!("[tekmerdb] listening on http://{}", bind_address);
    axum::serve(listener, app).await.unwrap();
}