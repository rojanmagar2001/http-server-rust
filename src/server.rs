use anyhow::Result;
use log::{error, info};
use tokio::net::TcpListener;

use crate::handlers;

pub async fn run(addr: &str) -> Result<()> {
    info!("Binding to {}", addr);
    let listener = TcpListener::bind(addr).await?;

    info!("Server listening on {}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                // Spawn an independent task per connection
                tokio::spawn(async move {
                    if let Err(e) = handlers::handle_request(stream).await {
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
