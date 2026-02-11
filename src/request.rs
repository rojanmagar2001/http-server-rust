use std::net::SocketAddr;

use anyhow::{Context, Result, bail};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    net::TcpStream,
};

pub type Key = String;
pub type Value = String;

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub http_version: String,
    pub headers: Vec<(Key, Value)>,
    pub body: Option<Vec<u8>>,
    // optionally expose peer addr for logging or handlers
    pub peer_addr: Option<SocketAddr>,
}

impl Request {
    /// Asynchronously parse a Request from a TcpStream.
    /// Returns the parsed Request and the original TcpStream back (ready for writing).
    pub async fn from_stream(stream: TcpStream) -> Result<(Self, TcpStream)> {
        // Use BufReader to read lines and then read the body bytes
        let peer_addr = stream.peer_addr().ok();
        let mut reader = BufReader::new(stream);

        // Read request line
        let mut request_line = String::new();
        let n = reader
            .read_line(&mut request_line)
            .await
            .context("reading request line")?;
        if n == 0 {
            bail!("connection closed before request line");
        }

        let parts: Vec<&str> = request_line
            .trim_end_matches(|c| c == '\r' || c == '\n')
            .split_whitespace()
            .collect();
        let (method, path, http_version) = match parts.as_slice() {
            [m, p, v] => (m.to_string(), p.to_string(), v.to_string()),
            _ => bail!("invalid request line: {}", request_line.trim_end()),
        };

        // Read headers
        let mut headers: Vec<(Key, Value)> = Vec::new();
        loop {
            let mut header_line = String::new();
            let n = reader
                .read_line(&mut header_line)
                .await
                .context("reading header line")?;
            if n == 0 {
                // EOF reached
                break;
            }
            let header_line_trimmed = header_line.trim_end_matches(|c| c == '\r' || c == '\n');
            if header_line_trimmed.is_empty() {
                // End of headers
                break;
            }
            if let Some((k, v)) = header_line_trimmed.split_once(':') {
                headers.push((k.trim().to_string(), v.trim().to_string()));
            } else {
                bail!("malformed header line: {}", header_line_trimmed);
            }
        }

        // Find Content-Length header if present (case-insensitive)
        let mut content_length: Option<usize> = None;
        for (k, v) in &headers {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = Some(
                    v.trim()
                        .parse::<usize>()
                        .context("parsing Content-Length header")?,
                );
                break;
            }
        }

        // Read body if needed
        let body = if let Some(len) = content_length {
            if len > 0 {
                let mut buf = vec![0u8; len];
                reader
                    .read_exact(&mut buf)
                    .await
                    .context("reading request body")?;
                Some(buf)
            } else {
                None
            }
        } else {
            None
        };

        // Retrieve the original TcpStream back from the reader
        let stream = reader.into_inner();

        Ok((
            Request {
                method,
                path,
                http_version,
                headers,
                body,
                peer_addr,
            },
            stream,
        ))
    }

    /// Helper to look up header value case-insensitively
    pub fn header_value(&self, name: &str) -> Option<&str> {
        for (k, v) in &self.headers {
            if k.eq_ignore_ascii_case(name) {
                return Some(v.as_str());
            }
        }
        None
    }
}
