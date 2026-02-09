use std::{
    io::{self, Read},
    net::{TcpListener, TcpStream},
};

/// End user
struct Client;

/// Computer hosting the web app
struct Server {
    connection: TcpListener,
}

enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl Server {
    fn new(addr: &str) -> Server {
        let listener = TcpListener::bind(addr).unwrap();

        Server {
            connection: listener,
        }
    }
}

/// Sent from the Client
struct Request {
    // version: // 0.9, 1.0, 1.1, 2.0
    /// Represents matching routes to things that
    /// our server might know about
    resource: String,

    ///
    method: HttpMethod,

    headers: std::collections::HashMap<String, Vec<String>>,

    body: String,
}

fn read_header_line(stream: &TcpStream) -> io::Result<String> {
    let mut buf = Vec::with_capacity(0x1000);

    while let Some(Ok(byte)) = stream.bytes().next() {
        if byte == b'\n' {
            let header_line = String::from_utf8(buf)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Not an HTTP header"))?;
            return Ok(header_line);
        }

        buf.push(byte);
    }

    Err(io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "client aborted early",
    ))
}

impl Request {
    fn new(stream: TcpStream) -> io::Result<Request> {
        // GET / HTTP/1.1
        let http_metadata = read_header_line(&stream)?;

        todo!()
    }
}

/// Sent from the Server
struct Response;

fn main() {
    let server = Server::new("0.0.0.0:8080");

    for _stream in server.connection.incoming().flatten() {}
}
