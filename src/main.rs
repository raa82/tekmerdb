mod engine;
mod storage;
mod api;

use std::sync::{Arc, Mutex};
use engine::engine::Engine;
use api::router;

#[tokio::main]
async fn main() {
    println!("pfodb starting...");

    let engine = Engine::new("data/crb.bin").expect("failed to start engine");
    let state = Arc::new(Mutex::new(engine));

    let app = router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("pfodb listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}