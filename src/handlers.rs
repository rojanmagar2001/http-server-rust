use anyhow::{Context, Result};
use log::{debug, error};
use tokio::net::TcpStream;

use crate::{request::Request, response::Response};

pub async fn handle_request(stream: TcpStream) -> Result<()> {
    debug!("accepted new connection");

    // Parse request and get the stream back for writing
    let (request, mut stream) = Request::from_stream(stream)
        .await
        .context("parsing request")?;

    debug!(
        "{} {} from {:?}",
        request.method, request.path, request.peer_addr
    );

    let response = if request.path == "/" {
        debug!("root path requested");
        Response::ok_text("")
    } else if request.path.starts_with("/user-agent") {
        match request.header_value("User-Agent") {
            None => {
                error!("User-Agent header not found in request: {:?}", request);
                Response::not_found()
            }
            Some(ua) => {
                let r = Response::ok_text(ua);
                r
            }
        }
    } else if request.path.starts_with("/echo/") {
        if let Some((_, echoed)) = request.path.split_once("/echo/") {
            debug!("echo path requested: {}", echoed);
            Response::ok_text(echoed)
        } else {
            error!("invalid echo path: {}", request.path);
            Response::not_found()
        }
    } else {
        debug!("unknown path: {}", request.path);
        Response::not_found()
    };

    response
        .write_to(&mut stream)
        .await
        .context("writing response")?;
    Ok(())
}
