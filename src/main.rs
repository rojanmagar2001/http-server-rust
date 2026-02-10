use anyhow::{Result, bail};
use log::{debug, error, info};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
};

const SUCCESS_RESPONSE: &str = "HTTP/1.1 200 OK\r\n\r\n";
const ERROR_RESPONSE: &str = "HTTP/1.1 404 Not Found\r\n\r\n";

type Key = String;
type Value = String;

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    http_version: String,
    headers: Vec<(Key, Value)>,
    body: Option<String>,
}

impl TryFrom<TcpStream> for Request {
    type Error = anyhow::Error;
    fn try_from(stream: TcpStream) -> Result<Self> {
        let mut request_buffer = BufReader::new(&stream);
        let mut request_line = String::new();

        request_buffer.read_line(&mut request_line)?;

        let request_line: Vec<&str> = request_line.split_whitespace().collect();

        let (method, path, http_version) = match request_line[..] {
            [method, path, http_version] => {
                debug!("Correct format found: `{}`", request_line.join(" "));
                (
                    method.to_string(),
                    path.to_string(),
                    http_version.to_string(),
                )
            }
            _ => {
                bail!("Invalid request line: `{}`", request_line.join(" "));
            }
        };

        let mut headers: Vec<(Key, Value)> = Vec::new();

        loop {
            let mut header_line = String::new();
            let next_header = request_buffer.read_line(&mut header_line)?;
            if header_line == "\r\n" || next_header == 0 {
                break;
            }
            if let Some((key, value)) = header_line.split_once(": ") {
                headers.push((key.to_string(), value.trim_end().to_string()));
            } else {
                error!("Invalid header line in request. Request: `{header_line}`");
            }
        }

        Ok(Request {
            method,
            path,
            http_version,
            headers,
            body: None,
        })
    }
}

fn handle_request(mut stream: TcpStream) -> Result<()> {
    debug!("accepted new connection");

    let request = Request::try_from(stream.try_clone()?)?;

    let response = if request.path == "/" {
        debug!("root path requested");
        SUCCESS_RESPONSE.to_string()
    } else if request.path.starts_with("/user-agent") {
        let mut user_agent = None;
        for (key, value) in &request.headers {
            if key == "User-Agent" {
                user_agent = Some(value);
                break;
            }
        }

        match user_agent {
            None => {
                error!("User-Agent header not found in request. Request: {request:?}",);
                ERROR_RESPONSE.to_string()
            }
            Some(user_agent) => {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                    user_agent.len(),
                    user_agent,
                );
                response
            }
        }
    } else if request.path.starts_with("/echo/") {
        let echo_path = request.path.split_once("/echo/");
        match echo_path {
            Some((_, path)) => {
                debug!("echo path requested: {path}");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                    path.len(),
                    path
                );
                response
            }
            _ => {
                error!("Invalid echo path in request. Request: `{:?}`", request);
                ERROR_RESPONSE.to_string()
            }
        }
    } else {
        debug!("unknown path: {}", request.path);
        ERROR_RESPONSE.to_string()
    };

    stream.write(response.as_bytes())?;
    stream.flush()?;

    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();
    info!("Server started");

    let listener = TcpListener::bind("127.0.0.1:4221").unwrap();

    let mut response_handles = Vec::new();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let handle = std::thread::spawn(|| handle_request(stream));
                response_handles.push(handle);
            }
            Err(e) => {
                error!("error: {}", e);
            }
        }
    }

    for handle in response_handles {
        handle.join().unwrap()?;
    }

    Ok(())
}
