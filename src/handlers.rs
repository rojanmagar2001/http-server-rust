use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use log::{debug, error};
use tokio::{
    fs,
    io::{self, AsyncWriteExt},
    net::TcpStream,
};

use crate::{request::Request, response::Response};

pub async fn handle_request(stream: TcpStream, files_dir: Arc<PathBuf>) -> Result<()> {
    debug!("accepted new connection");

    // Parse request and get the stream back for writing
    let (request, mut stream) = Request::from_stream(stream)
        .await
        .context("parsing request")?;

    debug!(
        "{} {} from {:?}",
        request.method, request.path, request.peer_addr
    );

    // Implement /files/{filename}
    if request.path.starts_with("/files/") {
        // Extract filename part
        if let Some((_, filename)) = request.path.split_once("/files/") {
            // Validate filename: it must be a single normal component (no separators, no .., no root/prefix)
            let path = Path::new(filename);
            let mut normal_count = 0usize;
            let mut ok = true;
            for comp in path.components() {
                match comp {
                    Component::Normal(_) => normal_count += 1,
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok || normal_count != 1 {
                // Use Response to produce an exact headerless 404
                Response::status_only(404, "Not Found")
                    .write_to(&mut stream)
                    .await?;
                return Ok(());
            }

            let mut file_path = files_dir.as_ref().clone();
            file_path.push(filename);

            // Check file exists and is a file
            match fs::metadata(&file_path).await {
                Ok(meta) if meta.is_file() => {
                    // Open the file for streaming
                    let mut file = fs::File::open(&file_path).await.context("opening file")?;
                    let file_size = meta.len();

                    // Prepare headers and stream file without buffering whole file
                    let mut resp = Response::new(200, "OK");
                    resp.header("Content-Type", "application/octet-stream");
                    resp.header("Content-Length", &file_size.to_string());

                    // Write headers first
                    resp.write_headers(&mut stream)
                        .await
                        .context("writing file headers")?;

                    // Stream body
                    let bytes_copied = io::copy(&mut file, &mut stream)
                        .await
                        .context("streaming file")?;
                    // Optionally log bytes_copied
                    debug!("streamed {} bytes for file {}", bytes_copied, filename);

                    // flush to be safe
                    stream.flush().await?;

                    return Ok(());
                }
                _ => {
                    // file not found or not a regular file -> exact 404 via Response
                    Response::status_only(404, "Not Found")
                        .write_to(&mut stream)
                        .await?;
                    return Ok(());
                }
            }
        } else {
            // Malformed path (shouldn't really happen), send headerless 404 via Response
            Response::status_only(404, "Not Found")
                .write_to(&mut stream)
                .await?;
            return Ok(());
        }
    }

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
