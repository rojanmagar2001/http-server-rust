use anyhow::Result;
use log::{debug, error, info};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
};

const SUCCESS_RESPONSE: &[u8] = "HTTP/1.1 200 OK\r\n\r\n".as_bytes();
const ERROR_RESPONSE: &[u8] = "HTTP/1.1 404 Not Found\r\n\r\n".as_bytes();

type Key = String;
type Value = String;

fn main() -> Result<()> {
    env_logger::init();
    info!("Server started");

    let listener = TcpListener::bind("127.0.0.1:4221").unwrap();

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                debug!("accepted new connection");

                let mut request_buffer = BufReader::new(&stream);
                let mut request_line = String::new();

                request_buffer.read_line(&mut request_line)?;

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

                let path: Vec<&str> = request_line.split_whitespace().collect();
                match path[..] {
                    ["GET", path, "HTTP/1.1"] => {
                        if path == "/" {
                            debug!("root path requested");
                            stream.write(SUCCESS_RESPONSE)?;
                            stream.flush()?;
                        } else if path.starts_with("/user-agent") {
                            let mut user_agent = None;
                            for (key, value) in headers {
                                if key == "User-Agent" {
                                    user_agent = Some(value);
                                    break;
                                }
                            }

                            match user_agent {
                                None => {
                                    error!(
                                        "User-Agent header not found in request. Request: {request_line}"
                                    );
                                    stream.write(ERROR_RESPONSE)?;
                                    stream.flush()?;
                                }
                                Some(user_agent) => {
                                    let response = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                                        user_agent.len(),
                                        user_agent,
                                    );
                                    stream.write(response.as_bytes())?;
                                    stream.flush()?;
                                }
                            }
                        } else if path.starts_with("/echo/") {
                            let echo_path = path.split_once("/echo/");
                            match echo_path {
                                Some((_, path)) => {
                                    debug!("echo path requested: {path}");
                                    let response = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                                        path.len(),
                                        path
                                    );
                                    stream.write(response.as_bytes())?;
                                    stream.flush()?;
                                }
                                _ => {
                                    error!(
                                        "Invalid echo path in request. Request: `{request_line}`"
                                    );
                                    stream.write(ERROR_RESPONSE)?;
                                    stream.flush()?;
                                }
                            }
                        } else {
                            debug!("unknown path: {path}");
                            stream.write(ERROR_RESPONSE)?;
                            stream.flush()?;
                        }
                    }
                    _ => {
                        error!("Invalid path in request. Input: `{request_line}`");
                        stream.write(ERROR_RESPONSE)?;
                    }
                }
            }
            Err(e) => {
                error!("error: {}", e);
            }
        }
    }

    Ok(())
}
