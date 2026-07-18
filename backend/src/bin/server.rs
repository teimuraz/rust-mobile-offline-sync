//! Standalone event server. `cargo run -p backend --bin server`
//!
//! Then push/pull with plain HTTP, e.g.:
//!   curl localhost:4000/events            # pull everything
//!   curl -XPOST localhost:4000/events -H 'content-type: application/json' -d '[]'

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:4000";
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    println!("event server listening on http://{addr}");
    axum::serve(listener, backend::server::build_router())
        .await
        .unwrap();
}
