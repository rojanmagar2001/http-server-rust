use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use log::{error, info};
use tokio::net::TcpListener;

use crate::handlers;

pub async fn run(addr: &str, files_dir: PathBuf) -> Result<()> {
    info!("Binding to {}", addr);
    let listener = TcpListener::bind(addr).await?;

    info!("Server listening on {}", addr);

    // Share directory path with connection tasks
    let files_dir = Arc::new(files_dir);

    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let dir = files_dir.clone();

                // Spawn an independent task per connection
                tokio::spawn(async move {
                    if let Err(e) = handlers::handle_request(stream, dir).await {
                        error!("request handling error: {:?}", e);
                    }
                });
            }
            Err(e) => {
                error!("accept error: {}", e);
            }
        }
    }
}
