use std::fmt::Write as FmtWrite;

use anyhow::Result;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[derive(Debug)]
pub struct Response {
    status_code: u16,
    reason: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,

    /// When true, only write the status line and the terminating CRLF CRLF
    /// e.g. "HTTP/1.1 404 Not Found\r\n\r\n"
    status_only: bool,
}

impl Response {
    pub fn new(status_code: u16, reason: &str) -> Self {
        Self {
            status_code,
            reason: reason.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            status_only: false,
        }
    }

    /// Convenience: normal 200 text response.
    pub fn ok_text(body: &str) -> Self {
        let mut r = Self::new(200, "OK");
        r.header("Content-Type", "text/plain")
            .body_bytes(body.as_bytes().to_vec());
        r
    }

    /// Convenience: 404 response with a plain-text body.
    pub fn not_found() -> Self {
        let mut r = Self::new(404, "Not Found");
        r.header("Content-Type", "text/plain")
            .body_bytes(b"Not Found".to_vec());
        r
    }

    /// Construct a "status only" response that will be written exactly as:
    /// `HTTP/1.1 {status} {reason}\r\n\r\n`
    pub fn status_only(status_code: u16, reason: &str) -> Self {
        Self {
            status_code,
            reason: reason.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            status_only: true,
        }
    }

    /// Append a header. Returns `&mut Self` for chaining.
    pub fn header(&mut self, key: &str, value: &str) -> &mut Self {
        self.headers.push((key.to_owned(), value.to_owned()));
        self
    }

    /// Set the response body from raw bytes. Returns `&mut Self` for chaining.
    pub fn body_bytes(&mut self, bytes: Vec<u8>) -> &mut Self {
        self.body = bytes;
        self
    }

    // ── Serialization helpers (shared logic) ─────────────────────────

    /// Write the status line and headers into a pre-allocated `String`,
    /// optionally injecting a `Content-Length` header when one is missing.
    ///
    /// When `include_content_length` is `true` and no explicit
    /// `Content-Length` header exists, `self.body.len()` is used.
    fn write_head(&self, buf: &mut String, include_content_length: bool) {
        // Status line
        let _ = write!(buf, "HTTP/1.1 {} {}\r\n", self.status_code, self.reason);

        if self.status_only {
            buf.push_str("\r\n");
            return;
        }

        // Headers
        let mut has_content_length = false;
        for (k, v) in &self.headers {
            if k.eq_ignore_ascii_case("content-length") {
                has_content_length = true;
            }
            let _ = write!(buf, "{}: {}\r\n", k, v);
        }

        if include_content_length && !has_content_length {
            let _ = write!(buf, "Content-Length: {}\r\n", self.body.len());
        }

        // Blank line terminates headers
        buf.push_str("\r\n");
    }

    /// Build full response (headers + body) as bytes.
    pub(crate) fn build_raw(&self) -> Vec<u8> {
        // Estimate: status + headers + body
        let estimated = 128 + self.headers.len() * 48 + self.body.len();
        let mut head = String::with_capacity(estimated);
        self.write_head(&mut head, true);

        let mut raw = head.into_bytes();
        if !self.status_only {
            raw.extend_from_slice(&self.body);
        }
        raw
    }

    /// Build header-only bytes (status line + headers + blank line).
    pub(crate) fn build_headers_raw(&self) -> Vec<u8> {
        let mut head = String::with_capacity(128 + self.headers.len() * 48);
        self.write_head(&mut head, true);
        head.into_bytes()
    }

    // ── Public write methods ─────────────────────────────────────────

    /// Write full response (headers + body) to the stream.
    pub async fn write_to(&self, stream: &mut TcpStream) -> Result<()> {
        let raw = self.build_raw();
        stream.write_all(&raw).await?;
        stream.flush().await?;
        Ok(())
    }

