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
    pub peer_addr: Option<SocketAddr>,
}

/// Trim trailing CR/LF characters from a line read from the network.
#[inline]
fn trim_line_ending(s: &str) -> &str {
    s.trim_end_matches(['\r', '\n'])
}

impl Request {
    /// Asynchronously parse a Request from a TcpStream.
    /// Returns the parsed Request and the original TcpStream back (ready for writing).
    pub async fn from_stream(stream: TcpStream) -> Result<(Self, TcpStream)> {
        let peer_addr = stream.peer_addr().ok();
        let mut reader = BufReader::new(stream);

        // Read and parse the request line
        let (method, path, http_version) = Self::read_request_line(&mut reader).await?;

        // Read headers
        let headers = Self::read_headers(&mut reader).await?;

        // Build a partial request so we can use header_value() for Content-Length
        let mut request = Self {
            method,
            path,
            http_version,
            headers,
            body: None,
            peer_addr,
        };

        // Read body based on Content-Length header (if present)
        let content_length = request
            .header_value("Content-Length")
            .map(|v| v.parse::<usize>())
            .transpose()
            .context("parsing Content-Length header")?;

        if let Some(len) = content_length.filter(|&len| len > 0) {
            let mut buf = vec![0u8; len];
            reader
                .read_exact(&mut buf)
                .await
                .context("reading request body")?;
            request.body = Some(buf);
        }

        let stream = reader.into_inner();
        Ok((request, stream))
    }

    /// Read and parse the HTTP request line (e.g. "GET / HTTP/1.1").
    async fn read_request_line(
        reader: &mut BufReader<TcpStream>,
    ) -> Result<(String, String, String)> {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .context("reading request line")?;
        if n == 0 {
            bail!("connection closed before request line");
        }

        let trimmed = trim_line_ending(&line);
        match trimmed.split_whitespace().collect::<Vec<_>>().as_slice() {
            [method, path, version] => {
                Ok((method.to_string(), path.to_string(), version.to_string()))
            }
            _ => bail!("invalid request line: {}", trimmed),
        }
    }

    /// Read all HTTP headers until the blank line delimiter.
    async fn read_headers(reader: &mut BufReader<TcpStream>) -> Result<Vec<(Key, Value)>> {
        let mut headers = Vec::new();
        let mut line = String::new();

        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .context("reading header line")?;
            if n == 0 {
                break; // EOF
            }

            let trimmed = trim_line_ending(&line);
            if trimmed.is_empty() {
                break; // End of headers
            }

            let (key, value) = trimmed
                .split_once(':')
                .with_context(|| format!("malformed header line: {}", trimmed))?;

            headers.push((key.trim().to_string(), value.trim().to_string()));
        }

