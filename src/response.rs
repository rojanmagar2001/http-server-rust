use anyhow::Result;
use tokio::{io::AsyncWriteExt, net::TcpStream};

#[derive(Debug)]
pub struct Response {
    status_code: u16,
    reason: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    pub fn new(status_code: u16, reason: &str) -> Self {
        Self {
            status_code,
            reason: reason.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn ok_text(body: &str) -> Self {
        let mut r = Response::new(200, "OK");
        r.header("Content-Type", "text/plain");
        r.body_bytes(body.as_bytes().to_vec());
        r
    }

    pub fn not_found() -> Self {
        let mut r = Response::new(404, "Not Found");
        r.header("Content-Type", "text/plain");
        r.body_bytes(b"Not Found".to_vec());
        r
    }

    pub fn header(&mut self, key: &str, value: &str) {
        self.headers.push((key.to_string(), value.to_string()));
    }

    pub fn body_bytes(&mut self, bytes: Vec<u8>) {
        self.body = bytes;
    }

    fn build_raw(&self) -> Vec<u8> {
        let mut raw = Vec::new();

        // Status_line
        raw.extend_from_slice(
            format!("HTTP/1.1 {} {} \r\n", self.status_code, self.reason).as_bytes(),
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

    /// Asynchronously write the response to the stream
    pub async fn write_to(&self, stream: &mut TcpStream) -> Result<()> {
        let raw = self.build_raw();
        stream.write_all(&raw).await?;
        stream.flush().await?;
        Ok(())
    }
}
