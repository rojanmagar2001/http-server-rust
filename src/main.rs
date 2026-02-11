use std::{env, path::PathBuf};

use anyhow::Result;
use log::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();
    info!("Starting http-server-rust (tokio)");

    // Read address from environment or use default
    info!("Starting http-server-rust (tokio)");

    // Read address from environment or use default
    let addr = env::var("ADDR").unwrap_or_else(|_| "127.0.0.1:4221".to_string());

    // Parse command line args for --directory <path>
    let mut args = env::args().skip(1);
    let mut files_dir: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        if arg == "--directory" {
            if let Some(val) = args.next() {
                files_dir = Some(PathBuf::from(val))
            }
        }
    }

    let files_dir = files_dir.unwrap_or_else(|| PathBuf::from("/tmp"));

    // Run the server (blocking)
    http_server_rust::server::run(&addr, files_dir).await?;

    Ok(())
}