        Ok(headers)
    }

    /// Look up a header value by name (case-insensitive).
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    // ── Helper ───────────────────────────────────────────────────────

    /// Create a TcpStream whose read-half contains `data`.
    ///
    /// Binds a listener on a random port, spawns a task that accepts a
    /// connection and writes `data`, then connects from the client side
    /// and returns the client stream (which will receive the data).
    async fn stream_from_bytes(data: &[u8]) -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let owned = data.to_vec();

        tokio::spawn(async move {
            let (mut server, _) = listener.accept().await.unwrap();
            server.write_all(&owned).await.unwrap();
            server.shutdown().await.unwrap();
        });

        TcpStream::connect(addr).await.unwrap()
    }

    // ── trim_line_ending ─────────────────────────────────────────────

    #[test]
    fn test_trim_line_ending_crlf() {
        assert_eq!(trim_line_ending("hello\r\n"), "hello");
    }

    #[test]
    fn test_trim_line_ending_lf() {
        assert_eq!(trim_line_ending("hello\n"), "hello");
    }

    #[test]
    fn test_trim_line_ending_cr() {
        assert_eq!(trim_line_ending("hello\r"), "hello");
    }

    #[test]
    fn test_trim_line_ending_none() {
        assert_eq!(trim_line_ending("hello"), "hello");
    }

    #[test]
    fn test_trim_line_ending_empty() {
        assert_eq!(trim_line_ending(""), "");
    }

    #[test]
    fn test_trim_line_ending_only_endings() {
        assert_eq!(trim_line_ending("\r\n"), "");
    }

    #[test]
    fn test_trim_line_ending_multiple_trailing() {
        assert_eq!(trim_line_ending("data\r\n\r\n"), "data");
    }

    // ── header_value ─────────────────────────────────────────────────

    fn make_request_with_headers(headers: Vec<(String, String)>) -> Request {
        Request {
            method: "GET".to_string(),
            path: "/".to_string(),
            http_version: "HTTP/1.1".to_string(),
            headers,
            body: None,
            peer_addr: None,
        }
    }

    #[test]
    fn test_header_value_exact_match() {
        let req = make_request_with_headers(vec![("Content-Type".into(), "text/plain".into())]);
        assert_eq!(req.header_value("Content-Type"), Some("text/plain"));
    }

    #[test]
    fn test_header_value_case_insensitive() {
        let req = make_request_with_headers(vec![("content-type".into(), "text/html".into())]);
        assert_eq!(req.header_value("Content-Type"), Some("text/html"));
        assert_eq!(req.header_value("CONTENT-TYPE"), Some("text/html"));
        assert_eq!(req.header_value("content-type"), Some("text/html"));
    }

    #[test]
    fn test_header_value_missing() {
        let req = make_request_with_headers(vec![("Content-Type".into(), "text/plain".into())]);
        assert_eq!(req.header_value("Authorization"), None);
    }

    #[test]
    fn test_header_value_empty_headers() {
        let req = make_request_with_headers(vec![]);
        assert_eq!(req.header_value("Anything"), None);
    }

    #[test]
    fn test_header_value_returns_first_match() {
        let req = make_request_with_headers(vec![
            ("X-Custom".into(), "first".into()),
            ("x-custom".into(), "second".into()),
        ]);
        // find() returns the first match
        assert_eq!(req.header_value("X-Custom"), Some("first"));
    }

    // ── from_stream: valid requests ──────────────────────────────────

    #[tokio::test]
    async fn test_from_stream_simple_get() {
        let raw = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let stream = stream_from_bytes(raw).await;

        let (req, _stream) = Request::from_stream(stream).await.unwrap();

        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/");
        assert_eq!(req.http_version, "HTTP/1.1");
        assert_eq!(req.headers.len(), 1);
        assert_eq!(req.header_value("Host"), Some("localhost"));
        assert!(req.body.is_none());
    }

    #[tokio::test]
    async fn test_from_stream_get_with_path() {
        let raw = b"GET /echo/hello HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let stream = stream_from_bytes(raw).await;

        let (req, _) = Request::from_stream(stream).await.unwrap();

        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/echo/hello");
        assert_eq!(req.http_version, "HTTP/1.1");
    }

    #[tokio::test]
    async fn test_from_stream_multiple_headers() {
        let raw = b"GET / HTTP/1.1\r\n\
                     Host: example.com\r\n\
                     User-Agent: test/1.0\r\n\
                     Accept: */*\r\n\
                     X-Custom: value\r\n\
                     \r\n";
        let stream = stream_from_bytes(raw).await;

        let (req, _) = Request::from_stream(stream).await.unwrap();

        assert_eq!(req.headers.len(), 4);
        assert_eq!(req.header_value("Host"), Some("example.com"));
        assert_eq!(req.header_value("User-Agent"), Some("test/1.0"));
        assert_eq!(req.header_value("Accept"), Some("*/*"));
        assert_eq!(req.header_value("X-Custom"), Some("value"));
    }

    #[tokio::test]
    async fn test_from_stream_post_with_body() {
        let body = b"hello world";
        let raw = format!(
            "POST /data HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap(),
        );
        let stream = stream_from_bytes(raw.as_bytes()).await;

        let (req, _) = Request::from_stream(stream).await.unwrap();

        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/data");
        assert_eq!(req.header_value("Content-Length"), Some("11"));
        assert_eq!(req.body.as_deref(), Some(b"hello world".as_slice()));
    }

    #[tokio::test]
    async fn test_from_stream_post_binary_body() {
        let body: Vec<u8> = vec![0x00, 0xFF, 0x0A, 0x0D, 0xBE, 0xEF];
        let mut raw = format!(
            "POST /upload HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        raw.extend_from_slice(&body);
        let stream = stream_from_bytes(&raw).await;

        let (req, _) = Request::from_stream(stream).await.unwrap();

        assert_eq!(req.method, "POST");
        assert_eq!(req.body.as_deref(), Some(body.as_slice()));
    }

    #[tokio::test]
    async fn test_from_stream_content_length_zero() {
        let raw = b"POST /empty HTTP/1.1\r\nContent-Length: 0\r\n\r\n";
        let stream = stream_from_bytes(raw).await;

        let (req, _) = Request::from_stream(stream).await.unwrap();

        assert_eq!(req.method, "POST");
        // Content-Length: 0 should result in no body
        assert!(req.body.is_none());
    }

    #[tokio::test]
    async fn test_from_stream_no_content_length_means_no_body() {
        let raw = b"GET /page HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let stream = stream_from_bytes(raw).await;

        let (req, _) = Request::from_stream(stream).await.unwrap();

        assert!(req.body.is_none());
    }

    #[tokio::test]
    async fn test_from_stream_content_length_case_insensitive() {
        let raw = b"POST /data HTTP/1.1\r\ncontent-length: 3\r\n\r\nabc";
        let stream = stream_from_bytes(raw).await;

        let (req, _) = Request::from_stream(stream).await.unwrap();

        assert_eq!(req.body.as_deref(), Some(b"abc".as_slice()));
    }

    #[tokio::test]
    async fn test_from_stream_header_with_colon_in_value() {
        let raw = b"GET / HTTP/1.1\r\nHost: localhost:8080\r\n\r\n";
        let stream = stream_from_bytes(raw).await;

        let (req, _) = Request::from_stream(stream).await.unwrap();

        // split_once(':') should keep everything after first colon as the value
        assert_eq!(req.header_value("Host"), Some("localhost:8080"));
    }

    #[tokio::test]
    async fn test_from_stream_no_headers() {
        let raw = b"GET / HTTP/1.1\r\n\r\n";
        let stream = stream_from_bytes(raw).await;

        let (req, _) = Request::from_stream(stream).await.unwrap();

        assert_eq!(req.method, "GET");
        assert!(req.headers.is_empty());
        assert!(req.body.is_none());
    }

    #[tokio::test]
    async fn test_from_stream_lf_only_line_endings() {
        // Some clients send LF-only instead of CRLF
        let raw = b"GET /lf HTTP/1.1\nHost: localhost\n\n";
        let stream = stream_from_bytes(raw).await;

        let (req, _) = Request::from_stream(stream).await.unwrap();

        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/lf");
        assert_eq!(req.header_value("Host"), Some("localhost"));
    }

    // ── from_stream: error cases ─────────────────────────────────────

    #[tokio::test]
    async fn test_from_stream_empty_stream() {
        let stream = stream_from_bytes(b"").await;
        let result = Request::from_stream(stream).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("connection closed"),
            "expected 'connection closed' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_from_stream_invalid_request_line_one_part() {
        let raw = b"INVALID\r\n\r\n";
        let stream = stream_from_bytes(raw).await;
        let result = Request::from_stream(stream).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid request line"),
            "expected 'invalid request line' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_from_stream_invalid_request_line_two_parts() {
        let raw = b"GET /\r\n\r\n";
        let stream = stream_from_bytes(raw).await;
        let result = Request::from_stream(stream).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid request line"));
    }

    #[tokio::test]
    async fn test_from_stream_malformed_header() {
        let raw = b"GET / HTTP/1.1\r\nBadHeaderNoColon\r\n\r\n";
        let stream = stream_from_bytes(raw).await;
        let result = Request::from_stream(stream).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("malformed header"),
            "expected 'malformed header' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_from_stream_invalid_content_length() {
        let raw = b"POST /data HTTP/1.1\r\nContent-Length: abc\r\n\r\n";
        let stream = stream_from_bytes(raw).await;
        let result = Request::from_stream(stream).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Content-Length"),
            "expected Content-Length parse error, got: {}",
            err
        );
    }

    // ── from_stream: stream is returned for writing ──────────────────

    #[tokio::test]
    async fn test_from_stream_returns_writable_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (req, mut write_stream) = Request::from_stream(stream).await.unwrap();
            assert_eq!(req.method, "GET");

            // Write a response back through the returned stream
            write_stream
                .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
                .await
                .unwrap();
            write_stream.shutdown().await.unwrap();
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: test\r\n\r\n")
            .await
            .unwrap();

        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.unwrap();
        server.await.unwrap();

        let response = String::from_utf8(buf).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
    }
}
