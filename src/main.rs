use std::{env, path::PathBuf};

use anyhow::Result;
use log::info;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    info!("Starting http-server-rust (tokio)");

    let addr = env::var("ADDR").unwrap_or_else(|_| "127.0.0.1:4221".to_string());

    let files_dir = parse_directory_arg().unwrap_or_else(|| PathBuf::from("/tmp"));

    http_server_rust::server::run(&addr, files_dir).await
}

/// Parse `--directory <path>` from command-line arguments.
fn parse_directory_arg() -> Option<PathBuf> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--directory" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}
