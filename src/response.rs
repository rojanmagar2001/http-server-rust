use anyhow::Result;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[derive(Debug)]
pub struct Response {
    status_code: u16,
    reason: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,

    // When true, only write the status line and the terminating CRLF CRLF
    // e.g. "HTTP/1.1 404 Not Found\r\n\r\n"
    status_only: bool,
}

impl Response {
    pub fn new(status_code: u16, reason: &str) -> Self {
        Self {
            status_code,
            reason: reason.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            status_only: false,
        }
    }

    /// Convenience: normal 200 text response
    pub fn ok_text(body: &str) -> Self {
        let mut r = Response::new(200, "OK");
        r.header("Content-Type", "text/plain");
        r.body_bytes(body.as_bytes().to_vec());
        r
    }

    /// Convenience: normal 404 with body
    pub fn not_found() -> Self {
        let mut r = Response::new(404, "Not Found");
        r.header("Content-Type", "text/plain");
        r.body_bytes(b"Not Found".to_vec());
        r
    }

    /// Construct a "status only" response that will be written exactly as:
    /// "HTTP/1.1 {status} {reason}\r\n\r\n"
    pub fn status_only(status_code: u16, reason: &str) -> Self {
        Self {
            status_code,
            reason: reason.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            status_only: true,
        }
    }

    pub fn header(&mut self, key: &str, value: &str) {
        self.headers.push((key.to_string(), value.to_string()));
    }

    pub fn body_bytes(&mut self, bytes: Vec<u8>) {
        self.body = bytes;
    }

    /// Build full response (headers + body) as bytes.
    fn build_raw(&self) -> Vec<u8> {
        if self.status_only {
            // exact status line + CRLF CRLF, no headers, no body
            return format!("HTTP/1.1 {} {}\r\n\r\n", self.status_code, self.reason).into_bytes();
        }

        let mut raw = Vec::new();

        // Status line
        raw.extend_from_slice(
            format!("HTTP/1.1 {} {}\r\n", self.status_code, self.reason).as_bytes(),
        );

        // Headers
        let mut has_content_length = false;
        for (k, v) in &self.headers {
            if k.eq_ignore_ascii_case("content-length") {
                has_content_length = true;
            }
            raw.extend_from_slice(format!("{}: {}\r\n", k, v).as_bytes());
        }
        if !has_content_length {
            raw.extend_from_slice(format!("Content-Length: {}\r\n", self.body.len()).as_bytes());
        }

        // End of headers
        raw.extend_from_slice(b"\r\n");

        // Body
        raw.extend_from_slice(&self.body);

        raw
    }

    /// Build header-only bytes (status line + headers + blank line), without the body.
    /// Ensures a Content-Length header is present (to allow streaming the body afterwards).
    fn build_headers_raw(&self) -> Vec<u8> {
        if self.status_only {
            return format!("HTTP/1.1 {} {}\r\n\r\n", self.status_code, self.reason).into_bytes();
        }

        let mut raw = Vec::new();

        // Status line
        raw.extend_from_slice(
            format!("HTTP/1.1 {} {}\r\n", self.status_code, self.reason).as_bytes(),
        );

        // Headers
        let mut has_content_length = false;
        for (k, v) in &self.headers {
            if k.eq_ignore_ascii_case("content-length") {
                has_content_length = true;
            }
            raw.extend_from_slice(format!("{}: {}\r\n", k, v).as_bytes());
        }

        // If the caller provided a body (in-memory), use its length; otherwise
        // leave to caller to set Content-Length header manually (we still set it here
        // to 0 if no body and none provided).
        if !has_content_length {
            raw.extend_from_slice(format!("Content-Length: {}\r\n", self.body.len()).as_bytes());
        }

        // End of headers
        raw.extend_from_slice(b"\r\n");

        raw
    }

    /// Write full response (headers+body) to the stream.
    pub async fn write_to(&self, stream: &mut TcpStream) -> Result<()> {
        let raw = self.build_raw();
        stream.write_all(&raw).await?;
        stream.flush().await?;
        Ok(())
    }

    /// Write only the headers (status line + headers + blank line) to the stream.
    /// Useful when the body will be streamed separately (e.g., from a file).
    pub async fn write_headers(&self, stream: &mut TcpStream) -> Result<()> {
        let raw = self.build_headers_raw();
        stream.write_all(&raw).await?;
        stream.flush().await?;
        Ok(())
    }
}
