use super::Error;
use base64;
use rand::Rng;
use sha1::{Digest, Sha1};
use std::str;
use tokio;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::prelude::*;

pub async fn upgrade(stream: TcpStream) -> Result<Upgrade, Error> {
    let (mut ws_stream, header) = parse_http_headers(stream).await?;
    if header.is_ws_upgrade() {
        ws_stream.write_all(header.upgrade_response().as_bytes()).await?;
        return Ok(Upgrade {
            stream: ws_stream,
            deflate_supported: header.is_deflate_supported(),
            client: false,
        });
    }
    const BAD_REQUEST_HTTP_RESPONSE: &[u8] = "HTTP/1.1 400 Bad Request\r\n\r\n".as_bytes();
    ws_stream.write_all(BAD_REQUEST_HTTP_RESPONSE).await?;
    Err(Error::InvalidUpgradeRequest)
}

#[derive(Debug)]
pub struct Upgrade {
    pub stream: TcpStream,
    pub deflate_supported: bool,
    pub client: bool,
}

#[derive(Debug)]
struct Header {
    connection: String,
    upgrade: String,
    version: String,
    key: String,
    extensions: String,
    accept: String,
}

fn split_header_line(line: &str) -> Option<(&str, &str)> {
    let mut splitter = line.splitn(2, ':');
    let first = splitter.next()?;
    let second = splitter.next()?;
    Some((first, second.trim()))
}

impl Header {
    fn new() -> Header {
        Header {
            connection: String::new(),
            upgrade: String::new(),
            version: String::new(),
            key: String::new(),
            extensions: String::new(),
            accept: String::new(),
        }
    }
    fn parse(&mut self, line: &str) {
        if let Some((key, value)) = split_header_line(&line) {
            match key {
                "Connection" => self.connection = value.to_string(),
                "Upgrade" => self.upgrade = value.to_string(),
                "Sec-WebSocket-Version" => self.version = value.to_string(),
                "Sec-WebSocket-Key" => self.key = value.to_string(),
                "Sec-WebSocket-Extensions" => self.add_extensions(value),
                "Sec-WebSocket-Accept" => self.accept = value.to_string(),
                //"Host"
                //"Origin"
                //_ => println!("other header: '{}' => '{}'", key, value),
                _ => (),
            }
        }
    }
    fn add_extensions(&mut self, ex: &str) {
        if !self.extensions.is_empty() {
            self.extensions.push_str(", ");
        }
        self.extensions.push_str(ex);
    }
    fn is_ws_upgrade(&self) -> bool {
        self.connection == "Upgrade"
            && (self.upgrade == "websocket" || self.upgrade == "WebSocket")
            && self.version == "13"
            && self.key.len() > 0
    }
    fn is_deflate_supported(&self) -> bool {
        self.extensions.contains("permessage-deflate")
    }
    fn upgrade_response(&self) -> String {
        const HEADER: &str = "HTTP/1.1 101 Switching Protocols\r\n\
            Upgrade: websocket\r\n\
            Server: yarws\r\n\
            Connection: Upgrade\r\n\
            Sec-WebSocket-Accept: ";
        let mut s = HEADER.to_string();
        s.push_str(&ws_accept(&self.key));
        s.push_str(&"\r\n");
        if self.is_deflate_supported() {
            s.push_str(
                "Sec-WebSocket-Extensions: permessage-deflate;client_no_context_takeover;server_no_context_takeover",
            );
            s.push_str(&"\r\n");
        }
        s.push_str(&"\r\n");
        s
    }
    fn is_ws_connect(&self, key: &str) -> bool {
        let accept = ws_accept(key);
        self.connection == "Upgrade"
            && (self.upgrade == "websocket" || self.upgrade == "WebSocket")
            && self.accept == accept
    }
}

fn ws_accept(key: &str) -> String {
    const WS_MAGIC_KEY: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut hasher = Sha1::new();
    let s = key.to_string() + WS_MAGIC_KEY;

    hasher.input(s.as_bytes());
    let hr = hasher.result();
    base64::encode(&hr)
}

pub async fn connect(mut stream: TcpStream, host: &str, path: &str) -> Result<Upgrade, Error> {
    let key = connect_key();
    stream.write_all(connect_header(host, path, &key).as_bytes()).await?;

    let (ws_stream, header) = parse_http_headers(stream).await?;
    if header.is_ws_connect(&key) {
        return Ok(Upgrade {
            stream: ws_stream,
            deflate_supported: header.is_deflate_supported(),
            client: true,
        });
    }
    Err(Error::InvalidUpgradeRequest)
}

async fn parse_http_headers(mut stream: TcpStream) -> Result<(TcpStream, Header), Error> {
    let mut header = Header::new();
    let mut line: Vec<u8> = Vec::new();
    loop {
        let byte = stream.read_u8().await?;
        if byte == 13 {
            continue;
        }
        if byte == 10 {
            if line.len() == 0 {
                break;
            }
            header.parse(&str::from_utf8(&line)?);
            line = Vec::new();
            continue;
        }
        line.push(byte);
    }
    Ok((stream, header))
}

fn connect_header(host: &str, path: &str, key: &str) -> String {
    let mut h = "GET ".to_owned()
        + path
        + " HTTP/1.1\r\n\
Connection: Upgrade\r\n\
Upgrade: websocket\r\n\
Sec-WebSocket-Version: 13\r\n\
Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits\r\n\
Sec-WebSocket-Key: ";
    h.push_str(key);
    h.push_str("\r\n");
    h.push_str("Host: ");
    h.push_str(host);
    h.push_str("\r\n\r\n");
    h
}

fn connect_key() -> String {
    let buf = rand::thread_rng().gen::<[u8; 16]>();
    base64::encode(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sha1_general() {
        let mut hasher = Sha1::new();
        hasher.input(b"hello world");
        let result = hasher.result();
        assert_eq!(result[..], hex!("2aae6c35c94fcfb415dbe95f408b9ce91ee846ed"));
    }
    #[test]
    fn test_ws_accept() {
        // example from the ws rfc: https://tools.ietf.org/html/rfc6455
        /* NOTE: As an example, if the value of the |Sec-WebSocket-Key| header
        field in the client's handshake were "dGhlIHNhbXBsZSBub25jZQ==", the
        server would append the string "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
        to form the string "dGhlIHNhbXBsZSBub25jZQ==258EAFA5-E914-47DA-95CA-
        C5AB0DC85B11".  The server would then take the SHA-1 hash of this
        string, giving the value 0xb3 0x7a 0x4f 0x2c 0xc0 0x62 0x4f 0x16 0x90
        0xf6 0x46 0x06 0xcf 0x38 0x59 0x45 0xb2 0xbe 0xc4 0xea.  This value
        is then base64-encoded, to give the value
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=", which would be returned in the
        |Sec-WebSocket-Accept| header field.
        */
        let acc = ws_accept("dGhlIHNhbXBsZSBub25jZQ==");
        assert_eq!(acc, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn ws_key() {
        let k = connect_key();
        println!("key {}", k);
        //println!("{}", connect_header(&k, "minus5.hr"));
    }
}
