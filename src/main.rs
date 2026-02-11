use std::env;

use anyhow::Result;
use log::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();
    info!("Starting http-server-rust");

    // Read address from environment or use default
    info!("Starting http-server-rust (tokio)");

    // Read address from environment or use default
    let addr = env::var("ADDR").unwrap_or_else(|_| "127.0.0.1:4221".to_string());

    // Run the server (blocking)
    http_server_rust::server::run(&addr).await?;

    Ok(())
}
