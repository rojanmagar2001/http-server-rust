use anyhow::Result;
use log::{debug, error};
use std::{io::Write, net::TcpListener};

fn main() -> Result<()> {
    env_logger::init();

    let listener = TcpListener::bind("127.0.0.1:4221").unwrap();

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                debug!("accepted new connection");

                let response = "HTTP/1.1 200 OK\r\n\r\n";

                stream.write(response.as_bytes())?;
            }
            Err(e) => {
                error!("error: {}", e);
            }
        }
    }

    Ok(())
}