    /// Write only the headers (status line + headers + blank line) to the stream.
    /// Useful when the body will be streamed separately (e.g. from a file).
    pub async fn write_headers(&self, stream: &mut TcpStream) -> Result<()> {
        let raw = self.build_headers_raw();
        stream.write_all(&raw).await?;
        stream.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Constructor tests ────────────────────────────────────────────

    #[test]
    fn test_new_defaults() {
        let r = Response::new(200, "OK");
        assert_eq!(r.status_code, 200);
        assert_eq!(r.reason, "OK");
        assert!(r.headers.is_empty());
        assert!(r.body.is_empty());
        assert!(!r.status_only);
    }

    #[test]
    fn test_ok_text() {
        let r = Response::ok_text("hello");
        assert_eq!(r.status_code, 200);
        assert_eq!(r.reason, "OK");
        assert_eq!(r.body, b"hello");
        assert!(!r.status_only);
        // Should have Content-Type header
        assert!(
            r.headers
                .iter()
                .any(|(k, v)| k == "Content-Type" && v == "text/plain")
        );
    }

    #[test]
    fn test_ok_text_empty_body() {
        let r = Response::ok_text("");
        assert_eq!(r.status_code, 200);
        assert!(r.body.is_empty());
    }

    #[test]
    fn test_not_found() {
        let r = Response::not_found();
        assert_eq!(r.status_code, 404);
        assert_eq!(r.reason, "Not Found");
        assert_eq!(r.body, b"Not Found");
        assert!(
            r.headers
                .iter()
                .any(|(k, v)| k == "Content-Type" && v == "text/plain")
        );
    }

    #[test]
    fn test_status_only() {
        let r = Response::status_only(404, "Not Found");
        assert_eq!(r.status_code, 404);
        assert_eq!(r.reason, "Not Found");
        assert!(r.headers.is_empty());
        assert!(r.body.is_empty());
        assert!(r.status_only);
    }

    // ── Builder chaining tests ───────────────────────────────────────

    #[test]
    fn test_header_chaining() {
        let mut r = Response::new(200, "OK");
        r.header("Content-Type", "text/html")
            .header("X-Custom", "value");

        assert_eq!(r.headers.len(), 2);
        assert_eq!(
            r.headers[0],
            ("Content-Type".to_owned(), "text/html".to_owned())
        );
        assert_eq!(r.headers[1], ("X-Custom".to_owned(), "value".to_owned()));
    }

    #[test]
    fn test_body_bytes_chaining() {
        let mut r = Response::new(200, "OK");
        r.header("Content-Type", "application/octet-stream")
            .body_bytes(vec![1, 2, 3, 4]);

        assert_eq!(r.body, vec![1, 2, 3, 4]);
        assert_eq!(r.headers.len(), 1);
    }

    #[test]
    fn test_body_bytes_replaces_previous() {
        let mut r = Response::new(200, "OK");
        r.body_bytes(b"first".to_vec());
        r.body_bytes(b"second".to_vec());
        assert_eq!(r.body, b"second");
    }

    // ── Serialization: build_raw ─────────────────────────────────────

    #[test]
    fn test_build_raw_simple_200() {
        let r = Response::ok_text("hello");
        let raw = r.build_raw();
        let text = String::from_utf8(raw).unwrap();

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: text/plain\r\n"));
        assert!(text.contains("Content-Length: 5\r\n"));
        // Headers end with blank line, then body
        assert!(text.ends_with("\r\n\r\nhello"));
    }

    #[test]
    fn test_build_raw_404_not_found() {
        let r = Response::not_found();
        let raw = r.build_raw();
        let text = String::from_utf8(raw).unwrap();

        assert!(text.starts_with("HTTP/1.1 404 Not Found\r\n"));
        assert!(text.contains("Content-Type: text/plain\r\n"));
        assert!(text.contains("Content-Length: 9\r\n"));
        assert!(text.ends_with("\r\n\r\nNot Found"));
    }

    #[test]
    fn test_build_raw_status_only() {
        let r = Response::status_only(404, "Not Found");
        let raw = r.build_raw();
        let text = String::from_utf8(raw).unwrap();

        // Exact output: status line + CRLF CRLF, nothing else
        assert_eq!(text, "HTTP/1.1 404 Not Found\r\n\r\n");
    }

    #[test]
    fn test_build_raw_status_only_204() {
        let r = Response::status_only(204, "No Content");
        let raw = r.build_raw();
        let text = String::from_utf8(raw).unwrap();
        assert_eq!(text, "HTTP/1.1 204 No Content\r\n\r\n");
    }

    #[test]
    fn test_build_raw_empty_body_has_zero_content_length() {
        let r = Response::ok_text("");
        let raw = r.build_raw();
        let text = String::from_utf8(raw).unwrap();

        assert!(text.contains("Content-Length: 0\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_build_raw_auto_content_length() {
        // When no explicit Content-Length header, build_raw injects one
        let mut r = Response::new(200, "OK");
        r.body_bytes(b"payload".to_vec());
        let raw = r.build_raw();
        let text = String::from_utf8(raw).unwrap();

        assert!(text.contains("Content-Length: 7\r\n"));
    }

    #[test]
    fn test_build_raw_explicit_content_length_not_duplicated() {
        // When caller explicitly sets Content-Length, it must not be duplicated
        let mut r = Response::new(200, "OK");
        r.header("Content-Length", "42")
            .body_bytes(b"short".to_vec());
        let raw = r.build_raw();
        let text = String::from_utf8(raw).unwrap();

        // Should contain the explicit value, not the auto-calculated one
        assert!(text.contains("Content-Length: 42\r\n"));
        // Should only appear once
        let count = text.matches("Content-Length").count();
        assert_eq!(count, 1, "Content-Length should appear exactly once");
    }

    #[test]
    fn test_build_raw_multiple_headers() {
        let mut r = Response::new(200, "OK");
        r.header("Content-Type", "application/json")
            .header("X-Request-Id", "abc-123")
            .header("Cache-Control", "no-cache")
            .body_bytes(b"{}".to_vec());
        let raw = r.build_raw();
        let text = String::from_utf8(raw).unwrap();

        assert!(text.contains("Content-Type: application/json\r\n"));
        assert!(text.contains("X-Request-Id: abc-123\r\n"));
        assert!(text.contains("Cache-Control: no-cache\r\n"));
        assert!(text.contains("Content-Length: 2\r\n"));
        assert!(text.ends_with("\r\n\r\n{}"));
    }

    #[test]
    fn test_build_raw_binary_body() {
        let mut r = Response::new(200, "OK");
        let binary = vec![0x00, 0xFF, 0x0A, 0x0D];
        r.header("Content-Type", "application/octet-stream")
            .body_bytes(binary.clone());
        let raw = r.build_raw();

        // Find the end of headers
        let header_end = b"\r\n\r\n";
        let pos = raw
            .windows(header_end.len())
            .position(|w| w == header_end)
            .expect("should have header terminator");
        let body_start = pos + header_end.len();
        assert_eq!(&raw[body_start..], &binary);
    }

    // ── Serialization: build_headers_raw ─────────────────────────────

    #[test]
    fn test_build_headers_raw_no_body_included() {
        let r = Response::ok_text("this body should NOT appear");
        let raw = r.build_headers_raw();
        let text = String::from_utf8(raw).unwrap();

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: text/plain\r\n"));
        // Content-Length should reflect the in-memory body length
        assert!(text.contains(&format!(
            "Content-Length: {}\r\n",
            "this body should NOT appear".len()
        )));
        // Must end after the blank line — no body bytes
        assert!(text.ends_with("\r\n\r\n"));
        assert!(!text.contains("this body should NOT appear"));
    }

    #[test]
    fn test_build_headers_raw_status_only() {
        let r = Response::status_only(301, "Moved Permanently");
        let raw = r.build_headers_raw();
        let text = String::from_utf8(raw).unwrap();

        assert_eq!(text, "HTTP/1.1 301 Moved Permanently\r\n\r\n");
    }

    #[test]
    fn test_build_headers_raw_with_explicit_content_length() {
        let mut r = Response::new(200, "OK");
        r.header("Content-Type", "application/octet-stream")
            .header("Content-Length", "1048576");
        let raw = r.build_headers_raw();
        let text = String::from_utf8(raw).unwrap();

        assert!(text.contains("Content-Length: 1048576\r\n"));
        let count = text.matches("Content-Length").count();
        assert_eq!(count, 1);
    }

    // ── Consistency: build_raw vs build_headers_raw ──────────────────

    #[test]
    fn test_headers_portion_matches_between_build_methods() {
        let mut r = Response::new(200, "OK");
        r.header("Content-Type", "text/plain")
            .header("X-Foo", "bar")
            .body_bytes(b"body content".to_vec());

        let full = String::from_utf8(r.build_raw()).unwrap();
        let headers_only = String::from_utf8(r.build_headers_raw()).unwrap();

        // The headers-only output should be a prefix of the full output
        assert!(
            full.starts_with(&headers_only),
            "build_raw should start with the same bytes as build_headers_raw"
        );
    }

    // ── write_to / write_headers (async, via TcpStream) ──────────────

    #[tokio::test]
    async fn test_write_to_stream() {
        use tokio::io::AsyncReadExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let writer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let r = Response::ok_text("streamed");
            r.write_to(&mut stream).await.unwrap();
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.unwrap();
        writer.await.unwrap();

        let text = String::from_utf8(buf).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.ends_with("streamed"));
    }

    #[tokio::test]
    async fn test_write_headers_stream() {
        use tokio::io::AsyncReadExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let writer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut r = Response::new(200, "OK");
            r.header("Content-Type", "application/octet-stream")
                .header("Content-Length", "0");
            r.write_headers(&mut stream).await.unwrap();
            // Explicitly drop / shutdown to signal EOF
            drop(stream);
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.unwrap();
        writer.await.unwrap();

        let text = String::from_utf8(buf).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: application/octet-stream\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }
}
