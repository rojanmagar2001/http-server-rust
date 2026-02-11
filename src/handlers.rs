use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use log::{debug, error};
use tokio::{
    fs,
    io::{self, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use crate::{request::Request, response::Response};

/// Top-level connection handler: loops to serve multiple requests on a
/// persistent HTTP/1.1 connection.
pub async fn handle_request(stream: TcpStream, files_dir: Arc<PathBuf>) -> Result<()> {
    debug!("accepted new connection");

    let peer_addr = stream.peer_addr().ok();
    let mut reader = tokio::io::BufReader::new(stream);

    loop {
        // Parse the next request — None means clean EOF (client closed)
        let request = match Request::from_reader(&mut reader, peer_addr).await? {
            Some(req) => req,
            None => {
                debug!("client closed connection");
                break;
            }
        };

        debug!(
            "{} {} from {:?}",
            request.method, request.path, request.peer_addr
        );

        // Check whether the client wants to close after this request
        let should_close = request
            .header_value("Connection")
            .is_some_and(|v| v.eq_ignore_ascii_case("close"));

        let stream = reader.get_mut();
        let response = route(&request, &files_dir, stream).await?;

        if let Some(resp) = response {
            resp.write_to(stream).await.context("writing response")?;
        }

        if should_close {
            debug!("closing connection (Connection: close)");
            break;
        }
    }

    Ok(())
}

/// Routes the request to the matching handler.
///
/// Returns `Some(Response)` for simple responses that should be written in full,
/// or `None` when the handler has already written directly to the stream (e.g. file streaming).
async fn route(
    request: &Request,
    files_dir: &Path,
    stream: &mut TcpStream,
) -> Result<Option<Response>> {
    if request.path == "/" {
        Ok(Some(handle_root()))
    } else if let Some(suffix) = request.path.strip_prefix("/echo/") {
        Ok(Some(handle_echo(suffix)))
    } else if request.path.starts_with("/user-agent") {
        Ok(Some(handle_user_agent(request)))
    } else if let Some(filename) = request.path.strip_prefix("/files/") {
        handle_files(filename, files_dir, stream, request).await
    } else {
        debug!("unknown path: {}", request.path);
        Ok(Some(Response::not_found()))
    }
}

// ---------------------------------------------------------------------------
// Individual route handlers
// ---------------------------------------------------------------------------

fn handle_root() -> Response {
    debug!("root path requested");
    Response::ok_text("")
}

fn handle_echo(echoed: &str) -> Response {
    debug!("echo path requested: {}", echoed);
    Response::ok_text(echoed)
}

fn handle_user_agent(request: &Request) -> Response {
    match request.header_value("User-Agent") {
        Some(ua) => Response::ok_text(ua),
        None => {
            error!("User-Agent header not found in request: {:?}", request);
            Response::not_found()
        }
    }
}

/// Serves a file from `files_dir`. Streams the body directly to `stream` so
/// that the entire file doesn't have to be buffered in memory.
///
/// Returns `Ok(None)` on success (response already written), or
/// `Ok(Some(Response))` for error responses that the caller should write.
async fn handle_files(
    filename: &str,
    files_dir: &Path,
    stream: &mut TcpStream,
    request: &Request,
) -> Result<Option<Response>> {
    if !is_valid_single_filename(filename) {
        return Ok(Some(Response::not_found()));
    }

    let file_path = files_dir.join(filename);

    match request.method.as_str() {
        "GET" => handle_file_get(&file_path, filename, stream).await,
        "POST" => handle_file_post(&file_path, request).await,
        _ => Ok(Some(Response::not_found())),
    }
}

async fn handle_file_get(
    file_path: &Path,
    filename: &str,
    stream: &mut TcpStream,
) -> Result<Option<Response>> {
    let meta = match fs::metadata(&file_path).await {
        Ok(m) if m.is_file() => m,
        _ => return Ok(Some(Response::not_found())),
    };

    let mut file = fs::File::open(&file_path).await.context("opening file")?;

    let mut resp = Response::new(200, "OK");
    resp.header("Content-Type", "application/octet-stream")
        .header("Content-Length", &meta.len().to_string());

    resp.write_headers(stream)
        .await
        .context("writing file headers")?;

    let bytes_copied = io::copy(&mut file, stream)
        .await
        .context("streaming file")?;

    stream.flush().await?;
    debug!("streamed {} bytes for file {}", bytes_copied, filename);

    Ok(None)
}

/// POST /files/{filename} — create/overwrite a file with the request body.
async fn handle_file_post(file_path: &Path, request: &Request) -> Result<Option<Response>> {
    let body = request.body.as_deref().unwrap_or_default();

    fs::write(file_path, body)
        .await
        .context("writing file to disk")?;

    debug!("created file {:?} ({} bytes)", file_path, body.len());

    Ok(Some(Response::created()))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `true` when `name` is exactly one normal path component
/// (no separators, no `..`, no absolute prefix).
fn is_valid_single_filename(name: &str) -> bool {
    let path = Path::new(name);
    let mut components = path.components();

    let is_normal = matches!(components.next(), Some(Component::Normal(_)));
    let is_single = components.next().is_none();

    is_normal && is_single
}

/// Spin up a server that handles a full persistent connection
/// (multiple requests on the same TCP stream), then returns the
/// address to connect to.
async fn persistent_server(files_dir: PathBuf) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let dir = Arc::new(files_dir);

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        handle_request(stream, dir).await.unwrap();
    });

    addr
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // ── Helper ───────────────────────────────────────────────────────

    /// Send a raw HTTP request through a real TCP connection and return
    /// the full response bytes.
    async fn send_raw_request(addr: std::net::SocketAddr, raw_request: &[u8]) -> Vec<u8> {
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(raw_request).await.unwrap();
        client.shutdown().await.unwrap();

        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.unwrap();
        buf
    }

    /// Spin up a one-shot server that handles exactly one request,
    /// returning the address to connect to.
    async fn one_shot_server(files_dir: PathBuf) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = Arc::new(files_dir);

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_request(stream, dir).await.unwrap();
        });

        addr
    }

    // ── is_valid_single_filename ─────────────────────────────────────

    #[test]
    fn test_valid_filename_simple() {
        assert!(is_valid_single_filename("hello.txt"));
    }

    #[test]
    fn test_valid_filename_no_extension() {
        assert!(is_valid_single_filename("README"));
    }

    #[test]
    fn test_valid_filename_dotfile() {
        assert!(is_valid_single_filename(".gitignore"));
    }

    #[test]
    fn test_valid_filename_with_spaces() {
        assert!(is_valid_single_filename("my file.txt"));
    }

    #[test]
    fn test_invalid_filename_empty() {
        assert!(!is_valid_single_filename(""));
    }

    #[test]
    fn test_invalid_filename_dot_dot() {
        assert!(!is_valid_single_filename(".."));
    }

    #[test]
    fn test_invalid_filename_traversal() {
        assert!(!is_valid_single_filename("../etc/passwd"));
    }

    #[test]
    fn test_invalid_filename_nested_path() {
        assert!(!is_valid_single_filename("a/b"));
    }

    #[test]
    fn test_invalid_filename_nested_deep() {
        assert!(!is_valid_single_filename("a/b/c.txt"));
    }

    #[test]
    fn test_invalid_filename_absolute() {
        assert!(!is_valid_single_filename("/etc/passwd"));
    }

    #[test]
    fn test_invalid_filename_dot() {
        assert!(!is_valid_single_filename("."));
    }

    #[test]
    fn test_trailing_slash_normalized_by_os() {
        // On Unix, Path::new("dir/").components() normalizes away the trailing
        // slash, yielding a single Normal("dir") component.  This is fine —
        // the downstream `meta.is_file()` check rejects directories anyway.
        assert!(is_valid_single_filename("dir/"));
    }

    // ── handle_root ──────────────────────────────────────────────────

    #[test]
    fn test_handle_root_returns_200() {
        let resp = handle_root();
        let raw = String::from_utf8(resp.build_raw()).unwrap();
        assert!(raw.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(raw.contains("Content-Type: text/plain\r\n"));
        assert!(raw.contains("Content-Length: 0\r\n"));
    }

    // ── handle_echo ──────────────────────────────────────────────────

    #[test]
    fn test_handle_echo_returns_body() {
        let resp = handle_echo("hello-world");
        let raw = String::from_utf8(resp.build_raw()).unwrap();
        assert!(raw.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(raw.contains("Content-Type: text/plain\r\n"));
        assert!(raw.contains("Content-Length: 11\r\n"));
        assert!(raw.ends_with("\r\n\r\nhello-world"));
    }

    #[test]
    fn test_handle_echo_empty_string() {
        let resp = handle_echo("");
        let raw = String::from_utf8(resp.build_raw()).unwrap();
        assert!(raw.contains("Content-Length: 0\r\n"));
        assert!(raw.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_handle_echo_special_characters() {
        let resp = handle_echo("hello world & foo=bar");
        let raw = String::from_utf8(resp.build_raw()).unwrap();
        assert!(raw.ends_with("\r\n\r\nhello world & foo=bar"));
    }

    // ── handle_user_agent ────────────────────────────────────────────

    #[test]
    fn test_handle_user_agent_present() {
        let req = Request {
            method: "GET".to_string(),
            path: "/user-agent".to_string(),
            http_version: "HTTP/1.1".to_string(),
            headers: vec![("User-Agent".into(), "curl/7.64.1".into())],
            body: None,
            peer_addr: None,
        };
        let resp = handle_user_agent(&req);
        let raw = String::from_utf8(resp.build_raw()).unwrap();
        assert!(raw.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(raw.ends_with("\r\n\r\ncurl/7.64.1"));
    }

    #[test]
    fn test_handle_user_agent_case_insensitive() {
        let req = Request {
            method: "GET".to_string(),
            path: "/user-agent".to_string(),
            http_version: "HTTP/1.1".to_string(),
            headers: vec![("user-agent".into(), "MyBot/2.0".into())],
            body: None,
            peer_addr: None,
        };
        let resp = handle_user_agent(&req);
        let raw = String::from_utf8(resp.build_raw()).unwrap();
        assert!(raw.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(raw.ends_with("\r\n\r\nMyBot/2.0"));
    }

    #[test]
    fn test_handle_user_agent_missing() {
        let req = Request {
            method: "GET".to_string(),
            path: "/user-agent".to_string(),
            http_version: "HTTP/1.1".to_string(),
            headers: vec![],
            body: None,
            peer_addr: None,
        };
        let resp = handle_user_agent(&req);
        let raw = String::from_utf8(resp.build_raw()).unwrap();
        assert!(raw.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }

    // ── Integration: route (through handle_request + real TCP) ───────

    #[tokio::test]
    async fn test_integration_get_root() {
        let addr = one_shot_server(PathBuf::from("/tmp")).await;
        let resp = send_raw_request(addr, b"GET / HTTP/1.1\r\nHost: test\r\n\r\n").await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Length: 0\r\n"));
    }

    #[tokio::test]
    async fn test_integration_get_echo() {
        let addr = one_shot_server(PathBuf::from("/tmp")).await;
        let resp = send_raw_request(addr, b"GET /echo/foobar HTTP/1.1\r\nHost: test\r\n\r\n").await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: text/plain\r\n"));
        assert!(text.ends_with("foobar"));
    }

    #[tokio::test]
    async fn test_integration_get_user_agent() {
        let addr = one_shot_server(PathBuf::from("/tmp")).await;
        let resp = send_raw_request(
            addr,
            b"GET /user-agent HTTP/1.1\r\nHost: test\r\nUser-Agent: TestAgent/1.0\r\n\r\n",
        )
        .await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.ends_with("TestAgent/1.0"));
    }

    #[tokio::test]
    async fn test_integration_unknown_path_returns_404() {
        let addr = one_shot_server(PathBuf::from("/tmp")).await;
        let resp = send_raw_request(addr, b"GET /nonexistent HTTP/1.1\r\nHost: test\r\n\r\n").await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }

    #[tokio::test]
    async fn test_integration_file_serving() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("testfile.txt");
        {
            let mut f = std::fs::File::create(&file_path).unwrap();
            f.write_all(b"file contents here").unwrap();
        }

        let addr = one_shot_server(tmp.path().to_path_buf()).await;
        let resp = send_raw_request(
            addr,
            b"GET /files/testfile.txt HTTP/1.1\r\nHost: test\r\n\r\n",
        )
        .await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: application/octet-stream\r\n"));
        assert!(text.contains("Content-Length: 18\r\n"));
        assert!(text.ends_with("file contents here"));
    }

    #[tokio::test]
    async fn test_integration_file_not_found() {
        let tmp = tempfile::tempdir().unwrap();

        let addr = one_shot_server(tmp.path().to_path_buf()).await;
        let resp = send_raw_request(
            addr,
            b"GET /files/does_not_exist.txt HTTP/1.1\r\nHost: test\r\n\r\n",
        )
        .await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }

    #[tokio::test]
    async fn test_integration_file_traversal_rejected() {
        let tmp = tempfile::tempdir().unwrap();

        let addr = one_shot_server(tmp.path().to_path_buf()).await;
        let resp = send_raw_request(
            addr,
            b"GET /files/../etc/passwd HTTP/1.1\r\nHost: test\r\n\r\n",
        )
        .await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }

    #[tokio::test]
    async fn test_integration_file_nested_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        // Create a nested file that should NOT be reachable
        std::fs::create_dir_all(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/secret.txt"), "secret").unwrap();

        let addr = one_shot_server(tmp.path().to_path_buf()).await;
        let resp = send_raw_request(
            addr,
            b"GET /files/sub/secret.txt HTTP/1.1\r\nHost: test\r\n\r\n",
        )
        .await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }

    #[tokio::test]
    async fn test_integration_file_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("empty.dat"), b"").unwrap();

        let addr = one_shot_server(tmp.path().to_path_buf()).await;
        let resp =
            send_raw_request(addr, b"GET /files/empty.dat HTTP/1.1\r\nHost: test\r\n\r\n").await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Length: 0\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[tokio::test]
    async fn test_integration_file_binary_content() {
        let tmp = tempfile::tempdir().unwrap();
        let binary: Vec<u8> = vec![0x00, 0x01, 0xFF, 0xFE, 0x0A, 0x0D];
        std::fs::write(tmp.path().join("bin.dat"), &binary).unwrap();

        let addr = one_shot_server(tmp.path().to_path_buf()).await;
        let resp =
            send_raw_request(addr, b"GET /files/bin.dat HTTP/1.1\r\nHost: test\r\n\r\n").await;

        // Find end of headers
        let header_end = b"\r\n\r\n";
        let pos = resp
            .windows(header_end.len())
            .position(|w| w == header_end)
            .expect("should have header terminator");
        let body = &resp[pos + header_end.len()..];

        assert_eq!(body, &binary);
    }

    // ── Integration: POST /files ─────────────────────────────────────

    #[tokio::test]
    async fn test_integration_post_file_creates_file() {
        let tmp = tempfile::tempdir().unwrap();

        let addr = one_shot_server(tmp.path().to_path_buf()).await;
        let body = b"hello file content";
        let req = format!(
            "POST /files/newfile.txt HTTP/1.1\r\n\
             Host: test\r\n\
             Content-Type: application/octet-stream\r\n\
             Content-Length: {}\r\n\
             \r\n\
             {}",
            body.len(),
            std::str::from_utf8(body).unwrap(),
        );
        let resp = send_raw_request(addr, req.as_bytes()).await;
        let text = String::from_utf8(resp).unwrap();

        assert!(
            text.starts_with("HTTP/1.1 201 Created\r\n"),
            "expected 201, got: {}",
            text
        );

        // Verify the file was actually created with correct content
        let written = std::fs::read(tmp.path().join("newfile.txt")).unwrap();
        assert_eq!(written, body);
    }

    #[tokio::test]
    async fn test_integration_post_file_overwrites_existing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("existing.txt"), b"old content").unwrap();

        let addr = one_shot_server(tmp.path().to_path_buf()).await;
        let body = b"new content";
        let req = format!(
            "POST /files/existing.txt HTTP/1.1\r\n\
             Host: test\r\n\
             Content-Type: application/octet-stream\r\n\
             Content-Length: {}\r\n\
             \r\n\
             {}",
            body.len(),
            std::str::from_utf8(body).unwrap(),
        );
        let resp = send_raw_request(addr, req.as_bytes()).await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 201 Created\r\n"));

        let written = std::fs::read(tmp.path().join("existing.txt")).unwrap();
        assert_eq!(written, body);
    }

    #[tokio::test]
    async fn test_integration_post_file_empty_body() {
        let tmp = tempfile::tempdir().unwrap();

        let addr = one_shot_server(tmp.path().to_path_buf()).await;
        let req = b"POST /files/empty.txt HTTP/1.1\r\n\
                     Host: test\r\n\
                     Content-Type: application/octet-stream\r\n\
                     Content-Length: 0\r\n\
                     \r\n";
        let resp = send_raw_request(addr, req).await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 201 Created\r\n"));

        let written = std::fs::read(tmp.path().join("empty.txt")).unwrap();
        assert!(written.is_empty());
    }

    #[tokio::test]
    async fn test_integration_post_file_traversal_rejected() {
        let tmp = tempfile::tempdir().unwrap();

        let addr = one_shot_server(tmp.path().to_path_buf()).await;
        let req = b"POST /files/../evil.txt HTTP/1.1\r\n\
                     Host: test\r\n\
                     Content-Type: application/octet-stream\r\n\
                     Content-Length: 4\r\n\
                     \r\n\
                     evil";
        let resp = send_raw_request(addr, req).await;
        let text = String::from_utf8(resp).unwrap();

        assert!(text.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }

    // ── Integration: persistent connections ──────────────────────────

    #[tokio::test]
    async fn test_persistent_two_requests_same_connection() {
        let addr = persistent_server(PathBuf::from("/tmp")).await;

        let mut client = TcpStream::connect(addr).await.unwrap();

        // First request: GET /echo/banana
        client
            .write_all(b"GET /echo/banana HTTP/1.1\r\nHost: test\r\n\r\n")
            .await
            .unwrap();

        let mut buf = vec![0u8; 4096];
        let n = client.read(&mut buf).await.unwrap();
        let resp1 = String::from_utf8_lossy(&buf[..n]);
        assert!(resp1.starts_with("HTTP/1.1 200 OK\r\n"), "resp1: {}", resp1);
        assert!(resp1.contains("banana"), "resp1 body missing: {}", resp1);

        // Second request on the same connection: GET /echo/apple
        client
            .write_all(b"GET /echo/apple HTTP/1.1\r\nHost: test\r\n\r\n")
            .await
            .unwrap();

        let n = client.read(&mut buf).await.unwrap();
        let resp2 = String::from_utf8_lossy(&buf[..n]);
        assert!(resp2.starts_with("HTTP/1.1 200 OK\r\n"), "resp2: {}", resp2);
        assert!(resp2.contains("apple"), "resp2 body missing: {}", resp2);

        // Close the connection
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_persistent_echo_then_user_agent() {
        let addr = persistent_server(PathBuf::from("/tmp")).await;

        let mut client = TcpStream::connect(addr).await.unwrap();

        // First request: GET /echo/banana
        client
            .write_all(b"GET /echo/banana HTTP/1.1\r\nHost: test\r\n\r\n")
            .await
            .unwrap();

        let mut buf = vec![0u8; 4096];
        let n = client.read(&mut buf).await.unwrap();
        let resp1 = String::from_utf8_lossy(&buf[..n]);
        assert!(resp1.starts_with("HTTP/1.1 200 OK\r\n"), "resp1: {}", resp1);
        assert!(resp1.contains("banana"), "resp1 body missing: {}", resp1);

        // Second request: GET /user-agent
        client
            .write_all(
                b"GET /user-agent HTTP/1.1\r\nHost: test\r\nUser-Agent: blueberry/apple\r\n\r\n",
            )
            .await
            .unwrap();

        let n = client.read(&mut buf).await.unwrap();
        let resp2 = String::from_utf8_lossy(&buf[..n]);
        assert!(resp2.starts_with("HTTP/1.1 200 OK\r\n"), "resp2: {}", resp2);
        assert!(
            resp2.contains("blueberry/apple"),
            "resp2 body missing: {}",
            resp2
        );

        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_persistent_connection_close_header() {
        let addr = persistent_server(PathBuf::from("/tmp")).await;

        let mut client = TcpStream::connect(addr).await.unwrap();

        // Send a request with Connection: close
        client
            .write_all(b"GET /echo/done HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();

        // Read the response
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.unwrap();
        let resp = String::from_utf8_lossy(&buf);
        assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"), "resp: {}", resp);
        assert!(resp.contains("done"), "resp body missing: {}", resp);

        // The server should have closed its side, so read_to_end returned.
        // (If the server didn't close, read_to_end would hang.)
    }

    #[tokio::test]
    async fn test_persistent_client_closes_after_first_request() {
        let addr = persistent_server(PathBuf::from("/tmp")).await;

        let mut client = TcpStream::connect(addr).await.unwrap();

        // Send one request, then close
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: test\r\n\r\n")
            .await
            .unwrap();

        let mut buf = vec![0u8; 4096];
        let n = client.read(&mut buf).await.unwrap();
        assert!(n > 0);

        // Close the client side — the server loop should exit cleanly
        client.shutdown().await.unwrap();
    }
}
